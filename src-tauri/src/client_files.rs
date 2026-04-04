//! Download and install loose game files (exe, dll, Data/) from the Funcom CDN.
//!
//! These files are served at `/client/{md5[:2]}/{md5[2:]}` and are either raw or
//! IOz2 compressed (LZMA with a custom header).
//!
//! The file manifest is embedded at compile time from `game_files.json` — a static
//! list of 1496 files with paths, sizes, and MD5 hashes. TSW hasn't been updated
//! since February 2017, so this manifest is frozen.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

/// Embedded game file manifest — compiled into the binary.
const GAME_FILES_JSON: &str = include_str!("game_files.json");

/// A single loose file entry from the manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct GameFileEntry {
    pub path: String,
    pub size: u64,
    pub md5: String,
}

/// Progress event for client file downloads.
#[derive(Debug, Clone, Serialize)]
pub struct ClientFilesProgress {
    pub files_completed: u32,
    pub files_total: u32,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
    pub phase: String,
    pub current_file: String,
}

/// Load the embedded manifest, filtering to files not already present on disk.
pub fn compute_client_file_plan(install_dir: &Path) -> Vec<GameFileEntry> {
    let all_files: Vec<GameFileEntry> =
        serde_json::from_str(GAME_FILES_JSON).expect("parse embedded game_files.json");

    all_files
        .into_iter()
        .filter(|f| {
            let dest = install_dir.join(&f.path);
            if dest.exists() {
                // File exists — check if size matches (skip re-download)
                if let Ok(meta) = std::fs::metadata(&dest) {
                    if meta.len() == f.size {
                        return false; // Already have it
                    }
                }
            }
            true
        })
        .collect()
}

/// Decompress IOz2 data: 4-byte magic "IOz2" + 4-byte LE decompressed size + LZMA stream.
/// If data doesn't start with IOz2, returns it unchanged (file is uncompressed).
pub fn decompress_ioz2(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 8 || &data[..4] != b"IOz2" {
        // Not IOz2 compressed — return as-is
        return Ok(data.to_vec());
    }

    let decompressed_size =
        u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;

    // The LZMA stream starts at byte 8. lzma-rs can decompress it directly
    // when we provide the stream in LZMA alone format.
    // LZMA alone format: props(5 bytes) + uncompressed_size(8 bytes LE) + compressed_data
    // The IOz2 stream has: props(5 bytes) + compressed_data (no size field)
    // We need to insert the size field.

    if data.len() < 13 {
        return Err("IOz2 data too short for LZMA header".into());
    }

    // Build LZMA-alone header: props(5 bytes from stream) + size(8 bytes LE)
    let mut lzma_data = Vec::with_capacity(13 + data.len() - 13);
    lzma_data.extend_from_slice(&data[8..13]); // 5-byte LZMA properties
    lzma_data.extend_from_slice(&(decompressed_size as u64).to_le_bytes()); // 8-byte size
    lzma_data.extend_from_slice(&data[13..]); // compressed payload

    let mut decompressed = Vec::with_capacity(decompressed_size);
    let mut reader = std::io::Cursor::new(&lzma_data);
    lzma_rs::lzma_decompress(&mut reader, &mut decompressed)
        .map_err(|e| format!("LZMA decompress failed: {}", e))?;

    if decompressed.len() != decompressed_size {
        return Err(format!(
            "IOz2 size mismatch: expected {}, got {}",
            decompressed_size,
            decompressed.len()
        ));
    }

    Ok(decompressed)
}

/// Build the CDN URL for a client file given its MD5 hash.
pub fn client_file_url(cdn_base: &str, md5_hex: &str) -> String {
    format!(
        "{}/client/{}/{}",
        cdn_base.trim_end_matches('/'),
        &md5_hex[..2],
        &md5_hex[2..]
    )
}

/// Download all missing client files with parallel HTTP/2, IOz2 decompression,
/// and MD5 verification. Emits progress events via the Tauri app handle.
pub async fn download_client_files(
    app: &tauri::AppHandle,
    cdn_base: &str,
    install_dir: &Path,
    pause_flag: &AtomicBool,
    cancel_flag: &AtomicBool,
) -> Result<u32, String> {
    use md5::Digest;
    use tauri::Emitter;

    let plan = compute_client_file_plan(install_dir);
    let files_total = plan.len() as u32;

    if files_total == 0 {
        let _ = app.emit(
            "client_files:progress",
            &ClientFilesProgress {
                files_completed: 0,
                files_total: 0,
                bytes_downloaded: 0,
                total_bytes: 0,
                phase: "complete".into(),
                current_file: String::new(),
            },
        );
        return Ok(0);
    }

    // Pre-create all directories
    let mut dirs: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for f in &plan {
        if let Some(parent) = install_dir.join(&f.path).parent() {
            dirs.insert(parent.to_path_buf());
        }
    }
    for dir in &dirs {
        let _ = std::fs::create_dir_all(dir);
    }

    let total_bytes: u64 = plan.iter().map(|f| f.size).sum();
    let semaphore = Arc::new(Semaphore::new(128));
    let bytes_downloaded = Arc::new(AtomicU64::new(0));
    let files_completed = Arc::new(AtomicU32::new(0));
    let files_failed = Arc::new(AtomicU32::new(0));

    // Emit initial progress
    let _ = app.emit(
        "client_files:progress",
        &ClientFilesProgress {
            files_completed: 0,
            files_total,
            bytes_downloaded: 0,
            total_bytes,
            phase: "downloading".into(),
            current_file: String::new(),
        },
    );

    let client = reqwest::Client::builder()
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(128)
        .tcp_keepalive(Duration::from_secs(30))
        .user_agent("") // Required — CDN blocks default UAs
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let cdn = cdn_base.to_string();
    let install = install_dir.to_path_buf();
    let mut handles = Vec::with_capacity(plan.len());

    for entry in plan {
        // Check cancel
        if cancel_flag.load(Ordering::Relaxed) {
            break;
        }
        // Pause loop
        while pause_flag.load(Ordering::Relaxed) {
            if cancel_flag.load(Ordering::Relaxed) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if cancel_flag.load(Ordering::Relaxed) {
            break;
        }

        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let client = client.clone();
        let cdn = cdn.clone();
        let install = install.clone();
        let bytes_dl = bytes_downloaded.clone();
        let files_comp = files_completed.clone();
        let files_fail = files_failed.clone();
        let app = app.clone();
        let ft = files_total;
        let tb = total_bytes;

        handles.push(tokio::spawn(async move {
            let _permit = permit;
            let url = client_file_url(&cdn, &entry.md5);
            let dest = install.join(&entry.path);

            // Download
            let resp = match client.get(&url).send().await {
                Ok(r) if r.status().is_success() => r,
                Ok(r) => {
                    log::warn!("Client file {} → HTTP {}", entry.path, r.status());
                    files_fail.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(e) => {
                    log::warn!("Client file {} → {}", entry.path, e);
                    files_fail.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            };

            let body = match resp.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("Client file {} read → {}", entry.path, e);
                    files_fail.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            };

            // Decompress IOz2 if needed
            let final_data = match decompress_ioz2(&body) {
                Ok(d) => d,
                Err(e) => {
                    log::warn!("Client file {} decompress → {}", entry.path, e);
                    files_fail.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            };

            // Verify MD5
            let mut hasher = md5::Md5::new();
            hasher.update(&final_data);
            let actual_md5 = format!("{:x}", hasher.finalize());
            if actual_md5 != entry.md5 {
                log::warn!(
                    "Client file {} MD5 mismatch: expected {}, got {}",
                    entry.path, entry.md5, actual_md5
                );
                files_fail.fetch_add(1, Ordering::Relaxed);
                return;
            }

            // Write to disk
            if let Err(e) = tokio::fs::write(&dest, &final_data).await {
                log::warn!("Client file {} write → {}", entry.path, e);
                files_fail.fetch_add(1, Ordering::Relaxed);
                return;
            }

            // Update progress
            let new_bytes = bytes_dl.fetch_add(final_data.len() as u64, Ordering::Relaxed)
                + final_data.len() as u64;
            let completed =
                files_comp.fetch_add(1, Ordering::Relaxed) + 1;

            // Emit progress every 20 files
            if completed % 20 == 0 || completed == ft {
                let _ = app.emit(
                    "client_files:progress",
                    &ClientFilesProgress {
                        files_completed: completed,
                        files_total: ft,
                        bytes_downloaded: new_bytes,
                        total_bytes: tb,
                        phase: "downloading".into(),
                        current_file: entry.path.clone(),
                    },
                );
            }
        }));
    }

    // Wait for all tasks
    for h in handles {
        let _ = h.await;
    }

    let final_completed = files_completed.load(Ordering::Relaxed);
    let final_failed = files_failed.load(Ordering::Relaxed);
    let final_bytes = bytes_downloaded.load(Ordering::Relaxed);

    let _ = app.emit(
        "client_files:progress",
        &ClientFilesProgress {
            files_completed: final_completed,
            files_total,
            bytes_downloaded: final_bytes,
            total_bytes,
            phase: if final_failed > 0 { "error" } else { "complete" }.into(),
            current_file: if final_failed > 0 {
                format!("{} files failed", final_failed)
            } else {
                String::new()
            },
        },
    );

    if final_failed > 0 {
        Err(format!(
            "{} of {} client files failed to download",
            final_failed, files_total
        ))
    } else {
        Ok(final_completed)
    }
}

/// Write the static files that aren't on the CDN but are needed for the game.
/// These are the known-static files from the Funcom installer.
pub fn write_static_files(install_dir: &Path) -> Result<(), String> {
    // LocalConfig.xml — CDN connection parameters
    let config_path = install_dir.join("LocalConfig.xml");
    if !config_path.exists() {
        let config = r#"<Config>

  <Self>

    <ConfigKey>Universe/Client/</ConfigKey>

  </Self>

  <Universe>

    <Client>

      <ClientFileName>TheSecretWorld.exe</ClientFileName>

      <ClientFileNameDX11>TheSecretWorldDX11.exe</ClientFileNameDX11>

      <HttpPatchFolder>TSWLiveSteam</HttpPatchFolder>

      <HttpPatchAddr>http://update.secretworld.com/tswupm</HttpPatchAddr>

      <ControlHttpPatchAddr>ControlHttpPatchAddr.secretworld.com/tswupm</ControlHttpPatchAddr>

      <HTTPMaxConnections>5</HTTPMaxConnections>

      <UniverseAddr>um.live.secretworld.com:7000</UniverseAddr>

      <PatchVersion>xb36bba4f8606fe8fda4fec2a747703bf</PatchVersion>

    </Client>

  </Universe>

</Config>
"#;
        std::fs::write(&config_path, config)
            .map_err(|e| format!("Failed to write LocalConfig.xml: {}", e))?;
    }

    // LanguagePrefs.xml — default to English
    let lang_dir = install_dir.join("Data/Gui/Default");
    let _ = std::fs::create_dir_all(&lang_dir);
    let lang_path = lang_dir.join("LanguagePrefs.xml");
    if !lang_path.exists() {
        let lang = r#"<Prefs><Value name="SelectedLanguage" value="en" /><Value name="SelectedAudioLanguage" value="en" /></Prefs>"#;
        std::fs::write(&lang_path, lang)
            .map_err(|e| format!("Failed to write LanguagePrefs.xml: {}", e))?;
    }

    // RDB directory
    let rdb_dir = install_dir.join("RDB");
    std::fs::create_dir_all(&rdb_dir)
        .map_err(|e| format!("Failed to create RDB dir: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_loads() {
        let files: Vec<GameFileEntry> =
            serde_json::from_str(GAME_FILES_JSON).expect("parse manifest");
        assert!(files.len() > 1400, "Expected 1400+ files, got {}", files.len());

        // Check critical files exist
        let has_dx11 = files.iter().any(|f| f.path == "TheSecretWorldDX11.exe");
        let has_dx9 = files.iter().any(|f| f.path == "TheSecretWorld.exe");
        assert!(has_dx11, "Missing TheSecretWorldDX11.exe");
        assert!(has_dx9, "Missing TheSecretWorld.exe");
    }

    #[test]
    fn test_decompress_ioz2_passthrough() {
        let data = b"hello world";
        let result = decompress_ioz2(data).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn test_decompress_ioz2_short() {
        let data = b"IOz2\x04\x00\x00\x00";
        let result = decompress_ioz2(data);
        assert!(result.is_err());
    }

    #[test]
    fn test_client_file_url() {
        let url = client_file_url(
            "https://update.secretworld.com/tswupm",
            "ac1292a51bfbb22a0f033ef8688aeddf",
        );
        assert_eq!(
            url,
            "https://update.secretworld.com/tswupm/client/ac/1292a51bfbb22a0f033ef8688aeddf"
        );
    }
}
