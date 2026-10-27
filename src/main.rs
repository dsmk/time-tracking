mod cli;
mod clock;
mod config;
mod db;
mod entries;
mod error;
mod model;
mod output;
mod reports;
mod timeutil;

use std::io::Write;

use clap::Parser;

use crate::cli::Cli;
use crate::config::Config;
use crate::error::{AppError, Result};

fn main() {
    let parsed = Cli::parse();
    match run(&parsed) {
        Ok(()) => {}
        Err(err) => {
            let _ = writeln!(std::io::stderr(), "error: {err}");
            std::process::exit(err.kind.exit_code());
        }
    }
}

fn run(parsed: &Cli) -> Result<()> {
    let cfg = load_config(parsed)?;
    for warning in &cfg.warnings {
        let _ = writeln!(std::io::stderr(), "warning: {warning}");
    }

    let clock = clock::from_env()?;
    let output = cli::run(parsed, &cfg, clock.as_ref())?;

    // Exactly one trailing newline, whatever the renderer produced: the CSV
    // writer already terminates its last row.
    let mut stdout = std::io::stdout();
    let _ = writeln!(stdout, "{}", output.stdout.trim_end_matches('\n'));
    let _ = stdout.flush();
    Ok(())
}

/// `config set` must still work when the current file is unreadable, so it
/// falls back to defaults rather than failing before it can fix the problem.
fn load_config(parsed: &Cli) -> std::result::Result<Config, AppError> {
    match Config::load() {
        Ok(cfg) => Ok(cfg),
        Err(err) => {
            if matches!(
                parsed.command,
                cli::Command::Config { which: cli::ConfigCommand::Set { .. } }
            ) {
                let _ = writeln!(std::io::stderr(), "warning: {err}");
                let path = config::config_path()?;
                Ok(Config {
                    timezone: jiff::tz::TimeZone::system(),
                    week_start: jiff::civil::Weekday::Monday,
                    precision: timeutil::Precision::Minute,
                    default_project: None,
                    path,
                    warnings: Vec::new(),
                })
            } else {
                Err(err)
            }
        }
    }
}
