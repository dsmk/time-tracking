use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use jiff::Timestamp;
use rusqlite::Connection;

use crate::clock::Clock;
use crate::config::{Config, config_path, database_path};
use crate::entries::{self, EntryChanges, ListFilter, NewEntry};
use crate::error::{Result, conflict, validation};
use crate::model::{collect_tags, normalize_description, normalize_project, normalize_tag};
use crate::output;
use crate::reports::{self, Period};
use crate::timeutil::{self, parse_date, parse_duration, parse_timestamp};

#[derive(Parser, Debug)]
#[command(
    name = "tt",
    version,
    about = "Local-first command-line time tracker",
    long_about = "Track time from the terminal. All data is stored in a local SQLite \
                  database; no network access is required.\n\n\
                  Database path precedence: --db, then $TT_DB, then the OS data directory.\n\
                  Configuration: $TT_CONFIG, otherwise the OS configuration directory."
)]
pub struct Cli {
    /// Database file to use; overrides $TT_DB and the default location
    #[arg(long, global = true, value_name = "PATH")]
    pub db: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create the database and apply schema migrations
    Init,
    /// Start a timer
    Start(StartArgs),
    /// Stop the running timer
    Stop(StopArgs),
    /// Show the running timer, if any
    Status(StatusArgs),
    /// Record a completed entry
    Add(AddArgs),
    /// List entries, newest first
    Log(LogArgs),
    /// Show one entry by id or unambiguous id prefix
    Show(ShowArgs),
    /// Change an existing entry
    Edit(EditArgs),
    /// Soft-delete an entry
    Delete(DeleteArgs),
    /// Summarize tracked time
    Report {
        #[command(subcommand)]
        which: ReportCommand,
    },
    /// Inspect or change configuration
    Config {
        #[command(subcommand)]
        which: ConfigCommand,
    },
}

#[derive(Args, Debug)]
pub struct StartArgs {
    /// What you are working on; `#tags` in the text become tags
    pub description: String,
    /// Assign a project
    #[arg(long, value_name = "TEXT")]
    pub project: Option<String>,
    /// Add a tag; repeatable
    #[arg(long = "tag", value_name = "TEXT")]
    pub tags: Vec<String>,
    /// Start at this time instead of now
    #[arg(long, value_name = "TIME")]
    pub at: Option<String>,
    /// Permit overlap with an existing entry
    #[arg(long)]
    pub allow_overlap: bool,
}

#[derive(Args, Debug)]
pub struct StopArgs {
    /// Stop at this time instead of now
    #[arg(long, value_name = "TIME")]
    pub at: Option<String>,
    /// Permit overlap with an existing entry
    #[arg(long)]
    pub allow_overlap: bool,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Emit JSON (`null` when no timer is running)
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// What you worked on; `#tags` in the text become tags
    pub description: String,
    /// When the work started
    #[arg(long, value_name = "TIME")]
    pub start: String,
    /// When the work ended
    #[arg(long, value_name = "TIME", group = "span")]
    pub end: Option<String>,
    /// How long the work took, for example 90m or 1h30m
    #[arg(long, value_name = "DURATION", group = "span")]
    pub duration: Option<String>,
    /// Assign a project
    #[arg(long, value_name = "TEXT")]
    pub project: Option<String>,
    /// Add a tag; repeatable
    #[arg(long = "tag", value_name = "TEXT")]
    pub tags: Vec<String>,
    /// Permit overlap with an existing entry
    #[arg(long)]
    pub allow_overlap: bool,
}

#[derive(Args, Debug)]
pub struct LogArgs {
    /// Entries touching today in the configured timezone
    #[arg(long, conflicts_with_all = ["from", "to"])]
    pub today: bool,
    /// Inclusive first calendar date
    #[arg(long, value_name = "DATE")]
    pub from: Option<String>,
    /// Inclusive last calendar date
    #[arg(long, value_name = "DATE")]
    pub to: Option<String>,
    /// Only entries in this project
    #[arg(long, value_name = "TEXT")]
    pub project: Option<String>,
    /// Only entries carrying this tag
    #[arg(long, value_name = "TEXT")]
    pub tag: Option<String>,
    /// Case-insensitive substring match on the description
    #[arg(long, value_name = "TEXT")]
    pub search: Option<String>,
    /// Return at most this many entries
    #[arg(long, value_name = "N")]
    pub limit: Option<i64>,
    /// Emit JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ShowArgs {
    /// Full UUID or an unambiguous prefix
    pub id: String,
    /// Emit JSON
    #[arg(long)]
    pub json: bool,
    /// Also match soft-deleted entries
    #[arg(long)]
    pub include_deleted: bool,
}

#[derive(Args, Debug)]
pub struct EditArgs {
    /// Full UUID or an unambiguous prefix
    pub id: String,
    /// Replace the description; `#tags` in the new text are added
    #[arg(long, value_name = "TEXT")]
    pub description: Option<String>,
    /// Set the project
    #[arg(long, value_name = "TEXT", conflicts_with = "clear_project")]
    pub project: Option<String>,
    /// Remove the project
    #[arg(long)]
    pub clear_project: bool,
    /// Replace the start time
    #[arg(long, value_name = "TIME")]
    pub start: Option<String>,
    /// Replace the end time; only valid for a completed entry
    #[arg(long, value_name = "TIME")]
    pub end: Option<String>,
    /// Replace all tags; repeatable
    #[arg(long = "tag", value_name = "TEXT")]
    pub tags: Vec<String>,
    /// Add a tag; repeatable
    #[arg(long = "add-tag", value_name = "TEXT")]
    pub add_tags: Vec<String>,
    /// Remove a tag; repeatable
    #[arg(long = "remove-tag", value_name = "TEXT")]
    pub remove_tags: Vec<String>,
    /// Permit overlap with an existing entry
    #[arg(long)]
    pub allow_overlap: bool,
}

#[derive(Args, Debug)]
pub struct DeleteArgs {
    /// Full UUID or an unambiguous prefix
    pub id: String,
    /// Skip the confirmation prompt
    #[arg(long)]
    pub yes: bool,
}

#[derive(Subcommand, Debug)]
pub enum ReportCommand {
    /// Summarize one week
    Week(WeekArgs),
    /// Summarize an inclusive date range
    Range(RangeArgs),
}

#[derive(Args, Debug)]
pub struct WeekArgs {
    /// Any date inside the week to report; defaults to today
    #[arg(long, value_name = "DATE")]
    pub week: Option<String>,
    /// Only entries in this project
    #[arg(long, value_name = "TEXT")]
    pub project: Option<String>,
    /// Only entries carrying this tag
    #[arg(long, value_name = "TEXT")]
    pub tag: Option<String>,
    /// Output format
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
}

#[derive(Args, Debug)]
pub struct RangeArgs {
    /// Inclusive first calendar date
    #[arg(long, value_name = "DATE")]
    pub from: String,
    /// Inclusive last calendar date
    #[arg(long, value_name = "DATE")]
    pub to: String,
    /// Only entries in this project
    #[arg(long, value_name = "TEXT")]
    pub project: Option<String>,
    /// Only entries carrying this tag
    #[arg(long, value_name = "TEXT")]
    pub tag: Option<String>,
    /// Output format
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
    Csv,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Print the effective configuration and the paths in use
    Show,
    /// Validate and store one setting
    Set {
        /// timezone, week_start, duration_precision, or default_project
        key: String,
        /// New value
        value: String,
    },
    /// Print the configuration file path
    Path,
}

/// What a command produced: text for stdout, and whether it was JSON (JSON
/// output must never be mixed with decorative text).
pub struct CommandOutput {
    pub stdout: String,
}

impl CommandOutput {
    fn text(body: impl Into<String>) -> CommandOutput {
        CommandOutput { stdout: body.into() }
    }
}

/// Open the database for commands that need it.
fn open_db(cli: &Cli) -> Result<Connection> {
    let path = database_path(cli.db.as_deref())?;
    crate::db::open(&path)
}

pub fn run(cli: &Cli, cfg: &Config, clock: &dyn Clock) -> Result<CommandOutput> {
    let now = clock.now();
    match &cli.command {
        Command::Init => {
            let path = database_path(cli.db.as_deref())?;
            let conn = crate::db::open(&path)?;
            let version = crate::db::schema_version(&conn)?;
            Ok(CommandOutput::text(format!(
                "Database ready at {} (schema version {version})",
                path.display()
            )))
        }
        Command::Start(args) => cmd_start(cli, cfg, args, now),
        Command::Stop(args) => cmd_stop(cli, cfg, args, now),
        Command::Status(args) => cmd_status(cli, cfg, args, now),
        Command::Add(args) => cmd_add(cli, cfg, args, now),
        Command::Log(args) => cmd_log(cli, cfg, args, now),
        Command::Show(args) => cmd_show(cli, cfg, args, now),
        Command::Edit(args) => cmd_edit(cli, cfg, args, now),
        Command::Delete(args) => cmd_delete(cli, cfg, args, now),
        Command::Report { which } => cmd_report(cli, cfg, which, now),
        Command::Config { which } => cmd_config(cli, cfg, which),
    }
}

/// Resolve `--at`/`--start`-style options against the configured timezone.
fn resolve_time(raw: Option<&String>, cfg: &Config, default: Timestamp) -> Result<Timestamp> {
    match raw {
        Some(text) => parse_timestamp(text, &cfg.timezone),
        None => Ok(default),
    }
}

fn build_new_entry(
    cfg: &Config,
    description: &str,
    project: Option<&String>,
    tags: &[String],
    start_at: Timestamp,
    end_at: Option<Timestamp>,
) -> Result<NewEntry> {
    let description = normalize_description(description)?;
    let project = match normalize_project(project.map(|s| s.as_str()))? {
        Some(project) => Some(project),
        None => cfg.default_project.clone(),
    };
    let tags = collect_tags(tags, &description)?;
    Ok(NewEntry {
        description,
        project,
        tags,
        start_at,
        end_at,
        timezone: cfg.timezone_name(),
    })
}

fn cmd_start(cli: &Cli, cfg: &Config, args: &StartArgs, now: Timestamp) -> Result<CommandOutput> {
    let start_at = resolve_time(args.at.as_ref(), cfg, now)?;
    let new = build_new_entry(cfg, &args.description, args.project.as_ref(), &args.tags, start_at, None)?;
    let mut conn = open_db(cli)?;
    let entry = entries::start(&mut conn, new, now, args.allow_overlap)?;
    Ok(CommandOutput::text(format!(
        "Started timer.\n{}",
        output::entry_detail(&entry, cfg, now)
    )))
}

fn cmd_stop(cli: &Cli, cfg: &Config, args: &StopArgs, now: Timestamp) -> Result<CommandOutput> {
    let at = resolve_time(args.at.as_ref(), cfg, now)?;
    let mut conn = open_db(cli)?;
    let entry = entries::stop(&mut conn, at, now, args.allow_overlap)?;
    Ok(CommandOutput::text(format!(
        "Stopped timer.\n{}",
        output::entry_detail(&entry, cfg, now)
    )))
}

fn cmd_status(cli: &Cli, cfg: &Config, args: &StatusArgs, now: Timestamp) -> Result<CommandOutput> {
    let conn = open_db(cli)?;
    let active = entries::running(&conn)?;
    if args.json {
        // Documented empty representation: JSON null.
        let body = match &active {
            Some(entry) => output::to_json_string(&output::entry_json(entry, &cfg.timezone, now))?,
            None => "null".to_string(),
        };
        return Ok(CommandOutput::text(body));
    }
    match active {
        Some(entry) => Ok(CommandOutput::text(output::entry_detail(&entry, cfg, now))),
        None => Ok(CommandOutput::text("No timer running")),
    }
}

fn cmd_add(cli: &Cli, cfg: &Config, args: &AddArgs, now: Timestamp) -> Result<CommandOutput> {
    let start_at = parse_timestamp(&args.start, &cfg.timezone)?;
    let end_at = match (&args.end, &args.duration) {
        (Some(end), None) => parse_timestamp(end, &cfg.timezone)?,
        (None, Some(duration)) => {
            let seconds = parse_duration(duration)?;
            Timestamp::from_second(start_at.as_second() + seconds).map_err(|e| {
                validation(format!("{} plus {duration} is out of range: {e}", args.start))
            })?
        }
        (None, None) => {
            return Err(validation("one of --end or --duration is required")
                .with_hint("for example: --end 2026-10-27T10:30 or --duration 90m"));
        }
        (Some(_), Some(_)) => {
            return Err(validation("--end and --duration cannot be combined"));
        }
    };

    let new = build_new_entry(
        cfg,
        &args.description,
        args.project.as_ref(),
        &args.tags,
        start_at,
        Some(end_at),
    )?;
    let mut conn = open_db(cli)?;
    let entry = entries::add(&mut conn, new, now, args.allow_overlap)?;
    Ok(CommandOutput::text(format!(
        "Added entry.\n{}",
        output::entry_detail(&entry, cfg, now)
    )))
}

fn cmd_log(cli: &Cli, cfg: &Config, args: &LogArgs, now: Timestamp) -> Result<CommandOutput> {
    if let Some(limit) = args.limit
        && limit <= 0
    {
        return Err(validation("--limit must be a positive number"));
    }

    let tz = &cfg.timezone;
    let mut filter = ListFilter {
        project: normalize_project(args.project.as_deref())?,
        tag: args.tag.as_deref().map(normalize_tag).transpose()?,
        search: args.search.clone(),
        limit: args.limit,
        ..Default::default()
    };

    if args.today {
        let today = timeutil::local_date(now, tz);
        filter.from = Some(timeutil::start_of_day(today, tz)?);
        filter.to = Some(timeutil::start_of_day(timeutil::add_days(today, 1)?, tz)?);
    } else {
        if let Some(from) = &args.from {
            filter.from = Some(timeutil::start_of_day(parse_date(from)?, tz)?);
        }
        if let Some(to) = &args.to {
            let last = parse_date(to)?;
            filter.to = Some(timeutil::start_of_day(timeutil::add_days(last, 1)?, tz)?);
        }
        if let (Some(from), Some(to)) = (&args.from, &args.to)
            && parse_date(to)? < parse_date(from)?
        {
            return Err(validation(format!("--from {from} is later than --to {to}")));
        }
    }

    let conn = open_db(cli)?;
    let found = entries::list(&conn, &filter)?;

    if args.json {
        let payload: Vec<_> =
            found.iter().map(|e| output::entry_json(e, tz, now)).collect();
        return Ok(CommandOutput::text(output::to_json_string(&payload)?));
    }
    Ok(CommandOutput::text(output::entry_table(&found, cfg, now)))
}

fn cmd_show(cli: &Cli, cfg: &Config, args: &ShowArgs, now: Timestamp) -> Result<CommandOutput> {
    let conn = open_db(cli)?;
    let entry = entries::find(&conn, &args.id, args.include_deleted)?;
    if args.json {
        return Ok(CommandOutput::text(output::to_json_string(&output::entry_json(
            &entry,
            &cfg.timezone,
            now,
        ))?));
    }
    Ok(CommandOutput::text(output::entry_detail(&entry, cfg, now)))
}

fn cmd_edit(cli: &Cli, cfg: &Config, args: &EditArgs, now: Timestamp) -> Result<CommandOutput> {
    let mut changes = EntryChanges::default();

    if let Some(description) = &args.description {
        let description = normalize_description(description)?;
        // `#tags` written into the new description are added to the entry.
        for tag in crate::model::tags_in_description(&description) {
            changes.add_tags.push(normalize_tag(&tag)?);
        }
        changes.description = Some(description);
    }
    if args.clear_project {
        changes.project = Some(None);
    } else if let Some(project) = &args.project {
        let normalized = normalize_project(Some(project))?;
        if normalized.is_none() {
            return Err(validation("--project was empty")
                .with_hint("use --clear-project to remove the project"));
        }
        changes.project = Some(normalized);
    }
    if let Some(start) = &args.start {
        changes.start_at = Some(parse_timestamp(start, &cfg.timezone)?);
    }
    if let Some(end) = &args.end {
        changes.end_at = Some(parse_timestamp(end, &cfg.timezone)?);
    }
    if !args.tags.is_empty() {
        let mut replacement = Vec::new();
        for raw in &args.tags {
            replacement.push(normalize_tag(raw)?);
        }
        changes.set_tags = Some(crate::model::dedupe_tags(replacement));
    }
    for raw in &args.add_tags {
        changes.add_tags.push(normalize_tag(raw)?);
    }
    for raw in &args.remove_tags {
        changes.remove_tags.push(normalize_tag(raw)?);
    }

    let mut conn = open_db(cli)?;
    let entry = entries::edit(&mut conn, &args.id, &changes, now, args.allow_overlap)?;
    Ok(CommandOutput::text(format!(
        "Updated entry.\n{}",
        output::entry_detail(&entry, cfg, now)
    )))
}

fn cmd_delete(cli: &Cli, cfg: &Config, args: &DeleteArgs, now: Timestamp) -> Result<CommandOutput> {
    let mut conn = open_db(cli)?;
    let entry = entries::find(&conn, &args.id, false)?;

    if !args.yes {
        let stdin = std::io::stdin();
        if !stdin.is_terminal() {
            return Err(validation(
                "refusing to delete without confirmation when standard input is not a terminal",
            )
            .with_hint("re-run with --yes to confirm"));
        }
        let mut stderr = std::io::stderr();
        if entry.is_running() {
            let _ = writeln!(
                stderr,
                "Entry {} \"{}\" is the RUNNING timer. Deleting discards it; \
                 the elapsed time is not recorded. Use `tt stop` instead to keep it.",
                entry.short_id(),
                entry.description
            );
        } else {
            let _ = writeln!(
                stderr,
                "Entry {} \"{}\" ({}) will be deleted.",
                entry.short_id(),
                entry.description,
                timeutil::format_duration(entry.duration_seconds(now), cfg.precision)
            );
        }
        let _ = write!(stderr, "Delete this entry? [y/N] ");
        let _ = stderr.flush();

        let mut answer = String::new();
        std::io::stdin()
            .read_line(&mut answer)
            .map_err(|e| validation(format!("cannot read confirmation: {e}")))?;
        if !is_affirmative(&answer) {
            return Err(conflict("deletion cancelled"));
        }
    }

    let was_running = entry.is_running();
    let deleted = entries::delete(&mut conn, &entry.id, now)?;
    let headline = if was_running {
        "Discarded the running timer; its elapsed time was not recorded."
    } else {
        "Deleted entry."
    };
    Ok(CommandOutput::text(format!(
        "{headline}\n{}",
        output::entry_detail(&deleted, cfg, now)
    )))
}

/// Only an explicit yes proceeds. Anything else cancels, including an empty
/// line, which is the capitalized `N` default in the `[y/N]` prompt.
fn is_affirmative(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn cmd_report(
    cli: &Cli,
    cfg: &Config,
    which: &ReportCommand,
    now: Timestamp,
) -> Result<CommandOutput> {
    let tz = &cfg.timezone;
    let (kind, title, period, project, tag, format) = match which {
        ReportCommand::Week(args) => {
            let anchor = match &args.week {
                Some(date) => parse_date(date)?,
                None => timeutil::local_date(now, tz),
            };
            let period = Period::for_week(anchor, cfg.week_start, tz)?;
            let title = format!(
                "Weekly report, week starting {} ({})",
                period.first_day,
                timeutil::weekday_name(cfg.week_start)
            );
            ("week", title, period, &args.project, &args.tag, args.format)
        }
        ReportCommand::Range(args) => {
            let period = Period::from_dates(parse_date(&args.from)?, parse_date(&args.to)?, tz)?;
            let title = format!("Range report, {} through {}", period.first_day, period.last_day);
            ("range", title, period, &args.project, &args.tag, args.format)
        }
    };

    let filter = ListFilter {
        from: Some(period.start),
        to: Some(period.end),
        project: normalize_project(project.as_deref())?,
        tag: tag.as_deref().map(normalize_tag).transpose()?,
        ..Default::default()
    };

    let conn = open_db(cli)?;
    let found = entries::list(&conn, &filter)?;
    let report = reports::build(kind, title, &found, &period, tz, now)?;

    let body = match format {
        Format::Text => output::report_text(&report, cfg.precision),
        Format::Json => output::to_json_string(&report)?,
        Format::Csv => output::report_csv(&report)?,
    };
    Ok(CommandOutput::text(body))
}

fn cmd_config(cli: &Cli, cfg: &Config, which: &ConfigCommand) -> Result<CommandOutput> {
    match which {
        ConfigCommand::Show => {
            let db = database_path(cli.db.as_deref())?;
            Ok(CommandOutput::text(format!(
                "timezone            = \"{}\"\n\
                 week_start          = \"{}\"\n\
                 duration_precision  = \"{}\"\n\
                 default_project     = \"{}\"\n\
                 \n\
                 config file         = {}\n\
                 database            = {}",
                cfg.timezone_name(),
                timeutil::weekday_name(cfg.week_start),
                match cfg.precision {
                    timeutil::Precision::Minute => "minute",
                    timeutil::Precision::Second => "second",
                },
                cfg.default_project.clone().unwrap_or_default(),
                cfg.path.display(),
                db.display(),
            )))
        }
        ConfigCommand::Set { key, value } => {
            let path = config_path()?;
            Config::set(&path, key, value)?;
            Ok(CommandOutput::text(format!(
                "Set {key} = \"{}\" in {}",
                value.trim(),
                path.display()
            )))
        }
        ConfigCommand::Path => Ok(CommandOutput::text(config_path()?.display().to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn only_an_explicit_yes_confirms_a_deletion() {
        for yes in ["y", "Y", "yes", "YES", " yes \n", "y\r\n"] {
            assert!(is_affirmative(yes), "{yes:?} should confirm");
        }
        // The empty line is the [y/N] default and must cancel.
        for no in ["", "\n", "n", "N", "no", "nope", "sure", "yep", "yes please"] {
            assert!(!is_affirmative(no), "{no:?} should cancel");
        }
    }

    #[test]
    fn the_command_tree_is_well_formed() {
        // Catches conflicting flags, duplicate names, and bad arg groups.
        Cli::command().debug_assert();
    }
}
