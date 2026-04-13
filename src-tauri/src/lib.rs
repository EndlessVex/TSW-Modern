mod tauri_reporter;

pub use tsw_core::bxml;
pub use tsw_core::client_files;
pub use tsw_core::config;
pub use tsw_core::download;
pub use tsw_core::encoder_native;
pub use tsw_core::rdb;
pub use tsw_core::rdbdata;
pub use tsw_core::redux;
pub use tsw_core::verify;

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

// ─── Process priority (Windows) ─────────────────────────────────────────────
#[cfg(target_os = "windows")]
const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x00004000;
#[cfg(target_os = "windows")]
const NORMAL_PRIORITY_CLASS: u32 = 0x00000020;

#[cfg(target_os = "windows")]
fn set_process_priority(priority: u32) {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetPriorityClass(hProcess: isize, dwPriorityClass: u32) -> i32;
        fn GetCurrentProcess() -> isize;
    }
    unsafe { SetPriorityClass(GetCurrentProcess(), priority); }
}

use download::{
    check_for_updates as check_updates_inner, create_client,
    DownloadConfig, PatchStatus,
};
use verify::{CorruptedEntry, VerifyResult};

/// Global cached patch status for `get_patch_status` command.
static PATCH_STATUS_CACHE: Mutex<Option<PatchStatus>> = Mutex::new(None);
/// Global flag to prevent concurrent patching.
static PATCHING_IN_PROGRESS: Mutex<bool> = Mutex::new(false);
/// Global pause flag for patching — checked between downloads.
static PATCH_PAUSED: AtomicBool = AtomicBool::new(false);
/// Global cancel flag for patching.
static PATCH_CANCEL: AtomicBool = AtomicBool::new(false);
/// Global flag to prevent concurrent verification.
static VERIFY_IN_PROGRESS: Mutex<bool> = Mutex::new(false);
/// Global cancellation flag for verification.
static VERIFY_CANCEL: AtomicBool = AtomicBool::new(false);
/// Cached result of the last verification scan.
static VERIFY_RESULT_CACHE: Mutex<Option<VerifyResult>> = Mutex::new(None);

#[derive(Serialize, Debug, PartialEq)]
pub struct InstallValidation {
    pub valid: bool,
    pub version: Option<String>,
    pub rdb_count: usize,
    pub message: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct AuthResult {
    pub success: bool,
    pub message: String,
}

/// Validate credentials format (fallback-first: no server connection, D007).
/// The game handles actual authentication via its built-in login screen.
pub fn authenticate_inner(username: &str, password: &str) -> AuthResult {
    if username.trim().is_empty() {
        return AuthResult {
            success: false,
            message: "Username is required.".into(),
        };
    }
    if password.is_empty() {
        return AuthResult {
            success: false,
            message: "Password is required.".into(),
        };
    }
    // Fallback approach: credentials are validated for format only.
    // The game's built-in login handles actual auth when launched without -loginkey.
    AuthResult {
        success: true,
        message: "Credentials accepted. Game will authenticate on launch.".into(),
    }
}

/// Validate a TSW install directory by checking for expected marker files.
///
/// Checks: TheSecretWorld.exe, TheSecretWorldDX11.exe, RDB/ with .rdbdata files,
/// LocalConfig.xml, and optionally reads Version.txt.
///
/// A directory is also valid if it contains LocalConfig.xml and RDB/ but no game
/// executables — this is the state after a fresh install before patching completes.
pub fn validate_install_dir_inner(path: &str) -> InstallValidation {
    let base = Path::new(path);

    if path.is_empty() {
        return InstallValidation {
            valid: false,
            version: None,
            rdb_count: 0,
            message: "Install path is empty.".into(),
        };
    }

    if !base.is_dir() {
        return InstallValidation {
            valid: false,
            version: None,
            rdb_count: 0,
            message: format!("Path does not exist or is not a directory: {}", path),
        };
    }

    let tsw_exe = base.join("TheSecretWorld.exe");
    let dx11_exe = base.join("TheSecretWorldDX11.exe");
    let rdb_dir = base.join("RDB");
    let local_config = base.join("LocalConfig.xml");
    let version_file = base.join("Version.txt");

    let has_tsw_exe = tsw_exe.is_file();
    let has_dx11_exe = dx11_exe.is_file();
    let has_rdb = rdb_dir.is_dir();
    let has_local_config = local_config.is_file();

    // Count .rdbdata files in RDB/
    let rdb_count = if has_rdb {
        fs::read_dir(&rdb_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        e.path()
                            .extension()
                            .map_or(false, |ext| ext == "rdbdata")
                    })
                    .count()
            })
            .unwrap_or(0)
    } else {
        0
    };

    // Read version string
    let version = fs::read_to_string(&version_file)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    // Fresh install state: LocalConfig.xml exists but no game executables yet.
    // The Funcom installer creates LocalConfig.xml + Data/ directory.
    // RDB/ directory may not exist yet — it's created during patching.
    // This is NOT an SWL misidentification — it's a valid pre-patch state.
    if has_local_config && !has_tsw_exe && !has_dx11_exe {
        return InstallValidation {
            valid: true,
            version,
            rdb_count,
            message: "Fresh TSW install detected — game files need patching.".into(),
        };
    }

    // Detect SWL (Secret World Legends) misidentification:
    // If DX11 exe or other markers exist but TheSecretWorld.exe is missing,
    // and it doesn't look like a fresh install, it might be SWL.
    if !has_tsw_exe && has_dx11_exe {
        return InstallValidation {
            valid: false,
            version: None,
            rdb_count: 0,
            message: "TheSecretWorld.exe not found but TheSecretWorldDX11.exe is present. \
                      This might be a Secret World Legends (SWL) install, not The Secret World (TSW)."
                .into(),
        };
    }

    let mut missing = Vec::new();
    if !has_tsw_exe {
        missing.push("TheSecretWorld.exe");
    }
    if !has_dx11_exe {
        missing.push("TheSecretWorldDX11.exe");
    }
    if !has_rdb {
        missing.push("RDB/");
    }
    if !has_local_config {
        missing.push("LocalConfig.xml");
    }

    if !missing.is_empty() {
        return InstallValidation {
            valid: false,
            version: None,
            rdb_count: 0,
            message: format!("Missing required files: {}", missing.join(", ")),
        };
    }

    InstallValidation {
        valid: true,
        version,
        rdb_count,
        message: "Valid TSW install directory.".into(),
    }
}

#[tauri::command]
fn validate_install_dir(path: String) -> Result<InstallValidation, String> {
    Ok(validate_install_dir_inner(&path))
}

/// Check common Windows install locations for an existing TSW install.
/// Returns the first valid path found, or None.
fn auto_detect_install_dir_inner() -> Option<String> {
    let candidates: Vec<std::path::PathBuf> = {
        let mut paths = Vec::new();

        // Standard Funcom installer default paths
        paths.push(std::path::PathBuf::from(r"C:\Program Files (x86)\Funcom\The Secret World"));
        paths.push(std::path::PathBuf::from(r"C:\Program Files\Funcom\The Secret World"));

        // Steam common install locations
        paths.push(std::path::PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps\common\The Secret World"));
        paths.push(std::path::PathBuf::from(r"C:\Program Files\Steam\steamapps\common\The Secret World"));

        // Check all drive letters for the Funcom default path
        for letter in b'D'..=b'Z' {
            let drive = format!("{}:", letter as char);
            paths.push(std::path::PathBuf::from(format!(r"{}\Program Files (x86)\Funcom\The Secret World", drive)));
            paths.push(std::path::PathBuf::from(format!(r"{}\Funcom\The Secret World", drive)));
            paths.push(std::path::PathBuf::from(format!(r"{}\Games\The Secret World", drive)));
        }

        paths
    };

    for candidate in &candidates {
        if candidate.is_dir() {
            let result = validate_install_dir_inner(&candidate.to_string_lossy());
            if result.valid {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }

    None
}

#[tauri::command]
fn auto_detect_install_dir() -> Result<Option<String>, String> {
    Ok(auto_detect_install_dir_inner())
}

#[tauri::command]
fn authenticate(username: String, password: String) -> Result<AuthResult, String> {
    Ok(authenticate_inner(&username, &password))
}

#[tauri::command]
fn launch_game(install_path: String, dx_version: String, login_key: Option<String>) -> Result<(), String> {
    let exe_name = if dx_version.eq_ignore_ascii_case("dx9") {
        "TheSecretWorld.exe"
    } else {
        "TheSecretWorldDX11.exe"
    };

    let base = Path::new(&install_path);
    let exe_path = base.join(exe_name);

    if !exe_path.is_file() {
        return Err(format!(
            "Executable not found: {} (resolved: {})",
            exe_name,
            exe_path.display()
        ));
    }

    let mut cmd = std::process::Command::new(&exe_path);
    cmd.current_dir(&install_path);

    if let Some(key) = login_key {
        cmd.args(["-loginkey", &key]);
    }

    cmd.spawn()
        .map_err(|e| format!("Failed to launch {}: {}", exe_name, e))?;

    Ok(())
}

/// Launch the ClientPatcher.exe which provides the game's login UI and "Start Game" button.
#[tauri::command]
fn launch_patcher(install_path: String) -> Result<(), String> {
    let base = std::path::Path::new(&install_path);
    let patcher = base.join("ClientPatcher.exe");

    if !patcher.is_file() {
        return Err(format!("ClientPatcher.exe not found at {}", patcher.display()));
    }

    std::process::Command::new(&patcher)
        .current_dir(base)
        .spawn()
        .map_err(|e| format!("Failed to launch ClientPatcher.exe: {}", e))?;

    Ok(())
}

#[tauri::command]
async fn check_for_updates_cmd(install_path: String) -> Result<PatchStatus, String> {
    let path = std::path::PathBuf::from(&install_path);

    // If RDB/le.idx doesn't exist, this is a fresh install that needs a full patch.
    // We can't compute exact file counts without the index files, but we know it needs updating.
    let le_idx_path = path.join("RDB").join("le.idx");
    if !le_idx_path.exists() {
        let status = PatchStatus {
            up_to_date: false,
            files_to_download: 0, // Unknown until index files are downloaded
            total_bytes: 0,
        };
        *PATCH_STATUS_CACHE.lock().map_err(|e| e.to_string())? = Some(status.clone());
        return Ok(status);
    }

    let status = check_updates_inner(&path).map_err(|e| e.to_string())?;

    // Cache the result
    if let Ok(mut cache) = PATCH_STATUS_CACHE.lock() {
        *cache = Some(status.clone());
    }

    Ok(status)
}

#[tauri::command]
fn get_patch_status_cmd() -> Result<PatchStatus, String> {
    match PATCH_STATUS_CACHE.lock() {
        Ok(cache) => cache
            .clone()
            .ok_or_else(|| "No patch status cached. Run check_for_updates first.".to_string()),
        Err(e) => Err(format!("Failed to read cache: {}", e)),
    }
}

/// Full install: write static files + download client files + download RDB resources.
/// This replaces the Funcom installer + ClientPatcher entirely.
#[tauri::command]
async fn start_full_install(app: tauri::AppHandle, install_path: String) -> Result<(), String> {

    // Prevent concurrent patching
    {
        let mut in_progress = PATCHING_IN_PROGRESS.lock().map_err(|e| e.to_string())?;
        if *in_progress {
            return Err("Installation is already in progress.".to_string());
        }
        *in_progress = true;
    }

    PATCH_PAUSED.store(false, Ordering::Relaxed);
    PATCH_CANCEL.store(false, Ordering::Relaxed);

    // Prevent system sleep during download (Windows)
    #[cfg(target_os = "windows")]
    {
        extern "system" {
            fn SetThreadExecutionState(esFlags: u32) -> u32;
        }
        // ES_CONTINUOUS | ES_SYSTEM_REQUIRED — prevent sleep
        unsafe { SetThreadExecutionState(0x80000001); }
    }

    let app_clone = app.clone();
    tokio::spawn(async move {
        let result = run_full_install_inner(&app_clone, &install_path).await;

        // Re-allow sleep
        #[cfg(target_os = "windows")]
        {
            extern "system" {
                fn SetThreadExecutionState(esFlags: u32) -> u32;
            }
            // ES_CONTINUOUS alone — restore normal sleep behavior
            unsafe { SetThreadExecutionState(0x80000000); }
        }

        if let Ok(mut in_progress) = PATCHING_IN_PROGRESS.lock() {
            *in_progress = false;
        }

        if let Err(e) = result {
            use tauri::Emitter;
            let _ = app_clone.emit(
                "patch:progress",
                &download::DownloadProgress {
                    bytes_downloaded: 0,
                    total_bytes: 0,
                    files_completed: 0,
                    files_total: 0,
                    speed_bps: 0,
                    current_file: format!("Error: {}", e),
                    phase: "error".into(),
                    failed_files: 0,
                },
            );
            log::error!("Full install failed: {}", e);
        }
    });

    Ok(())
}

async fn run_full_install_inner(
    app: &tauri::AppHandle,
    install_path: &str,
) -> Result<(), String> {
    log::info!("=== run_full_install_inner START === install_path={}", install_path);
    use tauri::Emitter;

    let base = std::path::PathBuf::from(install_path);

    // Ensure install directory exists — may need elevation for Program Files.
    // Try non-elevated first; if that fails, elevate our own exe with --prepare-dir.
    if let Err(_) = std::fs::create_dir_all(&base) {
        #[cfg(target_os = "windows")]
        {
            let our_exe = std::env::current_exe()
                .map_err(|e| format!("Failed to get exe path: {}", e))?;

            let status = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    &format!(
                        "Start-Process -FilePath '{}' -ArgumentList '--prepare-dir \"{}\"' -Verb RunAs -Wait -WindowStyle Hidden",
                        our_exe.display(),
                        install_path,
                    ),
                ])
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .status()
                .map_err(|e| format!("Failed to elevate: {}", e))?;

            if !status.success() {
                return Err("Failed to create install directory (elevation denied?)".into());
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            return Err(format!("Failed to create install directory: access denied"));
        }

        // Verify it was created
        if !base.exists() {
            return Err("Install directory was not created after elevation".into());
        }
    }

    log::info!("Phase 1: Writing static files...");
    // Phase 1: Write static files (LocalConfig.xml, LanguagePrefs.xml, RDB/ dir)
    let _ = app.emit(
        "patch:progress",
        &download::DownloadProgress {
            bytes_downloaded: 0, total_bytes: 0,
            files_completed: 0, files_total: 0,
            speed_bps: 0, current_file: "Preparing install directory...".into(),
            phase: "checking".into(), failed_files: 0,
        },
    );
    client_files::write_static_files(&base)?;

    // Check available disk space — need ~45GB minimum
    #[cfg(target_os = "windows")]
    {
        let free_bytes = get_free_disk_space(&base);
        let required_bytes = 45_000_000_000u64; // ~45 GB
        if free_bytes > 0 && free_bytes < required_bytes {
            return Err(format!(
                "Not enough disk space. Need {:.1} GB free, have {:.1} GB.",
                required_bytes as f64 / 1_073_741_824.0,
                free_bytes as f64 / 1_073_741_824.0,
            ));
        }
    }

    // Parse CDN config from the LocalConfig.xml we just wrote
    let patch_config =
        config::parse_local_config(&base.join("LocalConfig.xml")).map_err(|e| e.to_string())?;
    let cdn_base_url = patch_config.http_patch_addr.replace("http://", "https://");
    log::info!("CDN base URL: {}", cdn_base_url);

    log::info!("Phase 2: Downloading client files...");
    // Phase 2: Download client files (exe, dll, Data/) via /client/ path
    let _ = app.emit(
        "patch:progress",
        &download::DownloadProgress {
            bytes_downloaded: 0, total_bytes: 0,
            files_completed: 0, files_total: 0,
            speed_bps: 0, current_file: "Downloading game files...".into(),
            phase: "bootstrapping".into(), failed_files: 0,
        },
    );
    let reporter = tauri_reporter::TauriReporter::new(app.clone());
    client_files::download_client_files(
        &reporter, &cdn_base_url, &base, &PATCH_PAUSED, &PATCH_CANCEL,
    )
    .await?;

    if PATCH_CANCEL.load(Ordering::Relaxed) {
        return Err("Installation cancelled by user".into());
    }

    log::info!("Phase 3: Downloading RDBHashIndex.bin...");
    // Phase 3: Download RDBHashIndex.bin
    let _ = app.emit(
        "patch:progress",
        &download::DownloadProgress {
            bytes_downloaded: 0, total_bytes: 0,
            files_completed: 0, files_total: 0,
            speed_bps: 0, current_file: "Downloading patch index...".into(),
            phase: "bootstrapping".into(), failed_files: 0,
        },
    );

    let rdb_dir = base.join("RDB");
    std::fs::create_dir_all(&rdb_dir)
        .map_err(|e| format!("Failed to create RDB dir: {}", e))?;

    let hash_idx_path = rdb_dir.join("RDBHashIndex.bin");
    // Re-download if missing OR if the file is a stale version 4 index (~22 MB).
    // Version 7 is ~44 MB; anything under 30 MB is the old truncated version.
    let hash_idx_needs_download = !hash_idx_path.exists() || 
        hash_idx_path.metadata().map(|m| m.len() < 30_000_000).unwrap_or(true);
    if hash_idx_needs_download {
        let patch_info_url = format!("{}/PatchInfoClient.txt", patch_config.patch_base_url());
        let dl_config = download::DownloadConfig::default();
        let client = download::create_client(&dl_config).map_err(|e| e.to_string())?;

        let patch_info_text = client.get(&patch_info_url)
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

        log::info!("RDB hash: {}", rdb_hash);
        let hash_idx_url = format!("{}/rdb/full/{}", cdn_base_url.trim_end_matches('/'), rdb_hash);
        let response = client.get(&hash_idx_url)
            .send().await.map_err(|e| format!("RDBHashIndex.bin: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("CDN returned {} for RDBHashIndex.bin", response.status()));
        }

        let hash_idx_bytes = response.bytes().await
            .map_err(|e| format!("RDBHashIndex.bin read: {}", e))?;
        log::info!("RDBHashIndex.bin downloaded: {} bytes, magic={:?}", hash_idx_bytes.len(), &hash_idx_bytes[..4.min(hash_idx_bytes.len())]);

        let final_bytes = verify::decompress_cdn(&hash_idx_bytes)?;
        log::info!("RDBHashIndex.bin decompressed: {} bytes", final_bytes.len());

        tokio::fs::write(&hash_idx_path, &final_bytes).await
            .map_err(|e| format!("Write RDBHashIndex.bin: {}", e))?;
    }

    if PATCH_CANCEL.load(Ordering::Relaxed) {
        return Err("Installation cancelled by user".into());
    }

    // Phase 4: Download RDB resources to staging.
    // NOTE: Resources are downloaded to staging/ directory. They need to be
    // processed into RDB/XX.rdbdata files for the game to use them.
    // This is handled by the game's patcher on first launch, or by our
    // verify+repair flow after rdbdata files exist.
    run_patching_inner(app, install_path).await?;

    // Write pre-compiled bxml view cache files so the game doesn't need to
    // generate them on first launch. Avoids a crash when connecting with
    // an existing character on a fresh install.
    let install_base = std::path::PathBuf::from(install_path);
    match bxml::write_bxml_cache(&install_base) {
        Ok(n) => { if n > 0 { log::info!("Wrote {} bxml cache files", n); } }
        Err(e) => { log::warn!("Failed to write bxml cache: {}", e); }
    }

    // Register in Windows Add/Remove Programs + create Start Menu shortcut.
    // Uses --register CLI subcommand. No elevation needed:
    //   - HKCU registry: writable by current user
    //   - %APPDATA% Start Menu: per-user, writable by current user
    //   - uninstall.exe copy: game dir was granted Everyone:Full by --prepare-dir
    #[cfg(target_os = "windows")]
    {
        let install_path_owned = install_path.to_string();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(our_exe) = std::env::current_exe() {
                let _ = std::process::Command::new(&our_exe)
                    .args(["--register", &install_path_owned])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .creation_flags(0x08000000) // CREATE_NO_WINDOW
                    .status();
            }
        }).await;
    }

    // After all phases complete, emit final complete
    let _ = app.emit(
        "patch:progress",
        &download::DownloadProgress {
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

    Ok(())
}

#[tauri::command]
fn pause_patching() -> Result<(), String> {
    PATCH_PAUSED.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn resume_patching() -> Result<(), String> {
    PATCH_PAUSED.store(false, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn cancel_patching() -> Result<(), String> {
    PATCH_CANCEL.store(true, Ordering::Relaxed);
    PATCH_PAUSED.store(false, Ordering::Relaxed); // Unpause so the loop can exit
    Ok(())
}

#[tauri::command]
async fn start_patching(app: tauri::AppHandle, install_path: String) -> Result<(), String> {
    // Prevent concurrent patching
    {
        let mut in_progress = PATCHING_IN_PROGRESS.lock().map_err(|e| e.to_string())?;
        if *in_progress {
            return Err("Patching is already in progress.".to_string());
        }
        *in_progress = true;
    }

    // Reset pause/cancel flags
    PATCH_PAUSED.store(false, Ordering::Relaxed);
    PATCH_CANCEL.store(false, Ordering::Relaxed);

    // Spawn the download work on a background task so this command returns immediately
    let app_clone = app.clone();
    tokio::spawn(async move {
        let result = run_patching_inner(&app_clone, &install_path).await;

        // Clear the in-progress flag
        if let Ok(mut in_progress) = PATCHING_IN_PROGRESS.lock() {
            *in_progress = false;
        }

        if let Err(e) = result {
            use tauri::Emitter;
            let _ = app_clone.emit(
                "patch:progress",
                &download::DownloadProgress {
                    bytes_downloaded: 0,
                    total_bytes: 0,
                    files_completed: 0,
                    files_total: 0,
                    speed_bps: 0,
                    current_file: format!("Error: {}", e),
                    phase: "error".into(),
                    failed_files: 0,
                },
            );
            log::error!("Patching failed: {}", e);
        }
    });

    Ok(())
}

/// Inner patching logic — downloads RDB resources and writes to rdbdata containers.
///
/// Uses le.idx for placement data: each resource goes to a specific offset
/// in a specific rdbdata file. Downloads from CDN, decompresses IOz1,
/// writes directly to rdbdata — no staging files.
async fn run_patching_inner(
    app: &tauri::AppHandle,
    install_path: &str,
) -> Result<(), String> {
    log::info!("=== run_patching_inner START === install_path={}", install_path);

    // Lower process priority during patching so we don't starve the OS, UI,
    // or other applications. This is what Steam/Battle.net do during downloads.
    // The OS scheduler will give us idle CPU time without hard-capping throughput.
    #[cfg(target_os = "windows")]
    {
        set_process_priority(BELOW_NORMAL_PRIORITY_CLASS);
    }

    let reporter = tauri_reporter::TauriReporter::new(app.clone());
    let base = std::path::PathBuf::from(install_path);

    let result = tsw_core::install::run_install_pipeline(
        &base,
        &reporter,
        &PATCH_PAUSED,
        &PATCH_CANCEL,
    )
    .await;

    // Restore normal process priority now that patching is done.
    #[cfg(target_os = "windows")]
    {
        set_process_priority(NORMAL_PRIORITY_CLASS);
    }

    result
}

// ─── Verification commands ───────────────────────────────────────────────────

#[tauri::command]
async fn start_verification(app: tauri::AppHandle, install_path: String) -> Result<(), String> {
    // Prevent concurrent verification
    {
        let mut in_progress = VERIFY_IN_PROGRESS.lock().map_err(|e| e.to_string())?;
        if *in_progress {
            return Err("Verification is already in progress.".to_string());
        }
        *in_progress = true;
    }

    // Reset cancel flag
    VERIFY_CANCEL.store(false, Ordering::Relaxed);

    let app_clone = app.clone();
    tokio::spawn(async move {
        let result = run_verification_inner(&app_clone, &install_path);

        // Clear the in-progress flag
        if let Ok(mut in_progress) = VERIFY_IN_PROGRESS.lock() {
            *in_progress = false;
        }

        match result {
            Ok(verify_result) => {
                // Cache the result
                if let Ok(mut cache) = VERIFY_RESULT_CACHE.lock() {
                    *cache = Some(verify_result);
                }
            }
            Err(e) => {
                let reporter = tauri_reporter::TauriReporter::new(app_clone.clone());
                reporter.on_verify(&verify::VerifyProgress {
                    entries_checked: 0,
                    entries_total: 0,
                    corrupted_count: 0,
                    bytes_scanned: 0,
                    current_file: String::new(),
                    phase: format!("error: {}", e),
                });
                log::error!("Verification failed: {}", e);
            }
        }
    });

    Ok(())
}

/// Inner verification logic — runs on a blocking thread since it does synchronous I/O.
fn run_verification_inner(
    app: &tauri::AppHandle,
    install_path: &str,
) -> Result<VerifyResult, String> {
    let base = std::path::PathBuf::from(install_path);
    let le_index =
        rdb::parse_le_index(&base.join("RDB").join("le.idx")).map_err(|e| e.to_string())?;

    let cancel_flag = std::sync::Arc::new(AtomicBool::new(false));
    let cancel_ref = cancel_flag.clone();

    // Bridge the global cancel flag to the local Arc
    let reporter = tauri_reporter::TauriReporter::new(app.clone());
    let result = verify::verify_integrity(&base, &le_index, &cancel_flag, move |progress| {
        // Check global cancel and propagate to local flag
        if VERIFY_CANCEL.load(Ordering::Relaxed) {
            cancel_ref.store(true, Ordering::Relaxed);
        }
        reporter.on_verify(progress);
    })?;

    Ok(result)
}

#[tauri::command]
fn cancel_verification() -> Result<(), String> {
    VERIFY_CANCEL.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn get_verification_status() -> Result<Option<VerifyResult>, String> {
    match VERIFY_RESULT_CACHE.lock() {
        Ok(cache) => Ok(cache.clone()),
        Err(e) => Err(format!("Failed to read verification cache: {}", e)),
    }
}

// ─── Repair command ──────────────────────────────────────────────────────────

/// Repair result sent to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct RepairResult {
    pub files_repaired: u32,
    pub files_failed: u32,
    pub total_files: u32,
}

#[tauri::command]
async fn repair_corrupted(app: tauri::AppHandle, install_path: String) -> Result<(), String> {
    use tauri::Emitter;

    // Get cached corrupted entries
    let corrupted: Vec<CorruptedEntry> = {
        let cache = VERIFY_RESULT_CACHE.lock().map_err(|e| e.to_string())?;
        match cache.as_ref() {
            Some(result) if !result.corrupted.is_empty() => result.corrupted.clone(),
            Some(_) => return Err("No corrupted files found. Run verification first.".to_string()),
            None => return Err("No verification result cached. Run verification first.".to_string()),
        }
    };

    let app_clone = app.clone();
    tokio::spawn(async move {
        let result = run_repair_inner(&app_clone, &install_path, &corrupted).await;
        match result {
            Ok(_) => {
                log::info!("Repair completed successfully");
            }
            Err(e) => {
                let _ = app_clone.emit(
                    "patch:progress",
                    &download::DownloadProgress {
                        bytes_downloaded: 0,
                        total_bytes: 0,
                        files_completed: 0,
                        files_total: 0,
                        speed_bps: 0,
                        current_file: String::new(),
                        phase: "error".into(),
                        failed_files: 0,
                    },
                );
                log::error!("Repair failed: {}", e);
            }
        }
    });

    Ok(())
}

/// Inner repair logic — downloads corrupted entries from CDN, decompresses, writes back.
async fn run_repair_inner(
    app: &tauri::AppHandle,
    install_path: &str,
    corrupted: &[CorruptedEntry],
) -> Result<(), String> {
    use tauri::Emitter;

    let base = std::path::PathBuf::from(install_path);
    let patch_config =
        config::parse_local_config(&base.join("LocalConfig.xml")).map_err(|e| e.to_string())?;
    let cdn_base_url = patch_config.http_patch_addr.replace("http://", "https://");

    let dl_config = DownloadConfig::default();
    let client = create_client(&dl_config).map_err(|e| e.to_string())?;

    let total = corrupted.len() as u32;
    let mut repaired = 0u32;
    let mut failed = 0u32;

    for (_i, entry) in corrupted.iter().enumerate() {
        // Emit progress
        let _ = app.emit(
            "patch:progress",
            &download::DownloadProgress {
                bytes_downloaded: 0,
                total_bytes: 0,
                files_completed: repaired,
                files_total: total,
                speed_bps: 0,
                current_file: format!("type={} id={}", entry.rdb_type, entry.id),
                phase: "repairing".into(),
                failed_files: failed,
            },
        );

        // Build CDN URL from the expected hash
        let hash_bytes = hash_hex_to_bytes(&entry.expected_hash);
        let url = match hash_bytes {
            Some(h) => rdb::cdn_url_from_hash(&cdn_base_url, &h),
            None => {
                log::error!(
                    "Invalid hash for type={} id={}: {}",
                    entry.rdb_type, entry.id, entry.expected_hash
                );
                failed += 1;
                continue;
            }
        };

        // Download to temp file
        let tmp_dir = base.join("staging").join("repair");
        std::fs::create_dir_all(&tmp_dir)
            .map_err(|e| format!("Failed to create repair staging dir: {}", e))?;
        let tmp_path = tmp_dir.join(format!("{}_{}", entry.rdb_type, entry.id));

        // Download with retries (reuse download_single_file)
        let mut download_ok = false;
        let mut last_error = String::new();
        for attempt in 0..=dl_config.max_retries {
            if attempt > 0 {
                let delay = std::time::Duration::from_secs(1 << (attempt - 1));
                tokio::time::sleep(delay).await;
            }

            match download::download_single_file(&client, &url, &tmp_path, 0, 0).await {
                Ok(_) => {
                    download_ok = true;
                    break;
                }
                Err(e) => {
                    last_error = format!("{}", e);
                    log::warn!(
                        "Repair download attempt {}/{} failed for type={} id={}: {}",
                        attempt + 1, dl_config.max_retries + 1,
                        entry.rdb_type, entry.id, last_error
                    );
                }
            }
        }

        if !download_ok {
            log::error!(
                "Failed to download repair for type={} id={}: {}",
                entry.rdb_type, entry.id, last_error
            );
            failed += 1;
            continue;
        }

        // Read downloaded data, decompress IOz1
        let raw_data = match std::fs::read(&tmp_path) {
            Ok(d) => d,
            Err(e) => {
                log::error!("Failed to read repair file: {}", e);
                failed += 1;
                continue;
            }
        };

        let decompressed = match verify::decompress_cdn(&raw_data) {
            Ok(d) => d,
            Err(e) => {
                log::error!(
                    "Failed to decompress repair data for type={} id={}: {}",
                    entry.rdb_type, entry.id, e
                );
                failed += 1;
                continue;
            }
        };

        // Verify decompressed length matches expected
        if decompressed.len() != entry.length as usize {
            log::error!(
                "Decompressed length mismatch for type={} id={}: expected {}, got {}",
                entry.rdb_type, entry.id, entry.length, decompressed.len()
            );
            failed += 1;
            continue;
        }

        // Write back to rdbdata
        match verify::write_to_rdbdata(
            &base,
            entry.file_num,
            entry.offset as u64,
            &decompressed,
            entry.length as usize,
        ) {
            Ok(()) => {
                repaired += 1;
                log::info!(
                    "Repaired type={} id={} in {:02}.rdbdata",
                    entry.rdb_type, entry.id, entry.file_num
                );
            }
            Err(e) => {
                log::error!(
                    "Failed to write repair for type={} id={}: {}",
                    entry.rdb_type, entry.id, e
                );
                failed += 1;
            }
        }

        // Clean up temp file
        let _ = std::fs::remove_file(&tmp_path);
    }

    // Emit completion
    let phase = if failed > 0 { "error" } else { "complete" };
    let _ = app.emit(
        "patch:progress",
        &download::DownloadProgress {
            bytes_downloaded: 0,
            total_bytes: 0,
            files_completed: repaired,
            files_total: total,
            speed_bps: 0,
            current_file: String::new(),
            phase: phase.into(),
            failed_files: failed,
        },
    );

    if failed > 0 {
        Err(format!("{} of {} files failed to repair", failed, total))
    } else {
        Ok(())
    }
}

/// Parse a hex hash string back into 16 bytes.
fn hash_hex_to_bytes(hex: &str) -> Option<[u8; 16]> {
    if hex.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for i in 0..16 {
        bytes[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

// ─── Fresh Install Downloader ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct InstallerProgress {
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
    pub phase: String,
}

#[tauri::command]
async fn download_installer(app: tauri::AppHandle, install_dir: Option<String>) -> Result<(), String> {
    use futures::StreamExt;
    use tauri::Emitter;
    use tokio::io::AsyncWriteExt;

    let url = "http://cdn.funcom.com/downloads/tsw/client/TheSecretWorldInstaller.exe";
    let dest = std::env::temp_dir().join("TheSecretWorldInstaller.exe");

    let dl_config = DownloadConfig::default();
    let client = create_client(&dl_config).map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| {
            let _ = app.emit("installer:progress", InstallerProgress {
                bytes_downloaded: 0, total_bytes: 0, phase: "error".into(),
            });
            format!("Failed to download installer: {}", e)
        })?;

    if !resp.status().is_success() {
        let _ = app.emit("installer:progress", InstallerProgress {
            bytes_downloaded: 0, total_bytes: 0, phase: "error".into(),
        });
        return Err(format!("CDN returned status {}", resp.status()));
    }

    let total_bytes = resp.content_length().unwrap_or(0);
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(&dest).await.map_err(|e| {
        let _ = app.emit("installer:progress", InstallerProgress {
            bytes_downloaded: 0, total_bytes, phase: "error".into(),
        });
        format!("Failed to create temp file: {}", e)
    })?;

    let mut bytes_downloaded: u64 = 0;
    let mut last_emit = std::time::Instant::now();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            let _ = app.emit("installer:progress", InstallerProgress {
                bytes_downloaded, total_bytes, phase: "error".into(),
            });
            format!("Download stream error: {}", e)
        })?;

        file.write_all(&chunk).await.map_err(|e| {
            let _ = app.emit("installer:progress", InstallerProgress {
                bytes_downloaded, total_bytes, phase: "error".into(),
            });
            format!("Failed to write to temp file: {}", e)
        })?;

        bytes_downloaded += chunk.len() as u64;

        // Emit progress every 64KB or 200ms
        if bytes_downloaded - (bytes_downloaded % (64 * 1024)) != (bytes_downloaded - chunk.len() as u64) - ((bytes_downloaded - chunk.len() as u64) % (64 * 1024))
            || last_emit.elapsed() >= std::time::Duration::from_millis(200)
        {
            let _ = app.emit("installer:progress", InstallerProgress {
                bytes_downloaded, total_bytes, phase: "downloading".into(),
            });
            last_emit = std::time::Instant::now();
        }
    }

    file.flush().await.map_err(|e| format!("Failed to flush installer file: {}", e))?;
    // Explicitly close the file handle before spawning — Windows holds an
    // exclusive lock on open files (ERROR_SHARING_VIOLATION / os error 32)
    drop(file);

    // Emit installing phase
    let _ = app.emit("installer:progress", InstallerProgress {
        bytes_downloaded, total_bytes, phase: "installing".into(),
    });

    // The Funcom installer is Inno Setup 5.3.10. Use /VERYSILENT for a zero-interaction
    // install (no wizard, no progress bar — just the UAC prompt). /SP- suppresses the
    // "This will install..." confirmation. /SUPPRESSMSGBOXES catches any stray dialogs.
    //
    // The installer's [Run] section auto-launches ClientPatcher.exe after install.
    // Since the installer runs elevated (UAC), ClientPatcher inherits admin privileges,
    // so our non-elevated taskkill can't touch it. Solution: create a batch script that
    // runs the installer AND kills ClientPatcher, then execute the whole script elevated.

    let install_target = install_dir.unwrap_or_else(|| r"C:\Program Files (x86)\Funcom\The Secret World".to_string());

    // Use the install_target the user chose — we told the installer to put files there.
    // Fall back to auto-detect only if that path doesn't validate (shouldn't happen).
    let target_for_result = install_target.clone();

    // Elevate our own executable with --install flag. This makes the UAC dialog
    // show "TSW Modern Launcher" instead of "Windows Command Processor".
    // The elevated child process runs the installer silently and kills ClientPatcher.

    // Run the elevated install helper via PowerShell Start-Process -Verb RunAs.
    // This triggers one UAC prompt showing our app name.
    // -Wait ensures we block until the install completes.
    #[cfg(not(target_os = "windows"))]
    {
        return Err("Fresh install is only supported on Windows".into());
    }

    #[cfg(target_os = "windows")]
    {
        let our_exe = std::env::current_exe()
            .map_err(|e| format!("Failed to get current exe path: {}", e))?;

        let status = tokio::task::spawn_blocking(move || {
            std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    &format!(
                        "Start-Process -FilePath '{}' -ArgumentList '--install \"{}\" \"{}\"' -Verb RunAs -Wait -WindowStyle Hidden",
                        our_exe.display(),
                        dest.display(),
                        install_target,
                    ),
                ])
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .status()
        })
        .await
        .map_err(|e| format!("Installer task panicked: {}", e))?
        .map_err(|e| format!("Failed to launch installer: {}", e))?;

        if !status.success() {
            let _ = app.emit("installer:progress", InstallerProgress {
                bytes_downloaded, total_bytes, phase: "error".into(),
            });
            return Err(format!("Installer exited with code {:?}", status.code()));
        }
    }

    // Use the install target path — we told the installer to put files there.
    // Validate it; fall back to auto-detect if somehow invalid.
    let detected_path = {
        let result = validate_install_dir_inner(&target_for_result);
        if result.valid {
            Some(target_for_result)
        } else {
            auto_detect_install_dir_inner()
        }
    };

    // Emit complete with detected path
    let _ = app.emit("installer:progress", InstallerProgress {
        bytes_downloaded, total_bytes,
        phase: if let Some(ref p) = detected_path {
            format!("complete:{}", p)
        } else {
            "complete".into()
        },
    });

    Ok(())
}

// ─── Reddit News Feed ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsPost {
    pub title: String,
    pub author: String,
    pub created_utc: f64,
    pub permalink: String,
    pub score: i64,
    pub num_comments: i64,
}

#[tauri::command]
async fn fetch_news() -> Result<Vec<NewsPost>, String> {
    let client = reqwest::Client::builder()
        .user_agent("TSWModernLauncher/0.1.0")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let resp = client
        .get("https://www.reddit.com/r/TheSecretWorld.json?limit=10")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch Reddit news: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Reddit returned status {}", resp.status()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Reddit JSON: {}", e))?;

    let children = body
        .get("data")
        .and_then(|d| d.get("children"))
        .and_then(|c| c.as_array())
        .ok_or_else(|| "Unexpected Reddit JSON structure".to_string())?;

    let posts: Vec<NewsPost> = children
        .iter()
        .filter_map(|child| {
            let data = child.get("data")?;
            Some(NewsPost {
                title: data.get("title")?.as_str()?.to_string(),
                author: data.get("author")?.as_str()?.to_string(),
                created_utc: data.get("created_utc")?.as_f64()?,
                permalink: data.get("permalink")?.as_str()?.to_string(),
                score: data.get("score")?.as_i64()?,
                num_comments: data.get("num_comments")?.as_i64()?,
            })
        })
        .collect();

    Ok(posts)
}

// ─── Bundle mode commands ────────────────────────────────────────────────────

#[tauri::command]
async fn set_bundle_mode(app: tauri::AppHandle, mode: String) -> Result<(), String> {
    use tauri_plugin_store::StoreExt;

    if mode != "full" && mode != "minimum" {
        return Err(format!("Invalid bundle mode '{}'. Must be 'full' or 'minimum'.", mode));
    }

    let store = app.store("settings.json").map_err(|e| {
        log::warn!("Failed to open store for bundle_mode: {}", e);
        format!("Failed to open settings store: {}", e)
    })?;

    store.set("bundle_mode", serde_json::Value::String(mode));
    store.save().map_err(|e| {
        log::warn!("Failed to save bundle_mode: {}", e);
        format!("Failed to save settings: {}", e)
    })?;

    Ok(())
}

#[tauri::command]
async fn get_bundle_mode(app: tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_store::StoreExt;

    let store = match app.store("settings.json") {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Failed to open store for bundle_mode read: {}, defaulting to 'full'", e);
            return Ok("full".to_string());
        }
    };

    match store.get("bundle_mode") {
        Some(serde_json::Value::String(mode)) if mode == "full" || mode == "minimum" => {
            Ok(mode.clone())
        }
        _ => Ok("full".to_string()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Query the system for available display resolutions.
/// On Windows, uses EnumDisplaySettingsW to get actual monitor modes.
/// Returns deduplicated, sorted list of "WIDTHxHEIGHT" strings.
#[tauri::command]
fn get_display_modes() -> Vec<String> {
    let mut modes: Vec<String> = Vec::new();

    #[cfg(target_os = "windows")]
    {
        use std::mem;

        #[repr(C)]
        #[allow(non_snake_case)]
        struct DEVMODEW {
            dmDeviceName: [u16; 32],
            dmSpecVersion: u16,
            dmDriverVersion: u16,
            dmSize: u16,
            dmDriverExtra: u16,
            dmFields: u32,
            // Union: position/display
            dmPosition_x: i32,
            dmPosition_y: i32,
            dmDisplayOrientation: u32,
            dmDisplayFixedOutput: u32,
            dmColor: i16,
            dmDuplex: i16,
            dmYResolution: i16,
            dmTTOption: i16,
            dmCollate: i16,
            dmFormName: [u16; 32],
            dmLogPixels: u16,
            dmBitsPerPel: u32,
            dmPelsWidth: u32,
            dmPelsHeight: u32,
            dmDisplayFlags: u32,
            dmDisplayFrequency: u32,
            // ... remaining fields not needed
            _pad: [u8; 128],
        }

        extern "system" {
            fn EnumDisplaySettingsW(
                lpszDeviceName: *const u16,
                iModeNum: u32,
                lpDevMode: *mut DEVMODEW,
            ) -> i32;
        }

        let mut i = 0u32;
        loop {
            let mut dm: DEVMODEW = unsafe { mem::zeroed() };
            dm.dmSize = mem::size_of::<DEVMODEW>() as u16;
            let result = unsafe { EnumDisplaySettingsW(std::ptr::null(), i, &mut dm) };
            if result == 0 {
                break;
            }
            // Only include modes with at least 32-bit color and reasonable size
            if dm.dmBitsPerPel >= 32 && dm.dmPelsWidth >= 800 && dm.dmPelsHeight >= 600 {
                modes.push(format!("{}x{}", dm.dmPelsWidth, dm.dmPelsHeight));
            }
            i += 1;
        }
    }

    // Deduplicate and sort by width then height
    modes.sort_by(|a, b| {
        let parse = |s: &str| -> (u32, u32) {
            let parts: Vec<&str> = s.split('x').collect();
            let w = parts.first().and_then(|p| p.parse().ok()).unwrap_or(0);
            let h = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
            (w, h)
        };
        parse(a).cmp(&parse(b))
    });
    modes.dedup();

    // Fallback if detection returned nothing (non-Windows or no modes found)
    if modes.is_empty() {
        modes = vec![
            "800x600", "1024x768", "1280x720", "1280x800", "1280x1024",
            "1366x768", "1440x900", "1600x900", "1680x1050", "1920x1080",
            "2560x1440", "3840x2160",
        ].into_iter().map(String::from).collect();
    }

    modes
}

/// Get free disk space for the drive containing the given path.
/// Returns 0 if the check fails.
#[cfg(target_os = "windows")]
fn get_free_disk_space(path: &std::path::Path) -> u64 {
    use std::os::windows::ffi::OsStrExt;

    extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut free_bytes: u64 = 0;
    let mut _total: u64 = 0;
    let mut _total_free: u64 = 0;

    let result = unsafe {
        GetDiskFreeSpaceExW(wide.as_ptr(), &mut free_bytes, &mut _total, &mut _total_free)
    };

    if result != 0 { free_bytes } else { 0 }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            validate_install_dir,
            auto_detect_install_dir,
            launch_game,
            launch_patcher,
            authenticate,
            check_for_updates_cmd,
            get_patch_status_cmd,
            start_patching,
            start_full_install,
            pause_patching,
            resume_patching,
            cancel_patching,
            start_verification,
            cancel_verification,
            get_verification_status,
            repair_corrupted,
            set_bundle_mode,
            get_bundle_mode,
            fetch_news,
            download_installer,
            get_display_modes
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Real TSW install directory, relative to src-tauri/.
    /// Returns None if the game isn't installed locally (e.g. CI).
    fn tsw_path() -> Option<String> {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../The Secret World");
        path.canonicalize()
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    }

    #[test]
    fn valid_tsw_directory() {
        let path = match tsw_path() {
            Some(p) => p,
            None => return, // skip in CI
        };
        let result = validate_install_dir_inner(&path);
        assert!(result.valid, "Expected valid=true, got: {:?}", result);
        assert!(
            result.version.is_some(),
            "Expected version string, got None"
        );
        let version = result.version.as_ref().unwrap();
        assert!(
            version.contains("TSW") || version.contains("tsw"),
            "Version string should mention TSW, got: {}",
            version
        );
        // Real install has 42 .rdbdata files
        assert!(
            result.rdb_count > 0,
            "Expected rdb_count > 0, got {}",
            result.rdb_count
        );
        assert_eq!(result.message, "Valid TSW install directory.");
    }

    #[test]
    fn nonexistent_directory() {
        let result = validate_install_dir_inner("/tmp/definitely_does_not_exist_tsw_12345");
        assert!(!result.valid);
        assert!(result.message.contains("does not exist"));
        assert_eq!(result.rdb_count, 0);
        assert!(result.version.is_none());
    }

    #[test]
    fn empty_string_path() {
        let result = validate_install_dir_inner("");
        assert!(!result.valid);
        assert!(result.message.contains("empty"));
    }

    #[test]
    fn path_to_file_not_directory() {
        // Use Cargo.toml as a path that exists but is a file
        let path = format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR"));
        let result = validate_install_dir_inner(&path);
        assert!(!result.valid);
        assert!(result.message.contains("not a directory"));
    }

    #[test]
    fn empty_directory_missing_all_markers() {
        let tmp = std::env::temp_dir().join("tsw_test_empty_dir");
        let _ = fs::create_dir_all(&tmp);
        let result = validate_install_dir_inner(tmp.to_str().unwrap());
        assert!(!result.valid);
        assert!(result.message.contains("Missing required files"));
        assert!(result.message.contains("TheSecretWorld.exe"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn swl_detection_missing_tsw_exe() {
        // Simulate a directory that has DX11 exe and RDB but no TheSecretWorld.exe
        let tmp = std::env::temp_dir().join("tsw_test_swl_detect");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("RDB")).unwrap();
        fs::write(tmp.join("TheSecretWorldDX11.exe"), b"fake").unwrap();
        fs::write(tmp.join("LocalConfig.xml"), b"<config/>").unwrap();

        let result = validate_install_dir_inner(tmp.to_str().unwrap());
        assert!(!result.valid);
        assert!(
            result.message.contains("Secret World Legends")
                || result.message.contains("SWL"),
            "Should mention SWL, got: {}",
            result.message
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn fresh_install_no_executables() {
        // After Funcom installer runs: LocalConfig.xml exists, no RDB/, no .exe files
        let tmp = std::env::temp_dir().join("tsw_test_fresh_install");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("LocalConfig.xml"), b"<config/>").unwrap();

        let result = validate_install_dir_inner(tmp.to_str().unwrap());
        assert!(result.valid, "Fresh install should be valid, got: {}", result.message);
        assert!(result.message.contains("Fresh") || result.message.contains("patching"),
            "Should indicate fresh install state, got: {}", result.message);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn fresh_install_with_rdb() {
        // Partially patched state: LocalConfig.xml + RDB/ exist, no .exe files
        let tmp = std::env::temp_dir().join("tsw_test_fresh_install_rdb");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("RDB")).unwrap();
        fs::write(tmp.join("LocalConfig.xml"), b"<config/>").unwrap();

        let result = validate_install_dir_inner(tmp.to_str().unwrap());
        assert!(result.valid, "Fresh install with RDB should be valid, got: {}", result.message);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn directory_with_no_rdbdata_files() {
        // Has all marker files but RDB/ is empty
        let tmp = std::env::temp_dir().join("tsw_test_empty_rdb");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("RDB")).unwrap();
        fs::write(tmp.join("TheSecretWorld.exe"), b"fake").unwrap();
        fs::write(tmp.join("TheSecretWorldDX11.exe"), b"fake").unwrap();
        fs::write(tmp.join("LocalConfig.xml"), b"<config/>").unwrap();

        let result = validate_install_dir_inner(tmp.to_str().unwrap());
        assert!(result.valid, "Should be valid even with 0 rdbdata files");
        assert_eq!(result.rdb_count, 0);
        assert!(result.version.is_none()); // No Version.txt

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn directory_with_only_some_markers() {
        // Only TheSecretWorld.exe, missing everything else
        let tmp = std::env::temp_dir().join("tsw_test_partial");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("TheSecretWorld.exe"), b"fake").unwrap();

        let result = validate_install_dir_inner(tmp.to_str().unwrap());
        assert!(!result.valid);
        assert!(result.message.contains("Missing required files"));
        assert!(result.message.contains("TheSecretWorldDX11.exe"));
        assert!(result.message.contains("RDB/"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn path_with_special_characters() {
        let tmp = std::env::temp_dir().join("tsw test (special) chars & stuff!");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let result = validate_install_dir_inner(tmp.to_str().unwrap());
        assert!(!result.valid); // Missing files, but shouldn't crash
        assert!(result.message.contains("Missing required files"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn launch_game_validates_exe_exists() {
        // launch_game with a nonexistent path should return an error
        let result = launch_game(
            "/tmp/nonexistent_tsw_path".into(),
            "dx9".into(),
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("not found"),
            "Error should mention not found, got: {}",
            err
        );
    }

    #[test]
    fn launch_game_dx_version_mapping() {
        // "dx9" → TheSecretWorld.exe, "dx11" → TheSecretWorldDX11.exe
        // We can't actually launch, but we can test error messages mention the right exe
        let err9 = launch_game("/tmp/nonexistent".into(), "dx9".into(), None).unwrap_err();
        assert!(
            err9.contains("TheSecretWorld.exe"),
            "dx9 should map to TheSecretWorld.exe, got: {}",
            err9
        );

        let err11 = launch_game("/tmp/nonexistent".into(), "dx11".into(), None).unwrap_err();
        assert!(
            err11.contains("TheSecretWorldDX11.exe"),
            "dx11 should map to TheSecretWorldDX11.exe, got: {}",
            err11
        );
    }

    #[test]
    fn launch_game_dx9_case_insensitive() {
        let err = launch_game("/tmp/nonexistent".into(), "DX9".into(), None).unwrap_err();
        assert!(
            err.contains("TheSecretWorld.exe") && !err.contains("DX11"),
            "DX9 (uppercase) should map to TheSecretWorld.exe, got: {}",
            err
        );
    }

    // ─── authenticate tests ─────────────────────────────────────────────────

    #[test]
    fn authenticate_empty_username() {
        let result = authenticate_inner("", "password123");
        assert!(!result.success);
        assert!(
            result.message.contains("Username"),
            "Should mention username, got: {}",
            result.message
        );
    }

    #[test]
    fn authenticate_whitespace_username() {
        let result = authenticate_inner("   ", "password123");
        assert!(!result.success);
        assert!(result.message.contains("Username"));
    }

    #[test]
    fn authenticate_empty_password() {
        let result = authenticate_inner("user@example.com", "");
        assert!(!result.success);
        assert!(
            result.message.contains("Password"),
            "Should mention password, got: {}",
            result.message
        );
    }

    #[test]
    fn authenticate_both_empty() {
        // Username error takes priority
        let result = authenticate_inner("", "");
        assert!(!result.success);
        assert!(
            result.message.contains("Username"),
            "Username error should take priority when both empty, got: {}",
            result.message
        );
    }

    #[test]
    fn authenticate_valid_credentials() {
        let result = authenticate_inner("user@example.com", "secret123");
        assert!(result.success);
        assert!(
            result.message.contains("accepted"),
            "Should confirm acceptance, got: {}",
            result.message
        );
    }

    #[test]
    fn launch_game_login_key_none_backward_compatible() {
        // login_key: None should behave identically to the original signature
        let err = launch_game("/tmp/nonexistent".into(), "dx11".into(), None).unwrap_err();
        assert!(
            err.contains("not found"),
            "login_key: None should not change behavior, got: {}",
            err
        );
    }
}
