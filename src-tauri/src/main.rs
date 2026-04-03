// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // When launched with --install <installer_path> <target_dir>, run the installer
    // elevated (this process was re-launched via RunAs) and kill ClientPatcher after.
    // This makes the UAC dialog show "TSW Modern Launcher" instead of "cmd.exe".
    if args.len() >= 4 && args[1] == "--install" {
        let installer_path = &args[2];
        let target_dir = &args[3];

        // Spawn a background thread that continuously kills ClientPatcher.exe.
        // This runs DURING the install so ClientPatcher never gets a chance to show
        // its window — it's killed within ~200ms of spawning.
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

        // Run the Funcom installer silently (Inno Setup 5.3.10)
        let status = std::process::Command::new(installer_path)
            .args([
                "/VERYSILENT",
                "/SP-",
                "/SUPPRESSMSGBOXES",
                &format!("/DIR={}", target_dir),
            ])
            .status();

        // Give the killer a moment to catch any final ClientPatcher spawn, then stop it.
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
