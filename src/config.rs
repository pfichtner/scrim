use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FilesystemConfig {
    pub source: PathBuf,
    pub mountpoint: PathBuf,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub allow_symlinks: bool,
    #[serde(default = "default_true")]
    pub preserve_permissions: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FilterConfig {
    pub patterns: Vec<String>,
    #[serde(default = "default_true")]
    pub case_sensitive: bool,
    #[serde(default = "default_true")]
    pub hide_excluded_files: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub filesystem: FilesystemConfig,
    pub filter: FilterConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        serde_yaml::from_str(&text)
            .with_context(|| format!("failed to parse config file {}", path.display()))
    }
}

fn default_true() -> bool {
    true
}

fn default_log_level() -> String {
    "info".to_string()
}
