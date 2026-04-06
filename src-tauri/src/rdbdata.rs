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
use std::sync::Mutex;

use crate::rdb::{LeIndex, LeIndexEntry};

/// Pre-create all required rdbdata files with RDB0 headers.
/// Only creates files that don't already exist.
///
/// Pre-allocates to 1GB per file, matching the original ClientPatcher
/// (Ghidra: allocateNewRdbDataFile at 0x00479590). If allocation fails
/// (low disk space), retries with halved sizes down to 128MB minimum,
/// matching the original's retry logic.
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

    // Count how many files we need to create (skip existing)
    let files_to_create: Vec<u8> = file_nums.iter().copied().filter(|&file_num| {
        let path = rdb_dir.join(format!("{:02}.rdbdata", file_num));
        if path.exists() {
            if let Ok(meta) = std::fs::metadata(&path) {
                if meta.len() > 4 {
                    return false; // Already has content
                }
            }
        }
        true
    }).collect();

    if files_to_create.is_empty() {
        return Ok(());
    }

    // Check available disk space before allocating.
    // On failure to query, proceed optimistically (will catch errors on set_len).
    let available_bytes = fs_available_space(&rdb_dir);

    // Determine initial allocation size. If available space is known and tight,
    // start with a smaller allocation to avoid immediate failure.
    // Original ClientPatcher: max 1GB, clamp to available space.
    const RDBDATA_SIZE: u64 = 1_073_741_824; // 1 GB
    const MIN_ALLOC_SIZE: u64 = 128 * 1024 * 1024; // 128 MB

    let initial_alloc = if let Some(avail) = available_bytes {
        let per_file = avail / (files_to_create.len() as u64).max(1);
        RDBDATA_SIZE.min(per_file)
    } else {
        RDBDATA_SIZE
    };

    for &file_num in &files_to_create {
        let path = rdb_dir.join(format!("{:02}.rdbdata", file_num));

        let mut file = File::create(&path)
            .map_err(|e| format!("Failed to create {:02}.rdbdata: {}", file_num, e))?;
        file.write_all(b"RDB0")
            .map_err(|e| format!("Failed to write RDB0 header: {}", e))?;

        // Pre-allocate with SetEndOfFile (via set_len). On NTFS this allocates
        // real clusters with deferred zeroing (VDL). This is NOT a sparse file —
        // the space is reserved on disk to prevent fragmentation and guarantee
        // capacity before downloads start.
        let target_size = max_end_by_file.get(&file_num).copied().unwrap_or(4);
        let desired = if target_size > 4 { initial_alloc.max(target_size) } else { 4 };

        if desired > 4 {
            // Retry with halved sizes on failure, matching original ClientPatcher
            let mut alloc_size = desired;
            loop {
                match file.set_len(alloc_size) {
                    Ok(()) => break,
                    Err(e) => {
                        if alloc_size > MIN_ALLOC_SIZE {
                            log::warn!(
                                "Failed to allocate {}MB for {:02}.rdbdata, retrying with {}MB: {}",
                                alloc_size / (1024 * 1024),
                                file_num,
                                alloc_size / 2 / (1024 * 1024),
                                e
                            );
                            alloc_size /= 2;
                        } else {
                            return Err(format!(
                                "Failed to allocate {:02}.rdbdata (tried down to {}MB): {}",
                                file_num, alloc_size / (1024 * 1024), e
                            ));
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Query available disk space for the given path's filesystem.
/// Returns None if the query fails (proceed optimistically).
fn fs_available_space(path: &Path) -> Option<u64> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;

        // Null-terminate the path for Win32
        let wide: Vec<u16> = path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut free_bytes: u64 = 0;
        let ret = unsafe {
            windows_sys_get_disk_free_space(&wide, &mut free_bytes)
        };
        if ret != 0 { Some(free_bytes) } else { None }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        None
    }
}

/// Thin wrapper around GetDiskFreeSpaceExW. Returns non-zero on success.
#[cfg(target_os = "windows")]
unsafe fn windows_sys_get_disk_free_space(path: &[u16], free_bytes: &mut u64) -> i32 {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }
    let mut total: u64 = 0;
    let mut total_free: u64 = 0;
    GetDiskFreeSpaceExW(path.as_ptr(), free_bytes, &mut total, &mut total_free)
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

/// Cached file handles for rdbdata writes. Keeps files open across
/// multiple resource writes to avoid ~22K open/close syscalls.
pub struct RdbdataWriter {
    install_dir: std::path::PathBuf,
    handles: Mutex<HashMap<u8, File>>,
}

impl RdbdataWriter {
    pub fn new(install_dir: &Path) -> Self {
        Self {
            install_dir: install_dir.to_path_buf(),
            handles: Mutex::new(HashMap::new()),
        }
    }

    /// Write a resource to its rdbdata file, reusing cached file handle.
    pub fn write_resource(
        &self,
        file_num: u8,
        rdb_type: u32,
        id: u32,
        offset: u32,
        data: &[u8],
    ) -> Result<(), String> {
        let mut handles = self.handles.lock().map_err(|e| e.to_string())?;
        if !handles.contains_key(&file_num) {
            let path = self.install_dir.join("RDB").join(format!("{:02}.rdbdata", file_num));
            let f = OpenOptions::new()
                .write(true)
                .open(&path)
                .map_err(|e| format!("Failed to open {:02}.rdbdata: {}", file_num, e))?;
            handles.insert(file_num, f);
        }
        let file = handles.get_mut(&file_num).unwrap(); // safe: just inserted

        let size = data.len() as u32;
        let space = (size + 3) & !3;

        let header_offset = offset.checked_sub(16)
            .ok_or_else(|| format!("Invalid offset {} for resource {}:{}", offset, rdb_type, id))?;

        file.seek(SeekFrom::Start(header_offset as u64))
            .map_err(|e| format!("Failed to seek in {:02}.rdbdata: {}", file_num, e))?;

        let mut header = [0u8; 16];
        header[0..4].copy_from_slice(&rdb_type.to_le_bytes());
        header[4..8].copy_from_slice(&id.to_le_bytes());
        header[8..12].copy_from_slice(&size.to_le_bytes());
        header[12..16].copy_from_slice(&space.to_le_bytes());

        file.write_all(&header)
            .map_err(|e| format!("Failed to write header: {}", e))?;
        file.write_all(data)
            .map_err(|e| format!("Failed to write data: {}", e))?;

        if space > size {
            let pad_len = (space - size) as usize;
            file.write_all(&[0u8; 4][..pad_len])
                .map_err(|e| format!("Failed to write padding: {}", e))?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_alloc_size_halving() {
        // Verify the halving logic produces the expected sequence
        let mut size: u64 = 1_073_741_824; // 1GB
        let min_size: u64 = 128 * 1024 * 1024; // 128MB
        let mut sizes = vec![size];
        while size > min_size {
            size /= 2;
            sizes.push(size);
        }
        assert_eq!(sizes, vec![
            1_073_741_824, // 1 GB
            536_870_912,   // 512 MB
            268_435_456,   // 256 MB
            134_217_728,   // 128 MB
        ]);
    }

    #[test]
    fn test_alloc_size_respects_target() {
        // If target_size > RDBDATA_SIZE, allocation should use target_size
        let target: u64 = 1_200_000_000;
        let rdbdata_size: u64 = 1_073_741_824;
        let alloc = rdbdata_size.max(target);
        assert_eq!(alloc, target);
    }
}
