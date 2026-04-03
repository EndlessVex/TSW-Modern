// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // When launched with --install <installer_path> <target_dir>, run the installer
    // elevated (this process was re-launched via RunAs) and kill ClientPatcher after.
    // This makes the UAC dialog show "TSW Modern Launcher" instead of "cmd.exe".
    if args.len() >= 4 && args[1] == "--install" {
        let installer_path = &args[2];
        let target_dir = &args[3];

        // Run the Funcom installer silently (Inno Setup 5.3.10)
        let status = std::process::Command::new(installer_path)
            .args([
                "/VERYSILENT",
                "/SP-",
                "/SUPPRESSMSGBOXES",
                &format!("/DIR={}", target_dir),
            ])
            .status();

        if let Err(e) = status {
            eprintln!("Installer failed: {}", e);
            std::process::exit(1);
        }

        // Kill ClientPatcher.exe — it auto-launches from the installer's [Run] section.
        // We're already elevated so taskkill can reach it.
        std::thread::sleep(std::time::Duration::from_secs(2));
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "ClientPatcher.exe"])
            .output();
        std::thread::sleep(std::time::Duration::from_secs(1));
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "ClientPatcher.exe"])
            .output();

        std::process::exit(0);
    }

    tsw_modern_launcher::run();
}
