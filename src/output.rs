//! Rendering. Human output goes to stdout; warnings and anything uncertain go
//! to stderr so that `--format json` output stays machine-readable.

use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

use crate::error::{Error, Kind, Result};
use crate::ot::model::TimeEntry;
use crate::report::{Report, format_duration, hours};

pub fn warn(message: &str) {
    eprintln!("tt: note: {message}");
}

fn zoned(ts: Timestamp, tz: &TimeZone) -> Zoned {
    ts.to_zoned(tz.clone())
}

/// Seconds an entry covers, measured to `now` while it is running.
pub fn entry_seconds(entry: &TimeEntry, now: Timestamp) -> i64 {
    let end = entry.ended_at.unwrap_or_else(|| now.max(entry.started_at));
    (end.as_second() - entry.started_at.as_second()).max(0)
}

pub fn entry_json(entry: &TimeEntry, tz: &TimeZone, now: Timestamp) -> serde_json::Value {
    serde_json::json!({
        "id": entry.id,
        "taskId": entry.task_id,
        "taskTitle": entry.task_title,
        "projectId": entry.project_id,
        "projectName": entry.project_name,
        "note": entry.note,
        "startedAt": entry.started_at.to_string(),
        "startedLocal": zoned(entry.started_at, tz).strftime("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        "endedAt": entry.ended_at.map(|t| t.to_string()),
        "running": entry.is_running(),
        "seconds": entry_seconds(entry, now),
        // OpenTodo's own rounded duration, passed through unchanged.
        "openTodoMinutes": entry.minutes,
    })
}

/// One line per entry: `Mon 2026-10-26  09:00-09:25  25m  label  [task]`.
pub fn entry_line(entry: &TimeEntry, tz: &TimeZone, now: Timestamp) -> String {
    let start = zoned(entry.started_at, tz);
    let end = match entry.ended_at {
        Some(end) => zoned(end, tz).strftime("%H:%M").to_string(),
        None => "now".to_string(),
    };
    let duration = format_duration(entry_seconds(entry, now));
    let mut line = format!(
        "{}  {}-{}  {:>7}  {}",
        start.strftime("%a %Y-%m-%d"),
        start.strftime("%H:%M"),
        end,
        duration,
        entry.label()
    );
    if let Some(project) = &entry.project_name {
        line.push_str(&format!("  ({project})"));
    }
    if let Some(task) = &entry.task_id {
        line.push_str(&format!("  [{task}]"));
    }
    if entry.is_running() {
        line.push_str("  <- running");
    }
    line
}

pub fn print_entries_text(entries: &[TimeEntry], tz: &TimeZone, now: Timestamp) {
    if entries.is_empty() {
        println!("No time entries.");
        return;
    }
    for entry in entries {
        println!("{}", entry_line(entry, tz, now));
    }
    let total: i64 = entries.iter().map(|e| entry_seconds(e, now)).sum();
    println!();
    println!("{} entries, {} total", entries.len(), format_duration(total));
}

pub fn print_json(value: &serde_json::Value) -> Result<()> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| Error::new(Kind::Server, format!("could not render JSON: {e}")))?;
    println!("{text}");
    Ok(())
}

pub fn report_json(report: &Report) -> serde_json::Value {
    serde_json::json!({
        "from": report.period.from.to_string(),
        "to": report.period.to.to_string(),
        "timezone": report.timezone,
        "weekStart": report.week_start.map(|d| crate::config::weekday_name(d)),
        "totalSeconds": report.total_seconds,
        "totalHours": hours(report.total_seconds),
        "entryCount": report.entry_count,
        "days": report.days.iter().map(|day| serde_json::json!({
            "date": day.date.to_string(),
            "weekday": day.date.strftime("%A").to_string(),
            "seconds": day.seconds,
            "hours": hours(day.seconds),
        })).collect::<Vec<_>>(),
        "tasks": report.tasks.iter().map(|task| serde_json::json!({
            "taskId": task.task_id,
            "title": task.label,
            "project": task.project,
            "seconds": task.seconds,
            "hours": hours(task.seconds),
            "entries": task.entries,
        })).collect::<Vec<_>>(),
        "untaskedSeconds": report.untasked_seconds,
        "runningEntries": report.running_entries,
        // Never silently dropped: a consumer can refuse to invoice an
        // incomplete week.
        "incomplete": report.incomplete,
        "warnings": report.warnings,
    })
}

pub fn print_report_text(report: &Report) {
    let heading = match report.week_start {
        Some(start) => format!(
            "Week of {} to {} (week starts {}, {})",
            report.period.from,
            report.period.to,
            crate::config::weekday_name(start),
            report.timezone
        ),
        None => format!("{} to {} ({})", report.period.from, report.period.to, report.timezone),
    };
    println!("{heading}");
    println!("{}", "-".repeat(heading.len()));
    println!("Total: {}", format_duration(report.total_seconds));
    println!();

    println!("By day");
    for day in &report.days {
        println!(
            "  {}  {:>8}",
            day.date.strftime("%a %Y-%m-%d"),
            format_duration(day.seconds)
        );
    }
    println!();

    println!("By task");
    if report.tasks.is_empty() && report.untasked_seconds == 0 {
        println!("  (nothing tracked)");
    }
    for task in &report.tasks {
        let project = task.project.as_deref().map(|p| format!(" ({p})")).unwrap_or_default();
        println!(
            "  {:>8}  {}{}  [{}]",
            format_duration(task.seconds),
            task.label,
            project,
            task.task_id.as_deref().unwrap_or("-")
        );
    }
    if report.untasked_seconds > 0 {
        println!("  {:>8}  (no task)", format_duration(report.untasked_seconds));
    }

    if !report.warnings.is_empty() {
        println!();
        println!("Notes");
        for warning in &report.warnings {
            println!("  - {warning}");
        }
    }
}

/// Tidy CSV: one row per day, one per task, one total. The `section` column
/// lets a spreadsheet filter the parts apart.
pub fn print_report_csv(report: &Report) -> Result<()> {
    let mut writer = csv::Writer::from_writer(std::io::stdout());
    let csv_err = |e: csv::Error| Error::new(Kind::Server, format!("could not write CSV: {e}"));

    writer
        .write_record(["section", "key", "label", "seconds", "hours"])
        .map_err(csv_err)?;
    for day in &report.days {
        writer
            .write_record([
                "day",
                &day.date.to_string(),
                &day.date.strftime("%A").to_string(),
                &day.seconds.to_string(),
                &hours(day.seconds).to_string(),
            ])
            .map_err(csv_err)?;
    }
    for task in &report.tasks {
        writer
            .write_record([
                "task",
                task.task_id.as_deref().unwrap_or(""),
                &task.label,
                &task.seconds.to_string(),
                &hours(task.seconds).to_string(),
            ])
            .map_err(csv_err)?;
    }
    if report.untasked_seconds > 0 {
        writer
            .write_record([
                "task",
                "",
                "(no task)",
                &report.untasked_seconds.to_string(),
                &hours(report.untasked_seconds).to_string(),
            ])
            .map_err(csv_err)?;
    }
    writer
        .write_record([
            "total",
            &report.period.from.to_string(),
            &report.period.to.to_string(),
            &report.total_seconds.to_string(),
            &hours(report.total_seconds).to_string(),
        ])
        .map_err(csv_err)?;
    // An incomplete report says so inside the file, not only on the terminal.
    writer
        .write_record([
            "meta",
            "incomplete",
            if report.incomplete { "true" } else { "false" },
            "",
            "",
        ])
        .map_err(csv_err)?;
    for warning in &report.warnings {
        writer.write_record(["meta", "warning", warning, "", ""]).map_err(csv_err)?;
    }
    writer.flush().map_err(|e| Error::new(Kind::Server, format!("could not write CSV: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{self, Period};
    use jiff::civil::date;

    fn tz() -> TimeZone {
        TimeZone::get("America/New_York").expect("tzdb")
    }

    fn ts(raw: &str) -> Timestamp {
        raw.parse().expect("timestamp")
    }

    fn sample() -> TimeEntry {
        TimeEntry {
            id: "e1".into(),
            task_id: Some("8c2a".into()),
            task_title: Some("Design review".into()),
            project_id: Some("p1".into()),
            project_name: Some("Work".into()),
            note: Some("review".into()),
            started_at: ts("2026-10-26T13:00:00Z"),
            ended_at: Some(ts("2026-10-26T13:25:00Z")),
            minutes: Some(25),
        }
    }

    #[test]
    fn an_entry_line_shows_local_times() {
        let line = entry_line(&sample(), &tz(), ts("2026-10-26T20:00:00Z"));
        assert!(line.contains("Mon 2026-10-26"), "{line}");
        assert!(line.contains("09:00-09:25"), "{line}");
        assert!(line.contains("25m"), "{line}");
        assert!(line.contains("[8c2a]"), "{line}");
    }

    #[test]
    fn a_running_entry_is_marked_and_measured_to_now() {
        let mut entry = sample();
        entry.ended_at = None;
        let line = entry_line(&entry, &tz(), ts("2026-10-26T14:00:00Z"));
        assert!(line.contains("09:00-now"), "{line}");
        assert!(line.contains("running"), "{line}");
        assert_eq!(entry_seconds(&entry, ts("2026-10-26T14:00:00Z")), 3600);
    }

    #[test]
    fn report_json_carries_the_incomplete_flag_and_warnings() {
        let period = Period::new(date(2026, 10, 26), date(2026, 10, 26)).unwrap();
        let mut entry = sample();
        entry.ended_at = None;
        let built = report::build(
            period,
            &tz(),
            Some(jiff::civil::Weekday::Monday),
            &[entry],
            ts("2026-10-26T14:00:00Z"),
            vec![],
            false,
        )
        .unwrap();
        let json = report_json(&built);
        assert_eq!(json["incomplete"], true);
        assert_eq!(json["weekStart"], "monday");
        assert_eq!(json["totalSeconds"], 3600);
        assert!(json["warnings"].as_array().unwrap().len() >= 1);
    }

    #[test]
    fn entry_json_reports_both_utc_and_local_starts() {
        let json = entry_json(&sample(), &tz(), ts("2026-10-26T20:00:00Z"));
        assert_eq!(json["startedAt"], "2026-10-26T13:00:00Z");
        assert_eq!(json["startedLocal"], "2026-10-26T09:00:00-04:00");
        assert_eq!(json["openTodoMinutes"], 25);
        assert_eq!(json["seconds"], 1500);
    }
}
