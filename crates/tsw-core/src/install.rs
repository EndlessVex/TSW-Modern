//! Install pipeline orchestration.
//!
//! `run_install_pipeline` downloads the game's RDB resources, writes them
//! into `RDB/XX.rdbdata` files, and tracks completion so interrupted runs
//! can resume. It is called from both the Windows Tauri launcher (via
//! `src-tauri/src/lib.rs::run_patching_inner`) and the Linux CLI.
//!
//! Progress is reported through `ProgressReporter::on_download`. Pause and
//! cancel flags are shared `AtomicBool` values the caller can flip from a
//! UI button or a Ctrl-C handler.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::progress::ProgressReporter;

/// Run the full install pipeline for the game at `install_dir`.
///
/// Caller contract:
/// - `install_dir` must exist and contain `LocalConfig.xml`.
/// - `reporter` receives periodic `on_download` events with phase in
///   {"checking", "bootstrapping", "downloading", "patching", "complete"}.
/// - `pause_flag.store(true, ...)` pauses the download loop between files.
/// - `cancel_flag.store(true, ...)` causes the function to return early.
///
/// Returns `Err(String)` if a non-recoverable step fails (e.g., le.idx
/// download returns HTTP 5xx, rdbdata file creation fails, too many files
/// fail to download).
pub async fn run_install_pipeline(
    install_dir: &Path,
    reporter: &Arc<dyn ProgressReporter>,
    pause_flag: &AtomicBool,
    cancel_flag: &AtomicBool,
) -> Result<(), String> {
    let base = install_dir.to_path_buf();
    let rdb_dir = base.join("RDB");
    std::fs::create_dir_all(&rdb_dir)
        .map_err(|e| format!("Failed to create RDB dir: {}", e))?;

    // Emit checking phase
    reporter.on_download(&crate::download::DownloadProgress {
        bytes_downloaded: 0, total_bytes: 0,
        files_completed: 0, files_total: 0,
        speed_bps: 0, current_file: "Checking game state...".into(),
        phase: "checking".into(), failed_files: 0,
    });

    // Parse CDN config
    let patch_config =
        crate::config::parse_local_config(&base.join("LocalConfig.xml")).map_err(|e| e.to_string())?;
    let cdn_base_url = patch_config.http_patch_addr.replace("http://", "https://");

    let le_idx_path = rdb_dir.join("le.idx");
    let hash_idx_path = rdb_dir.join("RDBHashIndex.bin");

    // Create a shared HTTP client for bootstrap downloads
    let bootstrap_client = reqwest::Client::builder()
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .pool_max_idle_per_host(10)
        .user_agent("")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    // Step 1: Download le.idx from GitHub Releases if missing
    if !le_idx_path.exists() {
        reporter.on_download(&crate::download::DownloadProgress {
            bytes_downloaded: 0, total_bytes: 0,
            files_completed: 0, files_total: 0,
            speed_bps: 0, current_file: "Downloading resource index...".into(),
            phase: "bootstrapping".into(), failed_files: 0,
        });

        let le_idx_url = "https://github.com/EndlessVex/TSW-Modern/releases/download/game-data/le.idx.gz";
        log::info!("Downloading le.idx from: {}", le_idx_url);
        let response = bootstrap_client.get(le_idx_url)
            .send().await.map_err(|e| format!("Failed to download le.idx: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Failed to download le.idx: HTTP {}", response.status()));
        }

        let gz_bytes = response.bytes().await
            .map_err(|e| format!("Failed to read le.idx download: {}", e))?;

        // Decompress gzip
        use std::io::Read;
        let mut decoder = flate2::read::GzDecoder::new(&gz_bytes[..]);
        let mut le_idx_data = Vec::new();
        decoder.read_to_end(&mut le_idx_data)
            .map_err(|e| format!("Failed to decompress le.idx: {}", e))?;

        // Verify it's IBDR format
        if le_idx_data.len() < 4 || &le_idx_data[..4] != b"IBDR" {
            return Err("Downloaded le.idx has invalid format".into());
        }

        tokio::fs::write(&le_idx_path, &le_idx_data).await
            .map_err(|e| format!("Failed to write le.idx: {}", e))?;

        log::info!("Downloaded le.idx: {} bytes", le_idx_data.len());
    }

    // Step 2: Download RDBHashIndex.bin if missing
    // Re-download if missing OR if the file is a stale version 4 index (~22 MB).
    // Version 7 is ~44 MB; anything under 30 MB is the old truncated version.
    let hash_idx_needs_redownload = !hash_idx_path.exists() ||
        hash_idx_path.metadata().map(|m| m.len() < 30_000_000).unwrap_or(true);
    if hash_idx_needs_redownload {
        reporter.on_download(&crate::download::DownloadProgress {
            bytes_downloaded: 0, total_bytes: 0,
            files_completed: 0, files_total: 0,
            speed_bps: 0, current_file: "Downloading hash index...".into(),
            phase: "bootstrapping".into(), failed_files: 0,
        });

        let patch_info_url = format!("{}/PatchInfoClient.txt", patch_config.patch_base_url());
        let patch_info_text = bootstrap_client.get(&patch_info_url)
            .send().await.map_err(|e| format!("PatchInfoClient.txt: {}", e))?
            .text().await.map_err(|e| format!("PatchInfoClient.txt read: {}", e))?;

        // Prefer RDBHash-7 (version 7, full index) over the default RDBHash (version 4, half-size).
        // The game requires the version 7 hash index to locate all resources.
        let rdb_hash = patch_info_text.lines()
            .find(|l| l.starts_with("RDBHash-7="))
            .and_then(|l| l.strip_prefix("RDBHash-7="))
            .or_else(|| patch_info_text.lines()
                .find(|l| l.starts_with("RDBHash="))
                .and_then(|l| l.strip_prefix("RDBHash=")))
            .ok_or("RDBHash not found in PatchInfoClient.txt")?
            .to_string();

        let hash_idx_url = format!("{}/rdb/full/{}", cdn_base_url.trim_end_matches('/'), rdb_hash);
        let response = bootstrap_client.get(&hash_idx_url)
            .send().await.map_err(|e| format!("RDBHashIndex.bin: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("CDN returned {} for RDBHashIndex.bin", response.status()));
        }

        let hash_idx_bytes = response.bytes().await
            .map_err(|e| format!("RDBHashIndex.bin read: {}", e))?;

        let final_bytes = crate::verify::decompress_cdn(&hash_idx_bytes)?;

        tokio::fs::write(&hash_idx_path, &final_bytes).await
            .map_err(|e| format!("Write RDBHashIndex.bin: {}", e))?;
    }

    // Step 3: Parse indices and create rdbdata files
    let le_index = crate::rdb::parse_le_index(&le_idx_path).map_err(|e| e.to_string())?;
    let hash_index = crate::rdb::parse_hash_index(&hash_idx_path).map_err(|e| e.to_string())?;

    // Build set of valid resource hashes from the CDN hash index.
    // Resources not in this set are deleted/obsolete and should be skipped.
    let valid_hashes: std::collections::HashSet<[u8; 16]> = hash_index.entries.values()
        .map(|e| e.hash)
        .collect();

    // Create rdbdata container files
    // Create rdbdata container files (pre-allocated sparse files)
    reporter.on_download(&crate::download::DownloadProgress {
        bytes_downloaded: 0, total_bytes: 0,
        files_completed: 0, files_total: 0,
        speed_bps: 0, current_file: "Creating game data files...".into(),
        phase: "bootstrapping".into(), failed_files: 0,
    });
    crate::rdbdata::create_rdbdata_files(&base, &le_index, Some(&valid_hashes))?;
    let rdb_writer = std::sync::Arc::new(crate::rdbdata::RdbdataWriter::new(&base));

    // Emit bootstrapping status before heavy setup work.
    reporter.on_download(&crate::download::DownloadProgress {
        bytes_downloaded: 0, total_bytes: 0,
        files_completed: 0, files_total: 0,
        speed_bps: 0, current_file: "Initializing texture encoder...".into(),
        phase: "bootstrapping".into(), failed_files: 0,
    });
    // Build placement map for fast lookup
    let _placement_map = crate::rdbdata::build_placement_map(&le_index);

    // Step 4: Compute download plan — only resources not yet written to rdbdata
    // Check which resources already exist by verifying a sample from each rdbdata file.
    // For now, use the hash index to determine what needs downloading.
    // Resources already in rdbdata (from a previous partial run) are detected by
    // checking if the resource header exists at the expected offset.
    // Step 4: Compute download plan — skip resources already written to rdbdata.
    // Use a completion log (rdb_completed.log) to track which rdbdata files
    // have been fully written. This is more reliable than sampling headers
    // because a file can have some resources written but not all.
    let completion_log_path = rdb_dir.join("rdb_completed.log");
    let completed_files: std::collections::HashSet<u8> = if let Ok(data) = std::fs::read_to_string(&completion_log_path) {
        data.lines()
            .filter_map(|l| l.trim().parse::<u8>().ok())
            .collect()
    } else {
        std::collections::HashSet::new()
    };

    let cdn_base = &cdn_base_url;
    let mut tasks = Vec::new();

    for entry in &le_index.entries {
        if entry.file_num == 255 {
            continue;
        }

        // Skip resources not in the CDN hash index (deleted/obsolete).
        // This prevents downloading ~232 extra resources that the official
        // install marks as file_num=255, saving ~85MB and one rdbdata file.
        if !valid_hashes.contains(&entry.hash) {
            continue;
        }

        // If this file was fully completed in a previous run, skip it
        if completed_files.contains(&entry.file_num) {
            continue;
        }

        let hash_hex = crate::rdb::hex_encode(&entry.hash);
        let url = crate::rdb::cdn_url_from_hash(cdn_base, &entry.hash);
        tasks.push((entry.rdb_type, entry.id, entry.file_num, entry.offset, entry.length, url, hash_hex));
    }

    // Sort by (file_num, offset) for sequential disk writes — reduces NTFS random I/O
    tasks.sort_by_key(|(_, _, file_num, offset, _, _, _)| (*file_num, *offset));

    let files_total = tasks.len() as u32;
    let total_bytes: u64 = tasks.iter().map(|(_, _, _, _, len, _, _)| *len as u64).sum();

    if tasks.is_empty() {
        reporter.on_download(&crate::download::DownloadProgress {
            bytes_downloaded: 0, total_bytes: 0,
            files_completed: 0, files_total: 0,
            speed_bps: 0, current_file: String::new(),
            phase: "complete".into(), failed_files: 0,
        });
        return Ok(());
    }

    log::info!("RDB download plan: {} resources, {:.1} MB", files_total, total_bytes as f64 / 1_048_576.0);

    // Step 5: Download resources and write directly to rdbdata
    reporter.on_download(&crate::download::DownloadProgress {
        bytes_downloaded: 0, total_bytes,
        files_completed: 0, files_total,
        speed_bps: 0, current_file: "Downloading game resources...".into(),
        phase: "downloading".into(), failed_files: 0,
    });

    // Adapt concurrency to system resources.
    let available_ram_mb = crate::sys::available_ram_mb();
    let cpu_cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);

    // CPU slots: 1/3 of cores, minimum 1. Below-normal priority keeps us polite.
    let decompress_concurrent = (cpu_cores / 3).max(1);
    let cpu_semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(decompress_concurrent));

    // Download slots: most resources are tiny (<10KB) and process instantly,
    // so high download concurrency keeps the network saturated. The large
    // semaphore separately limits big files that eat memory.
    let main_concurrent = if available_ram_mb > 8000 {
        64
    } else if available_ram_mb > 4000 {
        32
    } else {
        16
    };

    // Large resource slots: limits how many big files (>1MB, up to 368MB) sit
    // in memory at once. Capped by RAM to avoid pressure on low-end systems.
    let large_concurrent = if available_ram_mb > 8000 {
        4
    } else if available_ram_mb > 4000 {
        2
    } else {
        1
    };

    log::info!("System: {} cores, {}MB RAM | download={}, decompress={}, large={}", cpu_cores, available_ram_mb, main_concurrent, decompress_concurrent, large_concurrent);

    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(main_concurrent));
    let large_semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(large_concurrent));
    let bytes_downloaded = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let files_completed = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let files_failed = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let files_processing = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let speed_tracker = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::download::SpeedTracker::new(std::time::Duration::from_secs(15))
    ));

    let dl_config = crate::download::DownloadConfig::default();
    let client = reqwest::Client::builder()
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .pool_max_idle_per_host(128)
        .tcp_keepalive(std::time::Duration::from_secs(30))
        .user_agent("")
        .connect_timeout(dl_config.connect_timeout)
        .build()
        .map_err(|e| format!("Failed to create download client: {}", e))?;

    let mut handles = Vec::with_capacity(tasks.len());

    for (rdb_type, id, file_num, offset, _length, url, hash_hex) in tasks {
        if cancel_flag.load(Ordering::Relaxed) {
            break;
        }
        while pause_flag.load(Ordering::Relaxed) {
            if cancel_flag.load(Ordering::Relaxed) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        if cancel_flag.load(Ordering::Relaxed) {
            break;
        }

        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let client = client.clone();
        let reporter_task = reporter.clone();
        let bytes_dl = bytes_downloaded.clone();
        let files_comp = files_completed.clone();
        let files_fail = files_failed.clone();
        let files_proc = files_processing.clone();
        let tracker = speed_tracker.clone();
        let large_sem = large_semaphore.clone();
        let cpu_sem = cpu_semaphore.clone();
        let writer = rdb_writer.clone();
        let is_large = _length > 1_000_000; // Resources > 1MB need large semaphore
        let cdn_size = _length as u64;
        let ft = files_total;
        let tb = total_bytes;

        handles.push(tokio::spawn(async move {
            let max_retries = 3u32;
            let mut downloaded_body = None;

            // Phase 1: Download (network-bound, keep permit for backpressure)
            {
                let _permit = permit;
                let _large_permit = if is_large {
                    Some(large_sem.clone().acquire_owned().await.unwrap())
                } else {
                    None
                };

                for attempt in 0..=max_retries {
                    if attempt > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
                    }

                    let resp = match client.get(&url).send().await {
                        Ok(r) if r.status().is_success() => r,
                        _ => continue,
                    };
                    let body = match resp.bytes().await {
                        Ok(b) => b,
                        Err(_) => continue,
                    };

                    downloaded_body = Some(body);
                    break;
                }
            } // download permit + large permit released here

            // Record network bytes as soon as download completes (before decompression).
            // This drives the speed display so users see actual download speed,
            // not processing throughput.
            let body = match downloaded_body {
                Some(b) => b,
                None => {
                    files_fail.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            };
            let net_bytes = bytes_dl.fetch_add(cdn_size, Ordering::Relaxed) + cdn_size;
            {
                let mut t = tracker.lock().await;
                t.record(net_bytes);
            }
            files_proc.fetch_add(1, Ordering::Relaxed);

            // Acquire large permit for decompress+write phase too (if large resource).
            // This prevents memory bloat from multiple large resources in-flight.
            let _large_permit2 = if is_large {
                Some(large_sem.clone().acquire_owned().await.unwrap())
            } else {
                None
            };

            let _cpu_permit = cpu_sem.acquire_owned().await.unwrap();

            let decompress_result = tokio::task::spawn_blocking(move || {
                let decompressed = crate::verify::decompress_cdn(&body)?;
                writer.write_resource(file_num, rdb_type, id, offset, &decompressed)?;
                Ok::<usize, String>(decompressed.len())
            }).await;

            drop(_cpu_permit);
            drop(_large_permit2);

            match decompress_result {
                Ok(Ok(_size)) => {
                    files_proc.fetch_sub(1, Ordering::Relaxed);
                    let completed = files_comp.fetch_add(1,
                        Ordering::Relaxed) + 1;
                    let cur_bytes = bytes_dl.load(Ordering::Relaxed);

                    if completed % 50 == 0 || completed == ft {
                        let speed = { tracker.lock().await.speed_bps() };
                        let failed = files_fail.load(Ordering::Relaxed);
                        let processing = files_proc.load(Ordering::Relaxed);
                        let pct = (cur_bytes as f64 / tb as f64 * 100.0).min(100.0);
                        let speed_mb = speed as f64 / 1_048_576.0;

                        // If all bytes are downloaded but files are still processing,
                        // show "patching" phase with 0 speed (CPU-bound work remaining)
                        let (phase, display_speed) = if cur_bytes >= tb && processing > 0 {
                            ("patching", 0u64)
                        } else {
                            ("downloading", speed)
                        };

                        log::info!(
                            "{}: {:.1}% | {}/{} files | {:.1} MB/s | {} processing | {} failed",
                            phase, pct, completed, ft, speed_mb, processing, failed
                        );
                        reporter_task.on_download(&crate::download::DownloadProgress {
                            bytes_downloaded: cur_bytes,
                            total_bytes: tb,
                            files_completed: completed,
                            files_total: ft,
                            speed_bps: display_speed,
                            current_file: hash_hex.clone(),
                            phase: phase.into(),
                            failed_files: failed,
                        });
                    }
                }
                Ok(Err(e)) => {
                    files_proc.fetch_sub(1, Ordering::Relaxed);
                    log::warn!("Process failed for {}:{}: {}", rdb_type, id, e);
                    files_fail.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    files_proc.fetch_sub(1, Ordering::Relaxed);
                    log::warn!("Task panicked for {}:{}: {}", rdb_type, id, e);
                    files_fail.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    let final_failed = files_failed.load(Ordering::Relaxed);
    let final_bytes = bytes_downloaded.load(Ordering::Relaxed);
    log::info!(
        "Download complete: {}/{} files, {:.1} MB written, {} failed",
        files_total - final_failed, files_total,
        final_bytes as f64 / 1_048_576.0, final_failed
    );
    if final_failed > 0 {
        return Err(format!(
            "{} of {} resources failed to download",
            final_failed, files_total
        ));
    }

    // All resources written successfully — mark all file_nums as complete
    let completion_log_path = rdb_dir.join("rdb_completed.log");
    let mut all_file_nums: Vec<u8> = le_index.entries.iter()
        .filter(|e| e.file_num != 255)
        .map(|e| e.file_num)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    all_file_nums.sort();
    let log_content = all_file_nums.iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("\n") + "\n";
    let _ = std::fs::write(&completion_log_path, &log_content);

    Ok(())
}
