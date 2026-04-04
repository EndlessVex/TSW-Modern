use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, Semaphore};

use crate::config::{parse_local_config, ConfigParseError};
use crate::rdb::{
    cdn_url_from_hash, hex_encode, parse_hash_index, parse_le_index, LeIndex, RdbHashIndex,
    RdbParseError,
};

// ─── Configuration ───────────────────────────────────────────────────────────

/// Configuration for the download engine.
#[derive(Debug, Clone)]
pub struct DownloadConfig {
    /// Maximum concurrent downloads (Semaphore bound). Default: 8.
    pub max_concurrent: usize,
    /// Maximum retry attempts per file on CDN errors. Default: 3.
    pub max_retries: u32,
    /// HTTP connect timeout. Default: 30s.
    pub connect_timeout: Duration,
    /// HTTP read timeout. Default: 60s.
    pub read_timeout: Duration,
    /// Connection pool idle timeout. Default: 90s.
    pub pool_idle_timeout: Duration,
    /// Maximum idle connections per host. Default: 10.
    pub pool_max_idle_per_host: usize,
    /// Base CDN URL (e.g. `http://update.secretworld.com/tswupm/TSWLiveSteam`).
    pub cdn_base_url: String,
    /// Directory where downloaded files are staged before integration.
    pub staging_dir: PathBuf,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 8,
            max_retries: 3,
            connect_timeout: Duration::from_secs(30),
            read_timeout: Duration::from_secs(60),
            pool_idle_timeout: Duration::from_secs(90),
            pool_max_idle_per_host: 128,
            cdn_base_url: String::new(),
            staging_dir: PathBuf::from("staging"),
        }
    }
}

// ─── Progress ────────────────────────────────────────────────────────────────

/// Real-time download progress emitted to the frontend via Tauri events.
#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
    pub files_completed: u32,
    pub files_total: u32,
    pub speed_bps: u64,
    pub current_file: String,
    /// Phase of the download pipeline: checking / downloading / complete / error.
    pub phase: String,
    pub failed_files: u32,
}

// ─── Download manifest (resume support) ──────────────────────────────────────

/// Per-file download state for resume support.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FileState {
    Pending,
    Partial { bytes_downloaded: u64 },
    Complete,
    Failed { reason: String },
}

/// Manifest tracking download progress across restarts.
/// Persisted to `{staging_dir}/download_manifest.json` for backward compat,
/// but completions are also tracked via an append-only log for performance.
/// On load, reads the append log first (fast), falls back to JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadManifest {
    /// Map from hex hash string → per-file state.
    pub files: HashMap<String, FileState>,
}

impl DownloadManifest {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    /// Load manifest from the append log (fast) or JSON fallback.
    pub fn load(staging_dir: &Path) -> Self {
        let log_path = staging_dir.join("completed.log");
        let json_path = staging_dir.join("download_manifest.json");

        // Try append log first — one hash per line, much faster to parse
        if let Ok(data) = std::fs::read_to_string(&log_path) {
            let mut manifest = Self::new();
            for line in data.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    manifest.files.insert(trimmed.to_string(), FileState::Complete);
                }
            }
            if !manifest.files.is_empty() {
                return manifest;
            }
        }

        // Fall back to JSON
        match std::fs::read_to_string(&json_path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_else(|e| {
                log::warn!("Corrupt download manifest at {}: {e}, starting fresh", json_path.display());
                Self::new()
            }),
            Err(_) => Self::new(),
        }
    }

    /// Persist manifest to JSON (for backward compat / final state).
    pub fn save(&self, staging_dir: &Path) -> io::Result<()> {
        let path = staging_dir.join("download_manifest.json");
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        std::fs::write(&path, data)
    }

    /// Get the byte offset for a file if it's partially downloaded.
    pub fn partial_bytes(&self, hash_hex: &str) -> u64 {
        match self.files.get(hash_hex) {
            Some(FileState::Partial { bytes_downloaded }) => *bytes_downloaded,
            _ => 0,
        }
    }
}

// ─── Download task / result types ────────────────────────────────────────────

/// A single file to download from the CDN.
#[derive(Debug, Clone)]
pub struct DownloadTask {
    /// Full CDN URL for this resource.
    pub url: String,
    /// Destination path within the staging directory.
    pub dest_path: PathBuf,
    /// Expected MD5 hash (16 bytes).
    pub expected_hash: [u8; 16],
    /// Expected file size from the hash index.
    pub expected_size: u64,
    /// Bytes already downloaded (for resume).
    pub partial_bytes: u64,
    /// Hex string of the hash (used as manifest key).
    pub hash_hex: String,
}

/// Result of a complete download run.
#[derive(Debug, Serialize)]
pub struct DownloadResult {
    pub files_completed: u32,
    pub files_failed: u32,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
}

/// Status returned by `check_for_updates`.
#[derive(Debug, Clone, Serialize)]
pub struct PatchStatus {
    pub up_to_date: bool,
    pub files_to_download: u32,
    pub total_bytes: u64,
}

// ─── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum DownloadError {
    Http(reqwest::Error),
    Io(io::Error),
    HashMismatch {
        expected: String,
        got: String,
        path: PathBuf,
    },
    SizeMismatch {
        expected: u64,
        got: u64,
    },
    RdbParse(RdbParseError),
    ConfigParse(ConfigParseError),
    RetriesExhausted {
        url: String,
        last_error: String,
    },
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DownloadError::Http(e) => write!(f, "HTTP error: {e}"),
            DownloadError::Io(e) => write!(f, "I/O error: {e}"),
            DownloadError::HashMismatch {
                expected,
                got,
                path,
            } => write!(
                f,
                "MD5 mismatch for {}: expected {expected}, got {got}",
                path.display()
            ),
            DownloadError::SizeMismatch { expected, got } => {
                write!(f, "Content-Length mismatch: expected {expected}, got {got}")
            }
            DownloadError::RdbParse(e) => write!(f, "RDB parse error: {e}"),
            DownloadError::ConfigParse(e) => write!(f, "Config parse error: {e}"),
            DownloadError::RetriesExhausted { url, last_error } => {
                write!(f, "retries exhausted for {url}: {last_error}")
            }
        }
    }
}

impl std::error::Error for DownloadError {}

impl From<reqwest::Error> for DownloadError {
    fn from(e: reqwest::Error) -> Self {
        DownloadError::Http(e)
    }
}

impl From<io::Error> for DownloadError {
    fn from(e: io::Error) -> Self {
        DownloadError::Io(e)
    }
}

impl From<RdbParseError> for DownloadError {
    fn from(e: RdbParseError) -> Self {
        DownloadError::RdbParse(e)
    }
}

impl From<ConfigParseError> for DownloadError {
    fn from(e: ConfigParseError) -> Self {
        DownloadError::ConfigParse(e)
    }
}

// ─── Client factory ──────────────────────────────────────────────────────────

/// Build a reqwest::Client with connection pool and timeout settings.
pub fn create_client(config: &DownloadConfig) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .connect_timeout(config.connect_timeout)
        .read_timeout(config.read_timeout)
        .pool_idle_timeout(config.pool_idle_timeout)
        .pool_max_idle_per_host(config.pool_max_idle_per_host)
        // CDN uses plain HTTP
        .danger_accept_invalid_certs(false)
        .user_agent("TSW-Modern-Launcher/0.1")
        .build()
}

// ─── Single-file download with resume ────────────────────────────────────────

/// Download a single file from the CDN, verifying its size.
///
/// If `partial_bytes > 0`, sends a `Range` header to resume from that offset.
/// Streams the response body to disk and verifies size after completion.
/// The index hash is a resource identifier (used in URLs), not a content hash
/// — CDN-served files are zlib-compressed with an IOz1 header.
/// Returns the total number of new bytes written.
pub async fn download_single_file(
    client: &reqwest::Client,
    url: &str,
    dest_path: &Path,
    expected_size: u64,
    partial_bytes: u64,
) -> Result<u64, DownloadError> {
    use futures::StreamExt;

    let mut request = client.get(url);

    // Resume support: seek to partial offset
    let append = partial_bytes > 0;
    if append {
        request = request.header("Range", format!("bytes={}-", partial_bytes));
    }

    let response = request.send().await?.error_for_status().map_err(|e| {
        DownloadError::Http(e)
    })?;

    // Validate Content-Length when available and not resuming
    if !append {
        if let Some(content_length) = response.content_length() {
            if expected_size > 0 && content_length != expected_size {
                return Err(DownloadError::SizeMismatch {
                    expected: expected_size,
                    got: content_length,
                });
            }
        }
    }

    // Small files (<64KB): download entire body to memory, write in one shot.
    // For very small files (<4KB), append to a shared blob file instead of
    // creating individual files — reduces NTFS file create overhead dramatically.
    // 49% of files are under 1KB; individual NTFS creates + Defender scanning
    // is the primary cause of download speed degradation on Windows.
    if expected_size < 65536 && !append {
        let body = response.bytes().await.map_err(DownloadError::Http)?;
        let bytes_written = body.len() as u64;

        // Size check
        if expected_size > 0 && bytes_written != expected_size {
            return Err(DownloadError::SizeMismatch {
                expected: expected_size,
                got: bytes_written,
            });
        }

        // Write to individual file — parent dirs pre-created by run_downloads
        tokio::fs::write(dest_path, &body).await?;

        return Ok(bytes_written);
    }

    // Ensure parent directory exists for large file streaming path
    // (may be outside the pre-created hex directories)
    if let Some(parent) = dest_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // Open file for writing (append if resuming, create if fresh)
    let mut file = if append {
        tokio::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(dest_path)
            .await?
    } else {
        tokio::fs::File::create(dest_path).await?
    };

    // Stream body to disk
    let mut stream = response.bytes_stream();
    let mut bytes_written: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(DownloadError::Http)?;
        file.write_all(&chunk).await?;
        bytes_written += chunk.len() as u64;
    }

    file.flush().await?;
    drop(file);

    // Verify file size matches expected
    if expected_size > 0 {
        let metadata = tokio::fs::metadata(dest_path).await?;
        let actual_total = metadata.len();
        let expected_total = if append {
            expected_size // total file size expected
        } else {
            expected_size
        };
        if actual_total != expected_total {
            let _ = tokio::fs::remove_file(dest_path).await;
            return Err(DownloadError::SizeMismatch {
                expected: expected_total,
                got: actual_total,
            });
        }
    }

    Ok(bytes_written)
}

// ─── Download plan computation ───────────────────────────────────────────────

/// The known frozen root hash of TSW's final le.idx.
/// TSW servers were shut down in 2018 — this hash will never change.
pub const TSW_FROZEN_ROOT_HASH: &str = "2018d1a9a0cff05d7f318cb68afffcc1";

/// Compute which files need to be downloaded by cross-referencing
/// the le.idx index with the hash index and existing manifest state.
///
/// Skips entries with `file_num == 255` (server-only resources).
/// Skips entries already marked `Complete` in the manifest.
/// For entries marked `Partial`, sets the correct byte offset.
pub fn compute_download_plan(
    le_index: &LeIndex,
    hash_index: &RdbHashIndex,
    cdn_base_url: &str,
    staging_dir: &Path,
    manifest: &DownloadManifest,
) -> Vec<DownloadTask> {
    let mut tasks = Vec::new();

    for entry in &le_index.entries {
        // Skip server-only entries
        if entry.file_num == 255 {
            continue;
        }

        let hash_hex = hex_encode(&entry.hash);

        // Skip already-completed files
        if matches!(manifest.files.get(&hash_hex), Some(FileState::Complete)) {
            continue;
        }

        // Look up expected file size from hash index
        let expected_size = hash_index
            .entries
            .get(&(entry.rdb_type, entry.id))
            .map(|h| h.file_size as u64)
            .unwrap_or(0);

        let url = cdn_url_from_hash(cdn_base_url, &entry.hash);

        // Destination: staging_dir/<first 2 hex chars>/<hash_hex>
        let dest_path = staging_dir
            .join(&hash_hex[..2])
            .join(&hash_hex);

        let partial_bytes = manifest.partial_bytes(&hash_hex);

        tasks.push(DownloadTask {
            url,
            dest_path,
            expected_hash: entry.hash,
            expected_size,
            partial_bytes,
            hash_hex,
        });
    }

    // Deduplicate by hash — multiple le.idx entries can reference the same resource
    tasks.sort_by(|a, b| a.hash_hex.cmp(&b.hash_hex));
    tasks.dedup_by(|a, b| a.hash_hex == b.hash_hex);

    tasks
}

/// Compute download plan from hash index alone (no le.idx needed).
/// Used for fresh installs where le.idx doesn't exist yet.
/// The hash index contains all the same data: type, id, hash, file_num, offset, length.
pub fn compute_download_plan_from_hash_index(
    hash_index: &RdbHashIndex,
    cdn_base_url: &str,
    staging_dir: &Path,
    manifest: &DownloadManifest,
) -> Vec<DownloadTask> {
    let mut tasks = Vec::new();

    for ((_rdb_type, _id), entry) in &hash_index.entries {
        let hash_hex = hex_encode(&entry.hash);

        // Skip already-completed files
        if matches!(manifest.files.get(&hash_hex), Some(FileState::Complete)) {
            continue;
        }

        let url = cdn_url_from_hash(cdn_base_url, &entry.hash);
        let dest_path = staging_dir.join(&hash_hex[..2]).join(&hash_hex);
        let partial_bytes = manifest.partial_bytes(&hash_hex);

        tasks.push(DownloadTask {
            url,
            dest_path,
            expected_hash: entry.hash,
            expected_size: entry.file_size as u64,
            partial_bytes,
            hash_hex,
        });
    }

    // Deduplicate by hash
    tasks.sort_by(|a, b| a.hash_hex.cmp(&b.hash_hex));
    tasks.dedup_by(|a, b| a.hash_hex == b.hash_hex);

    tasks
}

// ─── Speed tracking ──────────────────────────────────────────────────────────

/// Rolling window speed calculator using time-bucketed byte counts.
/// More stable than per-file samples — records bytes every time they're written,
/// not just when files complete. Produces smooth speed readings even for tiny files.
pub struct SpeedTracker {
    /// (timestamp, cumulative_bytes) snapshots taken periodically
    snapshots: Vec<(Instant, u64)>,
    /// Window duration for speed calculation
    window: Duration,
}

impl SpeedTracker {
    pub fn new(window: Duration) -> Self {
        Self {
            snapshots: Vec::new(),
            window,
        }
    }

    pub fn record(&mut self, cumulative_bytes: u64) {
        let now = Instant::now();
        // Only keep one snapshot per 200ms to avoid memory bloat
        if let Some(last) = self.snapshots.last() {
            if now.duration_since(last.0) < Duration::from_millis(200) {
                // Update the last snapshot's byte count instead of adding new
                if let Some(last_mut) = self.snapshots.last_mut() {
                    last_mut.1 = cumulative_bytes;
                }
                return;
            }
        }
        self.snapshots.push((now, cumulative_bytes));
        // Prune old snapshots
        let cutoff = now - self.window - Duration::from_secs(1);
        self.snapshots.retain(|(t, _)| *t >= cutoff);
    }

    pub fn speed_bps(&self) -> u64 {
        if self.snapshots.len() < 2 {
            return 0;
        }
        // Calculate speed over the last `window` duration
        let now = self.snapshots.last().unwrap().0;
        let cutoff = now - self.window;
        // Find the earliest snapshot within the window
        let earliest = self.snapshots.iter().find(|(t, _)| *t >= cutoff);
        let latest = self.snapshots.last().unwrap();
        if let Some(earliest) = earliest {
            let elapsed = latest.0.duration_since(earliest.0).as_secs_f64();
            if elapsed < 0.5 {
                return 0;
            }
            let bytes_delta = latest.1.saturating_sub(earliest.1);
            (bytes_delta as f64 / elapsed) as u64
        } else {
            0
        }
    }
}

// ─── Parallel download orchestrator ──────────────────────────────────────────

/// Run all downloads in parallel with bounded concurrency, retry, and progress.
///
/// Emits `patch:progress` events through the Tauri app handle.
/// Persists the manifest after each file completes for resume support.
pub async fn run_downloads(
    app_handle: &tauri::AppHandle,
    config: &DownloadConfig,
    client: &reqwest::Client,
    tasks: Vec<DownloadTask>,
    manifest: Arc<Mutex<DownloadManifest>>,
    pause_flag: &AtomicBool,
    cancel_flag: &AtomicBool,
) -> Result<DownloadResult, DownloadError> {
    use tauri::Emitter;

    let total_bytes: u64 = tasks.iter().map(|t| t.expected_size).sum();
    let files_total = tasks.len() as u32;

    if tasks.is_empty() {
        // Nothing to download — already up to date
        let _ = app_handle.emit(
            "patch:progress",
            &DownloadProgress {
                bytes_downloaded: 0,
                total_bytes: 0,
                files_completed: 0,
                files_total: 0,
                speed_bps: 0,
                current_file: String::new(),
                phase: "complete".into(),
                failed_files: 0,
            },
        );
        return Ok(DownloadResult {
            files_completed: 0,
            files_failed: 0,
            bytes_downloaded: 0,
            total_bytes: 0,
        });
    }

    let semaphore = Arc::new(Semaphore::new(config.max_concurrent));
    let bytes_downloaded = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let files_completed = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let failed_files = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let speed_tracker = Arc::new(Mutex::new(SpeedTracker::new(Duration::from_secs(15))));

    // Emit initial progress
    let _ = app_handle.emit(
        "patch:progress",
        &DownloadProgress {
            bytes_downloaded: 0,
            total_bytes,
            files_completed: 0,
            files_total,
            speed_bps: 0,
            current_file: String::new(),
            phase: "downloading".into(),
            failed_files: 0,
        },
    );

    // Pre-create all 256 staging subdirectories (00-ff) upfront.
    // Eliminates 650K redundant create_dir_all calls during download.
    for i in 0..=255u8 {
        let subdir = config.staging_dir.join(format!("{:02x}", i));
        let _ = std::fs::create_dir_all(&subdir);
    }

    // Batched completion log — accumulates hashes, flushes every 100 to disk
    let completion_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::with_capacity(100)));

    let mut handles = Vec::with_capacity(tasks.len());

    for task in tasks {
        // Check cancel flag before dispatching each task
        if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        // Pause loop — wait until unpaused or cancelled
        while pause_flag.load(std::sync::atomic::Ordering::Relaxed) {
            if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let client = client.clone();
        let app_handle = app_handle.clone();
        let manifest = manifest.clone();
        let completion_log = completion_log.clone();
        let staging_dir = config.staging_dir.clone();
        let max_retries = config.max_retries;
        let bytes_dl = bytes_downloaded.clone();
        let files_comp = files_completed.clone();
        let files_fail = failed_files.clone();
        let tracker = speed_tracker.clone();
        let total_bytes_val = total_bytes;
        let files_total_val = files_total;

        let handle = tokio::spawn(async move {
            let _permit = permit; // hold until done

            // Retry loop with exponential backoff + jitter
            let mut last_error = String::new();
            let mut attempt_partial = task.partial_bytes;

            for attempt in 0..=max_retries {
                if attempt > 0 {
                    // Exponential backoff: 1s, 2s, 4s + jitter up to 500ms
                    let base_delay = Duration::from_millis(500 * attempt as u64);
                    let jitter = Duration::from_millis(rand::random::<u64>() % 300);
                    tokio::time::sleep(base_delay + jitter).await;
                    log::info!(
                        "Retrying {} (attempt {}/{})",
                        task.hash_hex,
                        attempt + 1,
                        max_retries + 1
                    );
                }

                match download_single_file(
                    &client,
                    &task.url,
                    &task.dest_path,
                    task.expected_size,
                    attempt_partial,
                )
                .await
                {
                    Ok(bytes_written) => {
                        // Success — update state
                        let new_total = bytes_dl
                            .fetch_add(bytes_written, std::sync::atomic::Ordering::Relaxed)
                            + bytes_written;
                        let completed = files_comp
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                            + 1;

                        {
                            let mut t = tracker.lock().await;
                            t.record(new_total);
                        }
                        let speed = {
                            let t = tracker.lock().await;
                            t.speed_bps()
                        };

                        // Record completion in memory for dedup
                        {
                            let mut m = manifest.lock().await;
                            m.files.insert(task.hash_hex.clone(), FileState::Complete);
                        }

                        // Append to completion log — batched via shared writer
                        {
                            let mut log = completion_log.lock().await;
                            log.push(task.hash_hex.clone());
                            // Flush to disk every 100 completions
                            if log.len() >= 100 {
                                let batch: Vec<String> = log.drain(..).collect();
                                let log_path = staging_dir.join("completed.log");
                                if let Ok(mut f) = tokio::fs::OpenOptions::new()
                                    .create(true).append(true).open(&log_path).await
                                {
                                    use tokio::io::AsyncWriteExt;
                                    let data = batch.join("\n") + "\n";
                                    let _ = f.write_all(data.as_bytes()).await;
                                }
                            }
                        }

                        let failed = files_fail.load(std::sync::atomic::Ordering::Relaxed);

                        // Emit progress — throttle to every 20 files or when significant
                        // bytes are downloaded to avoid 650K IPC events
                        if completed % 20 == 0 || bytes_written > 100_000 || completed == files_total_val {
                            let _ = app_handle.emit(
                                "patch:progress",
                                &DownloadProgress {
                                    bytes_downloaded: new_total,
                                    total_bytes: total_bytes_val,
                                    files_completed: completed,
                                    files_total: files_total_val,
                                    speed_bps: speed,
                                    current_file: task.hash_hex.clone(),
                                    phase: "downloading".into(),
                                    failed_files: failed,
                                },
                            );
                        }

                        return Ok(());
                    }
                    Err(DownloadError::SizeMismatch { expected, got }) => {
                        // Size mismatch — file was deleted by download_single_file, retry from scratch
                        attempt_partial = 0;
                        last_error = format!(
                            "size mismatch: expected {expected}, got {got}"
                        );
                        log::warn!("{last_error}");
                    }
                    Err(e) => {
                        // On partial write failure, check what we have on disk for resume
                        if let Ok(meta) = tokio::fs::metadata(&task.dest_path).await {
                            attempt_partial = meta.len();
                        }
                        last_error = format!("{e}");
                        log::warn!(
                            "Download error for {} (attempt {}): {last_error}",
                            task.hash_hex,
                            attempt + 1
                        );
                    }
                }
            }

            // All retries exhausted
            files_fail.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            {
                let mut m = manifest.lock().await;
                m.files.insert(
                    task.hash_hex.clone(),
                    FileState::Failed {
                        reason: last_error.clone(),
                    },
                );
                let _ = m.save(&staging_dir);
            }
            log::error!("Retries exhausted for {}: {last_error}", task.hash_hex);

            Err(DownloadError::RetriesExhausted {
                url: task.url.clone(),
                last_error,
            })
        });

        handles.push(handle);
    }

    // Wait for all downloads
    let mut any_failed = false;
    for handle in handles {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => any_failed = true,
            Err(e) => {
                log::error!("Download task panicked: {e}");
                any_failed = true;
            }
        }
    }

    // Flush remaining completion log entries
    {
        let mut log = completion_log.lock().await;
        if !log.is_empty() {
            let batch: Vec<String> = log.drain(..).collect();
            let log_path = config.staging_dir.join("completed.log");
            if let Ok(mut f) = tokio::fs::OpenOptions::new()
                .create(true).append(true).open(&log_path).await
            {
                use tokio::io::AsyncWriteExt;
                let data = batch.join("\n") + "\n";
                let _ = f.write_all(data.as_bytes()).await;
            }
        }
    }

    let final_downloaded = bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed);
    let final_completed = files_completed.load(std::sync::atomic::Ordering::Relaxed);
    let final_failed = failed_files.load(std::sync::atomic::Ordering::Relaxed);

    // Emit final progress
    let phase = if any_failed { "error" } else { "complete" };
    let _ = app_handle.emit(
        "patch:progress",
        &DownloadProgress {
            bytes_downloaded: final_downloaded,
            total_bytes,
            files_completed: final_completed,
            files_total,
            speed_bps: 0,
            current_file: String::new(),
            phase: phase.into(),
            failed_files: final_failed,
        },
    );

    Ok(DownloadResult {
        files_completed: final_completed,
        files_failed: final_failed,
        bytes_downloaded: final_downloaded,
        total_bytes,
    })
}

// ─── Update checker ──────────────────────────────────────────────────────────

/// Check whether the local TSW install needs patching.
///
/// Reads `LocalConfig.xml` and `le.idx` from the install directory, compares
/// the le.idx root hash against the known frozen hash. For TSW (frozen game),
/// a patched install always matches — if it doesn't match, files are needed.
///
/// Also computes the full download plan to report file count and total bytes.
pub fn check_for_updates(install_path: &Path) -> Result<PatchStatus, DownloadError> {
    let config = parse_local_config(&install_path.join("LocalConfig.xml"))?;
    let le_index = parse_le_index(&install_path.join("RDB").join("le.idx"))?;
    let hash_index = parse_hash_index(&install_path.join("RDB").join("RDBHashIndex.bin"))?;

    let root_hash_hex = hex_encode(&le_index.root_hash);
    let up_to_date = root_hash_hex == TSW_FROZEN_ROOT_HASH;

    if up_to_date {
        return Ok(PatchStatus {
            up_to_date: true,
            files_to_download: 0,
            total_bytes: 0,
        });
    }

    // Not up to date — compute what needs downloading
    // CDN resources use http_patch_addr (without the folder suffix)
    let cdn_base_url = &config.http_patch_addr;
    let staging_dir = install_path.join("staging");
    let manifest = DownloadManifest::load(&staging_dir);

    let plan = compute_download_plan(&le_index, &hash_index, &cdn_base_url, &staging_dir, &manifest);
    let total_bytes: u64 = plan.iter().map(|t| t.expected_size).sum();

    Ok(PatchStatus {
        up_to_date: false,
        files_to_download: plan.len() as u32,
        total_bytes,
    })
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tsw_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../The Secret World")
    }

    // ── DownloadConfig defaults ──────────────────────────────────────

    #[test]
    fn download_config_defaults() {
        let cfg = DownloadConfig::default();
        assert_eq!(cfg.max_concurrent, 8);
        assert_eq!(cfg.max_retries, 3);
        assert_eq!(cfg.connect_timeout, Duration::from_secs(30));
        assert_eq!(cfg.read_timeout, Duration::from_secs(60));
        assert_eq!(cfg.pool_idle_timeout, Duration::from_secs(90));
        assert_eq!(cfg.pool_max_idle_per_host, 128);
    }

    // ── Client creation ──────────────────────────────────────────────

    #[test]
    fn download_create_client() {
        let cfg = DownloadConfig::default();
        let client = create_client(&cfg);
        assert!(client.is_ok(), "should create reqwest client");
    }

    // ── Manifest serialization round-trip ────────────────────────────

    #[test]
    fn download_manifest_roundtrip() {
        let tmp = std::env::temp_dir().join("test_manifest_roundtrip");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let mut manifest = DownloadManifest::new();
        manifest
            .files
            .insert("abc123".to_string(), FileState::Complete);
        manifest.files.insert(
            "def456".to_string(),
            FileState::Partial {
                bytes_downloaded: 1024,
            },
        );
        manifest.files.insert(
            "ghi789".to_string(),
            FileState::Failed {
                reason: "404 not found".to_string(),
            },
        );

        manifest.save(&tmp).unwrap();

        let loaded = DownloadManifest::load(&tmp);
        assert_eq!(loaded.files.len(), 3);
        assert_eq!(loaded.files.get("abc123"), Some(&FileState::Complete));
        assert_eq!(
            loaded.files.get("def456"),
            Some(&FileState::Partial {
                bytes_downloaded: 1024
            })
        );
        assert!(matches!(
            loaded.files.get("ghi789"),
            Some(FileState::Failed { .. })
        ));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn download_manifest_load_missing() {
        let tmp = std::env::temp_dir().join("test_manifest_missing_dir");
        let _ = fs::remove_dir_all(&tmp);
        // Don't create the dir — should return empty manifest
        let manifest = DownloadManifest::load(&tmp);
        assert!(manifest.files.is_empty());
    }

    #[test]
    fn download_manifest_partial_bytes() {
        let mut manifest = DownloadManifest::new();
        manifest.files.insert(
            "abc".to_string(),
            FileState::Partial {
                bytes_downloaded: 500,
            },
        );
        assert_eq!(manifest.partial_bytes("abc"), 500);
        assert_eq!(manifest.partial_bytes("nonexistent"), 0);
    }

    // ── compute_download_plan ────────────────────────────────────────

    #[test]
    fn download_plan_skips_file_num_255() {
        use crate::rdb::{HashIndexEntry, LeIndex, LeIndexEntry, RdbHashIndex};

        let le_index = LeIndex {
            root_hash: [0u8; 16],
            entries: vec![
                LeIndexEntry {
                    rdb_type: 1000001,
                    id: 1,
                    file_num: 0,
                    flags: 0,
                    offset: 0,
                    length: 100,
                    hash: [1u8; 16],
                },
                LeIndexEntry {
                    rdb_type: 1000001,
                    id: 2,
                    file_num: 255, // server-only — skip
                    flags: 0,
                    offset: 0,
                    length: 200,
                    hash: [2u8; 16],
                },
            ],
        };

        let mut entries = HashMap::new();
        entries.insert(
            (1000001u32, 1u32),
            HashIndexEntry {
                id: 1,
                file_size: 100,
                hash: [1u8; 16],
            },
        );
        entries.insert(
            (1000001u32, 2u32),
            HashIndexEntry {
                id: 2,
                file_size: 200,
                hash: [2u8; 16],
            },
        );
        let hash_index = RdbHashIndex { entries };

        let manifest = DownloadManifest::new();
        let staging = PathBuf::from("/tmp/staging_test");

        let plan = compute_download_plan(
            &le_index,
            &hash_index,
            "http://cdn.example.com",
            &staging,
            &manifest,
        );

        // Only entry with file_num=0 should be in the plan
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].expected_size, 100);
    }

    #[test]
    fn download_plan_skips_completed() {
        use crate::rdb::{HashIndexEntry, LeIndex, LeIndexEntry, RdbHashIndex};

        let hash_a = [0xAA; 16];
        let hash_b = [0xBB; 16];
        let hex_a = hex_encode(&hash_a);

        let le_index = LeIndex {
            root_hash: [0u8; 16],
            entries: vec![
                LeIndexEntry {
                    rdb_type: 1000001,
                    id: 1,
                    file_num: 0,
                    flags: 0,
                    offset: 0,
                    length: 100,
                    hash: hash_a,
                },
                LeIndexEntry {
                    rdb_type: 1000001,
                    id: 2,
                    file_num: 0,
                    flags: 0,
                    offset: 0,
                    length: 200,
                    hash: hash_b,
                },
            ],
        };

        let mut hi_entries = HashMap::new();
        hi_entries.insert(
            (1000001u32, 1u32),
            HashIndexEntry {
                id: 1,
                file_size: 100,
                hash: hash_a,
            },
        );
        hi_entries.insert(
            (1000001u32, 2u32),
            HashIndexEntry {
                id: 2,
                file_size: 200,
                hash: hash_b,
            },
        );
        let hash_index = RdbHashIndex {
            entries: hi_entries,
        };

        let mut manifest = DownloadManifest::new();
        manifest.files.insert(hex_a, FileState::Complete);

        let staging = PathBuf::from("/tmp/staging_test");
        let plan = compute_download_plan(
            &le_index,
            &hash_index,
            "http://cdn.example.com",
            &staging,
            &manifest,
        );

        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].expected_size, 200);
    }

    #[test]
    fn download_plan_empty_when_all_complete() {
        use crate::rdb::{HashIndexEntry, LeIndex, LeIndexEntry, RdbHashIndex};

        let hash = [0xCC; 16];
        let hex = hex_encode(&hash);

        let le_index = LeIndex {
            root_hash: [0u8; 16],
            entries: vec![LeIndexEntry {
                rdb_type: 1000001,
                id: 1,
                file_num: 0,
                flags: 0,
                offset: 0,
                length: 100,
                hash,
            }],
        };

        let mut hi_entries = HashMap::new();
        hi_entries.insert(
            (1000001u32, 1u32),
            HashIndexEntry {
                id: 1,
                file_size: 100,
                hash,
            },
        );
        let hash_index = RdbHashIndex {
            entries: hi_entries,
        };

        let mut manifest = DownloadManifest::new();
        manifest.files.insert(hex, FileState::Complete);

        let plan = compute_download_plan(
            &le_index,
            &hash_index,
            "http://cdn.example.com",
            &PathBuf::from("/tmp/staging"),
            &manifest,
        );

        assert!(plan.is_empty(), "all files complete → empty plan");
    }

    #[test]
    fn download_plan_partial_has_offset() {
        use crate::rdb::{HashIndexEntry, LeIndex, LeIndexEntry, RdbHashIndex};

        let hash = [0xDD; 16];
        let hex = hex_encode(&hash);

        let le_index = LeIndex {
            root_hash: [0u8; 16],
            entries: vec![LeIndexEntry {
                rdb_type: 1000001,
                id: 1,
                file_num: 0,
                flags: 0,
                offset: 0,
                length: 1000,
                hash,
            }],
        };

        let mut hi_entries = HashMap::new();
        hi_entries.insert(
            (1000001u32, 1u32),
            HashIndexEntry {
                id: 1,
                file_size: 1000,
                hash,
            },
        );
        let hash_index = RdbHashIndex {
            entries: hi_entries,
        };

        let mut manifest = DownloadManifest::new();
        manifest.files.insert(
            hex,
            FileState::Partial {
                bytes_downloaded: 500,
            },
        );

        let plan = compute_download_plan(
            &le_index,
            &hash_index,
            "http://cdn.example.com",
            &PathBuf::from("/tmp/staging"),
            &manifest,
        );

        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].partial_bytes, 500);
    }

    #[test]
    fn download_plan_deduplicates_by_hash() {
        use crate::rdb::{HashIndexEntry, LeIndex, LeIndexEntry, RdbHashIndex};

        let hash = [0xEE; 16];

        // Two entries with same hash (same resource, different types)
        let le_index = LeIndex {
            root_hash: [0u8; 16],
            entries: vec![
                LeIndexEntry {
                    rdb_type: 1000001,
                    id: 1,
                    file_num: 0,
                    flags: 0,
                    offset: 0,
                    length: 100,
                    hash,
                },
                LeIndexEntry {
                    rdb_type: 1000002,
                    id: 1,
                    file_num: 0,
                    flags: 0,
                    offset: 0,
                    length: 100,
                    hash,
                },
            ],
        };

        let mut hi_entries = HashMap::new();
        hi_entries.insert(
            (1000001u32, 1u32),
            HashIndexEntry {
                id: 1,
                file_size: 100,
                hash,
            },
        );
        hi_entries.insert(
            (1000002u32, 1u32),
            HashIndexEntry {
                id: 1,
                file_size: 100,
                hash,
            },
        );
        let hash_index = RdbHashIndex {
            entries: hi_entries,
        };

        let manifest = DownloadManifest::new();
        let plan = compute_download_plan(
            &le_index,
            &hash_index,
            "http://cdn.example.com",
            &PathBuf::from("/tmp/staging"),
            &manifest,
        );

        assert_eq!(plan.len(), 1, "duplicate hashes should be deduplicated");
    }

    // ── URL integration with rdb module ──────────────────────────────

    #[test]
    fn download_url_construction_integration() {
        let hash: [u8; 16] = [
            0x1d, 0xa5, 0x4e, 0x9f, 0xc9, 0xd4, 0x92, 0x88, 0x9f, 0xbe, 0x08, 0x3d, 0xf4, 0xda,
            0xbe, 0xf2,
        ];
        let url = cdn_url_from_hash(
            "http://update.secretworld.com/tswupm/TSWLiveSteam",
            &hash,
        );
        assert_eq!(
            url,
            "http://update.secretworld.com/tswupm/TSWLiveSteam/rdb/res/1d/a/54e9fc9d492889fbe083df4dabef2"
        );
    }

    // ── Total bytes immutability ─────────────────────────────────────

    #[test]
    fn download_total_bytes_computed_from_hash_index() {
        use crate::rdb::{HashIndexEntry, LeIndex, LeIndexEntry, RdbHashIndex};

        let hash_a = [0x11; 16];
        let hash_b = [0x22; 16];

        let le_index = LeIndex {
            root_hash: [0u8; 16],
            entries: vec![
                LeIndexEntry {
                    rdb_type: 1000001,
                    id: 1,
                    file_num: 0,
                    flags: 0,
                    offset: 0,
                    length: 0,
                    hash: hash_a,
                },
                LeIndexEntry {
                    rdb_type: 1000001,
                    id: 2,
                    file_num: 0,
                    flags: 0,
                    offset: 0,
                    length: 0,
                    hash: hash_b,
                },
            ],
        };

        let mut hi_entries = HashMap::new();
        hi_entries.insert(
            (1000001u32, 1u32),
            HashIndexEntry {
                id: 1,
                file_size: 500,
                hash: hash_a,
            },
        );
        hi_entries.insert(
            (1000001u32, 2u32),
            HashIndexEntry {
                id: 2,
                file_size: 300,
                hash: hash_b,
            },
        );
        let hash_index = RdbHashIndex {
            entries: hi_entries,
        };

        let manifest = DownloadManifest::new();
        let plan = compute_download_plan(
            &le_index,
            &hash_index,
            "http://cdn.example.com",
            &PathBuf::from("/tmp/staging"),
            &manifest,
        );

        // total_bytes is computed from file_size in hash index, immutable once set
        let total: u64 = plan.iter().map(|t| t.expected_size).sum();
        assert_eq!(total, 800);
    }

    // ── check_for_updates with real game files ───────────────────────

    #[test]
    fn download_check_for_updates_real_install() {
        let status = check_for_updates(&tsw_dir()).expect("check_for_updates");
        // TSW is a frozen game — local root hash matches the frozen hash
        assert!(
            status.up_to_date,
            "TSW install should be up to date (frozen game)"
        );
        assert_eq!(status.files_to_download, 0);
        assert_eq!(status.total_bytes, 0);
    }

    // ── Progress struct serialization ────────────────────────────────

    #[test]
    fn download_progress_serializes() {
        let progress = DownloadProgress {
            bytes_downloaded: 1024,
            total_bytes: 2048,
            files_completed: 1,
            files_total: 10,
            speed_bps: 512,
            current_file: "abc123".to_string(),
            phase: "downloading".to_string(),
            failed_files: 0,
        };
        let json = serde_json::to_string(&progress).unwrap();
        assert!(json.contains("\"bytes_downloaded\":1024"));
        assert!(json.contains("\"total_bytes\":2048"));
        assert!(json.contains("\"phase\":\"downloading\""));
    }

    // ── Error display ────────────────────────────────────────────────

    #[test]
    fn download_error_display() {
        let err = DownloadError::HashMismatch {
            expected: "aaa".to_string(),
            got: "bbb".to_string(),
            path: PathBuf::from("/tmp/test.bin"),
        };
        let msg = format!("{err}");
        assert!(msg.contains("MD5 mismatch"));
        assert!(msg.contains("aaa"));
        assert!(msg.contains("bbb"));

        let err2 = DownloadError::RetriesExhausted {
            url: "http://example.com/file".to_string(),
            last_error: "timeout".to_string(),
        };
        let msg2 = format!("{err2}");
        assert!(msg2.contains("retries exhausted"));
    }

    // ── Integration test (requires live CDN) ─────────────────────────

    #[tokio::test]
    #[ignore] // Run with: cargo test -- --ignored cdn_integration
    async fn cdn_integration_download_small_resource() {
        // CDN resource URLs use http_patch_addr (without folder suffix)
        let cfg = DownloadConfig {
            cdn_base_url: "http://update.secretworld.com/tswupm".to_string(),
            ..Default::default()
        };
        let client = create_client(&cfg).expect("create client");

        // Known small resource: le.idx entry 0 → type 1000001, id 3
        // Hash: 1da54e9fc9d492889fbe083df4dabef2, size: 89 bytes
        // The hash is a resource identifier for URLs, not a content hash.
        // CDN files are zlib-compressed with an IOz1 header.
        let hash: [u8; 16] = [
            0x1d, 0xa5, 0x4e, 0x9f, 0xc9, 0xd4, 0x92, 0x88, 0x9f, 0xbe, 0x08, 0x3d, 0xf4, 0xda,
            0xbe, 0xf2,
        ];
        let url = cdn_url_from_hash(&cfg.cdn_base_url, &hash);
        let tmp_dir = std::env::temp_dir().join("tsw_cdn_test");
        let _ = fs::create_dir_all(&tmp_dir);
        let dest = tmp_dir.join("test_resource.bin");

        let bytes_written = download_single_file(&client, &url, &dest, 89, 0)
            .await
            .expect("download should succeed");

        assert_eq!(bytes_written, 89, "should download exactly 89 bytes");

        // Verify file on disk
        let data = fs::read(&dest).expect("read downloaded file");
        assert_eq!(data.len(), 89);

        // Verify IOz1 header (zlib-compressed CDN resource)
        assert_eq!(&data[..4], b"IOz1", "CDN resource should have IOz1 header");

        let _ = fs::remove_dir_all(&tmp_dir);
    }
}
