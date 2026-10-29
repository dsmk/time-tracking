//! `tt` — a terminal client for OpenTodo time tracking.
//!
//! OpenTodo is the system of record. `tt` runs the documented `ot` client,
//! reads its documented JSON, and keeps no time-entry storage of its own.

mod clock;
mod cli;
mod commands;
mod config;
mod error;
mod ot;
mod output;
mod report;

use std::process::ExitCode;

use clap::Parser;
use jiff::tz::TimeZone;

use cli::{Cli, Command};
use commands::App;
use config::Config;
use error::{Result, usage};
use ot::runner::ProcessRunner;
use ot::{OpenTodo, OtCli};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("tt: {err}");
            ExitCode::from(err.exit_code() as u8)
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let app = build_app(&cli)?;

    match &cli.command {
        Command::Start(args) => commands::timer::start(&app, args),
        Command::Stop(args) => commands::timer::stop(&app, args),
        Command::Status(args) => commands::timer::status(&app, args),
        Command::Log(args) => commands::entries::run(&app, args),
        Command::Report(command) => commands::reports::run(&app, command),
        Command::Sync(args) => commands::sync::run(&app, args),
    }
}

fn build_app(cli: &Cli) -> Result<App> {
    let mut config = Config::load()?;
    if let Some(command) = &cli.ot_command {
        if command.trim().is_empty() {
            return Err(usage("--ot-command must not be empty"));
        }
        config.ot_command = command.clone();
    }
    if let Some(name) = &cli.timezone {
        config.timezone = TimeZone::get(name).map_err(|e| {
            usage(format!("unknown timezone {name:?}: {e}"))
                .with_hint("use an IANA name such as America/New_York")
        })?;
        config.timezone_source = config::TimezoneSource::Config;
    }

    let runner = ProcessRunner::new(config.timeout_seconds);
    let client = OtCli::new(config.ot_command.clone(), runner);

    // Fail early and clearly when the client is missing or speaks a JSON
    // version these parsers were not written for.
    if config.version_check
        && let Some(warning) = ot::check_version(&client.version()?)?
    {
        output::warn(&warning);
    }

    let tz = config.timezone.clone();
    Ok(App { config, tz, clock: clock::from_env()?, ot: Box::new(client) })
}
