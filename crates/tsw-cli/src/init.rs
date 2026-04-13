//! `tsw-downloader init` — first-run interactive setup.

use anyhow::{Context, Result};
use dialoguer::Input;
use std::path::{Path, PathBuf};

use crate::args::InitArgs;
use crate::config_file::{self, Config, DownloadSection, InstallSection, CURRENT_SCHEMA_VERSION};

pub fn run(args: InitArgs, config_path_override: Option<PathBuf>) -> Result<i32> {
    let config_path = match config_path_override {
        Some(p) => p,
        None => config_file::default_config_path()?,
    };

    if config_path.exists() && !args.force {
        let existing = config_file::read(&config_path)?;
        println!("Config already exists at: {}", config_path.display());
        println!("  install.dir = {}", existing.install.dir.display());
        println!();
        println!("Pass --force to overwrite, or edit the file directly.");
        return Ok(0);
    }

    let install_dir = match args.install_dir {
        Some(dir) => dir,
        None => prompt_for_install_dir()?,
    };

    let install_dir = validate_install_dir(&install_dir)?;

    let config = Config {
        schema_version: CURRENT_SCHEMA_VERSION,
        install: InstallSection {
            dir: install_dir.clone(),
        },
        download: DownloadSection::default(),
    };

    config_file::write(&config_path, &config)?;

    println!("Saved config to {}", config_path.display());
    println!("  install.dir = {}", install_dir.display());
    Ok(0)
}

fn prompt_for_install_dir() -> Result<PathBuf> {
    let default = default_install_dir_suggestion();
    let input: String = Input::new()
        .with_prompt("Install directory")
        .default(default.to_string_lossy().to_string())
        .interact_text()
        .context("reading install directory from stdin")?;
    Ok(PathBuf::from(input))
}

fn default_install_dir_suggestion() -> PathBuf {
    directories::UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .map(|home: PathBuf| home.join("Games").join("TheSecretWorld"))
        .unwrap_or_else(|| PathBuf::from("./TheSecretWorld"))
}

/// Validate that the path is absolute and either exists or can be created.
fn validate_install_dir(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        anyhow::bail!(
            "install directory must be an absolute path, got: {}",
            path.display()
        );
    }

    if path.exists() {
        if !path.is_dir() {
            anyhow::bail!("{} exists but is not a directory", path.display());
        }
        return Ok(path.canonicalize()?);
    }

    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot determine parent of {}", path.display()))?;
    if !parent.exists() {
        anyhow::bail!("parent directory does not exist: {}", parent.display());
    }

    println!("Directory does not exist: {}", path.display());
    let create: String = Input::new()
        .with_prompt("Create it?")
        .default("y".to_string())
        .interact_text()?;
    if !matches!(create.trim().to_lowercase().as_str(), "y" | "yes") {
        anyhow::bail!("aborted — install directory not created");
    }
    std::fs::create_dir_all(path).with_context(|| format!("creating {}", path.display()))?;

    Ok(path.canonicalize()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_relative_path() {
        let result = validate_install_dir(Path::new("relative/path"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("absolute"));
    }

    #[test]
    fn accepts_existing_temp_dir() {
        let tmp = std::env::temp_dir();
        assert!(tmp.is_absolute());
        assert!(validate_install_dir(&tmp).is_ok());
    }
}
