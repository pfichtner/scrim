use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    tracing_subscriber::fmt::init();

    let cli = scrim::cli::Cli::parse();
    let result = match cli.command {
        scrim::cli::Commands::Mount {
            source,
            mountpoint,
            config,
        } => scrim::cli::mount(source, mountpoint, config),
        scrim::cli::Commands::Healthcheck { mountpoint, config } => {
            scrim::cli::healthcheck(mountpoint, config)
        }
        scrim::cli::Commands::Validate { config } => scrim::cli::validate(config),
        scrim::cli::Commands::Config { action } => match action {
            scrim::cli::ConfigAction::Show { config } => scrim::cli::config_show(config),
        },
        scrim::cli::Commands::Unmount { mountpoint } => scrim::cli::unmount(mountpoint),
    };

    match result {
        Ok(()) => ExitCode::from(scrim::errors::EXIT_SUCCESS),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(error.code)
        }
    }
}
