use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
use std::collections::HashMap;

/// Diagnostic download: measures CDN time vs disk time vs manifest time
/// to identify exactly what causes the 30% slowdown.

#[derive(Debug, Clone)]
struct BenchTask {
    url: String,
    expected_size: u64,
    hash_hex: String,
}

fn parse_hash_index_entries(path: &Path) -> Vec<BenchTask> {
    let data = std::fs::read(path).expect("read hash index");
    let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let num_types = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
    let entry_size: usize = if version <= 4 { 29 } else { 47 };

    let mut tasks = Vec::new();
    let mut seen = std::collections::HashSet::new();
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
            if seen.insert(hex.clone()) {
                let url = format!("{}/rdb/res/{}/{}/{}", base_url, &hex[0..2], &hex[2..3], &hex[3..]);
                tasks.push(BenchTask { url, expected_size: file_size as u64, hash_hex: hex });
            }
            offset += entry_size;
        }
    }
    tasks
}

/// Download to memory only — no disk writes. Measures pure CDN throughput.
async fn bench_cdn_only(client: &reqwest::Client, tasks: &[BenchTask], count: usize) {
    println!("\n=== TEST 1: CDN-only (no disk writes) — {} files ===", count);
    let sem = Arc::new(Semaphore::new(128));
    let bytes = Arc::new(AtomicU64::new(0));
    let files = Arc::new(AtomicU32::new(0));
    let start = Instant::now();

    let mut handles = Vec::new();
    for task in tasks.iter().take(count) {
        let permit = sem.clone().acquire_owned().await.unwrap();
        let c = client.clone();
        let url = task.url.clone();
        let b = bytes.clone();
        let f = files.clone();
        handles.push(tokio::spawn(async move {
            let _p = permit;
            if let Ok(resp) = c.get(&url).send().await {
                if let Ok(body) = resp.bytes().await {
                    b.fetch_add(body.len() as u64, Ordering::Relaxed);
                    f.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }

    // Progress
    let b2 = bytes.clone();
    let f2 = files.clone();
    let reporter = tokio::spawn(async move {
        let mut last = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let cur = b2.load(Ordering::Relaxed);
            let fc = f2.load(Ordering::Relaxed);
            let speed = (cur - last) as f64 / 5.0 / 1_048_576.0;
            last = cur;
            eprintln!("  CDN-only: {fc}/{count} files, {:.1} MB, {speed:.1} MB/s", cur as f64/1_048_576.0);
        }
    });

    for h in handles { let _ = h.await; }
    reporter.abort();

    let elapsed = start.elapsed();
    let total = bytes.load(Ordering::Relaxed);
    println!("  {} files, {:.1} MB in {:.1}s = {:.1} MB/s, {:.0} files/sec",
        files.load(Ordering::Relaxed),
        total as f64 / 1_048_576.0, elapsed.as_secs_f64(),
        total as f64 / elapsed.as_secs_f64() / 1_048_576.0,
        files.load(Ordering::Relaxed) as f64 / elapsed.as_secs_f64());
}

/// Download with disk writes — measures combined CDN + disk throughput.
async fn bench_with_disk(client: &reqwest::Client, tasks: &[BenchTask], staging: &Path, count: usize) {
    println!("\n=== TEST 2: CDN + disk writes — {} files ===", count);
    let _ = std::fs::remove_dir_all(staging);
    std::fs::create_dir_all(staging).unwrap();

    let sem = Arc::new(Semaphore::new(128));
    let bytes = Arc::new(AtomicU64::new(0));
    let files = Arc::new(AtomicU32::new(0));
    let disk_time = Arc::new(AtomicU64::new(0)); // nanoseconds
    let start = Instant::now();

    let mut handles = Vec::new();
    for task in tasks.iter().take(count) {
        let permit = sem.clone().acquire_owned().await.unwrap();
        let c = client.clone();
        let url = task.url.clone();
        let dest = staging.join(&task.hash_hex[..2]).join(&task.hash_hex);
        let b = bytes.clone();
        let f = files.clone();
        let dt = disk_time.clone();
        handles.push(tokio::spawn(async move {
            let _p = permit;
            if let Ok(resp) = c.get(&url).send().await {
                if let Ok(body) = resp.bytes().await {
                    let disk_start = Instant::now();
                    if let Some(parent) = dest.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    let _ = tokio::fs::write(&dest, &body).await;
                    dt.fetch_add(disk_start.elapsed().as_nanos() as u64, Ordering::Relaxed);

                    b.fetch_add(body.len() as u64, Ordering::Relaxed);
                    f.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }

    let b2 = bytes.clone();
    let f2 = files.clone();
    let reporter = tokio::spawn(async move {
        let mut last = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let cur = b2.load(Ordering::Relaxed);
            let fc = f2.load(Ordering::Relaxed);
            let speed = (cur - last) as f64 / 5.0 / 1_048_576.0;
            last = cur;
            eprintln!("  disk: {fc}/{count} files, {:.1} MB, {speed:.1} MB/s", cur as f64/1_048_576.0);
        }
    });

    for h in handles { let _ = h.await; }
    reporter.abort();

    let elapsed = start.elapsed();
    let total = bytes.load(Ordering::Relaxed);
    let disk_ns = disk_time.load(Ordering::Relaxed);
    println!("  {} files, {:.1} MB in {:.1}s = {:.1} MB/s",
        files.load(Ordering::Relaxed),
        total as f64 / 1_048_576.0, elapsed.as_secs_f64(),
        total as f64 / elapsed.as_secs_f64() / 1_048_576.0);
    println!("  Total disk write time: {:.1}s ({:.1}% of wall time)",
        disk_ns as f64 / 1e9,
        disk_ns as f64 / 1e9 / elapsed.as_secs_f64() * 100.0 / 128.0);

    let _ = std::fs::remove_dir_all(staging);
}

/// Download with disk + manifest — the full pipeline.
async fn bench_full_pipeline(client: &reqwest::Client, tasks: &[BenchTask], staging: &Path, count: usize) {
    println!("\n=== TEST 3: Full pipeline (CDN + disk + manifest) — {} files ===", count);
    let _ = std::fs::remove_dir_all(staging);
    std::fs::create_dir_all(staging).unwrap();

    let sem = Arc::new(Semaphore::new(128));
    let bytes = Arc::new(AtomicU64::new(0));
    let files = Arc::new(AtomicU32::new(0));
    let manifest: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let manifest_time = Arc::new(AtomicU64::new(0));
    let manifest_path = staging.join("manifest.json");
    let start = Instant::now();

    let mut handles = Vec::new();
    for task in tasks.iter().take(count) {
        let permit = sem.clone().acquire_owned().await.unwrap();
        let c = client.clone();
        let url = task.url.clone();
        let hash = task.hash_hex.clone();
        let dest = staging.join(&task.hash_hex[..2]).join(&task.hash_hex);
        let b = bytes.clone();
        let f = files.clone();
        let m = manifest.clone();
        let mt = manifest_time.clone();
        let mp = manifest_path.clone();
        let count_u32 = count as u32;
        handles.push(tokio::spawn(async move {
            let _p = permit;
            if let Ok(resp) = c.get(&url).send().await {
                if let Ok(body) = resp.bytes().await {
                    if let Some(parent) = dest.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    let _ = tokio::fs::write(&dest, &body).await;
                    b.fetch_add(body.len() as u64, Ordering::Relaxed);
                    let completed = f.fetch_add(1, Ordering::Relaxed) + 1;

                    // Manifest update
                    let ms = Instant::now();
                    {
                        let mut guard = m.lock().await;
                        guard.insert(hash, "complete".to_string());
                        if completed % 50 == 0 || completed == count_u32 {
                            let json = serde_json::to_string(&*guard).unwrap_or_default();
                            let _ = tokio::fs::write(&mp, json.as_bytes()).await;
                        }
                    }
                    mt.fetch_add(ms.elapsed().as_nanos() as u64, Ordering::Relaxed);
                }
            }
        }));
    }

    let b2 = bytes.clone();
    let f2 = files.clone();
    let m2 = manifest_time.clone();
    let reporter = tokio::spawn(async move {
        let mut last_bytes = 0u64;
        let mut last_files = 0u32;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let cur = b2.load(Ordering::Relaxed);
            let fc = f2.load(Ordering::Relaxed);
            let mt = m2.load(Ordering::Relaxed);
            let speed = (cur - last_bytes) as f64 / 5.0 / 1_048_576.0;
            let frate = (fc - last_files) as f64 / 5.0;
            last_bytes = cur;
            last_files = fc;
            eprintln!("  full: {fc}/{count} files, {:.1} MB, {speed:.1} MB/s, {frate:.0} f/s, manifest_time={:.1}s",
                cur as f64/1_048_576.0, mt as f64/1e9);
        }
    });

    for h in handles { let _ = h.await; }
    reporter.abort();

    let elapsed = start.elapsed();
    let total = bytes.load(Ordering::Relaxed);
    let mt = manifest_time.load(Ordering::Relaxed);
    println!("  {} files, {:.1} MB in {:.1}s = {:.1} MB/s",
        files.load(Ordering::Relaxed),
        total as f64 / 1_048_576.0, elapsed.as_secs_f64(),
        total as f64 / elapsed.as_secs_f64() / 1_048_576.0);
    println!("  Manifest time: {:.1}s total ({:.1}% of wall time per stream)",
        mt as f64/1e9, mt as f64/1e9 / elapsed.as_secs_f64() * 100.0 / 128.0);
    // Check manifest size
    if let Ok(meta) = std::fs::metadata(&manifest_path) {
        println!("  Manifest file size: {:.1} MB", meta.len() as f64 / 1_048_576.0);
    }

    let _ = std::fs::remove_dir_all(staging);
}

/// Sustained download with periodic speed reporting — simulates real patch.
async fn bench_sustained(client: &reqwest::Client, tasks: &[BenchTask], staging: &Path, count: usize) {
    println!("\n=== TEST 4: Sustained download — {} files (watching for degradation) ===", count);
    let _ = std::fs::remove_dir_all(staging);
    std::fs::create_dir_all(staging).unwrap();

    let sem = Arc::new(Semaphore::new(128));
    let bytes = Arc::new(AtomicU64::new(0));
    let files = Arc::new(AtomicU32::new(0));
    let start = Instant::now();

    let mut handles = Vec::new();
    for task in tasks.iter().take(count) {
        let permit = sem.clone().acquire_owned().await.unwrap();
        let c = client.clone();
        let url = task.url.clone();
        let dest = staging.join(&task.hash_hex[..2]).join(&task.hash_hex);
        let b = bytes.clone();
        let f = files.clone();
        handles.push(tokio::spawn(async move {
            let _p = permit;
            if let Ok(resp) = c.get(&url).send().await {
                if let Ok(body) = resp.bytes().await {
                    if let Some(parent) = dest.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    let _ = tokio::fs::write(&dest, &body).await;
                    b.fetch_add(body.len() as u64, Ordering::Relaxed);
                    f.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }

    let b2 = bytes.clone();
    let f2 = files.clone();
    let count_f = count as f64;
    let reporter = tokio::spawn(async move {
        let mut last = 0u64;
        let mut interval = 0u32;
        println!("  {:>6} {:>8} {:>10} {:>10} {:>10}", "Time", "Files", "MB", "MB/s", "Files/s");
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            interval += 1;
            let cur = b2.load(Ordering::Relaxed);
            let fc = f2.load(Ordering::Relaxed);
            let speed = (cur - last) as f64 / 10.0 / 1_048_576.0;
            let frate = fc as f64 / (interval as f64 * 10.0);
            let pct = fc as f64 / count_f * 100.0;
            last = cur;
            println!("  {:>5}s {:>6} ({:>5.1}%) {:>8.1} MB {:>8.1} {:>8.0}",
                interval * 10, fc, pct, cur as f64/1_048_576.0, speed, frate);
        }
    });

    for h in handles { let _ = h.await; }
    reporter.abort();

    let elapsed = start.elapsed();
    let total = bytes.load(Ordering::Relaxed);
    println!("\n  TOTAL: {} files, {:.1} MB in {:.1}s = {:.1} MB/s avg",
        files.load(Ordering::Relaxed),
        total as f64 / 1_048_576.0, elapsed.as_secs_f64(),
        total as f64 / elapsed.as_secs_f64() / 1_048_576.0);

    let _ = std::fs::remove_dir_all(staging);
}

#[tokio::main]
async fn main() {
    let hash_index_path = Path::new("../The Secret World/RDB/RDBHashIndex.bin");
    if !hash_index_path.exists() {
        eprintln!("Hash index not found");
        std::process::exit(1);
    }

    println!("Parsing hash index...");
    let tasks = parse_hash_index_entries(hash_index_path);
    println!("Total unique files: {}", tasks.len());

    let client = reqwest::Client::builder()
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(128)
        .tcp_keepalive(Duration::from_secs(30))
        .build()
        .unwrap();

    let staging = PathBuf::from("../bench_staging");

    // Test 1: Pure CDN speed (10K files)
    bench_cdn_only(&client, &tasks, 10000).await;

    // Test 2: CDN + disk (10K files)
    bench_with_disk(&client, &tasks, &staging, 10000).await;

    // Test 3: Full pipeline with manifest (10K files)
    bench_full_pipeline(&client, &tasks, &staging, 10000).await;

    // Test 4: Sustained 50K files with periodic reporting (watch for degradation)
    bench_sustained(&client, &tasks, &staging, 50000).await;
}
