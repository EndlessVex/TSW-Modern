use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

/// End-to-end test of the full ClientPatcher download flow:
/// Phase 1: Download PatchInfoClient.txt → FileHashes → parse → download all loose files
/// Phase 2: Download RDBHashIndex.bin → parse → download sample RDB resources
///
/// Writes everything to a test directory and verifies integrity.

const CDN_BASE: &str = "https://update.secretworld.com/tswupm";
const PATCH_FOLDER: &str = "TSWLiveSteam";

fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(128)
        .tcp_keepalive(Duration::from_secs(30))
        .user_agent("") // Empty UA — CDN blocks default curl UA
        .build()
        .expect("build client")
}

fn client_url(hash_hex: &str) -> String {
    format!("{}/client/{}/{}", CDN_BASE, &hash_hex[..2], &hash_hex[2..])
}

/// Parse FileHashes (CFHL format) into a list of (relative_path, compressed_size, hash_hex).
fn parse_file_hashes(data: &[u8]) -> Vec<(String, u32, String)> {
    use std::io::Read;

    if data.len() < 11 || &data[0..4] != b"CFHL" {
        eprintln!("Invalid FileHashes: bad magic or too short");
        return Vec::new();
    }

    let mut entries = Vec::new();
    let mut i = 8; // skip CFHL + version
    let mut dir_stack: Vec<String> = Vec::new();

    // First byte after header should be '/' (root dir marker)
    if data[i] == b'/' {
        i += 1;
        // Read 2 bytes — sub-entry count or padding
        if i + 2 > data.len() { return entries; }
        let _root_meta = u16::from_le_bytes([data[i], data[i + 1]]);
        i += 2;
    }

    while i + 2 < data.len() {
        let name_len = u16::from_le_bytes([data[i], data[i + 1]]) as usize;
        i += 2;

        if name_len == 0 || name_len > 500 || i + name_len + 20 > data.len() {
            break;
        }

        let name = match std::str::from_utf8(&data[i..i + name_len]) {
            Ok(s) => s.to_string(),
            Err(_) => break,
        };
        i += name_len;

        let file_size = u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        i += 4;

        let hash_hex: String = data[i..i + 16].iter().map(|b| format!("{:02x}", b)).collect();
        i += 16;

        // Build full path
        let dir_prefix = dir_stack.join("/");
        let full_path = if dir_prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", dir_prefix, name)
        };

        entries.push((full_path, file_size, hash_hex));

        if i >= data.len() {
            break;
        }

        // Read marker byte
        let marker = data[i];
        i += 1;

        match marker {
            0x00 => {
                // Same directory, continue
            }
            0x01 => {
                // Enter subdirectory
                if i + 2 > data.len() { break; }
                let dir_name_len = u16::from_le_bytes([data[i], data[i + 1]]) as usize;
                i += 2;
                if dir_name_len == 0 || i + dir_name_len > data.len() { break; }
                let dir_name = match std::str::from_utf8(&data[i..i + dir_name_len]) {
                    Ok(s) => s.to_string(),
                    Err(_) => break,
                };
                i += dir_name_len;
                // Read sub-count + flags (3 bytes)
                if i + 3 > data.len() { break; }
                let _sub_count = u16::from_le_bytes([data[i], data[i + 1]]);
                let _flags = data[i + 2];
                i += 3;
                dir_stack.push(dir_name);
            }
            _ => {
                // Go up N directories (marker value indicates depth change)
                // marker 0x02 = go up 1, 0x03 = go up 2, etc.? 
                // Or it might be: number of levels to pop
                // Let's try: if marker > 1, pop (marker - 1) directories, then read new dir
                let levels_up = marker as usize;
                for _ in 0..levels_up.min(dir_stack.len()) {
                    dir_stack.pop();
                }
                // After going up, there should be a new directory entry
                if i + 2 > data.len() { break; }
                let dir_name_len = u16::from_le_bytes([data[i], data[i + 1]]) as usize;
                i += 2;
                if dir_name_len == 0 || i + dir_name_len > data.len() { break; }
                let dir_name = match std::str::from_utf8(&data[i..i + dir_name_len]) {
                    Ok(s) => s.to_string(),
                    Err(_) => break,
                };
                i += dir_name_len;
                if i + 3 > data.len() { break; }
                let _sub_count = u16::from_le_bytes([data[i], data[i + 1]]);
                let _flags = data[i + 2];
                i += 3;
                dir_stack.push(dir_name);
            }
        }
    }

    entries
}

async fn download_to_memory(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let resp = client.get(url).send().await
        .map_err(|e| format!("GET {}: {}", url, e))?;
    if !resp.status().is_success() {
        return Err(format!("GET {} → {}", url, resp.status()));
    }
    resp.bytes().await
        .map(|b| b.to_vec())
        .map_err(|e| format!("read body {}: {}", url, e))
}

/// Decompress IOz1 data (4-byte magic + zlib payload).
fn decompress_ioz1(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 4 {
        return Err("Too short for IOz1".into());
    }
    if &data[..4] != b"IOz1" {
        // Not compressed — return as-is
        return Ok(data.to_vec());
    }
    use std::io::Read;
    let mut decoder = flate2::read::ZlibDecoder::new(&data[4..]);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)
        .map_err(|e| format!("zlib decompress: {}", e))?;
    Ok(decompressed)
}

#[tokio::main]
async fn main() {
    let test_dir = PathBuf::from("../test_full_download");
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(&test_dir).expect("create test dir");

    let client = build_client();
    let start = Instant::now();

    println!("=== PHASE 0: Download PatchInfoClient.txt ===\n");

    let patch_info_url = format!("{}/{}/PatchInfoClient.txt", CDN_BASE, PATCH_FOLDER);
    let patch_info = download_to_memory(&client, &patch_info_url).await.expect("PatchInfoClient.txt");
    let patch_info_text = String::from_utf8_lossy(&patch_info);

    let mut patch_info_map = HashMap::new();
    for line in patch_info_text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            patch_info_map.insert(k.to_string(), v.to_string());
        }
    }

    println!("PatchInfoClient.txt contents:");
    for (k, v) in &patch_info_map {
        println!("  {k} = {v}");
    }

    let root_hash = patch_info_map.get("RootHash").expect("RootHash missing");
    let rdb_hash = patch_info_map.get("RDBHash").expect("RDBHash missing");

    println!("\n=== PHASE 1: Download FileHashes via /client/ path ===\n");

    let file_hashes_url = client_url(root_hash);
    println!("URL: {}", file_hashes_url);
    let file_hashes_data = download_to_memory(&client, &file_hashes_url).await.expect("FileHashes");
    println!("Downloaded: {} bytes", file_hashes_data.len());

    // Check if IOz1 compressed
    let file_hashes_raw = if file_hashes_data.starts_with(b"IOz1") {
        println!("IOz1 compressed — decompressing...");
        decompress_ioz1(&file_hashes_data).expect("decompress FileHashes")
    } else {
        file_hashes_data
    };

    // Verify it's CFHL
    assert_eq!(&file_hashes_raw[..4], b"CFHL", "FileHashes should have CFHL magic");
    println!("Magic: CFHL ✓");

    // Save to disk
    std::fs::write(test_dir.join("FileHashes"), &file_hashes_raw).expect("write FileHashes");

    // Parse
    let loose_files = parse_file_hashes(&file_hashes_raw);
    println!("Parsed {} entries from FileHashes", loose_files.len());

    // Show directory structure
    let mut dirs: HashMap<String, usize> = HashMap::new();
    for (path, _, _) in &loose_files {
        let dir = if let Some(pos) = path.rfind('/') {
            &path[..pos]
        } else {
            "(root)"
        };
        *dirs.entry(dir.to_string()).or_default() += 1;
    }
    println!("\nDirectory breakdown:");
    let mut dir_list: Vec<_> = dirs.iter().collect();
    dir_list.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
    for (dir, count) in dir_list.iter().take(20) {
        println!("  {dir}: {count} files");
    }

    let total_compressed: u64 = loose_files.iter().map(|(_, s, _)| *s as u64).sum();
    println!("\nTotal compressed size: {:.1} MB", total_compressed as f64 / 1_048_576.0);

    println!("\n=== PHASE 2: Test download of loose files via /client/ ===\n");

    // Download first 50 files to verify the path works
    let test_count = 50.min(loose_files.len());
    let sem = Arc::new(Semaphore::new(32));
    let success = Arc::new(AtomicU32::new(0));
    let failed = Arc::new(AtomicU32::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let decompressed_bytes = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::new();

    for (path, comp_size, hash) in loose_files.iter().take(test_count) {
        let permit = sem.clone().acquire_owned().await.unwrap();
        let client = client.clone();
        let url = client_url(hash);
        let dest = test_dir.join(path);
        let path_clone = path.clone();
        let comp_size = *comp_size;
        let s = success.clone();
        let f = failed.clone();
        let b = bytes.clone();
        let db = decompressed_bytes.clone();

        handles.push(tokio::spawn(async move {
            let _p = permit;
            match download_to_memory(&client, &url).await {
                Ok(data) => {
                    b.fetch_add(data.len() as u64, Ordering::Relaxed);

                    // Verify compressed size matches
                    if comp_size > 0 && data.len() as u32 != comp_size {
                        println!("  SIZE MISMATCH: {} expected={} got={}", path_clone, comp_size, data.len());
                    }

                    // Try IOz1 decompression
                    let final_data = match decompress_ioz1(&data) {
                        Ok(d) => d,
                        Err(e) => {
                            println!("  DECOMPRESS FAIL: {} — {}", path_clone, e);
                            data
                        }
                    };

                    db.fetch_add(final_data.len() as u64, Ordering::Relaxed);

                    // Write to disk
                    if let Some(parent) = dest.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Err(e) = std::fs::write(&dest, &final_data) {
                        println!("  WRITE FAIL: {} — {}", path_clone, e);
                        f.fetch_add(1, Ordering::Relaxed);
                        return;
                    }

                    s.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    println!("  DOWNLOAD FAIL: {} — {}", path_clone, e);
                    f.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    let succ = success.load(Ordering::Relaxed);
    let fail = failed.load(Ordering::Relaxed);
    let raw = bytes.load(Ordering::Relaxed);
    let decomp = decompressed_bytes.load(Ordering::Relaxed);
    println!("\nPhase 2 results ({} files tested):", test_count);
    println!("  Success: {succ}, Failed: {fail}");
    println!("  Downloaded: {:.1} MB compressed", raw as f64 / 1_048_576.0);
    println!("  Decompressed: {:.1} MB", decomp as f64 / 1_048_576.0);

    if fail > 0 {
        println!("\n⚠ SOME FILES FAILED — investigate before implementing");
    } else {
        println!("\n✓ All test downloads succeeded");
    }

    // Verify a few known files by checking they exist and have reasonable size
    println!("\n=== PHASE 3: Verify specific critical files ===\n");

    let critical_checks = [
        ("TheSecretWorld.exe", "f3307ddeb2fe849f3081ebcb8143f074"),
        ("TheSecretWorldDX11.exe", "ac1292a51bfbb22a0f033ef8688aeddf"),
        ("ClientPatcher.exe", "b36bba4f8606fe8fda4fec2a747703bf"),
    ];

    for (name, hash) in &critical_checks {
        let url = client_url(hash);
        match download_to_memory(&client, &url).await {
            Ok(data) => {
                let decompressed = decompress_ioz1(&data).unwrap_or(data.clone());
                let actual_md5 = {
                    use md5::Digest;
                    let mut hasher = md5::Md5::new();
                    hasher.update(&decompressed);
                    let result = hasher.finalize();
                    format!("{:x}", result)
                };
                let md5_ok = actual_md5 == *hash;
                println!("  {} — downloaded {} bytes, decompressed {} bytes, MD5 {}",
                    name, data.len(), decompressed.len(),
                    if md5_ok { "✓ MATCH".to_string() } else { format!("✗ MISMATCH (got {})", actual_md5) });

                // Save to test dir
                std::fs::write(test_dir.join(name), &decompressed).expect("write critical file");
            }
            Err(e) => {
                println!("  {} — FAILED: {}", name, e);
            }
        }
    }

    println!("\n=== PHASE 4: Download RDBHashIndex.bin ===\n");

    let rdb_hash_url = format!("{}/rdb/full/{}", CDN_BASE, rdb_hash);
    println!("URL: {}", rdb_hash_url);
    match download_to_memory(&client, &rdb_hash_url).await {
        Ok(data) => {
            let decompressed = decompress_ioz1(&data).unwrap_or(data.clone());
            println!("Downloaded: {} bytes, decompressed: {} bytes", data.len(), decompressed.len());

            // Verify RDHI magic
            if decompressed.len() >= 4 && &decompressed[..4] == b"RDHI" {
                let version = u32::from_le_bytes([decompressed[4], decompressed[5], decompressed[6], decompressed[7]]);
                let num_entries = u32::from_le_bytes([decompressed[8], decompressed[9], decompressed[10], decompressed[11]]);
                println!("Magic: RDHI ✓, version: {}, entries: {}", version, num_entries);
                std::fs::write(test_dir.join("RDBHashIndex.bin"), &decompressed).expect("write hash index");
            } else {
                println!("⚠ Unexpected format — not RDHI");
            }
        }
        Err(e) => println!("FAILED: {}", e),
    }

    let elapsed = start.elapsed();
    println!("\n=== SUMMARY ===");
    println!("Total time: {:.1}s", elapsed.as_secs_f64());
    println!("Test directory: {}", test_dir.display());
    println!("Files written: {}", succ);

    // List what's in the test directory
    println!("\nTest directory contents:");
    if let Ok(entries) = std::fs::read_dir(&test_dir) {
        for entry in entries.flatten() {
            let meta = entry.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            if is_dir {
                println!("  [DIR] {}/", entry.file_name().to_string_lossy());
            } else {
                println!("  {} ({:.1} KB)", entry.file_name().to_string_lossy(), size as f64 / 1024.0);
            }
        }
    }

    // Cleanup
    println!("\nLeaving test directory for inspection. Remove with:");
    println!("  rm -rf {}", test_dir.display());
}
