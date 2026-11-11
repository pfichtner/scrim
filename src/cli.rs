use std::path::PathBuf;

use crate::config::Config;
use crate::errors::{AppError, Result};
use crate::filter::Filter;

const DEFAULT_CONFIG: &str = "/etc/scrim/config.yaml";

#[derive(Debug, clap::Parser)]
#[command(name = "scrim")]
#[command(about = "Scrim - writable, filtered FUSE bind mounts")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, clap::Subcommand)]
pub enum Commands {
    Mount {
        #[arg(long)]
        source: PathBuf,

        #[arg(long)]
        mountpoint: PathBuf,

        #[arg(long, default_value = DEFAULT_CONFIG)]
        config: PathBuf,
    },

    Healthcheck {
        #[arg(long)]
        mountpoint: Option<PathBuf>,

        #[arg(long)]
        config: Option<PathBuf>,
    },

    Validate {
        #[arg(long, default_value = DEFAULT_CONFIG)]
        config: PathBuf,
    },

    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    Unmount {
        #[arg(long)]
        mountpoint: PathBuf,
    },
}

#[derive(Debug, clap::Subcommand)]
pub enum ConfigAction {
    Show {
        #[arg(long, default_value = DEFAULT_CONFIG)]
        config: PathBuf,
    },
}

pub fn mount(source: PathBuf, mountpoint: PathBuf, config_path: PathBuf) -> Result<()> {
    let config = load_config(&config_path)?;
    let filter = build_filter(&config)?;

    crate::mount::mount(crate::mount::MountContext {
        source,
        mountpoint,
        filter,
        read_only: config.filesystem.read_only,
        allow_symlinks: config.filesystem.allow_symlinks,
        preserve_permissions: config.filesystem.preserve_permissions,
    })
}

pub fn healthcheck(mountpoint: Option<PathBuf>, config_path: Option<PathBuf>) -> Result<()> {
    let mountpoint = match mountpoint {
        Some(mountpoint) => mountpoint,
        None => {
            let config_path = config_path.ok_or_else(|| {
                AppError::config("provide --mountpoint or --config for healthcheck")
            })?;
            load_config(&config_path)?.filesystem.mountpoint
        }
    };

    let result = crate::health::health_check(&mountpoint);
    if result.status == crate::health::HealthStatus::Healthy {
        println!("{}: {}", mountpoint.display(), result.message);
        Ok(())
    } else {
        Err(AppError::unhealthy(result.message))
    }
}

pub fn validate(config_path: PathBuf) -> Result<()> {
    let config = load_config(&config_path)?;
    build_filter(&config)?;
    println!("Configuration is valid: {}", config_path.display());
    Ok(())
}

pub fn config_show(config_path: PathBuf) -> Result<()> {
    let config = load_config(&config_path)?;
    let yaml = serde_yaml::to_string(&config)
        .map_err(|error| AppError::config(format!("failed to render configuration: {error}")))?;
    print!("{yaml}");
    Ok(())
}

pub fn unmount(mountpoint: PathBuf) -> Result<()> {
    crate::mount::unmount(&mountpoint)
}

fn load_config(path: &std::path::Path) -> Result<Config> {
    Config::load(path).map_err(|error| AppError::config(error.to_string()))
}

fn build_filter(config: &Config) -> Result<Filter> {
    Filter::new(config.filter.patterns.clone(), config.filter.case_sensitive)
        .map_err(|error| AppError::config(format!("invalid filter pattern: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_show_subcommand_parses() {
        let cli = <Cli as clap::Parser>::try_parse_from([
            "scrim",
            "config",
            "show",
            "--config",
            "/tmp/scrim.yaml",
        ])
        .unwrap();
        match cli.command {
            Commands::Config {
                action: ConfigAction::Show { config },
            } => assert_eq!(config, PathBuf::from("/tmp/scrim.yaml")),
            _ => panic!("expected config show"),
        }
    }
}
