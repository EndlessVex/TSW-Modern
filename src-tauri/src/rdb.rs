use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::Path;

/// A single entry in the le.idx index — links an RDB (type, id) pair to its
/// location in a bundle file and the MD5 hash of the resource payload.
#[derive(Debug, Clone)]
pub struct LeIndexEntry {
    pub rdb_type: u32,
    pub id: u32,
    pub file_num: u8,
    pub flags: u8,
    pub offset: u32,
    pub length: u32,
    /// 16-byte MD5 hash of the resource.
    pub hash: [u8; 16],
}

/// The complete le.idx — root hash plus every resource entry.
#[derive(Debug)]
pub struct LeIndex {
    /// 16-byte root hash from the header — the CDN uses this as the version
    /// fingerprint of the full index.
    pub root_hash: [u8; 16],
    pub entries: Vec<LeIndexEntry>,
}

/// Errors produced by the le.idx and RDBHashIndex.bin parsers.
#[derive(Debug)]
pub enum RdbParseError {
    Io(io::Error),
    /// Magic bytes didn't match the expected value.
    BadMagic { expected: &'static str, got: [u8; 4] },
    /// Version field wasn't the expected value.
    BadVersion { expected: u32, got: u32 },
    /// File was too short to contain the declared entries.
    Truncated { expected_len: u64, actual_len: u64 },
}

impl fmt::Display for RdbParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RdbParseError::Io(e) => write!(f, "I/O error: {e}"),
            RdbParseError::BadMagic { expected, got } => {
                write!(
                    f,
                    "bad magic bytes: expected {expected}, got {:?}",
                    std::str::from_utf8(got).unwrap_or("<non-utf8>")
                )
            }
            RdbParseError::BadVersion { expected, got } => {
                write!(f, "unsupported version: expected {expected}, got {got}")
            }
            RdbParseError::Truncated {
                expected_len,
                actual_len,
            } => {
                write!(
                    f,
                    "file truncated: need at least {expected_len} bytes, got {actual_len}"
                )
            }
        }
    }
}

impl std::error::Error for RdbParseError {}

impl From<io::Error> for RdbParseError {
    fn from(e: io::Error) -> Self {
        RdbParseError::Io(e)
    }
}

// ─── le.idx parser ───────────────────────────────────────────────────────────

const IBDR_MAGIC: &[u8; 4] = b"IBDR";
const IBDR_VERSION: u32 = 7;
/// Header: 4 magic + 4 version + 16 root_hash + 4 entry_count = 28 bytes.
const IBDR_HEADER_LEN: u64 = 28;
const INDEX_ENTRY_SIZE: u64 = 8; // u32 type + u32 id
const DETAIL_ENTRY_SIZE: u64 = 28; // u8 file_num + u8 flags + u16 unk + u32 offset + u32 length + 16 hash

/// Parse a le.idx file from the given path.
///
/// Reads the header (magic `IBDR`, version 7, 16-byte root hash, u32 entry
/// count), the index section (type + id per entry), and the detail section
/// (file_num, flags, offset, length, hash per entry).  Stops before the
/// bundles section — we don't need it.
pub fn parse_le_index(path: &Path) -> Result<LeIndex, RdbParseError> {
    let data = fs::read(path).map_err(|e| {
        RdbParseError::Io(io::Error::new(
            e.kind(),
            format!("{}: {e}", path.display()),
        ))
    })?;
    let file_len = data.len() as u64;

    if file_len < IBDR_HEADER_LEN {
        return Err(RdbParseError::Truncated {
            expected_len: IBDR_HEADER_LEN,
            actual_len: file_len,
        });
    }

    let mut cur = Cursor::new(&data);

    // Magic
    let mut magic = [0u8; 4];
    cur.read_exact(&mut magic)?;
    if &magic != IBDR_MAGIC {
        return Err(RdbParseError::BadMagic {
            expected: "IBDR",
            got: magic,
        });
    }

    // Version
    let version = read_u32_le(&mut cur)?;
    if version != IBDR_VERSION {
        return Err(RdbParseError::BadVersion {
            expected: IBDR_VERSION,
            got: version,
        });
    }

    // Root hash
    let mut root_hash = [0u8; 16];
    cur.read_exact(&mut root_hash)?;

    // Entry count
    let entry_count = read_u32_le(&mut cur)? as u64;

    // Validate file is large enough for index + detail sections (bundles come after).
    let min_len = IBDR_HEADER_LEN + entry_count * INDEX_ENTRY_SIZE + entry_count * DETAIL_ENTRY_SIZE;
    if file_len < min_len {
        return Err(RdbParseError::Truncated {
            expected_len: min_len,
            actual_len: file_len,
        });
    }

    // Read index section (type + id pairs)
    let mut types = Vec::with_capacity(entry_count as usize);
    let mut ids = Vec::with_capacity(entry_count as usize);
    for _ in 0..entry_count {
        types.push(read_u32_le(&mut cur)?);
        ids.push(read_u32_le(&mut cur)?);
    }

    // Read detail section
    let mut entries = Vec::with_capacity(entry_count as usize);
    for i in 0..entry_count as usize {
        let mut detail = [0u8; 28];
        cur.read_exact(&mut detail)?;
        let file_num = detail[0];
        let flags = detail[1];
        // detail[2..4] is u16 unknown — skip
        let offset = u32::from_le_bytes([detail[4], detail[5], detail[6], detail[7]]);
        let length = u32::from_le_bytes([detail[8], detail[9], detail[10], detail[11]]);
        let mut hash = [0u8; 16];
        hash.copy_from_slice(&detail[12..28]);

        entries.push(LeIndexEntry {
            rdb_type: types[i],
            id: ids[i],
            file_num,
            flags,
            offset,
            length,
            hash,
        });
    }

    Ok(LeIndex {
        root_hash,
        entries,
    })
}

// ─── RDBHashIndex.bin parser ─────────────────────────────────────────────────

const RDHI_MAGIC: &[u8; 4] = b"RDHI";
const RDHI_VERSION: u32 = 7;
const RDHI_HEADER_LEN: u64 = 16;
#[allow(dead_code)] // documents the per-entry byte layout
const HASH_ENTRY_SIZE: u64 = 47; // u32 id + u32 file_size + u32 unknown + 16 hash + 19 unknown

/// One entry in the hash index — maps a resource to its decompressed file size
/// and MD5 hash as known by the CDN.
#[derive(Debug, Clone)]
pub struct HashIndexEntry {
    pub id: u32,
    pub file_size: u32,
    pub hash: [u8; 16],
}

/// The complete RDBHashIndex.bin — maps (type, id) to (file_size, hash).
#[derive(Debug)]
pub struct RdbHashIndex {
    pub entries: HashMap<(u32, u32), HashIndexEntry>,
}

/// Parse an RDBHashIndex.bin file.
///
/// Header: 4 magic (`RDHI`) + 4 version + 4 num_entries + 4 num_types.
/// Then `num_types` groups, each: 4 type + 4 count + count × entry-bytes.
///
/// Version 4 (CDN): 29-byte entries (id + file_size + unknown + hash + flags).
/// Version 7 (local): 47-byte entries (same fields + 18 bytes of location data).
pub fn parse_hash_index(path: &Path) -> Result<RdbHashIndex, RdbParseError> {
    let data = fs::read(path).map_err(|e| {
        RdbParseError::Io(io::Error::new(
            e.kind(),
            format!("{}: {e}", path.display()),
        ))
    })?;
    let file_len = data.len() as u64;

    if file_len < RDHI_HEADER_LEN {
        return Err(RdbParseError::Truncated {
            expected_len: RDHI_HEADER_LEN,
            actual_len: file_len,
        });
    }

    let mut cur = Cursor::new(&data);

    let mut magic = [0u8; 4];
    cur.read_exact(&mut magic)?;
    if &magic != RDHI_MAGIC {
        return Err(RdbParseError::BadMagic {
            expected: "RDHI",
            got: magic,
        });
    }

    let version = read_u32_le(&mut cur)?;
    // Version 4 = CDN format (29-byte entries), version 7 = local format (47-byte entries)
    let entry_size: usize = match version {
        4 => 29,
        5 | 6 | 7 => 47,
        _ => {
            return Err(RdbParseError::BadVersion {
                expected: RDHI_VERSION,
                got: version,
            })
        }
    };

    let num_entries = read_u32_le(&mut cur)?;
    let num_types = read_u32_le(&mut cur)?;

    let mut entries = HashMap::with_capacity(num_entries as usize);

    for _ in 0..num_types {
        let rdb_type = read_u32_le(&mut cur)?;
        let count = read_u32_le(&mut cur)?;

        for _ in 0..count {
            let mut raw = vec![0u8; entry_size];
            cur.read_exact(&mut raw)?;

            let id = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
            let file_size = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]);
            // raw[8..12] is u32 unknown
            let mut hash = [0u8; 16];
            hash.copy_from_slice(&raw[12..28]);
            // remaining bytes (1 for v4, 19 for v7) are skipped

            entries.insert(
                (rdb_type, id),
                HashIndexEntry {
                    id,
                    file_size,
                    hash,
                },
            );
        }
    }

    Ok(RdbHashIndex { entries })
}

// ─── Bundle section parser ───────────────────────────────────────────────────

/// A bundle in the le.idx — groups (type, id) entry references.
#[derive(Debug, Clone)]
pub struct Bundle {
    pub name: String,
    /// (rdb_type, resource_id) pairs that belong to this bundle.
    pub entries: Vec<(u32, u32)>,
}

/// Parse the bundle section from a le.idx file.
///
/// The bundle section starts immediately after the header (28 bytes) + index
/// section (entry_count × 8) + detail section (entry_count × 28).
///
/// Format:
/// - `u32` num_bundles
/// - Per bundle:
///   - `u32` name_length
///   - `[name_length]` name bytes (ASCII)
///   - `u8` flag
///   - `u24_LE` entry_count (3 bytes, little-endian)
///   - `[entry_count × 8]` entries, each: `u8(pad) + u24_LE(type) + u8(pad) + u24_LE(id)`
///   - `u8` separator
pub fn parse_bundles(path: &Path) -> Result<Vec<Bundle>, RdbParseError> {
    let data = fs::read(path).map_err(|e| {
        RdbParseError::Io(io::Error::new(
            e.kind(),
            format!("{}: {e}", path.display()),
        ))
    })?;
    let file_len = data.len() as u64;

    if file_len < IBDR_HEADER_LEN {
        return Err(RdbParseError::Truncated {
            expected_len: IBDR_HEADER_LEN,
            actual_len: file_len,
        });
    }

    let mut cur = Cursor::new(&data);

    // Validate header
    let mut magic = [0u8; 4];
    cur.read_exact(&mut magic)?;
    if &magic != IBDR_MAGIC {
        return Err(RdbParseError::BadMagic {
            expected: "IBDR",
            got: magic,
        });
    }

    let version = read_u32_le(&mut cur)?;
    if version != IBDR_VERSION {
        return Err(RdbParseError::BadVersion {
            expected: IBDR_VERSION,
            got: version,
        });
    }

    // Skip root hash
    let mut _root_hash = [0u8; 16];
    cur.read_exact(&mut _root_hash)?;

    let entry_count = read_u32_le(&mut cur)? as u64;

    // Seek past index + detail sections to reach bundles
    let bundle_offset = IBDR_HEADER_LEN + entry_count * INDEX_ENTRY_SIZE + entry_count * DETAIL_ENTRY_SIZE;
    if file_len < bundle_offset + 4 {
        return Err(RdbParseError::Truncated {
            expected_len: bundle_offset + 4,
            actual_len: file_len,
        });
    }
    cur.set_position(bundle_offset);

    let num_bundles = read_u32_le(&mut cur)?;
    let mut bundles = Vec::with_capacity(num_bundles as usize);

    for _ in 0..num_bundles {
        let name_len = read_u32_le(&mut cur)?;
        let mut name_bytes = vec![0u8; name_len as usize];
        cur.read_exact(&mut name_bytes).map_err(|_| RdbParseError::Truncated {
            expected_len: cur.position() + name_len as u64,
            actual_len: file_len,
        })?;
        let name = String::from_utf8_lossy(&name_bytes).to_string();

        // u8 flag + u24_LE entry_count
        let mut flag_and_count = [0u8; 4];
        cur.read_exact(&mut flag_and_count).map_err(|_| RdbParseError::Truncated {
            expected_len: cur.position() + 4,
            actual_len: file_len,
        })?;
        // flag_and_count[0] = flag (0x00 or 0x01), [1..4] = u24_LE entry_count
        let bundle_entry_count =
            flag_and_count[1] as u32
            | (flag_and_count[2] as u32) << 8
            | (flag_and_count[3] as u32) << 16;

        let mut entries = Vec::with_capacity(bundle_entry_count as usize);
        for _ in 0..bundle_entry_count {
            let mut raw = [0u8; 8];
            cur.read_exact(&mut raw).map_err(|_| RdbParseError::Truncated {
                expected_len: cur.position() + 8,
                actual_len: file_len,
            })?;
            // u8(pad) + u24_LE(type) + u8(pad) + u24_LE(id)
            let rdb_type = raw[1] as u32 | (raw[2] as u32) << 8 | (raw[3] as u32) << 16;
            let resource_id = raw[5] as u32 | (raw[6] as u32) << 8 | (raw[7] as u32) << 16;
            entries.push((rdb_type, resource_id));
        }

        // u8 separator
        let mut _sep = [0u8; 1];
        cur.read_exact(&mut _sep).map_err(|_| RdbParseError::Truncated {
            expected_len: cur.position() + 1,
            actual_len: file_len,
        })?;

        bundles.push(Bundle { name, entries });
    }

    Ok(bundles)
}

// ─── CDN URL construction ────────────────────────────────────────────────────

/// Build a CDN resource URL from a base URL and a 16-byte MD5 hash.
///
/// Pattern: `{base_url}/rdb/res/{hash[0..2]}/{hash[2..3]}/{hash[3..]}`
/// where the hex string is split into a 2-char prefix, 1-char middle, and the rest.
///
/// Example: hash `1da54e9fc9d492889fbe083df4dabef2`
///       → `{base}/rdb/res/1d/a/54e9fc9d492889fbe083df4dabef2`
pub fn cdn_url_from_hash(base_url: &str, hash: &[u8; 16]) -> String {
    let hex = hex_encode(hash);
    format!(
        "{}/rdb/res/{}/{}/{}",
        base_url.trim_end_matches('/'),
        &hex[0..2],
        &hex[2..3],
        &hex[3..]
    )
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn read_u32_le(r: &mut impl Read) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

/// Encode 16 bytes as lowercase hex without pulling in the `hex` crate.
pub fn hex_encode(bytes: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tsw_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../The Secret World")
    }

    fn le_idx_path() -> PathBuf {
        tsw_dir().join("RDB/le.idx")
    }

    fn hash_index_path() -> PathBuf {
        tsw_dir().join("RDB/RDBHashIndex.bin")
    }

    // ── le.idx happy-path ──────────────────────────────────────────────

    #[test]
    fn le_idx_header() {
        let idx = parse_le_index(&le_idx_path()).expect("parse le.idx");
        assert_eq!(idx.entries.len(), 709_723);
        assert_eq!(
            hex_encode(&idx.root_hash),
            "2018d1a9a0cff05d7f318cb68afffcc1"
        );
    }

    #[test]
    fn le_idx_first_entry() {
        let idx = parse_le_index(&le_idx_path()).expect("parse le.idx");
        let e = &idx.entries[0];
        assert_eq!(e.rdb_type, 1_000_001);
        assert_eq!(e.id, 3);
        assert_eq!(e.file_num, 0);
        assert_eq!(hex_encode(&e.hash), "1da54e9fc9d492889fbe083df4dabef2");
    }

    #[test]
    fn le_idx_last_entry() {
        let idx = parse_le_index(&le_idx_path()).expect("parse le.idx");
        let e = idx.entries.last().expect("should have entries");
        assert_eq!(e.rdb_type, 1_070_020);
        assert_eq!(e.id, 1_095_725);
        assert_eq!(e.file_num, 255);
    }

    // ── RDBHashIndex.bin happy-path ────────────────────────────────────

    #[test]
    fn hash_index_header() {
        let hi = parse_hash_index(&hash_index_path()).expect("parse hash index");
        assert_eq!(hi.entries.len(), 709_723);
    }

    #[test]
    fn hash_index_first_entry() {
        let hi = parse_hash_index(&hash_index_path()).expect("parse hash index");
        let entry = hi.entries.get(&(1_000_001, 3)).expect("entry (1000001, 3)");
        assert_eq!(entry.file_size, 89);
        assert_eq!(
            hex_encode(&entry.hash),
            "1da54e9fc9d492889fbe083df4dabef2"
        );
    }

    #[test]
    fn hash_index_type_count() {
        // Verify we got entries from all 169 type groups by counting distinct types.
        let hi = parse_hash_index(&hash_index_path()).expect("parse hash index");
        let distinct_types: std::collections::HashSet<u32> =
            hi.entries.keys().map(|(t, _)| *t).collect();
        assert_eq!(distinct_types.len(), 169);
    }

    // ── CDN URL ────────────────────────────────────────────────────────

    #[test]
    fn cdn_url_construction() {
        let hash: [u8; 16] = [
            0x1d, 0xa5, 0x4e, 0x9f, 0xc9, 0xd4, 0x92, 0x88, 0x9f, 0xbe, 0x08, 0x3d, 0xf4, 0xda,
            0xbe, 0xf2,
        ];
        let url = cdn_url_from_hash("http://cdn.example.com", &hash);
        assert_eq!(
            url,
            "http://cdn.example.com/rdb/res/1d/a/54e9fc9d492889fbe083df4dabef2"
        );
    }

    #[test]
    fn cdn_url_trims_trailing_slash() {
        let hash = [0u8; 16]; // all zeros
        let url = cdn_url_from_hash("http://cdn.example.com/", &hash);
        assert!(url.starts_with("http://cdn.example.com/rdb/res/"));
        assert!(!url.contains("//rdb"));
    }

    // ── Negative / error cases ─────────────────────────────────────────

    #[test]
    fn le_idx_bad_magic() {
        let mut data = vec![0u8; 128];
        data[..4].copy_from_slice(b"XXXX");
        let tmp = std::env::temp_dir().join("bad_magic_le.idx");
        fs::write(&tmp, &data).unwrap();
        let err = parse_le_index(&tmp).unwrap_err();
        assert!(
            matches!(err, RdbParseError::BadMagic { .. }),
            "expected BadMagic, got: {err}"
        );
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn le_idx_bad_version() {
        let mut data = vec![0u8; 128];
        data[..4].copy_from_slice(b"IBDR");
        data[4..8].copy_from_slice(&99u32.to_le_bytes()); // wrong version
        let tmp = std::env::temp_dir().join("bad_version_le.idx");
        fs::write(&tmp, &data).unwrap();
        let err = parse_le_index(&tmp).unwrap_err();
        assert!(
            matches!(err, RdbParseError::BadVersion { expected: 7, got: 99 }),
            "expected BadVersion, got: {err}"
        );
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn le_idx_truncated() {
        // File shorter than the 28-byte header
        let data = vec![0u8; 10];
        let tmp = std::env::temp_dir().join("truncated_le.idx");
        fs::write(&tmp, &data).unwrap();
        let err = parse_le_index(&tmp).unwrap_err();
        assert!(
            matches!(err, RdbParseError::Truncated { .. }),
            "expected Truncated, got: {err}"
        );
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn le_idx_truncated_entries() {
        // Valid header claiming 100 entries but file too short to contain them
        let mut data = vec![0u8; 28];
        data[..4].copy_from_slice(b"IBDR");
        data[4..8].copy_from_slice(&7u32.to_le_bytes());
        // root_hash: 16 bytes of zeros
        data[24..28].copy_from_slice(&100u32.to_le_bytes()); // claim 100 entries
        let tmp = std::env::temp_dir().join("trunc_entries_le.idx");
        fs::write(&tmp, &data).unwrap();
        let err = parse_le_index(&tmp).unwrap_err();
        assert!(
            matches!(err, RdbParseError::Truncated { .. }),
            "expected Truncated, got: {err}"
        );
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn le_idx_zero_entries() {
        // Valid file with 0 entries — should parse fine
        let mut data = vec![0u8; 28];
        data[..4].copy_from_slice(b"IBDR");
        data[4..8].copy_from_slice(&7u32.to_le_bytes());
        // entry_count = 0
        let tmp = std::env::temp_dir().join("zero_entries_le.idx");
        fs::write(&tmp, &data).unwrap();
        let idx = parse_le_index(&tmp).expect("should parse 0-entry file");
        assert_eq!(idx.entries.len(), 0);
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn le_idx_file_not_found() {
        let err = parse_le_index(Path::new("/tmp/definitely_nonexistent_le.idx")).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("definitely_nonexistent_le.idx"), "error should contain path: {msg}");
    }

    #[test]
    fn hash_index_bad_magic() {
        let mut data = vec![0u8; 128];
        data[..4].copy_from_slice(b"NOPE");
        let tmp = std::env::temp_dir().join("bad_magic_rdhi.bin");
        fs::write(&tmp, &data).unwrap();
        let err = parse_hash_index(&tmp).unwrap_err();
        assert!(
            matches!(err, RdbParseError::BadMagic { .. }),
            "expected BadMagic, got: {err}"
        );
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn hash_index_bad_version() {
        let mut data = vec![0u8; 128];
        data[..4].copy_from_slice(b"RDHI");
        data[4..8].copy_from_slice(&42u32.to_le_bytes());
        let tmp = std::env::temp_dir().join("bad_ver_rdhi.bin");
        fs::write(&tmp, &data).unwrap();
        let err = parse_hash_index(&tmp).unwrap_err();
        assert!(
            matches!(err, RdbParseError::BadVersion { expected: 7, got: 42 }),
            "expected BadVersion, got: {err}"
        );
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn hash_index_file_not_found() {
        let err =
            parse_hash_index(Path::new("/tmp/definitely_nonexistent_rdhi.bin")).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("definitely_nonexistent_rdhi.bin"), "error should contain path: {msg}");
    }

    #[test]
    fn le_idx_file_num_255_flagged() {
        // Verify entries with file_num=255 exist — these are server-only resources
        // that should be skipped during download decisions.
        let idx = parse_le_index(&le_idx_path()).expect("parse le.idx");
        let server_only: Vec<_> = idx.entries.iter().filter(|e| e.file_num == 255).collect();
        assert!(
            !server_only.is_empty(),
            "should have at least one file_num=255 entry"
        );
        // Last entry is one of them
        let last = idx.entries.last().unwrap();
        assert_eq!(last.file_num, 255);
    }

    // ── Bundle section ─────────────────────────────────────────────────

    #[test]
    fn test_parse_bundles_real_le_idx() {
        let bundles = parse_bundles(&le_idx_path()).expect("parse bundles");
        assert_eq!(
            bundles.len(),
            42,
            "Expected 42 bundles, got {}",
            bundles.len()
        );
        // First bundle is "1000"
        assert_eq!(bundles[0].name, "1000");
        // Last bundle is "default-resources"
        assert_eq!(bundles[41].name, "default-resources");
    }

    #[test]
    fn test_bundle_entry_counts() {
        let bundles = parse_bundles(&le_idx_path()).expect("parse bundles");
        let total: usize = bundles.iter().map(|b| b.entries.len()).sum();
        // Research says ~1,202,680 total refs
        assert_eq!(
            total, 1_202_680,
            "Expected ~1,202,680 total bundle entries, got {}",
            total
        );
    }

    #[test]
    fn test_bundle_entry_types_valid() {
        // Verify that parsed entry types/ids are in reasonable ranges
        let bundles = parse_bundles(&le_idx_path()).expect("parse bundles");
        for bundle in &bundles {
            for &(rdb_type, _id) in &bundle.entries {
                assert!(
                    rdb_type >= 1_000_000 && rdb_type < 2_000_000,
                    "Bundle '{}' has entry with unexpected type: {}",
                    bundle.name,
                    rdb_type
                );
            }
        }
    }

    #[test]
    fn test_parse_bundles_zero_entries() {
        // A valid le.idx with 0 entries should parse 0 bundles
        // (the bundle section offset would be right after the header, with num_bundles=0)
        let mut data = vec![0u8; 32]; // 28 header + 4 for num_bundles
        data[..4].copy_from_slice(b"IBDR");
        data[4..8].copy_from_slice(&7u32.to_le_bytes());
        // entry_count = 0, so bundle section starts at offset 28
        data[28..32].copy_from_slice(&0u32.to_le_bytes()); // 0 bundles

        let tmp = std::env::temp_dir().join("zero_bundles_le.idx");
        fs::write(&tmp, &data).unwrap();
        let bundles = parse_bundles(&tmp).expect("should parse 0-bundle file");
        assert_eq!(bundles.len(), 0);
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_parse_bundles_truncated() {
        // Valid header but file truncated before bundle section
        let mut data = vec![0u8; 28];
        data[..4].copy_from_slice(b"IBDR");
        data[4..8].copy_from_slice(&7u32.to_le_bytes());
        data[24..28].copy_from_slice(&100u32.to_le_bytes()); // 100 entries
        // File ends here — no index/detail/bundle sections
        let tmp = std::env::temp_dir().join("trunc_bundles_le.idx");
        fs::write(&tmp, &data).unwrap();
        let err = parse_bundles(&tmp).unwrap_err();
        assert!(
            matches!(err, RdbParseError::Truncated { .. }),
            "expected Truncated, got: {err}"
        );
        let _ = fs::remove_file(&tmp);
    }
}
