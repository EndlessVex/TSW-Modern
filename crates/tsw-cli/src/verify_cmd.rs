//! `tsw-downloader verify` — verify file integrity against le.idx.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::args::VerifyArgs;
use crate::config_file;
use crate::errors::VerifyFoundCorrupted;
use crate::reporter::CliReporter;

pub fn run(args: VerifyArgs, config_path_override: Option<PathBuf>) -> Result<i32> {
    let install_dir = resolve_install_dir(args.install_dir, config_path_override)?;

    let le_idx_path = install_dir.join("RDB").join("le.idx");
    let le_index = tsw_core::rdb::parse_le_index(&le_idx_path)
        .with_context(|| format!("parsing {}", le_idx_path.display()))?;

    let cancel_flag = Arc::new(AtomicBool::new(false));
    let reporter: Arc<dyn tsw_core::progress::ProgressReporter> = Arc::new(CliReporter::new());
    let reporter_for_callback = Arc::clone(&reporter);

    let result = tsw_core::verify::verify_integrity(
        &install_dir,
        &le_index,
        &cancel_flag,
        move |p| reporter_for_callback.on_verify(p),
    )
    .map_err(|e| anyhow::anyhow!(e))?;

    if args.json {
        let json = serde_json::to_string_pretty(&result)
            .context("serializing verify result to JSON")?;
        println!("{}", json);
    } else {
        println!("Entries checked: {}", result.entries_checked);
        println!("Corrupted: {}", result.corrupted.len());
    }

    if let Some(report_path) = args.report {
        let report = serde_json::to_string_pretty(&result.corrupted)
            .context("serializing corrupted list")?;
        std::fs::write(&report_path, report)
            .with_context(|| format!("writing report to {}", report_path.display()))?;
        println!("Report written to {}", report_path.display());
    }

    if !result.corrupted.is_empty() {
        return Err(VerifyFoundCorrupted {
            count: result.corrupted.len() as u64,
        }
        .into());
    }

    Ok(0)
}

fn resolve_install_dir(
    override_dir: Option<PathBuf>,
    config_path_override: Option<PathBuf>,
) -> Result<PathBuf> {
    if let Some(dir) = override_dir {
        return Ok(dir);
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
