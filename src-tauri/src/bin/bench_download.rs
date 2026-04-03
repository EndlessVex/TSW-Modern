use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

/// Benchmark CDN download throughput at various concurrency levels.
/// Downloads real files from the Funcom CDN to measure actual speed.

#[derive(Debug, Clone)]
struct BenchTask {
    url: String,
    expected_size: u64,
    hash_hex: String,
}

fn parse_hash_index_entries(path: &Path) -> Vec<BenchTask> {
    let data = std::fs::read(path).expect("read hash index");
    let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let _num_entries = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let num_types = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
    let entry_size: usize = if version <= 4 { 29 } else { 47 };

    let mut tasks = Vec::new();
    let mut offset = 16usize;
    let base_url = "https://update.secretworld.com/tswupm";

    for _ in 0..num_types {
        let _rdb_type = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
        let count = u32::from_le_bytes([data[offset+4], data[offset+5], data[offset+6], data[offset+7]]);
        offset += 8;

        for _ in 0..count {
            let file_size = u32::from_le_bytes([data[offset+4], data[offset+5], data[offset+6], data[offset+7]]);
            let mut hash = [0u8; 16];
            hash.copy_from_slice(&data[offset+12..offset+28]);

            let hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
            let url = format!("{}/rdb/res/{}/{}/{}", base_url, &hex[0..2], &hex[2..3], &hex[3..]);

            tasks.push(BenchTask { url, expected_size: file_size as u64, hash_hex: hex });
            offset += entry_size;
        }
    }
    tasks
}

async fn bench_download(
    client: &reqwest::Client,
    tasks: &[BenchTask],
    concurrency: usize,
    max_files: usize,
    label: &str,
) {
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let bytes_downloaded = Arc::new(AtomicU64::new(0));
    let files_completed = Arc::new(AtomicU32::new(0));
    let files_failed = Arc::new(AtomicU32::new(0));

    let start = Instant::now();
    let mut handles = Vec::new();
    let task_count = tasks.len().min(max_files);

    for task in tasks.iter().take(max_files) {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let client = client.clone();
        let url = task.url.clone();
        let bytes_dl = bytes_downloaded.clone();
        let files_comp = files_completed.clone();
        let files_fail = files_failed.clone();

        let handle = tokio::spawn(async move {
            let _permit = permit;
            match client.get(&url).send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        match resp.bytes().await {
                            Ok(body) => {
                                bytes_dl.fetch_add(body.len() as u64, Ordering::Relaxed);
                                files_comp.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(_) => { files_fail.fetch_add(1, Ordering::Relaxed); }
                        }
                    } else {
                        files_fail.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Err(_) => { files_fail.fetch_add(1, Ordering::Relaxed); }
            }
        });
        handles.push(handle);
    }

    // Progress reporter
    let bytes_dl_report = bytes_downloaded.clone();
    let files_report = files_completed.clone();
    let label_owned = label.to_string();
    let reporter = tokio::spawn(async move {
        let mut last_bytes = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let current_bytes = bytes_dl_report.load(Ordering::Relaxed);
            let current_files = files_report.load(Ordering::Relaxed);
            let speed = (current_bytes - last_bytes) as f64 / 5.0 / 1_048_576.0;
            last_bytes = current_bytes;
            eprintln!("  [{}] {current_files}/{task_count} files, {:.1} MB downloaded, {speed:.1} MB/s",
                label_owned, current_bytes as f64 / 1_048_576.0);
        }
    });

    for handle in handles {
        let _ = handle.await;
    }
    reporter.abort();

    let elapsed = start.elapsed();
    let total_bytes = bytes_downloaded.load(Ordering::Relaxed);
    let total_files = files_completed.load(Ordering::Relaxed);
    let failed = files_failed.load(Ordering::Relaxed);
    let speed_mbps = total_bytes as f64 / elapsed.as_secs_f64() / 1_048_576.0;
    let files_per_sec = total_files as f64 / elapsed.as_secs_f64();

    println!("\n=== {label} ===");
    println!("  Concurrency: {concurrency}");
    println!("  Files: {total_files}/{task_count} completed, {failed} failed");
    println!("  Bytes: {:.1} MB in {:.1}s", total_bytes as f64 / 1_048_576.0, elapsed.as_secs_f64());
    println!("  Speed: {speed_mbps:.1} MB/s");
    println!("  Files/sec: {files_per_sec:.0}");
    println!("  Avg latency: {:.1}ms/file", elapsed.as_millis() as f64 / total_files.max(1) as f64 * concurrency as f64);
}

#[tokio::main]
async fn main() {
    let hash_index_path = Path::new("../The Secret World/RDB/RDBHashIndex.bin");
    if !hash_index_path.exists() {
        eprintln!("Hash index not found at {:?}", hash_index_path);
        std::process::exit(1);
    }

    println!("Parsing hash index...");
    let mut all_tasks = parse_hash_index_entries(hash_index_path);

    // Deduplicate by hash
    all_tasks.sort_by(|a, b| a.hash_hex.cmp(&b.hash_hex));
    all_tasks.dedup_by(|a, b| a.hash_hex == b.hash_hex);

    // Split into size categories
    let tiny: Vec<_> = all_tasks.iter().filter(|t| t.expected_size < 1024).cloned().collect();
    let small: Vec<_> = all_tasks.iter().filter(|t| t.expected_size >= 1024 && t.expected_size < 100*1024).cloned().collect();
    let medium: Vec<_> = all_tasks.iter().filter(|t| t.expected_size >= 100*1024 && t.expected_size < 1024*1024).cloned().collect();
    let large: Vec<_> = all_tasks.iter().filter(|t| t.expected_size >= 1024*1024).cloned().collect();

    println!("Total unique files: {}", all_tasks.len());
    println!("Tiny (<1KB): {}", tiny.len());
    println!("Small (1KB-100KB): {}", small.len());
    println!("Medium (100KB-1MB): {}", medium.len());
    println!("Large (>1MB): {}", large.len());

    // Build HTTP client with connection pooling
    let client = reqwest::Client::builder()
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(100)
        .tcp_keepalive(Duration::from_secs(30))
        .build()
        .expect("build client");

    println!("\n--- Running benchmarks (1000 files each category) ---\n");

    // Test different concurrency levels with tiny files
    for concurrency in [32, 64, 128, 256] {
        bench_download(&client, &tiny, concurrency, 1000,
            &format!("Tiny <1KB @ {concurrency}")).await;
    }

    // Test medium files at different concurrencies
    for concurrency in [32, 64, 128] {
        bench_download(&client, &medium, concurrency, 200,
            &format!("Medium 100KB-1MB @ {concurrency}")).await;
    }

    // Test large files
    bench_download(&client, &large, 16, 50, "Large >1MB @ 16").await;

    println!("\n--- Testing if CDN rate-limits ---\n");

    // Sustained download of 5000 tiny files at 128 concurrent
    bench_download(&client, &tiny, 128, 5000, "Sustained tiny @ 128 (5000 files)").await;
}
