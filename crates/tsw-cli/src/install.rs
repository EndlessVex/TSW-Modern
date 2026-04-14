//! `tsw-downloader install` — download and install The Secret World.
//!
//! The bulk of the work happens in `tsw_core::install::run_install_pipeline`.
//! This module handles the Linux-facing concerns: first-run install-directory
//! prompt, config save, Ctrl-C handling, client files, post-install bxml cache,
//! and the optional verify pass.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::args::InstallArgs;
use crate::config_file::{self, Config, DownloadSection, InstallSection, CURRENT_SCHEMA_VERSION};
use crate::errors::{UserCancelled, VerifyFoundCorrupted};
use crate::init;
use crate::reporter::CliReporter;

pub fn run(args: InstallArgs, config_path_override: Option<PathBuf>) -> Result<i32> {
    let install_dir = resolve_or_prompt_install_dir(&args, config_path_override)?;

    if let Some(n) = args.concurrency {
        if !confirm_concurrency_override(n, args.yes)? {
            println!("Aborted.");
            return Ok(0);
        }
    }

    let reporter: Arc<dyn tsw_core::progress::ProgressReporter> = Arc::new(CliReporter::new());

    let cancel_flag: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let pause_flag: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

    install_ctrlc_handler(&cancel_flag)?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;

    let concurrency_override = args.concurrency;
    runtime.block_on(async {
        run_pipeline(
            &args,
            &install_dir,
            &reporter,
            &pause_flag,
            &cancel_flag,
            concurrency_override,
        )
        .await
    })?;

    if cancel_flag.load(Ordering::Relaxed) {
        return Err(UserCancelled.into());
    }

    println!("Install complete: {}", install_dir.display());
    Ok(0)
}

async fn run_pipeline(
    args: &InstallArgs,
    install_dir: &Path,
    reporter: &Arc<dyn tsw_core::progress::ProgressReporter>,
    pause_flag: &Arc<AtomicBool>,
    cancel_flag: &Arc<AtomicBool>,
    concurrency_override: Option<usize>,
) -> Result<()> {
    // Write embedded static files first. This creates LocalConfig.xml with
    // the default Funcom CDN URL if it doesn't already exist, plus the
    // LanguagePrefs.xml and RDB/ directory. The Windows launcher uses this
    // same function on fresh installs — no user-supplied LocalConfig.xml is
    // needed.
    tsw_core::client_files::write_static_files(install_dir).map_err(|e| anyhow::anyhow!(e))?;

    // Parse LocalConfig.xml to get the CDN URL.
    let local_config_path = install_dir.join("LocalConfig.xml");
    let patch_config = tsw_core::config::parse_local_config(&local_config_path)
        .with_context(|| format!("parsing {}", local_config_path.display()))?;
    let cdn_base_url = patch_config.http_patch_addr.replace("http://", "https://");

    // Loose client files (Data/, exes, dlls). Downloads via the CDN.
    if !args.skip_client_files {
        tsw_core::client_files::download_client_files(
            reporter,
            &cdn_base_url,
            install_dir,
            pause_flag,
            cancel_flag,
        )
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    }

    if cancel_flag.load(Ordering::Relaxed) {
        return Ok(());
    }

    // RDB install pipeline — handles bootstrap (le.idx, RDBHashIndex.bin),
    // rdbdata file creation, and the parallel download loop. Same function
    // the Windows launcher uses, with an optional concurrency override from
    // --concurrency.
    tsw_core::install::run_install_pipeline(
        install_dir,
        reporter,
        pause_flag,
        cancel_flag,
        concurrency_override,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    if cancel_flag.load(Ordering::Relaxed) {
        return Ok(());
    }

    // Post-install bxml/shader cache writes.
    if !args.skip_bxml {
        tsw_core::bxml::write_bxml_cache(install_dir).map_err(|e| anyhow::anyhow!(e))?;
    }

    // Post-install verify pass. Parses le.idx again and walks every entry.
    // Texture entries (type 1010004) are filtered out of the error count on
    // non-Windows-x86 targets because the Rust fallback encoder produces
    // valid-but-not-bit-identical DXT output. See verify_cmd::TEXTURE_RDB_TYPE
    // and the --skip-textures flag for the full explanation.
    if !args.no_verify {
        let le_idx_path = install_dir.join("RDB").join("le.idx");
        let le_index = tsw_core::rdb::parse_le_index(&le_idx_path)
            .with_context(|| format!("parsing {}", le_idx_path.display()))?;
        let verify_cancel = Arc::new(AtomicBool::new(false));
        let reporter_for_verify = Arc::clone(reporter);
        let verify_result = tsw_core::verify::verify_integrity(
            install_dir,
            &le_index,
            &verify_cancel,
            move |p| reporter_for_verify.on_verify(p),
        )
        .map_err(|e| anyhow::anyhow!(e))?;

        let (texture_mismatches, real_corrupted): (Vec<_>, Vec<_>) = verify_result
            .corrupted
            .iter()
            .cloned()
            .partition(|e| e.rdb_type == crate::verify_cmd::TEXTURE_RDB_TYPE);

        if args.skip_textures && !texture_mismatches.is_empty() {
            println!();
            println!(
                "Verify finished: {} non-texture corrupted, {} texture mismatches skipped",
                real_corrupted.len(),
                texture_mismatches.len()
            );
            crate::verify_cmd::print_texture_skip_note();
        }

        let effective_corrupted = if args.skip_textures {
            real_corrupted
        } else {
            verify_result.corrupted
        };

        if !effective_corrupted.is_empty() {
            return Err(VerifyFoundCorrupted {
                count: effective_corrupted.len() as u64,
            }
            .into());
        }
    }

    Ok(())
}

/// Resolve the install directory, prompting the user interactively if nothing
/// has been configured yet. Saves the chosen directory to the config file so
/// subsequent runs of `install`, `verify`, or `uninstall` don't need to ask
/// again.
fn resolve_or_prompt_install_dir(
    args: &InstallArgs,
    config_path_override: Option<PathBuf>,
) -> Result<PathBuf> {
    // Explicit --install-dir flag wins.
    if let Some(dir) = &args.install_dir {
        let canonical = init::ensure_install_dir_exists(dir)?;
        save_install_dir_to_config(&canonical, config_path_override.clone())?;
        return Ok(canonical);
    }

    // Existing config wins next.
    let config_path = config_path_override
        .clone()
        .map(Ok)
        .unwrap_or_else(config_file::default_config_path)?;

    if config_path.exists() {
        let config = config_file::read(&config_path)?;
        let saved = &config.install.dir;
        // Re-create the directory if the user rm -rf'd it between runs so
        // `install` is idempotent.
        return init::ensure_install_dir_exists(saved);
    }

    // No flag, no config — first-run prompt for a directory, then create it.
    println!("No install directory configured yet. Let's set one up.");
    let chosen = init::prompt_for_install_dir_interactive()?;
    let canonical = init::ensure_install_dir_exists(&chosen)?;
    save_install_dir_to_config(&canonical, config_path_override)?;
    println!("Saved install directory to {}", config_path.display());
    Ok(canonical)
}

fn save_install_dir_to_config(
    install_dir: &Path,
    config_path_override: Option<PathBuf>,
) -> Result<()> {
    let config_path = config_path_override
        .map(Ok)
        .unwrap_or_else(config_file::default_config_path)?;
    let config = Config {
        schema_version: CURRENT_SCHEMA_VERSION,
        install: InstallSection {
            dir: install_dir.to_path_buf(),
        },
        download: DownloadSection::default(),
    };
    config_file::write(&config_path, &config)?;
    Ok(())
}

/// Explain what --concurrency does, warn about unusual values, and give the
/// user one last chance to back out. Returns `Ok(true)` if the user accepts
/// (or --yes was passed), `Ok(false)` if they decline, and an `Err` for
/// unusable values (like 0).
fn confirm_concurrency_override(n: usize, yes: bool) -> Result<bool> {
    if n == 0 {
        anyhow::bail!("--concurrency must be at least 1");
    }

    println!();
    println!("You passed --concurrency {}, which overrides the automatic tuning.", n);
    println!();
    println!("The default picks a connection count based on available RAM:");
    println!("  over 8 GB available: 64 concurrent downloads");
    println!("  over 4 GB available: 32 concurrent downloads");
    println!("  otherwise:           16 concurrent downloads");
    println!();
    println!("Higher values saturate more bandwidth but cost memory (large");
    println!("files buffer in RAM until written) and may get rate-limited by");
    println!("the CDN. Lower values are slower than the default.");

    if n > 128 {
        println!();
        println!(
            "WARNING: {} is unusually high. The CDN may throttle or refuse",
            n
        );
        println!("connections, and memory pressure can cause large-file downloads");
        println!("to fail. 64 is a safe upper bound for most systems.");
    } else if n < 4 {
        println!();
        println!(
            "WARNING: {} will be significantly slower than the 16-connection",
            n
        );
        println!("minimum default. Only use this if you're intentionally rate-limiting.");
    }

    println!();

    if yes {
        println!("--yes flag set, proceeding without confirmation.");
        return Ok(true);
    }

    use dialoguer::Input;
    let response: String = Input::new()
        .with_prompt("Proceed with this concurrency setting? [y/N]")
        .default("n".to_string())
        .allow_empty(true)
        .interact_text()
        .context("reading confirmation from stdin")?;

    Ok(matches!(
        response.trim().to_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn install_ctrlc_handler(cancel_flag: &Arc<AtomicBool>) -> Result<()> {
    let flag = Arc::clone(cancel_flag);
    ctrlc::set_handler(move || {
        if flag.load(Ordering::Relaxed) {
            eprintln!("\nAbort.");
            std::process::exit(130);
        }
        eprintln!("\nCancelling — press Ctrl-C again to abort.");
        flag.store(true, Ordering::Relaxed);
    })
    .context("installing Ctrl-C handler")?;
    Ok(())
}
