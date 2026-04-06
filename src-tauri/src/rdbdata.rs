//! Write downloaded RDB resources directly into rdbdata container files.
//!
//! rdbdata format:
//!   File header: 'RDB0' (4 bytes)
//!   Resources packed sequentially:
//!     Resource header (16 bytes): type(u32) + id(u32) + size(u32) + space(u32)
//!     Resource data: [size] bytes of decompressed content
//!     Padding: [space - size] bytes
//!
//! le.idx maps (type, id) → (file_num, offset, length) telling us exactly
//! where each resource belongs in which rdbdata file.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use crate::rdb::{LeIndex, LeIndexEntry};

/// Pre-create all required rdbdata files with RDB0 headers.
/// Only creates files that don't already exist.
pub fn create_rdbdata_files(
    install_dir: &Path,
    le_index: &LeIndex,
    valid_hashes: Option<&std::collections::HashSet<[u8; 16]>>,
) -> Result<(), String> {
    let rdb_dir = install_dir.join("RDB");
    std::fs::create_dir_all(&rdb_dir)
        .map_err(|e| format!("Failed to create RDB dir: {}", e))?;

    // Find all unique file_nums and their max end offsets for pre-allocation
    let mut file_nums: std::collections::HashSet<u8> = std::collections::HashSet::new();
    let mut max_end_by_file: HashMap<u8, u64> = HashMap::new();

    for entry in &le_index.entries {
        if entry.file_num == 255 {
            continue;
        }
        // Skip resources not in valid set (if filter provided)
        if let Some(valid) = valid_hashes {
            if !valid.contains(&entry.hash) {
                continue;
            }
        }
        file_nums.insert(entry.file_num);
        let end = entry.offset as u64 + entry.length as u64;
        let current = max_end_by_file.entry(entry.file_num).or_insert(0);
        if end > *current {
            *current = end;
        }
    }

    for &file_num in &file_nums {
        let path = rdb_dir.join(format!("{:02}.rdbdata", file_num));
        if path.exists() {
            // File exists — check if it has content beyond the header
            if let Ok(meta) = std::fs::metadata(&path) {
                if meta.len() > 4 {
                    continue; // Already has content, don't recreate
                }
            }
        }

        // Create file with RDB0 header and pre-allocate to full size.
        // set_len creates a sparse file on NTFS — fast allocation without
        // writing zeros. Without this, seeking past EOF to write a resource
        // at offset 800MB causes NTFS to zero-fill the entire gap — extremely
        // slow with 128 concurrent writers.
        let mut file = File::create(&path)
            .map_err(|e| format!("Failed to create {:02}.rdbdata: {}", file_num, e))?;
        file.write_all(b"RDB0")
            .map_err(|e| format!("Failed to write RDB0 header: {}", e))?;

        // Pre-allocate to exactly 1 GB (1,073,741,824 bytes) to match the
        // original ClientPatcher. The game may expect fixed-size containers
        // with zero-padded regions beyond the last resource.
        const RDBDATA_SIZE: u64 = 1_073_741_824;
        let target_size = max_end_by_file.get(&file_num).copied().unwrap_or(4);
        let alloc_size = if target_size > 4 { RDBDATA_SIZE.max(target_size) } else { 4 };
        if alloc_size > 4 {
            file.set_len(alloc_size)
                .map_err(|e| format!("Failed to allocate {:02}.rdbdata: {}", file_num, e))?;
        }
    }

    Ok(())
}

/// Build a lookup map from (type, id) → le.idx entry for fast resource placement.
pub fn build_placement_map(le_index: &LeIndex) -> HashMap<(u32, u32), &LeIndexEntry> {
    let mut map = HashMap::with_capacity(le_index.entries.len());
    for entry in &le_index.entries {
        if entry.file_num != 255 {
            map.insert((entry.rdb_type, entry.id), entry);
        }
    }
    map
}

/// Write a single decompressed resource into its rdbdata container file.
///
/// Writes the 16-byte resource header at (offset - 16) and the data at offset.
/// The offset comes from le.idx and points to the data start (after header).
pub fn write_resource_to_rdbdata(
    install_dir: &Path,
    file_num: u8,
    rdb_type: u32,
    id: u32,
    offset: u32,
    data: &[u8],
) -> Result<(), String> {
    let path = install_dir.join("RDB").join(format!("{:02}.rdbdata", file_num));

    let mut file = OpenOptions::new()
        .write(true)
        .open(&path)
        .map_err(|e| format!("Failed to open {:02}.rdbdata: {}", file_num, e))?;

    let size = data.len() as u32;
    // Space = size rounded up to next 4-byte boundary (observed padding pattern)
    let space = (size + 3) & !3;

    // Write resource header at offset - 16
    let header_offset = offset.checked_sub(16)
        .ok_or_else(|| format!("Invalid offset {} for resource {}:{}", offset, rdb_type, id))?;

    file.seek(SeekFrom::Start(header_offset as u64))
        .map_err(|e| format!("Failed to seek in {:02}.rdbdata: {}", file_num, e))?;

    // Header: type(4) + id(4) + size(4) + space(4)
    let mut header = [0u8; 16];
    header[0..4].copy_from_slice(&rdb_type.to_le_bytes());
    header[4..8].copy_from_slice(&id.to_le_bytes());
    header[8..12].copy_from_slice(&size.to_le_bytes());
    header[12..16].copy_from_slice(&space.to_le_bytes());

    file.write_all(&header)
        .map_err(|e| format!("Failed to write header in {:02}.rdbdata: {}", file_num, e))?;

    // Write resource data
    file.write_all(data)
        .map_err(|e| format!("Failed to write data in {:02}.rdbdata: {}", file_num, e))?;

    // Write padding zeros if space > size (always 0-3 bytes)
    if space > size {
        let pad_len = (space - size) as usize;
        file.write_all(&[0u8; 4][..pad_len])
            .map_err(|e| format!("Failed to write padding in {:02}.rdbdata: {}", file_num, e))?;
    }

    Ok(())
}
