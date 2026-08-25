//! Path resolution shared by every subcommand.
//!
//! Precedence per setting: CLI flag > environment variable > default.

use anyhow::Context;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    /// Main Caddyfile (global options + import line after `init`).
    pub caddyfile: PathBuf,
    /// Root of the enabled/disabled split.
    pub vhosts_dir: PathBuf,
    /// Where automatic backups land.
    pub backup_dir: PathBuf,
}

impl Paths {
    pub fn resolve(cli_config: Option<PathBuf>, cli_vhosts: Option<PathBuf>) -> Self {
        let caddyfile = cli_config
            .or_else(|| env_path("CADDYFILE_PATH"))
            .unwrap_or_else(|| PathBuf::from("/etc/caddy/Caddyfile"));

        let vhosts_dir = cli_vhosts
            .or_else(|| env_path("VHOSTS_DIR"))
            .unwrap_or_else(|| default_sibling(&caddyfile, "vhosts"));

        let backup_dir =
            env_path("CADDY_BACKUP_DIR").unwrap_or_else(|| default_sibling(&caddyfile, "backups"));

        Paths {
            caddyfile,
            vhosts_dir,
            backup_dir,
        }
    }

    pub fn enabled_dir(&self) -> PathBuf {
        self.vhosts_dir.join("enabled")
    }

    pub fn disabled_dir(&self) -> PathBuf {
        self.vhosts_dir.join("disabled")
    }

    /// Import line pointing at the enabled vhosts, always using `/` separators
    /// (Caddy's glob only understands forward slashes).
    pub fn import_line(&self) -> String {
        let dir = self.enabled_dir().to_string_lossy().replace('\\', "/");
        format!("import {dir}/*.caddy")
    }
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var(key)
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

fn default_sibling(file: &Path, name: &str) -> PathBuf {
    file.parent()
        .map(|p| p.join(name))
        .unwrap_or_else(|| PathBuf::from(name))
}

pub fn ensure_dirs(paths: &Paths) -> anyhow::Result<()> {
    for dir in [
        paths.enabled_dir(),
        paths.disabled_dir(),
        paths.backup_dir.clone(),
    ] {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating directory {}", dir.display()))?;
    }
    Ok(())
}
