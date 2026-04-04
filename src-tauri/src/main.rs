// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Perform the actual uninstall: confirmation dialog, delete files, clean registry.
/// Extracted as a function so both --uninstall and --uninstall-from-temp can call it.
#[cfg(target_os = "windows")]
fn do_uninstall(target_dir: &str) {
    use std::os::windows::process::CommandExt;

    extern "system" {
        fn MessageBoxW(
            hWnd: *const std::ffi::c_void,
            lpText: *const u16,
            lpCaption: *const u16,
            uType: u32,
        ) -> i32;
    }

    fn to_wide(s: &str) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let msg = to_wide(&format!(
        "This will remove The Secret World from:\n{}\n\nAll game files will be deleted.\n\nContinue?",
        target_dir
    ));
    let caption = to_wide("Uninstall The Secret World");

    let result = unsafe {
        MessageBoxW(std::ptr::null(), msg.as_ptr(), caption.as_ptr(), 0x24)
    };

    // MB_YESNO: Yes=6, No=7
    if result != 6 {
        std::process::exit(0);
    }

    if let Err(e) = std::fs::remove_dir_all(target_dir) {
        let err_msg = to_wide(&format!("Failed to remove files: {}", e));
        let err_cap = to_wide("Uninstall Error");
        unsafe { MessageBoxW(std::ptr::null(), err_msg.as_ptr(), err_cap.as_ptr(), 0x10); }
        std::process::exit(1);
    }

    // Remove the parent directory if it's now empty (e.g. C:\Program Files (x86)\Funcom).
    // Only removes if empty — won't touch it if other games live there.
    let target_path = std::path::Path::new(target_dir);
    if let Some(parent) = target_path.parent() {
        let _ = std::fs::remove_dir(parent); // fails silently if not empty
    }

    // Remove Start Menu shortcut (check both per-user and system-wide)
    if let Ok(appdata) = std::env::var("APPDATA") {
        let user_shortcut = std::path::Path::new(&appdata)
            .join(r"Microsoft\Windows\Start Menu\Programs\The Secret World.lnk");
        let _ = std::fs::remove_file(&user_shortcut);
    }
    let system_shortcut = std::path::Path::new(
        r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\The Secret World.lnk",
    );
    let _ = std::fs::remove_file(system_shortcut);

    // Clean all registry entries (correct key + broken legacy variants)
    let keys_to_delete = [
        r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\TheSecretWorld_TSWDownloader",
        r#"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\TheSecretWorld_TSWDownloader""#,
        r#"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\TheSecretWorld_TSWDownloader""#,
    ];
    for key in &keys_to_delete {
        let _ = std::process::Command::new("reg")
            .args(["delete", key, "/f"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .status();
    }

    let done_msg = to_wide("The Secret World has been uninstalled.");
    let done_cap = to_wide("Uninstall Complete");
    unsafe { MessageBoxW(std::ptr::null(), done_msg.as_ptr(), done_cap.as_ptr(), 0x40); }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // --prepare-dir <path>: Create install directory and set permissions.
    if args.len() >= 3 && args[1] == "--prepare-dir" {
        let target_dir = &args[2];

        if let Err(e) = std::fs::create_dir_all(target_dir) {
            eprintln!("Failed to create directory: {}", e);
            std::process::exit(1);
        }

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            let _ = std::process::Command::new("icacls")
                .args([target_dir, "/grant", "Everyone:(OI)(CI)F", "/T"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .status();
        }

        std::process::exit(0);
    }

    // --register <path>: Register the game in Windows Add/Remove Programs.
    // Copies our exe into the game dir as uninstall.exe so it's self-contained.
    if args.len() >= 3 && args[1] == "--register" {
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;

            let target_dir = &args[2];
            let install_dir = std::path::Path::new(target_dir);

            // Copy ourselves into the game directory as uninstall.exe
            let uninstall_exe = install_dir.join("uninstall.exe");
            if let Ok(our_exe) = std::env::current_exe() {
                let _ = std::fs::copy(&our_exe, &uninstall_exe);
            }

            let uninstall_cmd = format!(
                "\"{}\" --uninstall \"{}\"",
                uninstall_exe.display(),
                target_dir
            );
            let icon_path = install_dir.join("ClientPatcher.exe");

            let reg_key = r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\TheSecretWorld_TSWDownloader";

            // Clean up broken legacy key with trailing quote in name
            let broken_key = r#"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\TheSecretWorld_TSWDownloader""#;
            let _ = std::process::Command::new("reg")
                .args(["delete", broken_key, "/f"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .creation_flags(0x08000000)
                .status();

            let str_values: Vec<(&str, String)> = vec![
                ("DisplayName", "The Secret World".to_string()),
                ("Publisher", "Funcom".to_string()),
                ("DisplayVersion", "1.15".to_string()),
                ("UninstallString", uninstall_cmd),
                ("InstallLocation", target_dir.to_string()),
                ("DisplayIcon", icon_path.display().to_string()),
            ];

            for (name, value) in &str_values {
                let _ = std::process::Command::new("reg")
                    .args(["add", reg_key, "/v", name, "/t", "REG_SZ", "/d", value, "/f"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .creation_flags(0x08000000)
                    .status();
            }

            let dword_values: Vec<(&str, &str)> = vec![
                ("EstimatedSize", "44040192"),
                ("NoModify", "1"),
                ("NoRepair", "1"),
            ];

            for (name, value) in &dword_values {
                let _ = std::process::Command::new("reg")
                    .args(["add", reg_key, "/v", name, "/t", "REG_DWORD", "/d", value, "/f"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .creation_flags(0x08000000)
                    .status();
            }

            // Create Start Menu shortcut to ClientPatcher.exe.
            // Use per-user Start Menu (%APPDATA%\...\Programs\) — no elevation needed.
            // Windows Search indexes per-user shortcuts the same as system-wide ones.
            let patcher_exe = install_dir.join("ClientPatcher.exe");
            let start_menu = if let Ok(appdata) = std::env::var("APPDATA") {
                std::path::PathBuf::from(appdata)
                    .join(r"Microsoft\Windows\Start Menu\Programs")
            } else {
                // Fallback to system-wide (would need elevation, best-effort)
                std::path::PathBuf::from(
                    r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs",
                )
            };
            let shortcut_path = start_menu.join("The Secret World.lnk");

            if start_menu.is_dir() {
                let ps_script = format!(
                    "$ws = New-Object -ComObject WScript.Shell; \
                     $sc = $ws.CreateShortcut('{}'); \
                     $sc.TargetPath = '{}'; \
                     $sc.WorkingDirectory = '{}'; \
                     $sc.IconLocation = '{},0'; \
                     $sc.Description = 'The Secret World'; \
                     $sc.Save()",
                    shortcut_path.display(),
                    patcher_exe.display(),
                    install_dir.display(),
                    patcher_exe.display(),
                );

                let _ = std::process::Command::new("powershell")
                    .args(["-NoProfile", "-Command", &ps_script])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .creation_flags(0x08000000)
                    .status();
            }
        }

        std::process::exit(0);
    }

    // --uninstall <path>: Remove the game and registry entry.
    // If running from inside the game directory (as uninstall.exe), we first
    // copy ourselves to %TEMP% and relaunch from there so we can delete the
    // game directory. Same pattern as Inno Setup's self-deletion.
    if args.len() >= 3 && args[1] == "--uninstall" {
        let target_dir = &args[2];

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;

            // If we're running from inside the target dir, copy to temp and relaunch.
            // The relaunched copy passes --uninstall-from-temp so we don't loop.
            if let Ok(our_exe) = std::env::current_exe() {
                let our_dir = our_exe.parent().unwrap_or(std::path::Path::new(""));
                let target_path = std::path::Path::new(target_dir);
                // Canonicalize both to compare reliably (handles trailing slashes, case, etc.)
                let our_canon = our_dir.canonicalize().unwrap_or_else(|_| our_dir.to_path_buf());
                let target_canon = target_path.canonicalize().unwrap_or_else(|_| target_path.to_path_buf());

                if our_canon == target_canon {
                    let temp_exe = std::env::temp_dir().join("tsw_uninstall_temp.exe");
                    if std::fs::copy(&our_exe, &temp_exe).is_ok() {
                        let _ = std::process::Command::new(&temp_exe)
                            .args(["--uninstall-from-temp", target_dir])
                            .creation_flags(0x08000000)
                            .spawn();
                        std::process::exit(0);
                    }
                    // If copy fails, fall through and try anyway
                }
            }

            do_uninstall(target_dir);
        }

        std::process::exit(0);
    }

    // --uninstall-from-temp <path>: Called by --uninstall after copying to %TEMP%.
    // Does the actual work, then schedules self-deletion of the temp copy.
    if args.len() >= 3 && args[1] == "--uninstall-from-temp" {
        let target_dir = &args[2];

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;

            do_uninstall(target_dir);

            // Schedule deletion of our temp copy after a short delay (best-effort).
            // ping -n 2 adds ~1s delay so we've exited before del runs.
            if let Ok(our_exe) = std::env::current_exe() {
                let _ = std::process::Command::new("cmd")
                    .args([
                        "/C",
                        &format!(
                            "ping -n 2 127.0.0.1 >nul & del /f /q \"{}\"",
                            our_exe.display()
                        ),
                    ])
                    .creation_flags(0x08000000) // CREATE_NO_WINDOW
                    .spawn();
            }
        }

        std::process::exit(0);
    }

    // --install: Legacy Funcom installer fallback (not used in normal flow)
    if args.len() >= 4 && args[1] == "--install" {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let installer_path = &args[2];
        let target_dir = &args[3];

        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let killer = std::thread::spawn(move || {
            #[cfg(target_os = "windows")]
            use std::os::windows::process::CommandExt;
            while !stop_clone.load(Ordering::Relaxed) {
                #[allow(unused_mut)]
                let mut cmd = std::process::Command::new("taskkill");
                cmd.args(["/F", "/IM", "ClientPatcher.exe"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                #[cfg(target_os = "windows")]
                cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
                let _ = cmd.status();
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        });

        let status = std::process::Command::new(installer_path)
            .args(["/VERYSILENT", "/SP-", "/SUPPRESSMSGBOXES", &format!("/DIR={}", target_dir)])
            .status();

        std::thread::sleep(std::time::Duration::from_secs(3));
        stop.store(true, Ordering::Relaxed);
        let _ = killer.join();

        if let Err(e) = status {
            eprintln!("Installer failed: {}", e);
            std::process::exit(1);
        }

        std::process::exit(0);
    }

    tsw_modern_launcher::run();
}
