//! Config file read/write for tsw-downloader.
//!
//! Config lives at `~/.config/tsw-downloader/config.toml` (or the
//! platform equivalent via the `directories` crate).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub schema_version: u32,
    pub install: InstallSection,
    #[serde(default)]
    pub download: DownloadSection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstallSection {
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DownloadSection {
    // Reserved for future tuning knobs. Empty today because
    // run_install_pipeline uses RAM-adaptive concurrency internally.
}

impl Default for DownloadSection {
    fn default() -> Self {
        Self {}
    }
}

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Resolve the default config file path via the `directories` crate.
///
/// On Linux this is `~/.config/tsw-downloader/config.toml`.
/// On macOS: `~/Library/Application Support/tsw-downloader/config.toml`.
/// On Windows: `%APPDATA%\tsw-downloader\config.toml`.
pub fn default_config_path() -> Result<PathBuf> {
    let proj = directories::ProjectDirs::from("", "", "tsw-downloader")
        .context("unable to resolve config directory for this platform")?;
    Ok(proj.config_dir().join("config.toml"))
}

/// Read and validate the config at the given path.
pub fn read(path: &Path) -> Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading config file {}", path.display()))?;
    let config: Config = toml::from_str(&text)
        .with_context(|| format!("parsing config file {}", path.display()))?;
    check_schema_version(&config)?;
    Ok(config)
}

/// Write the config to the given path, creating parent directories as needed.
pub fn write(path: &Path, config: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating config directory {}", parent.display()))?;
    }
    let text = toml::to_string_pretty(config).context("serializing config to TOML")?;
    std::fs::write(path, text)
        .with_context(|| format!("writing config file {}", path.display()))?;
    Ok(())
}

/// Validate schema version. Returns an error for unknown versions.
pub fn check_schema_version(config: &Config) -> Result<()> {
    if config.schema_version != CURRENT_SCHEMA_VERSION {
        anyhow::bail!(
            "unknown schema version {} (expected {}); config was written by a newer tsw-downloader — upgrade or delete it",
            config.schema_version,
            CURRENT_SCHEMA_VERSION
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_toml() {
        let original = Config {
            schema_version: CURRENT_SCHEMA_VERSION,
            install: InstallSection {
                dir: PathBuf::from("/home/test/Games/TSW"),
            },
            download: DownloadSection::default(),
        };
        let serialized = toml::to_string(&original).unwrap();
        let parsed: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn rejects_unknown_schema_version() {
        let toml_str = r#"
schema_version = 999
[install]
dir = "/tmp/x"
"#;
        let parsed: Config = toml::from_str(toml_str).unwrap();
        let err = check_schema_version(&parsed).unwrap_err();
        assert!(err.to_string().contains("unknown schema version"));
    }
}
