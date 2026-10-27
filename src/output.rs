use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::Serialize;

use crate::config::Config;
use crate::error::{AppError, ErrorKind, Result};
use crate::model::Entry;
use crate::reports::Report;
use crate::timeutil::{
    Precision, format_duration, format_local_date, format_local_datetime, format_local_time,
    to_storage,
};

/// Stable JSON shape for one entry. Timestamps are UTC with `Z`; local forms
/// carry an explicit offset. Durations are integer seconds.
#[derive(Debug, Serialize)]
pub struct EntryJson {
    pub id: String,
    pub description: String,
    pub project: Option<String>,
    pub tags: Vec<String>,
    pub start_at: String,
    pub end_at: Option<String>,
    pub start_local: String,
    pub end_local: Option<String>,
    pub date: String,
    pub running: bool,
    pub duration_seconds: i64,
    pub timezone: String,
    pub created_at: String,
    pub updated_at: String,
    pub revision: i64,
    pub deleted_at: Option<String>,
}

pub fn entry_json(entry: &Entry, tz: &TimeZone, now: Timestamp) -> EntryJson {
    let local = |ts: Timestamp| ts.to_zoned(tz.clone()).to_string();
    EntryJson {
        id: entry.id.clone(),
        description: entry.description.clone(),
        project: entry.project.clone(),
        tags: entry.tags.clone(),
        start_at: to_storage(entry.start_at),
        end_at: entry.end_at.map(to_storage),
        start_local: local(entry.start_at),
        end_local: entry.end_at.map(local),
        date: format_local_date(entry.start_at, tz),
        running: entry.is_running(),
        duration_seconds: entry.duration_seconds(now),
        timezone: entry.timezone.clone(),
        created_at: to_storage(entry.created_at),
        updated_at: to_storage(entry.updated_at),
        revision: entry.revision,
        deleted_at: entry.deleted_at.map(to_storage),
    }
}

pub fn to_json_string<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string_pretty(value).map_err(|e| {
        AppError::new(ErrorKind::Validation, format!("cannot serialize JSON output: {e}"))
    })
}

/// Multi-line detail view used by `show` and by the confirmations that
/// `start`, `stop`, `add`, `edit`, and `delete` print.
pub fn entry_detail(entry: &Entry, cfg: &Config, now: Timestamp) -> String {
    let tz = &cfg.timezone;
    let p = cfg.precision;
    let mut lines = vec![
        format!("{:<12}{}", "ID", entry.id),
        format!("{:<12}{}", "Description", entry.description),
        format!(
            "{:<12}{}",
            "Project",
            entry.project.clone().unwrap_or_else(|| "(none)".into())
        ),
        format!(
            "{:<12}{}",
            "Tags",
            if entry.tags.is_empty() { "(none)".to_string() } else { entry.tags.join(", ") }
        ),
        format!("{:<12}{}", "Start", format_local_datetime(entry.start_at, tz, p)),
        format!(
            "{:<12}{}",
            "End",
            match entry.end_at {
                Some(end) => format_local_datetime(end, tz, p),
                None => "(running)".to_string(),
            }
        ),
        format!("{:<12}{}", "Duration", format_duration(entry.duration_seconds(now), p)),
    ];
    if let Some(deleted) = entry.deleted_at {
        lines.push(format!("{:<12}{}", "Deleted", format_local_datetime(deleted, tz, p)));
    }
    lines.push(format!("{:<12}{}", "Revision", entry.revision));
    lines.push(format!("{:<12}{}", "Timezone", entry.timezone));
    lines.join("\n")
}

/// Aligned table used by `tt log`. Local times are shown as `YYYY-MM-DD` plus
/// `HH:MM` (or `HH:MM:SS` when `duration_precision = "second"`).
pub fn entry_table(entries: &[Entry], cfg: &Config, now: Timestamp) -> String {
    if entries.is_empty() {
        return "No entries matched.".to_string();
    }
    let tz = &cfg.timezone;
    let p = cfg.precision;

    let headers = ["DATE", "START", "END", "DURATION", "DESCRIPTION", "PROJECT", "TAGS", "ID"];
    let mut rows: Vec<[String; 8]> = Vec::new();

    for entry in entries {
        let end = match entry.end_at {
            Some(end) => {
                let mut text = format_local_time(end, tz, p);
                // Flag an end time that lands on a later local day.
                let start_date = entry.start_at.to_zoned(tz.clone()).date();
                let end_date = end.to_zoned(tz.clone()).date();
                if end_date != start_date {
                    text.push_str(&format!(" (+{})", (end_date - start_date).get_days()));
                }
                text
            }
            None => "in progress".to_string(),
        };
        rows.push([
            format_local_date(entry.start_at, tz),
            format_local_time(entry.start_at, tz, p),
            end,
            format_duration(entry.duration_seconds(now), p),
            entry.description.clone(),
            entry.project_label().to_string(),
            entry.tags.join(","),
            entry.short_id(),
        ]);
    }

    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }

    let render = |cells: &[String]| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                let pad = widths[i] - cell.chars().count();
                if i == cells.len() - 1 {
                    cell.clone()
                } else {
                    format!("{cell}{}", " ".repeat(pad))
                }
            })
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };

    let mut out = vec![render(&headers.map(String::from))];
    for row in &rows {
        out.push(render(row));
    }
    out.join("\n")
}

/// Readable report. Display rounding is applied here and nowhere else.
pub fn report_text(report: &Report, precision: Precision) -> String {
    let mut out = Vec::new();
    out.push(report.title.clone());
    out.push(format!(
        "{} through {} ({})",
        report.first_day, report.last_day, report.timezone
    ));
    out.push(String::new());
    out.push(format!(
        "Total: {} across {} {}",
        format_duration(report.total_seconds, precision),
        report.entry_count,
        if report.entry_count == 1 { "entry" } else { "entries" }
    ));

    let width = |groups: &[crate::reports::Group]| {
        groups.iter().map(|g| g.name.chars().count()).max().unwrap_or(0).max(10)
    };

    out.push(String::new());
    out.push("By project".to_string());
    if report.by_project.is_empty() {
        out.push("  (no time recorded)".to_string());
    } else {
        let w = width(&report.by_project);
        for group in &report.by_project {
            out.push(format!(
                "  {:<w$}  {}",
                group.name,
                format_duration(group.seconds, precision),
                w = w
            ));
        }
    }

    out.push(String::new());
    out.push("By day".to_string());
    for day in &report.by_day {
        out.push(format!(
            "  {} {}  {}",
            &day.weekday[..3],
            day.date,
            format_duration(day.seconds, precision)
        ));
    }

    if !report.by_tag.is_empty() {
        out.push(String::new());
        out.push("By tag".to_string());
        let w = width(&report.by_tag);
        for group in &report.by_tag {
            out.push(format!(
                "  {:<w$}  {}",
                group.name,
                format_duration(group.seconds, precision),
                w = w
            ));
        }
        out.push(
            "  (an entry counts toward each of its tags, so tag totals may exceed the total)"
                .to_string(),
        );
    }

    if !report.overlaps.is_empty() {
        out.push(String::new());
        out.push(format!(
            "Warning: {} overlapping {} contribute to this report; time is counted more than once.",
            report.overlaps.len(),
            if report.overlaps.len() == 1 { "pair" } else { "pairs" }
        ));
        for pair in &report.overlaps {
            let short = |id: &str| id.chars().take(8).collect::<String>();
            out.push(format!("  {} overlaps {}", short(&pair.first), short(&pair.second)));
        }
    }

    out.join("\n")
}

/// Long-format CSV: one row per aggregate group, plus a `total` row.
pub fn report_csv(report: &Report) -> Result<String> {
    let mut writer = csv::Writer::from_writer(Vec::new());
    let fail = |e: csv::Error| {
        AppError::new(ErrorKind::Validation, format!("cannot write CSV output: {e}"))
    };

    writer
        .write_record(["period_start", "period_end", "group_type", "group_name", "seconds"])
        .map_err(fail)?;

    let mut row = |group_type: &str, name: &str, seconds: i64| -> Result<()> {
        writer
            .write_record([
                report.first_day.as_str(),
                report.last_day.as_str(),
                group_type,
                name,
                &seconds.to_string(),
            ])
            .map_err(fail)
    };

    row("total", "total", report.total_seconds)?;
    for group in &report.by_project {
        row("project", &group.name, group.seconds)?;
    }
    for day in &report.by_day {
        row("day", &day.date, day.seconds)?;
    }
    for group in &report.by_tag {
        row("tag", &group.name, group.seconds)?;
    }

    let bytes = writer.into_inner().map_err(|e| {
        AppError::new(ErrorKind::Validation, format!("cannot write CSV output: {e}"))
    })?;
    String::from_utf8(bytes).map_err(|e| {
        AppError::new(ErrorKind::Validation, format!("CSV output was not valid UTF-8: {e}"))
    })
}
