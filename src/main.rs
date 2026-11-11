use std::process::ExitCode;

use clap::Parser;

mod cli;
mod config;
mod errors;
mod filesystem;
mod filter;
mod health;
mod mount;

fn main() -> ExitCode {
    tracing_subscriber::fmt::init();

    let cli = cli::Cli::parse();
    let result = match cli.command {
        cli::Commands::Mount {
            source,
            mountpoint,
            config,
        } => cli::mount(source, mountpoint, config),
        cli::Commands::Healthcheck { mountpoint, config } => cli::healthcheck(mountpoint, config),
        cli::Commands::Validate { config } => cli::validate(config),
        cli::Commands::Config { action } => match action {
            cli::ConfigAction::Show { config } => cli::config_show(config),
        },
        cli::Commands::Unmount { mountpoint } => cli::unmount(mountpoint),
    };

    match result {
        Ok(()) => ExitCode::from(errors::EXIT_SUCCESS),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(error.code)
        }
    }
}
