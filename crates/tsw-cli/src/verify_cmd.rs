//! `tsw-downloader verify` — verify file integrity against le.idx.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::args::VerifyArgs;
use crate::config_file;
use crate::errors::VerifyFoundCorrupted;
use crate::reporter::CliReporter;

/// RDB type for compressed texture resources. The only resource type whose
/// hash depends on the encoder pipeline, and therefore the only type that
/// shows false-positive corruption on non-Windows-x86 targets.
pub(crate) const TEXTURE_RDB_TYPE: u32 = 1010004;

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

    // Split the corrupted list into "real" corruption and texture mismatches.
    // On non-Windows-x86 targets the encoder falls through to a Rust fallback
    // whose output differs from the original patcher by ~1 ULP on ~78% of
    // textures — valid DXT1 blocks, but not bit-identical to le.idx hashes.
    // The `--skip-textures` flag (default true on these targets) filters
    // type 1010004 entries out of the real-corruption count.
    let (texture_mismatches, real_corrupted): (Vec<_>, Vec<_>) = result
        .corrupted
        .iter()
        .cloned()
        .partition(|e| e.rdb_type == TEXTURE_RDB_TYPE);

    if args.json {
        let json = serde_json::to_string_pretty(&result)
            .context("serializing verify result to JSON")?;
        println!("{}", json);
    } else {
        println!("Entries checked: {}", result.entries_checked);
        if args.skip_textures {
            println!("Corrupted (non-texture): {}", real_corrupted.len());
            if !texture_mismatches.is_empty() {
                println!(
                    "Texture mismatches (skipped): {} — see --skip-textures=false to include",
                    texture_mismatches.len()
                );
                print_texture_skip_note();
            }
        } else {
            println!("Corrupted: {}", result.corrupted.len());
        }
    }

    if let Some(report_path) = args.report {
        let report = serde_json::to_string_pretty(&result.corrupted)
            .context("serializing corrupted list")?;
        std::fs::write(&report_path, report)
            .with_context(|| format!("writing report to {}", report_path.display()))?;
        println!("Report written to {}", report_path.display());
    }

    let effective_corrupted = if args.skip_textures {
        &real_corrupted
    } else {
        &result.corrupted
    };

    if !effective_corrupted.is_empty() {
        return Err(VerifyFoundCorrupted {
            count: effective_corrupted.len() as u64,
        }
        .into());
    }

    Ok(0)
}

/// Print the explanation the user needs for the texture-skip default. Called
/// whenever skip-textures actually filtered something out of the count.
pub(crate) fn print_texture_skip_note() {
    println!();
    println!("Note: texture verification is skipped by default on this build.");
    println!("Texture mip generation relies on x87 floating-point behavior");
    println!("that the Rust fallback encoder only matches bit-for-bit on 32-bit");
    println!("Windows. On every other target, individual bytes drift by ~1 ULP,");
    println!("which shows up as a hash mismatch even though the textures are");
    println!("valid DXT1 blocks and render correctly in-game. Pass");
    println!("--skip-textures=false to include them in the count anyway.");
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
