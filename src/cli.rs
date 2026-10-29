use clap::{Parser, Subcommand, ValueEnum};

use crate::config::Format;

/// Terminal client for OpenTodo time tracking.
///
/// tt is a thin front end: OpenTodo owns the data. Every operation runs through
/// the documented `ot` command-line client, and tt keeps no time-entry
/// database of its own.
#[derive(Debug, Parser)]
#[command(name = "tt", version, about, long_about = None)]
pub struct Cli {
    /// Path or command name for the OpenTodo CLI (overrides the config file).
    #[arg(long, global = true, value_name = "COMMAND")]
    pub ot_command: Option<String>,

    /// IANA timezone used for day and week boundaries (overrides the config file).
    #[arg(long, global = true, value_name = "ZONE")]
    pub timezone: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FormatArg {
    Text,
    Json,
    Csv,
}

impl From<FormatArg> for Format {
    fn from(arg: FormatArg) -> Format {
        match arg {
            FormatArg::Text => Format::Text,
            FormatArg::Json => Format::Json,
            FormatArg::Csv => Format::Csv,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start a timer on an OpenTodo task.
    Start(StartArgs),
    /// Stop the running timer.
    Stop(JsonFlag),
    /// Show the running timer.
    Status(JsonFlag),
    /// List time entries.
    Log(LogArgs),
    /// Summarise tracked time.
    #[command(subcommand)]
    Report(ReportCommand),
    /// Synchronisation: `tt sync status`, or `tt sync`.
    Sync(SyncArgs),
}

#[derive(Debug, clap::Args)]
pub struct JsonFlag {
    /// Print JSON instead of text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, clap::Args)]
pub struct StartArgs {
    /// Note for the entry. OpenTodo attaches time to a task, so this is the
    /// entry's note, not a task name.
    #[arg(value_name = "NOTE")]
    pub note: Option<String>,

    /// OpenTodo task identifier. Required unless tt can prompt in a terminal.
    #[arg(long, value_name = "TASK_ID")]
    pub task: Option<String>,

    /// Allow OpenTodo to end the entry that is already running.
    #[arg(long)]
    pub switch: bool,

    /// Print JSON instead of text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, clap::Args)]
pub struct LogArgs {
    /// Entries touching today in the selected timezone.
    #[arg(long, conflicts_with_all = ["from", "to"])]
    pub today: bool,

    /// Inclusive start date, YYYY-MM-DD.
    #[arg(long, value_name = "DATE")]
    pub from: Option<String>,

    /// Inclusive end date, YYYY-MM-DD.
    #[arg(long, value_name = "DATE")]
    pub to: Option<String>,

    /// Only entries for this OpenTodo task.
    #[arg(long, value_name = "TASK_ID")]
    pub task: Option<String>,

    /// Output format.
    #[arg(long, value_enum, value_name = "FORMAT")]
    pub format: Option<FormatArg>,

    /// Maximum number of entries to request from OpenTodo.
    #[arg(long, value_name = "N")]
    pub limit: Option<u32>,
}

#[derive(Debug, Subcommand)]
pub enum ReportCommand {
    /// Weekly report.
    Week(WeekArgs),
    /// Report over an inclusive date range.
    Range(RangeArgs),
}

#[derive(Debug, clap::Args)]
pub struct WeekArgs {
    /// Any date inside the week to report on. Defaults to today.
    #[arg(long, value_name = "DATE")]
    pub week: Option<String>,

    /// First day of the week (overrides the config file).
    #[arg(long, value_name = "DAY")]
    pub week_start: Option<String>,

    /// Output format.
    #[arg(long, value_enum, value_name = "FORMAT")]
    pub format: Option<FormatArg>,

    /// Maximum number of entries to request from OpenTodo.
    #[arg(long, value_name = "N")]
    pub limit: Option<u32>,
}

#[derive(Debug, clap::Args)]
pub struct RangeArgs {
    /// Inclusive start date, YYYY-MM-DD.
    #[arg(long, value_name = "DATE")]
    pub from: String,

    /// Inclusive end date, YYYY-MM-DD.
    #[arg(long, value_name = "DATE")]
    pub to: String,

    /// Output format.
    #[arg(long, value_enum, value_name = "FORMAT")]
    pub format: Option<FormatArg>,

    /// Maximum number of entries to request from OpenTodo.
    #[arg(long, value_name = "N")]
    pub limit: Option<u32>,
}

#[derive(Debug, clap::Args)]
pub struct SyncArgs {
    #[command(subcommand)]
    pub command: Option<SyncCommand>,
}

#[derive(Debug, Subcommand)]
pub enum SyncCommand {
    /// Report what OpenTodo discloses about synchronisation.
    Status(JsonFlag),
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_tree_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn start_takes_an_optional_note_and_a_task() {
        let cli = Cli::parse_from(["tt", "start", "Design review", "--task", "8c2a"]);
        match cli.command {
            Command::Start(args) => {
                assert_eq!(args.note.as_deref(), Some("Design review"));
                assert_eq!(args.task.as_deref(), Some("8c2a"));
                assert!(!args.switch);
            }
            other => panic!("expected start, got {other:?}"),
        }
    }

    #[test]
    fn today_cannot_be_combined_with_an_explicit_range() {
        let err = Cli::try_parse_from(["tt", "log", "--today", "--from", "2026-10-20"])
            .expect_err("conflicting options");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn report_range_requires_both_ends() {
        let err = Cli::try_parse_from(["tt", "report", "range", "--from", "2026-10-01"])
            .expect_err("--to is required");
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn sync_has_an_optional_status_subcommand() {
        let bare = Cli::parse_from(["tt", "sync"]);
        match bare.command {
            Command::Sync(args) => assert!(args.command.is_none()),
            other => panic!("expected sync, got {other:?}"),
        }
        let status = Cli::parse_from(["tt", "sync", "status"]);
        match status.command {
            Command::Sync(args) => assert!(matches!(args.command, Some(SyncCommand::Status(_)))),
            other => panic!("expected sync status, got {other:?}"),
        }
    }
}
