//! `tsw-downloader uninstall` — delete game files with safety guardrails.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::args::UninstallArgs;
use crate::config_file;

/// Guardrail error — uninstall refuses to proceed.
#[derive(Debug, thiserror::Error)]
pub enum GuardrailError {
    #[error("uninstall target must be an absolute path, got: {0}")]
    NotAbsolute(PathBuf),

    #[error("uninstall target is in the path blocklist: {0}")]
    Blocklisted(PathBuf),

    #[error("uninstall target has too few path components: {0}")]
    TooShallow(PathBuf),

    #[error("{0} does not look like a TSW install (no marker files found). Use --force to override.")]
    NoMarkerFiles(PathBuf),
}

/// Check all guardrails against the given path. Returns `Ok(())` if safe
/// to delete, `Err(GuardrailError)` otherwise.
pub fn check_guardrails(path: &Path, force_skip_markers: bool) -> Result<(), GuardrailError> {
    check_absolute(path)?;
    check_blocklist(path)?;
    check_component_count(path)?;
    if !force_skip_markers {
        check_marker_files(path)?;
    }
    Ok(())
}

fn check_absolute(path: &Path) -> Result<(), GuardrailError> {
    if !path.is_absolute() {
        return Err(GuardrailError::NotAbsolute(path.to_path_buf()));
    }
    Ok(())
}

fn check_blocklist(path: &Path) -> Result<(), GuardrailError> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    // Unix/Linux system directories — rejected regardless of host OS
    // because the binary may receive Unix-style paths when invoked
    // under WSL or similar cross-platform shells.
    const SYSTEM_BLOCKLIST: &[&str] = &[
        "/", "/home", "/root", "/usr", "/etc", "/var", "/opt", "/bin", "/sbin",
        "/lib", "/lib64", "/boot", "/dev", "/proc", "/sys", "/tmp", "/mnt", "/media",
        "/srv", "/run",
    ];

    for &blocked in SYSTEM_BLOCKLIST {
        let blocked_path = Path::new(blocked);
        if canonical == blocked_path {
            return Err(GuardrailError::Blocklisted(canonical));
        }
    }

    // Windows system directories. The Windows build of tsw-downloader
    // (produced by the `build-windows` CI job as a side artifact) makes
    // the uninstall guardrails relevant on Windows too. Compare via
    // lowercased string to handle drive-letter case and backslash drift
    // from canonicalize's extended-path prefix (\\?\C:\...).
    #[cfg(windows)]
    {
        let canonical_str = canonical.to_string_lossy().to_ascii_lowercase();
        // Strip Windows extended-path prefix if present.
        let normalized = canonical_str
            .strip_prefix(r"\\?\")
            .unwrap_or(&canonical_str)
            .trim_end_matches('\\');

        const WINDOWS_BLOCKLIST: &[&str] = &[
            "c:",
            "c:\\",
            "c:\\windows",
            "c:\\program files",
            "c:\\program files (x86)",
            "c:\\users",
            "c:\\programdata",
            "d:",
            "d:\\",
            "e:",
            "e:\\",
        ];
        for &blocked in WINDOWS_BLOCKLIST {
            if normalized == blocked {
                return Err(GuardrailError::Blocklisted(canonical));
            }
        }

        // Also refuse the current user's profile root (C:\Users\<name>)
        // and common user-data subdirectories. The generic home check
        // below also runs on Windows but relies on PathBuf equality,
        // which fails against canonical's \\?\ extended prefix — so we
        // do a normalized string comparison here instead.
        if let Some(home) = home_dir() {
            let home_str = home.to_string_lossy().to_ascii_lowercase();
            let home_normalized = home_str
                .strip_prefix(r"\\?\")
                .unwrap_or(&home_str)
                .trim_end_matches('\\')
                .to_string();

            const USER_SUBDIRS: &[&str] = &[
                "",
                "desktop",
                "documents",
                "downloads",
                "pictures",
                "videos",
                "music",
                "appdata",
                "appdata\\local",
                "appdata\\roaming",
            ];
            for suffix in USER_SUBDIRS {
                let blocked = if suffix.is_empty() {
                    home_normalized.clone()
                } else {
                    format!("{}\\{}", home_normalized, suffix)
                };
                if normalized == blocked {
                    return Err(GuardrailError::Blocklisted(canonical));
                }
            }
        }
    }

    if let Some(home) = home_dir() {
        let home_blocklist: &[&str] = &[
            "",
            "Desktop",
            "Documents",
            "Downloads",
            ".config",
            ".local",
            ".local/share",
            "Games",
            ".steam",
            ".wine",
        ];
        for suffix in home_blocklist {
            let blocked = if suffix.is_empty() {
                home.clone()
            } else {
                home.join(suffix)
            };
            if canonical == blocked {
                return Err(GuardrailError::Blocklisted(canonical));
            }
        }
    }

    Ok(())
}

fn check_component_count(path: &Path) -> Result<(), GuardrailError> {
    // Under $HOME, require at least 2 additional components so that
    // `$HOME/Games/TSW` (a typical Lutris/Proton install location) is
    // allowed while `$HOME/x` or `$HOME/Games` alone is not — those
    // latter cases are caught by the explicit blocklist above anyway.
    // Under `/` (no home match), require at least 4 components so that
    // `/foo` and `/var/tsw` are rejected but `/mnt/games/tsw/install`
    // passes. The blocklist above already handles the dangerous roots.
    let components = path.components().count();
    let min_components = match home_dir() {
        Some(home) if path.starts_with(&home) => 2 + home.components().count(),
        _ => 4,
    };
    if components < min_components {
        return Err(GuardrailError::TooShallow(path.to_path_buf()));
    }
    Ok(())
}

fn check_marker_files(path: &Path) -> Result<(), GuardrailError> {
    const MARKERS: &[&str] = &[
        "RDB/le.idx",
        "RDB/RDBHashIndex.bin",
        "TheSecretWorld.exe",
        "ClientPatcher.exe",
    ];
    for marker in MARKERS {
        if path.join(marker).exists() {
            return Ok(());
        }
    }
    Err(GuardrailError::NoMarkerFiles(path.to_path_buf()))
}

fn home_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|u| u.home_dir().to_path_buf())
}

pub fn run(args: UninstallArgs, config_path_override: Option<PathBuf>) -> Result<i32> {
    let install_dir = resolve_install_dir(&args, config_path_override.clone())?;
    let canonical = install_dir.canonicalize().unwrap_or(install_dir.clone());

    check_guardrails(&canonical, args.force).map_err(|e| anyhow::anyhow!(e))?;

    let (file_count, total_bytes) = tally_directory(&canonical)?;
    let size_gb = total_bytes as f64 / 1_000_000_000.0;

    println!("About to delete all files in:");
    println!("  {}", canonical.display());
    println!("Estimated size: {:.1} GB ({} files)", size_gb, file_count);

    if !args.yes {
        if !prompt_delete_confirmation()? {
            println!("Aborted.");
            return Ok(0);
        }
    }

    walk_and_delete(&canonical)?;
    println!("Removed.");

    if args.purge {
        let config_path = config_path_override
            .map(Ok)
            .unwrap_or_else(config_file::default_config_path)?;
        if let Some(config_dir) = config_path.parent() {
            if config_dir.exists() {
                println!();
                println!(
                    "--purge: also delete config directory {}?",
                    config_dir.display()
                );
                if !args.yes && !prompt_delete_confirmation()? {
                    println!("Config kept.");
                    return Ok(0);
                }
                std::fs::remove_dir_all(config_dir)
                    .with_context(|| format!("removing {}", config_dir.display()))?;
                println!("Config removed.");
            }
        }
    }

    Ok(0)
}

fn resolve_install_dir(
    args: &UninstallArgs,
    config_path_override: Option<PathBuf>,
) -> Result<PathBuf> {
    if let Some(dir) = &args.install_dir {
        return Ok(dir.clone());
    }
    let config_path = config_path_override
        .map(Ok)
        .unwrap_or_else(config_file::default_config_path)?;
    if !config_path.exists() {
        anyhow::bail!(
            "no install directory provided and no config at {} — run `tsw-downloader init` first",
            config_path.display()
        );
    }
    let config = config_file::read(&config_path)?;
    Ok(config.install.dir)
}

fn tally_directory(path: &Path) -> Result<(u64, u64)> {
    let mut file_count = 0u64;
    let mut total_bytes = 0u64;
    for entry in walkdir::WalkDir::new(path)
        .same_file_system(true)
        .follow_links(false)
    {
        let entry = entry.context("walking directory for tally")?;
        if entry.file_type().is_file() {
            file_count += 1;
            if let Ok(meta) = entry.metadata() {
                total_bytes += meta.len();
            }
        }
    }
    Ok((file_count, total_bytes))
}

fn prompt_delete_confirmation() -> Result<bool> {
    use dialoguer::Input;
    let input: String = Input::new()
        .with_prompt("Type DELETE to confirm")
        .allow_empty(true)
        .interact_text()
        .context("reading confirmation from stdin")?;
    Ok(input.trim() == "DELETE")
}

fn walk_and_delete(path: &Path) -> Result<()> {
    // contents_first=true: children before parent directories.
    // same_file_system=true: refuse to cross mount points.
    // follow_links=false: never descend into symlinked directories.
    for entry in walkdir::WalkDir::new(path)
        .same_file_system(true)
        .contents_first(true)
        .follow_links(false)
    {
        let entry = entry.context("walking directory for deletion")?;
        let entry_path = entry.path();

        let sym_meta = std::fs::symlink_metadata(entry_path)
            .with_context(|| format!("stat {}", entry_path.display()))?;
        let file_type = sym_meta.file_type();

        if file_type.is_symlink() {
            std::fs::remove_file(entry_path)
                .with_context(|| format!("unlinking symlink {}", entry_path.display()))?;
        } else if file_type.is_file() {
            std::fs::remove_file(entry_path)
                .with_context(|| format!("deleting {}", entry_path.display()))?;
        } else if file_type.is_dir() {
            if entry_path == path {
                continue;
            }
            std::fs::remove_dir(entry_path)
                .with_context(|| format!("removing directory {}", entry_path.display()))?;
        }
    }

    std::fs::remove_dir(path)
        .with_context(|| format!("removing root directory {}", path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_relative_path() {
        let result = check_guardrails(Path::new("relative/path"), true);
        assert!(matches!(result, Err(GuardrailError::NotAbsolute(_))));
    }

    #[test]
    #[cfg(unix)]
    fn rejects_usr() {
        // Unix-only: /usr is a real system path on Linux/macOS.
        // On Windows, /usr is not a meaningful absolute path.
        let result = check_guardrails(Path::new("/usr"), true);
        assert!(matches!(result, Err(GuardrailError::Blocklisted(_))));
    }

    #[test]
    fn rejects_home_directory() {
        if let Some(home) = home_dir() {
            let result = check_guardrails(&home, true);
            assert!(result.is_err());
        }
    }

    #[test]
    fn rejects_home_games() {
        if let Some(home) = home_dir() {
            let games = home.join("Games");
            let result = check_guardrails(&games, true);
            assert!(matches!(result, Err(GuardrailError::Blocklisted(_))));
        }
    }

    #[test]
    #[cfg(unix)]
    fn rejects_shallow_path() {
        // Unix-only: /foo is a single-component absolute path on Linux.
        // On Windows, /foo has different semantics.
        let result = check_guardrails(Path::new("/foo"), true);
        assert!(result.is_err());
    }

    #[test]
    #[cfg(unix)]
    fn rejects_missing_markers_without_force() {
        // Use a path under $HOME with enough components to pass the
        // depth check but NOT in the blocklist, so the only remaining
        // guardrail is the marker-file check. That way we assert the
        // exact reason for rejection (NoMarkerFiles) rather than some
        // earlier check accidentally firing first.
        let Some(home) = home_dir() else { return };
        let base = home
            .join("tsw-guardrail-test-nomark")
            .join("nested")
            .join("empty");
        std::fs::create_dir_all(&base).ok();
        let result = check_guardrails(&base, false);
        assert!(
            matches!(result, Err(GuardrailError::NoMarkerFiles(_))),
            "expected NoMarkerFiles, got {:?}",
            result
        );
        let _ = std::fs::remove_dir_all(home.join("tsw-guardrail-test-nomark"));
    }

    #[test]
    #[cfg(unix)]
    fn accepts_home_games_tsw_with_markers() {
        // Regression test for the first-run UX bug where ~/Games/TSW
        // was rejected as TooShallow. Construct a fake install at
        // $HOME/tsw-guardrail-test-accept/Games/TSW with a marker file
        // and verify all guardrails pass.
        let Some(home) = home_dir() else { return };
        let base = home
            .join("tsw-guardrail-test-accept")
            .join("Games")
            .join("TSW");
        let rdb = base.join("RDB");
        std::fs::create_dir_all(&rdb).ok();
        std::fs::write(rdb.join("le.idx"), b"fake").ok();

        let result = check_guardrails(&base, false);
        assert!(
            result.is_ok(),
            "expected guardrails to accept $HOME/.../Games/TSW with markers, got {:?}",
            result
        );
        let _ = std::fs::remove_dir_all(home.join("tsw-guardrail-test-accept"));
    }
}
