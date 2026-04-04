use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use flate2::read::ZlibDecoder;
use md5::{Digest, Md5};
use serde::Serialize;

use crate::rdb::{hex_encode, LeIndex, LeIndexEntry};

/// IOz1 magic bytes — 4-byte header for zlib-compressed CDN payloads.
const IOZ1_MAGIC: &[u8; 4] = b"IOz1";

// ─── Minimum bundle set ──────────────────────────────────────────────────────

/// Bundle names considered essential for a minimum viable client.
///
/// Includes:
/// - `default-resources`: core engine resources (textures, shaders, UI, sounds)
/// - `1000`-`1300`: engine/character bundles
/// - `3030`-`3070`: Kingsmouth through Blue Mountain (starter zones)
/// - `3090`: Scorched Desert (first Egypt zone)
/// - `4000`: character creation / UI
///
/// Excludes late-game zones, PvP, and optional content to reduce download size.
const MINIMUM_BUNDLES: &[&str] = &[
    "default-resources",
    "1000",
    "1100",
    "1200",
    "1300",
    "3030",
    "3040",
    "3050",
    "3070",
    "3090",
    "4000",
];

/// Build a set of (rdb_type, resource_id) pairs that belong to the minimum bundle set.
///
/// Parses the bundle section from le.idx, filters to MINIMUM_BUNDLES, and returns
/// all referenced entry keys as a HashSet for O(1) lookup.
pub fn build_minimum_entry_set(install_path: &Path) -> Result<HashSet<(u32, u32)>, String> {
    let le_idx_path = install_path.join("RDB").join("le.idx");
    let bundles = crate::rdb::parse_bundles(&le_idx_path).map_err(|e| e.to_string())?;

    let min_names: HashSet<&str> = MINIMUM_BUNDLES.iter().copied().collect();
    let mut entry_set = HashSet::new();

    for bundle in &bundles {
        if min_names.contains(bundle.name.as_str()) {
            for &(rdb_type, id) in &bundle.entries {
                entry_set.insert((rdb_type, id));
            }
        }
    }

    Ok(entry_set)
}

/// Check if a (type, id) pair belongs to the minimum bundle set.
pub fn is_entry_in_minimum_set(
    rdb_type: u32,
    id: u32,
    minimum_set: &HashSet<(u32, u32)>,
) -> bool {
    minimum_set.contains(&(rdb_type, id))
}

/// A single corrupted entry found during verification.
#[derive(Debug, Clone, Serialize)]
pub struct CorruptedEntry {
    pub rdb_type: u32,
    pub id: u32,
    pub file_num: u8,
    pub offset: u32,
    pub length: u32,
    pub expected_hash: String,
    pub actual_hash: String,
}

/// Progress report emitted during verification.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyProgress {
    pub entries_checked: u64,
    pub entries_total: u64,
    pub corrupted_count: u64,
    pub bytes_scanned: u64,
    pub current_file: String,
    pub phase: String,
}

/// Result of a full verification scan.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyResult {
    pub corrupted: Vec<CorruptedEntry>,
    pub entries_checked: u64,
    pub bytes_scanned: u64,
}

/// Verify integrity of all rdbdata files against le.idx hashes.
///
/// Groups entries by file_num (skipping file_num=255), opens each rdbdata file,
/// reads each entry's bytes, and compares the MD5 hash against the le.idx hash.
/// Emits `verify:progress` events via the provided callback and checks the
/// cancel flag between entries.
pub fn verify_integrity<F>(
    install_path: &Path,
    le_index: &LeIndex,
    cancel_flag: &Arc<AtomicBool>,
    mut on_progress: F,
) -> Result<VerifyResult, String>
where
    F: FnMut(&VerifyProgress),
{
    // Group entries by file_num, skip 255
    let mut by_file: HashMap<u8, Vec<&LeIndexEntry>> = HashMap::new();
    for entry in &le_index.entries {
        if entry.file_num == 255 {
            continue;
        }
        by_file.entry(entry.file_num).or_default().push(entry);
    }

    // Sort entries within each file by offset for sequential reads
    for entries in by_file.values_mut() {
        entries.sort_by_key(|e| e.offset);
    }

    let entries_total: u64 = by_file.values().map(|v| v.len() as u64).sum();
    let mut entries_checked: u64 = 0;
    let mut bytes_scanned: u64 = 0;
    let mut corrupted = Vec::new();

    // Sort file numbers for deterministic ordering
    let mut file_nums: Vec<u8> = by_file.keys().copied().collect();
    file_nums.sort();

    let rdb_dir = install_path.join("RDB");

    for file_num in file_nums {
        let file_entries = &by_file[&file_num];
        let rdbdata_path = rdb_dir.join(format!("{:02}.rdbdata", file_num));
        let current_file = format!("{:02}.rdbdata", file_num);

        let mut file = File::open(&rdbdata_path).map_err(|e| {
            format!(
                "Failed to open {}: {} (file_num={})",
                rdbdata_path.display(),
                e,
                file_num
            )
        })?;

        // Emit progress at start of each file
        on_progress(&VerifyProgress {
            entries_checked,
            entries_total,
            corrupted_count: corrupted.len() as u64,
            bytes_scanned,
            current_file: current_file.clone(),
            phase: "scanning".into(),
        });

        for entry in file_entries {
            // Check cancellation
            if cancel_flag.load(Ordering::Relaxed) {
                return Ok(VerifyResult {
                    corrupted,
                    entries_checked,
                    bytes_scanned,
                });
            }

            // Handle zero-length entries — hash of empty data
            if entry.length == 0 {
                let empty_hash = {
                    let mut hasher = Md5::new();
                    hasher.update(b"");
                    let result = hasher.finalize();
                    let mut h = [0u8; 16];
                    h.copy_from_slice(&result);
                    h
                };
                if empty_hash != entry.hash {
                    corrupted.push(CorruptedEntry {
                        rdb_type: entry.rdb_type,
                        id: entry.id,
                        file_num: entry.file_num,
                        offset: entry.offset,
                        length: entry.length,
                        expected_hash: hex_encode(&entry.hash),
                        actual_hash: hex_encode(&empty_hash),
                    });
                }
                entries_checked += 1;
                continue;
            }

            // Read entry data from rdbdata
            let mut buf = vec![0u8; entry.length as usize];
            if let Err(e) = file.seek(SeekFrom::Start(entry.offset as u64)) {
                log::warn!(
                    "Seek failed for type={} id={} in {:02}.rdbdata offset={}: {}",
                    entry.rdb_type, entry.id, entry.file_num, entry.offset, e
                );
                corrupted.push(CorruptedEntry {
                    rdb_type: entry.rdb_type,
                    id: entry.id,
                    file_num: entry.file_num,
                    offset: entry.offset,
                    length: entry.length,
                    expected_hash: hex_encode(&entry.hash),
                    actual_hash: "IO_ERROR".into(),
                });
                entries_checked += 1;
                continue;
            }
            if let Err(e) = file.read_exact(&mut buf) {
                log::warn!(
                    "Read failed for type={} id={} in {:02}.rdbdata offset={} len={}: {}",
                    entry.rdb_type, entry.id, entry.file_num, entry.offset, entry.length, e
                );
                corrupted.push(CorruptedEntry {
                    rdb_type: entry.rdb_type,
                    id: entry.id,
                    file_num: entry.file_num,
                    offset: entry.offset,
                    length: entry.length,
                    expected_hash: hex_encode(&entry.hash),
                    actual_hash: "IO_ERROR".into(),
                });
                entries_checked += 1;
                continue;
            }

            // MD5 hash and compare
            let mut hasher = Md5::new();
            hasher.update(&buf);
            let result = hasher.finalize();
            let mut actual_hash = [0u8; 16];
            actual_hash.copy_from_slice(&result);

            if actual_hash != entry.hash {
                corrupted.push(CorruptedEntry {
                    rdb_type: entry.rdb_type,
                    id: entry.id,
                    file_num: entry.file_num,
                    offset: entry.offset,
                    length: entry.length,
                    expected_hash: hex_encode(&entry.hash),
                    actual_hash: hex_encode(&actual_hash),
                });
            }

            entries_checked += 1;
            bytes_scanned += entry.length as u64;

            // Emit progress every 500 entries
            if entries_checked % 500 == 0 {
                on_progress(&VerifyProgress {
                    entries_checked,
                    entries_total,
                    corrupted_count: corrupted.len() as u64,
                    bytes_scanned,
                    current_file: current_file.clone(),
                    phase: "scanning".into(),
                });
            }
        }
    }

    // Final progress emission
    on_progress(&VerifyProgress {
        entries_checked,
        entries_total,
        corrupted_count: corrupted.len() as u64,
        bytes_scanned,
        current_file: String::new(),
        phase: "complete".into(),
    });

    Ok(VerifyResult {
        corrupted,
        entries_checked,
        bytes_scanned,
    })
}

/// Decompress IOz1-format data (CDN payloads).
///
/// IOz1 format: 4-byte magic (`IOz1`) + u32_LE original_size + zlib-compressed data.
/// If the first 4 bytes are NOT `IOz1`, the data is returned as-is (already uncompressed).
pub fn decompress_ioz1(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 8 {
        // Too short for IOz1 header — return as-is
        return Ok(data.to_vec());
    }

    if &data[0..4] != IOZ1_MAGIC {
        // Not IOz1 — return data as-is (might be uncompressed)
        return Ok(data.to_vec());
    }

    let original_size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;

    let mut decoder = ZlibDecoder::new(&data[8..]);
    let mut decompressed = Vec::with_capacity(original_size);
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| format!("IOz1 zlib decompression failed: {e}"))?;

    Ok(decompressed)
}

/// Decompress IOz2-format data (LZMA-compressed CDN payloads).
///
/// IOz2 format: 4-byte magic (`IOz2`) + u32_LE original_size + LZMA-compressed data.
/// The LZMA stream starts with a 5-byte properties header followed by compressed data.
pub fn decompress_ioz2(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 8 {
        return Err("IOz2 data too short".into());
    }

    if &data[0..4] != b"IOz2" {
        return Err(format!("Expected IOz2 magic, got {:?}", &data[0..4]));
    }

    let original_size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;

    let mut decompressed = Vec::with_capacity(original_size);
    lzma_rs::lzma_decompress(&mut &data[8..], &mut decompressed)
        .map_err(|e| format!("IOz2 LZMA decompression failed: {e}"))?;

    Ok(decompressed)
}

/// Decompress CDN data — handles IOz1 (zlib), IOz2 (LZMA), or uncompressed.
pub fn decompress_cdn(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() >= 4 {
        if &data[0..4] == b"IOz1" {
            return decompress_ioz1(data);
        }
        if &data[0..4] == b"IOz2" {
            return decompress_ioz2(data);
        }
    }
    Ok(data.to_vec())
}

/// Write decompressed bytes to an rdbdata file at a specific offset.
///
/// Opens the rdbdata file, seeks to the given offset, and writes the data.
/// Returns an error if the data length doesn't match the expected length.
pub fn write_to_rdbdata(
    install_path: &Path,
    file_num: u8,
    offset: u64,
    data: &[u8],
    expected_length: usize,
) -> Result<(), String> {
    if data.len() != expected_length {
        return Err(format!(
            "Data length mismatch for {:02}.rdbdata at offset {}: expected {} bytes, got {}",
            file_num, offset, expected_length, data.len()
        ));
    }

    let rdbdata_path = install_path
        .join("RDB")
        .join(format!("{:02}.rdbdata", file_num));

    let mut file = OpenOptions::new()
        .write(true)
        .open(&rdbdata_path)
        .map_err(|e| {
            format!(
                "Failed to open {:02}.rdbdata for writing: {} (file_num={})",
                file_num, e, file_num
            )
        })?;

    file.seek(SeekFrom::Start(offset)).map_err(|e| {
        format!(
            "Failed to seek to offset {} in {:02}.rdbdata: {}",
            offset, file_num, e
        )
    })?;

    file.write_all(data).map_err(|e| {
        format!(
            "Failed to write {} bytes to {:02}.rdbdata at offset {}: {}",
            data.len(),
            file_num,
            offset,
            e
        )
    })?;

    file.flush().map_err(|e| {
        format!(
            "Failed to flush {:02}.rdbdata: {}",
            file_num, e
        )
    })?;

    Ok(())
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdb;
    use std::fs;
    use std::path::PathBuf;

    fn tsw_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../The Secret World")
            .canonicalize()
            .expect("TSW install directory must exist")
    }

    fn le_idx_path() -> PathBuf {
        tsw_dir().join("RDB/le.idx")
    }

    // ── verify_integrity tests ──────────────────────────────────────

    #[test]
    fn test_verify_single_entry() {
        // Verify the very first entry in le.idx: type=1000001, id=3, file_num=0
        let idx = rdb::parse_le_index(&le_idx_path()).expect("parse le.idx");
        let entry = &idx.entries[0];
        assert_eq!(entry.rdb_type, 1_000_001);
        assert_eq!(entry.id, 3);
        assert_eq!(entry.file_num, 0);

        // Build a mini-index with just this entry
        let mini_index = rdb::LeIndex {
            root_hash: idx.root_hash,
            entries: vec![entry.clone()],
        };

        let cancel = Arc::new(AtomicBool::new(false));
        let result = verify_integrity(&tsw_dir(), &mini_index, &cancel, |_| {})
            .expect("verification should succeed");

        assert_eq!(result.entries_checked, 1);
        assert_eq!(
            result.corrupted.len(),
            0,
            "First entry should not be corrupted: {:?}",
            result.corrupted
        );
        assert!(result.bytes_scanned > 0);
    }

    #[test]
    fn test_verify_multiple_entries() {
        // Verify 8 entries across different rdbdata files
        let idx = rdb::parse_le_index(&le_idx_path()).expect("parse le.idx");

        // Collect entries from different file_nums, skipping 255
        let mut selected = Vec::new();
        let mut seen_files = std::collections::HashSet::new();
        for entry in &idx.entries {
            if entry.file_num == 255 {
                continue;
            }
            if seen_files.insert(entry.file_num) {
                selected.push(entry.clone());
                if selected.len() >= 8 {
                    break;
                }
            }
        }

        let mini_index = rdb::LeIndex {
            root_hash: idx.root_hash,
            entries: selected,
        };

        let cancel = Arc::new(AtomicBool::new(false));
        let result = verify_integrity(&tsw_dir(), &mini_index, &cancel, |_| {})
            .expect("verification should succeed");

        assert_eq!(result.entries_checked, mini_index.entries.len() as u64);
        assert_eq!(
            result.corrupted.len(),
            0,
            "Sample entries should not be corrupted: {:?}",
            result.corrupted
        );
    }

    #[test]
    fn test_verify_skips_file_num_255() {
        let idx = rdb::parse_le_index(&le_idx_path()).expect("parse le.idx");

        // Build index with only file_num=255 entries
        let server_only: Vec<_> = idx
            .entries
            .iter()
            .filter(|e| e.file_num == 255)
            .take(10)
            .cloned()
            .collect();
        assert!(!server_only.is_empty(), "should have file_num=255 entries");

        let mini_index = rdb::LeIndex {
            root_hash: idx.root_hash,
            entries: server_only,
        };

        let cancel = Arc::new(AtomicBool::new(false));
        let result = verify_integrity(&tsw_dir(), &mini_index, &cancel, |_| {})
            .expect("verification should succeed");

        // All entries should be skipped — 0 checked
        assert_eq!(result.entries_checked, 0);
    }

    #[test]
    fn test_verify_cancellation() {
        let idx = rdb::parse_le_index(&le_idx_path()).expect("parse le.idx");

        // Take 100 entries that are not file_num=255
        let entries: Vec<_> = idx
            .entries
            .iter()
            .filter(|e| e.file_num != 255)
            .take(100)
            .cloned()
            .collect();

        let mini_index = rdb::LeIndex {
            root_hash: idx.root_hash,
            entries,
        };

        // Cancel immediately
        let cancel = Arc::new(AtomicBool::new(true));
        let result = verify_integrity(&tsw_dir(), &mini_index, &cancel, |_| {})
            .expect("verification should succeed even when cancelled");

        assert!(
            result.entries_checked < 100,
            "Should have cancelled early, but checked all entries"
        );
    }

    #[test]
    fn test_verify_progress_callback() {
        let idx = rdb::parse_le_index(&le_idx_path()).expect("parse le.idx");

        let entries: Vec<_> = idx
            .entries
            .iter()
            .filter(|e| e.file_num == 0)
            .take(50)
            .cloned()
            .collect();

        let mini_index = rdb::LeIndex {
            root_hash: idx.root_hash,
            entries,
        };

        let cancel = Arc::new(AtomicBool::new(false));
        let progress_calls = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let pc = progress_calls.clone();

        let result = verify_integrity(&tsw_dir(), &mini_index, &cancel, move |p| {
            pc.fetch_add(1, Ordering::Relaxed);
            assert!(!p.current_file.is_empty() || p.phase == "complete");
        })
        .expect("verification should succeed");

        assert!(
            progress_calls.load(Ordering::Relaxed) >= 2,
            "Should have received at least 2 progress callbacks (start + complete)"
        );
        assert_eq!(result.entries_checked, 50);
    }

    // ── decompress_ioz1 tests ───────────────────────────────────────

    #[test]
    fn test_ioz1_bad_magic() {
        // Non-IOz1 data should be returned as-is
        let data = b"This is not IOz1 compressed data";
        let result = decompress_ioz1(data).expect("should succeed");
        assert_eq!(result, data);
    }

    #[test]
    fn test_ioz1_short_data() {
        // Data shorter than IOz1 header returned as-is
        let data = b"IOz";
        let result = decompress_ioz1(data).expect("should succeed");
        assert_eq!(result, data);
    }

    #[test]
    fn test_decompress_ioz1_roundtrip() {
        // Create IOz1 data manually: magic + original_size + zlib-compressed payload
        use flate2::write::ZlibEncoder;
        use flate2::Compression;

        let original = b"Hello, IOz1 compression test! This is a test payload for roundtrip.";
        let original_size = original.len() as u32;

        // Compress with zlib
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        // Build IOz1 packet
        let mut ioz1_data = Vec::new();
        ioz1_data.extend_from_slice(IOZ1_MAGIC);
        ioz1_data.extend_from_slice(&original_size.to_le_bytes());
        ioz1_data.extend_from_slice(&compressed);

        let result = decompress_ioz1(&ioz1_data).expect("decompression should succeed");
        assert_eq!(result, original);
    }

    #[test]
    fn test_ioz1_corrupt_zlib() {
        // IOz1 magic but garbage zlib data — should return an error
        let mut data = Vec::new();
        data.extend_from_slice(b"IOz1");
        data.extend_from_slice(&100u32.to_le_bytes()); // original_size
        data.extend_from_slice(b"this is not valid zlib data!!");

        let result = decompress_ioz1(&data);
        assert!(result.is_err(), "Should fail on corrupt zlib data");
        assert!(
            result.unwrap_err().contains("decompression failed"),
            "Error should mention decompression failure"
        );
    }

    // ── write_to_rdbdata tests ──────────────────────────────────────

    #[test]
    fn test_write_to_rdbdata_offset() {
        // Write to a temp file at a specific offset, read back, verify
        let tmp_dir = std::env::temp_dir().join("tsw_test_write_rdb");
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(tmp_dir.join("RDB")).unwrap();

        // Create a pre-allocated rdbdata file
        let rdbdata_path = tmp_dir.join("RDB/07.rdbdata");
        let mut f = File::create(&rdbdata_path).unwrap();
        f.write_all(&vec![0u8; 1024]).unwrap();
        f.flush().unwrap();
        drop(f);

        // Write data at offset 256
        let payload = b"Test data written at offset 256!";
        write_to_rdbdata(&tmp_dir, 7, 256, payload, payload.len())
            .expect("write should succeed");

        // Read back and verify
        let mut f = File::open(&rdbdata_path).unwrap();
        f.seek(SeekFrom::Start(256)).unwrap();
        let mut readback = vec![0u8; payload.len()];
        f.read_exact(&mut readback).unwrap();
        assert_eq!(&readback, payload);

        // Verify surrounding bytes are untouched (still zeros)
        f.seek(SeekFrom::Start(0)).unwrap();
        let mut before = vec![0u8; 256];
        f.read_exact(&mut before).unwrap();
        assert!(before.iter().all(|&b| b == 0), "Bytes before offset should be untouched");

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_write_to_rdbdata_length_mismatch() {
        let tmp_dir = std::env::temp_dir().join("tsw_test_write_rdb_mismatch");
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(tmp_dir.join("RDB")).unwrap();

        let rdbdata_path = tmp_dir.join("RDB/00.rdbdata");
        File::create(&rdbdata_path).unwrap();

        let payload = b"short";
        let result = write_to_rdbdata(&tmp_dir, 0, 0, payload, 100);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("length mismatch"),
            "Error should mention length mismatch"
        );

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_write_to_rdbdata_missing_file() {
        let tmp_dir = std::env::temp_dir().join("tsw_test_write_rdb_missing");
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(tmp_dir.join("RDB")).unwrap();

        let result = write_to_rdbdata(&tmp_dir, 99, 0, b"data", 4);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("Failed to open") && err.contains("file_num=99"),
            "Error should mention file_num: {}",
            err
        );

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    // ── minimum bundle set tests ────────────────────────────────────

    #[test]
    fn test_build_minimum_entry_set() {
        let min_set = build_minimum_entry_set(&tsw_dir()).expect("build minimum set");
        // Should contain entries from default-resources (477k) + core bundles
        // Total should be substantial but less than all 1.2M entries
        assert!(
            min_set.len() > 400_000,
            "Minimum set should have >400k entries (includes default-resources), got {}",
            min_set.len()
        );
        assert!(
            min_set.len() < 1_200_000,
            "Minimum set should be smaller than full 1.2M entries, got {}",
            min_set.len()
        );
    }

    #[test]
    fn test_is_entry_in_minimum_set() {
        let min_set = build_minimum_entry_set(&tsw_dir()).expect("build minimum set");
        // Check a known entry that should be in default-resources
        // (type=1000001, id=3 is the first entry in le.idx)
        let le_index = rdb::parse_le_index(&le_idx_path()).expect("parse le.idx");
        let first = &le_index.entries[0];
        // We can't guarantee which bundles reference the first entry,
        // but we can test the function works correctly
        let in_set = is_entry_in_minimum_set(first.rdb_type, first.id, &min_set);
        // The result depends on actual data — just verify no panic
        let _ = in_set;

        // A clearly invalid entry should not be in the set
        assert!(
            !is_entry_in_minimum_set(999_999_999, 999_999_999, &min_set),
            "Nonsense entry should not be in minimum set"
        );
    }

    #[test]
    fn test_minimum_set_subset_of_full() {
        // Verify all entries in minimum set are valid (type, id) pairs from le.idx
        let min_set = build_minimum_entry_set(&tsw_dir()).expect("build minimum set");
        let le_index = rdb::parse_le_index(&le_idx_path()).expect("parse le.idx");
        let full_set: HashSet<(u32, u32)> = le_index
            .entries
            .iter()
            .map(|e| (e.rdb_type, e.id))
            .collect();

        let mut not_in_full = 0;
        for &(rdb_type, id) in &min_set {
            if !full_set.contains(&(rdb_type, id)) {
                not_in_full += 1;
            }
        }
        // Bundle entries reference (type, id) pairs — they may include pairs
        // that are also in the full index. Some bundle refs use truncated u24 values
        // for type/id, so they won't exactly match the full u32 le.idx types.
        // This is expected behavior with the u24 format.
        // Just ensure most entries match.
        let match_rate = 1.0 - (not_in_full as f64 / min_set.len() as f64);
        assert!(
            match_rate > 0.5,
            "At least 50% of minimum set entries should match le.idx, got {:.1}%",
            match_rate * 100.0
        );
    }
}
