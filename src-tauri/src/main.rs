// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // --prepare-dir <path>: Create install directory, set permissions,
    // and register the game in Windows Add/Remove Programs.
    if args.len() >= 3 && args[1] == "--prepare-dir" {
        let target_dir = &args[2];
        let path = std::path::Path::new(target_dir);

        if let Err(e) = std::fs::create_dir_all(path) {
            eprintln!("Failed to create directory: {}", e);
            std::process::exit(1);
        }

        #[cfg(target_os = "windows")]
        {
            // Grant write permissions
            let _ = std::process::Command::new("icacls")
                .args([target_dir, "/grant", "Everyone:(OI)(CI)F", "/T"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();

            // Register in Add/Remove Programs
            let our_exe = std::env::current_exe().unwrap_or_default();
            let uninstall_cmd = format!(
                "\"{}\" --uninstall \"{}\"",
                our_exe.display(),
                target_dir
            );
            let icon_path = std::path::Path::new(target_dir)
                .join("ClientPatcher.exe");

            let reg_key = r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\TheSecretWorld_TSWDownloader";

            let reg_cmds = [
                format!("reg add \"{}\" /v DisplayName /t REG_SZ /d \"The Secret World\" /f", reg_key),
                format!("reg add \"{}\" /v Publisher /t REG_SZ /d \"Funcom\" /f", reg_key),
                format!("reg add \"{}\" /v DisplayVersion /t REG_SZ /d \"1.15\" /f", reg_key),
                format!("reg add \"{}\" /v UninstallString /t REG_SZ /d \"{}\" /f", reg_key, uninstall_cmd),
                format!("reg add \"{}\" /v InstallLocation /t REG_SZ /d \"{}\" /f", reg_key, target_dir),
                format!("reg add \"{}\" /v DisplayIcon /t REG_SZ /d \"{}\" /f", reg_key, icon_path.display()),
                format!("reg add \"{}\" /v EstimatedSize /t REG_DWORD /d 44040192 /f", reg_key), // ~42GB in KB
                format!("reg add \"{}\" /v NoModify /t REG_DWORD /d 1 /f", reg_key),
                format!("reg add \"{}\" /v NoRepair /t REG_DWORD /d 1 /f", reg_key),
            ];

            for cmd in &reg_cmds {
                let _ = std::process::Command::new("cmd")
                    .args(["/C", cmd])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        }

        std::process::exit(0);
    }

    // --uninstall <path>: Remove the game installation and registry entry.
    if args.len() >= 3 && args[1] == "--uninstall" {
        let target_dir = &args[2];

        // Confirm with the user via a simple message box
        #[cfg(target_os = "windows")]
        {
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
            // MB_YESNO | MB_ICONQUESTION = 0x24
            let result = unsafe {
                MessageBoxW(std::ptr::null(), msg.as_ptr(), caption.as_ptr(), 0x24)
            };

            if result != 6 {
                // IDYES = 6
                std::process::exit(0);
            }

            // Remove the install directory
            if let Err(e) = std::fs::remove_dir_all(target_dir) {
                let err_msg = to_wide(&format!("Failed to remove files: {}", e));
                let err_cap = to_wide("Uninstall Error");
                unsafe { MessageBoxW(std::ptr::null(), err_msg.as_ptr(), err_cap.as_ptr(), 0x10); }
                std::process::exit(1);
            }

            // Remove registry entry
            let reg_key = r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\TheSecretWorld_TSWDownloader";
            let _ = std::process::Command::new("reg")
                .args(["delete", reg_key, "/f"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();

            let done_msg = to_wide("The Secret World has been uninstalled.");
            let done_cap = to_wide("Uninstall Complete");
            unsafe { MessageBoxW(std::ptr::null(), done_msg.as_ptr(), done_cap.as_ptr(), 0x40); }
        }

        std::process::exit(0);
    }

    // --install <installer_path> <target_dir>: Run Funcom installer silently.
    // Kept as fallback — not used in normal flow.
    if args.len() >= 4 && args[1] == "--install" {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let installer_path = &args[2];
        let target_dir = &args[3];

        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let killer = std::thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                let _ = std::process::Command::new("taskkill")
                    .args(["/F", "/IM", "ClientPatcher.exe"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        });

        let status = std::process::Command::new(installer_path)
            .args([
                "/VERYSILENT",
                "/SP-",
                "/SUPPRESSMSGBOXES",
                &format!("/DIR={}", target_dir),
            ])
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
