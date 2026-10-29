//! `tt report week` and `tt report range`.

use super::{App, parse_date, resolve_limit};
use crate::cli::{RangeArgs, ReportCommand, WeekArgs};
use crate::config::{Format, parse_weekday};
use crate::error::Result;
use crate::output;
use crate::report::{self, Period};

pub fn run(app: &App, command: &ReportCommand) -> Result<()> {
    match command {
        ReportCommand::Week(args) => week(app, args),
        ReportCommand::Range(args) => range(app, args),
    }
}

fn week(app: &App, args: &WeekArgs) -> Result<()> {
    let anchor = match &args.week {
        Some(raw) => parse_date(raw)?,
        None => app.today(),
    };
    let week_start = match &args.week_start {
        Some(raw) => parse_weekday(raw)?,
        None => app.config.week_start,
    };
    let period = Period::week(anchor, week_start)?;
    emit(app, period, Some(week_start), args.format.map(Format::from), args.limit)
}

fn range(app: &App, args: &RangeArgs) -> Result<()> {
    let period = Period::new(parse_date(&args.from)?, parse_date(&args.to)?)?;
    emit(app, period, None, args.format.map(Format::from), args.limit)
}

fn emit(
    app: &App,
    period: Period,
    week_start: Option<jiff::civil::Weekday>,
    format: Option<Format>,
    limit: Option<u32>,
) -> Result<()> {
    let format = format.unwrap_or(app.config.report_format);
    let limit = resolve_limit(limit, &app.config)?;
    let fetched = app.fetch(period, None, limit)?;
    let built = report::build(
        period,
        &app.tz,
        week_start,
        &fetched.entries,
        fetched.now,
        fetched.warnings,
        fetched.incomplete,
    )?;

    match format {
        Format::Text => {
            output::print_report_text(&built);
            Ok(())
        }
        Format::Json => output::print_json(&output::report_json(&built)),
        Format::Csv => {
            // Warnings would corrupt stdout for a CSV consumer, so they also go
            // to stderr; the file itself carries them as `meta` rows.
            for warning in &built.warnings {
                output::warn(warning);
            }
            output::print_report_csv(&built)
        }
    }
}
