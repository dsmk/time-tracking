//! `tt log` — list time entries.

use jiff::ToSpan;

use super::{App, parse_date, resolve_limit};
use crate::cli::LogArgs;
use crate::config::Format;
use crate::error::{Result, usage};
use crate::ot::model::TimeEntry;
use crate::output;
use crate::report::{Period, overlap_seconds};

/// Days covered by `tt log` when no dates are given. Explicit beats guessing at
/// whatever default `ot time list` applies.
const DEFAULT_DAYS: i64 = 7;

pub fn run(app: &App, args: &LogArgs) -> Result<()> {
    let format = match args.format.map(Format::from) {
        Some(Format::Csv) => {
            return Err(usage("tt log supports --format text or json")
                .with_hint("for CSV, use `tt report range --format csv`"));
        }
        Some(format) => format,
        None => Format::Text,
    };

    let period = period_for(app, args)?;
    let limit = resolve_limit(args.limit, &app.config)?;
    let fetched = app.fetch(period, args.task.clone(), limit)?;

    // `ot` interprets --from/--to in its own timezone, so the window was
    // widened; keep only entries that really touch the requested local days.
    let (lo, hi) = period.bounds(&app.tz)?;
    let now = fetched.now;
    let entries: Vec<TimeEntry> = fetched
        .entries
        .into_iter()
        .filter(|e| {
            let end = e.ended_at.unwrap_or_else(|| now.max(e.started_at));
            overlap_seconds(e.started_at, end, lo, hi) > 0 || (e.is_running() && e.started_at < hi)
        })
        .collect();

    for warning in &fetched.warnings {
        output::warn(warning);
    }

    match format {
        Format::Json => {
            let items: Vec<serde_json::Value> =
                entries.iter().map(|e| output::entry_json(e, &app.tz, now)).collect();
            output::print_json(&serde_json::Value::Array(items))
        }
        Format::Text => {
            println!("{} to {} ({})", period.from, period.to, app.timezone_label());
            println!();
            output::print_entries_text(&entries, &app.tz, now);
            Ok(())
        }
        Format::Csv => unreachable!("csv is rejected above"),
    }
}

fn period_for(app: &App, args: &LogArgs) -> Result<Period> {
    let today = app.today();
    if args.today {
        return Period::new(today, today);
    }
    match (&args.from, &args.to) {
        (None, None) => {
            let from = today
                .checked_sub((DEFAULT_DAYS - 1).days())
                .map_err(|e| usage(format!("cannot compute the default period: {e}")))?;
            Period::new(from, today)
        }
        (Some(from), None) => Period::new(parse_date(from)?, today),
        (None, Some(to)) => {
            let to = parse_date(to)?;
            let from = to
                .checked_sub((DEFAULT_DAYS - 1).days())
                .map_err(|e| usage(format!("cannot compute the default period: {e}")))?;
            Period::new(from, to)
        }
        (Some(from), Some(to)) => Period::new(parse_date(from)?, parse_date(to)?),
    }
}
