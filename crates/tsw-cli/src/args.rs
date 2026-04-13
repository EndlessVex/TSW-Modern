//! clap argument definitions for the `tsw-downloader` binary.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "tsw-downloader",
    version,
    about = "Download, verify, and uninstall The Secret World for Linux",
    long_about = None,
)]
pub struct Cli {
    /// Config file path (default: ~/.config/tsw-downloader/config.toml).
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    /// Enable info-level logging. Repeat for debug (`-vv`).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Suppress non-error output.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Disable ANSI color in output.
    #[arg(long, global = true)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// First-run interactive setup.
    Init(InitArgs),

    /// Download and install the game (or resume / update).
    Install(InstallArgs),

    /// Verify file integrity against the manifest.
    Verify(VerifyArgs),

    /// Remove game files (with safety guardrails).
    Uninstall(UninstallArgs),
}

#[derive(Debug, Parser)]
pub struct InitArgs {
    /// Skip the interactive prompt, use this path.
    #[arg(long)]
    pub install_dir: Option<PathBuf>,

    /// Overwrite an existing config without asking.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Parser)]
pub struct InstallArgs {
    /// Override config; use this install directory.
    #[arg(long)]
    pub install_dir: Option<PathBuf>,

    /// Skip loose client files (Data/, exes, dlls) — RDB only.
    #[arg(long)]
    pub skip_client_files: bool,

    /// Skip post-install bxml/shader cache writes.
    #[arg(long)]
    pub skip_bxml: bool,

    /// Skip post-install verify pass.
    #[arg(long)]
    pub no_verify: bool,

    /// Don't prompt for confirmation.
    #[arg(short = 'y', long)]
    pub yes: bool,
}

#[derive(Debug, Parser)]
pub struct VerifyArgs {
    /// Override config; use this install directory.
    #[arg(long)]
    pub install_dir: Option<PathBuf>,

    /// Emit JSON report instead of human output.
    #[arg(long)]
    pub json: bool,

    /// Write full corrupted-file list to a file.
    #[arg(long)]
    pub report: Option<PathBuf>,
}

#[derive(Debug, Parser)]
pub struct UninstallArgs {
    /// Override config; path to remove.
    #[arg(long)]
    pub install_dir: Option<PathBuf>,

    /// Also delete ~/.config/tsw-downloader/.
    #[arg(long)]
    pub purge: bool,

    /// Don't prompt for confirmation.
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Bypass the "looks like TSW" marker check.
    #[arg(long)]
    pub force: bool,
}
