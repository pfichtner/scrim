use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

mod cli;
mod config;
mod filesystem;
mod filter;
mod health;
mod mount;

#[derive(Parser)]
#[command(name = "fuse-filter")]
#[command(about = "Docker FUSE Filter - writable, filtered bind mounts")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    Mount {
        #[arg(long)]
        source: PathBuf,

        #[arg(long)]
        mountpoint: PathBuf,

        #[arg(long)]
        config: PathBuf,
    },

    Healthcheck {
        #[arg(long)]
        mountpoint: PathBuf,
    },

    Validate {
        #[arg(long)]
        config: PathBuf,
    },

    ConfigShow {
        #[arg(long)]
        config: PathBuf,
    },

    Unmount {
        #[arg(long)]
        mountpoint: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Mount {
            source,
            mountpoint,
            config,
        } => {
            cli::mount(source, mountpoint, config).await?;
        }
        Commands::Healthcheck { mountpoint } => {
            cli::healthcheck(mountpoint).await?;
        }
        Commands::Validate { config } => {
            cli::validate(config).await?;
        }
        Commands::ConfigShow { config } => {
            cli::config_show(config).await?;
        }
        Commands::Unmount { mountpoint } => {
            cli::unmount(mountpoint).await?;
        }
    }

    Ok(())
}
