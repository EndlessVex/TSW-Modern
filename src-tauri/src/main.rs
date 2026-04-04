// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // --prepare-dir <path>: Create the install directory and grant current user
    // full access. Runs elevated via UAC so it can write to Program Files.
    // After this, all downloads run non-elevated.
    if args.len() >= 3 && args[1] == "--prepare-dir" {
        let target_dir = &args[2];
        let path = std::path::Path::new(target_dir);

        // Create the directory tree
        if let Err(e) = std::fs::create_dir_all(path) {
            eprintln!("Failed to create directory: {}", e);
            std::process::exit(1);
        }

        // Grant Everyone full control so the non-elevated launcher can write files.
        // Using icacls which is available on all modern Windows.
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

    // --install <installer_path> <target_dir>: Run Funcom installer silently
    // and kill ClientPatcher. Used as a fallback if someone has the installer.
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
