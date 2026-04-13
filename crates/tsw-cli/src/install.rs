//! `tsw-downloader install` — download and install The Secret World.
//!
//! The bulk of the work happens in `tsw_core::install::run_install_pipeline`.
//! This module handles the Linux-facing concerns: config resolution,
//! install-directory validation, Ctrl-C handling, client files, post-install
//! bxml cache, and the optional verify pass.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::args::InstallArgs;
use crate::config_file;
use crate::errors::{ConfigError, UserCancelled, VerifyFoundCorrupted};
use crate::reporter::CliReporter;

pub fn run(args: InstallArgs, config_path_override: Option<PathBuf>) -> Result<i32> {
    let install_dir = resolve_install_dir(&args, config_path_override)?;

    if !install_dir.exists() {
        if !args.yes {
            anyhow::bail!(
                "install directory does not exist: {} (pass --yes to create it)",
                install_dir.display()
            );
        }
        std::fs::create_dir_all(&install_dir)
            .with_context(|| format!("creating {}", install_dir.display()))?;
    }

    let reporter: Arc<dyn tsw_core::progress::ProgressReporter> = Arc::new(CliReporter::new());

    let cancel_flag: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let pause_flag: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

    install_ctrlc_handler(&cancel_flag)?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;

    runtime.block_on(async {
        run_pipeline(&args, &install_dir, &reporter, &pause_flag, &cancel_flag).await
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
) -> Result<()> {
    // The user must provide a directory that already has LocalConfig.xml.
    // The Linux CLI does not bootstrap a fresh install from scratch —
    // the user must have either copied LocalConfig.xml from an existing
    // install or extracted it from the Funcom installer via Wine/Proton.
    let local_config_path = install_dir.join("LocalConfig.xml");
    if !local_config_path.exists() {
        anyhow::bail!(
            "{} is missing. Copy it from an existing TSW install or extract \
             it from the Funcom installer before running `tsw-downloader install`.",
            local_config_path.display()
        );
    }

    // Fail fast with a CLI-friendly message if LocalConfig.xml is malformed.
    let patch_config = tsw_core::config::parse_local_config(&local_config_path)
        .with_context(|| format!("parsing {}", local_config_path.display()))?;
    let cdn_base_url = patch_config.http_patch_addr.replace("http://", "https://");

    // Write embedded static files (post-install scripts and such).
    tsw_core::client_files::write_static_files(install_dir).map_err(|e| anyhow::anyhow!(e))?;

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

    // RDB install pipeline — the big one. Handles bootstrap (le.idx,
    // RDBHashIndex.bin), rdbdata file creation, and the parallel download
    // loop. Same function the Windows launcher uses.
    tsw_core::install::run_install_pipeline(install_dir, reporter, pause_flag, cancel_flag)
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

        if !verify_result.corrupted.is_empty() {
            return Err(VerifyFoundCorrupted {
                count: verify_result.corrupted.len() as u64,
            }
            .into());
        }
    }

    Ok(())
}

fn resolve_install_dir(
    args: &InstallArgs,
    config_path_override: Option<PathBuf>,
) -> Result<PathBuf> {
    if let Some(dir) = &args.install_dir {
        return Ok(dir.clone());
    }
    let config_path = config_path_override
        .map(Ok)
        .unwrap_or_else(config_file::default_config_path)?;
    if !config_path.exists() {
        return Err(ConfigError(format!(
            "no install directory provided and no config at {} — run `tsw-downloader init` first",
            config_path.display()
        ))
        .into());
    }
    let config = config_file::read(&config_path)?;
    Ok(config.install.dir)
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
