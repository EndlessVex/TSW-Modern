// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
            let _ = std::process::Command::new("icacls")
                .args([target_dir, "/grant", "Everyone:(OI)(CI)F", "/T"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }

        std::process::exit(0);
    }

    // --register <path>: Register the game in Windows Add/Remove Programs.
    // Runs elevated so it can write to HKCU (no elevation needed).
    if args.len() >= 3 && args[1] == "--register" {
        #[cfg(target_os = "windows")]
        {
            let target_dir = &args[2];
            let our_exe = std::env::current_exe().unwrap_or_default();
            let uninstall_cmd = format!(
                "\"{}\" --uninstall \"{}\"",
                our_exe.display(),
                target_dir
            );
            let icon_path = std::path::Path::new(target_dir).join("ClientPatcher.exe");

            let reg_key = r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\TheSecretWorld_TSWDownloader";

            let values = [
                format!("/v DisplayName /t REG_SZ /d \"The Secret World\" /f"),
                format!("/v Publisher /t REG_SZ /d \"Funcom\" /f"),
                format!("/v DisplayVersion /t REG_SZ /d \"1.15\" /f"),
                format!("/v UninstallString /t REG_SZ /d \"{}\" /f", uninstall_cmd),
                format!("/v InstallLocation /t REG_SZ /d \"{}\" /f", target_dir),
                format!("/v DisplayIcon /t REG_SZ /d \"{}\" /f", icon_path.display()),
                format!("/v EstimatedSize /t REG_DWORD /d 44040192 /f"),
                format!("/v NoModify /t REG_DWORD /d 1 /f"),
                format!("/v NoRepair /t REG_DWORD /d 1 /f"),
            ];

            for val in &values {
                let cmd = format!("reg add \"{}\" {}", reg_key, val);
                let _ = std::process::Command::new("cmd")
                    .args(["/C", &cmd])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        }

        std::process::exit(0);
    }

    // --uninstall <path>: Remove the game and registry entry.
    if args.len() >= 3 && args[1] == "--uninstall" {
        let target_dir = &args[2];

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

            let result = unsafe {
                MessageBoxW(std::ptr::null(), msg.as_ptr(), caption.as_ptr(), 0x24)
            };

            if result != 6 {
                std::process::exit(0);
            }

            if let Err(e) = std::fs::remove_dir_all(target_dir) {
                let err_msg = to_wide(&format!("Failed to remove files: {}", e));
                let err_cap = to_wide("Uninstall Error");
                unsafe { MessageBoxW(std::ptr::null(), err_msg.as_ptr(), err_cap.as_ptr(), 0x10); }
                std::process::exit(1);
            }

            let reg_key = r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\TheSecretWorld_TSWDownloader";
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

    // --install: Legacy Funcom installer fallback (not used in normal flow)
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
