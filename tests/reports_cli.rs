//! `tt report week` and `tt report range` against a mock `ot`.

mod support;

use support::{Harness, entry, ms};

const NOW: &str = "2026-10-28T18:00:00Z"; // Wed 14:00 America/New_York

/// Monday 1h, Tuesday 1h30, plus an hour straddling Tuesday midnight.
fn week_entries() -> String {
    format!(
        "[{},{},{}]",
        entry("a", "8c2a", "Design review", ms("2026-10-26T13:00:00Z"), Some(ms("2026-10-26T14:00:00Z"))),
        entry("b", "3f1d", "Planning", ms("2026-10-27T18:00:00Z"), Some(ms("2026-10-27T19:30:00Z"))),
        entry("c", "8c2a", "Design review", ms("2026-10-28T03:30:00Z"), Some(ms("2026-10-28T04:30:00Z"))),
    )
}

#[test]
fn the_weekly_report_covers_the_configured_week_and_states_its_rules() {
    let h = Harness::new();
    h.respond("time-list", &week_entries());

    let out = h.ok(NOW, &["report", "week"]);
    assert!(
        out.contains("Week of 2026-10-26 to 2026-11-01 (week starts monday, America/New_York)"),
        "the week start and timezone are documented in the output: {out}"
    );
    assert!(out.contains("Total: 3h 30m"), "{out}");
    assert!(out.contains("Mon 2026-10-26    1h 00m"), "{out}");
    assert!(out.contains("Tue 2026-10-27    2h 00m"), "an hour splits across midnight: {out}");
    assert!(out.contains("Wed 2026-10-28       30m"), "{out}");
    assert!(out.contains("Sun 2026-11-01"), "every day of the week appears: {out}");
}

#[test]
fn task_totals_merge_entries_and_name_the_project() {
    let h = Harness::new();
    h.respond("time-list", &week_entries());

    let out = h.ok(NOW, &["report", "week"]);
    let tasks = out.split("By task").nth(1).expect("a task section");
    let first = tasks.lines().find(|l| l.contains("Design review")).expect("design review row");
    assert!(first.contains("2h 00m"), "{first}");
    assert!(first.contains("(Work)"), "{first}");
    assert!(first.contains("[8c2a]"), "{first}");
}

#[test]
fn the_week_start_can_be_overridden_on_the_command_line() {
    let h = Harness::new();
    h.respond("time-list", "[]");

    let out = h.ok(NOW, &["report", "week", "--week-start", "sunday"]);
    assert!(out.contains("Week of 2026-10-25 to 2026-10-31 (week starts sunday"), "{out}");
}

#[test]
fn a_past_week_can_be_selected_by_any_date_inside_it() {
    let h = Harness::new();
    h.respond("time-list", "[]");

    let out = h.ok(NOW, &["report", "week", "--week", "2026-10-21"]);
    assert!(out.contains("Week of 2026-10-19 to 2026-10-25"), "{out}");
    assert_eq!(
        h.calls_matching(&["time", "list"])[0],
        vec!["time", "list", "--json", "--from=2026-10-18", "--to=2026-10-26", "--limit=5000"]
    );
}

#[test]
fn the_json_report_carries_totals_days_tasks_and_honesty_flags() {
    let h = Harness::new();
    h.respond("time-list", &week_entries());

    let json = h.json(NOW, &["report", "week", "--format", "json"]);
    assert_eq!(json["from"], "2026-10-26");
    assert_eq!(json["to"], "2026-11-01");
    assert_eq!(json["timezone"], "America/New_York");
    assert_eq!(json["weekStart"], "monday");
    assert_eq!(json["totalSeconds"], 12600);
    assert_eq!(json["totalHours"], 3.5);
    assert_eq!(json["incomplete"], false);
    assert_eq!(json["untaskedSeconds"], 0);

    let days = json["days"].as_array().expect("days");
    assert_eq!(days.len(), 7);
    assert_eq!(days[1]["seconds"], 7200);
    assert_eq!(days[1]["weekday"], "Tuesday");
    let summed: i64 = days.iter().map(|d| d["seconds"].as_i64().unwrap()).sum();
    assert_eq!(summed, 12600, "daily totals must sum to the period total");

    let tasks = json["tasks"].as_array().expect("tasks");
    assert_eq!(tasks[0]["taskId"], "8c2a");
    assert_eq!(tasks[0]["seconds"], 7200);
    assert_eq!(tasks[0]["entries"], 2);
}

#[test]
fn a_running_entry_makes_the_report_say_it_is_incomplete() {
    let h = Harness::new();
    let running = entry("r", "8c2a", "Design review", ms("2026-10-28T17:00:00Z"), None);
    h.respond("time-list", &format!("[{running}]"));

    let json = h.json(NOW, &["report", "week", "--format", "json"]);
    assert_eq!(json["incomplete"], true);
    assert_eq!(json["runningEntries"], 1);
    assert_eq!(json["totalSeconds"], 3600, "counted up to now");
    let warnings = json["warnings"].as_array().expect("warnings");
    assert!(warnings.iter().any(|w| w.as_str().unwrap().contains("still running")), "{warnings:?}");
}

#[test]
fn an_untasked_entry_is_reported_separately() {
    let h = Harness::new();
    let untasked = format!(
        r#"[{{"id":"u1","taskId":null,"startedAt":{},"endedAt":{},"minutes":30}}]"#,
        ms("2026-10-26T13:00:00Z"),
        ms("2026-10-26T13:30:00Z")
    );
    h.respond("time-list", &untasked);

    let out = h.ok(NOW, &["report", "week"]);
    assert!(out.contains("30m  (no task)"), "{out}");

    let h2 = Harness::new();
    h2.respond("time-list", &untasked);
    let json = h2.json(NOW, &["report", "week", "--format", "json"]);
    assert_eq!(json["untaskedSeconds"], 1800);
}

#[test]
fn the_range_report_uses_inclusive_local_dates() {
    let h = Harness::new();
    h.respond("time-list", &week_entries());

    let out = h.ok(NOW, &["report", "range", "--from", "2026-10-26", "--to", "2026-10-27"]);
    assert!(out.contains("2026-10-26 to 2026-10-27 (America/New_York)"), "{out}");
    // Wednesday's half hour falls outside the range and is clipped away.
    assert!(out.contains("Total: 3h 00m"), "{out}");
}

#[test]
fn csv_output_is_valid_and_carries_the_incomplete_marker() {
    let h = Harness::new();
    h.respond("time-list", &week_entries());

    let csv = h.ok(NOW, &["report", "range", "--from", "2026-10-26", "--to", "2026-10-28", "--format", "csv"]);
    let mut reader = csv::ReaderBuilder::new().from_reader(csv.as_bytes());
    let headers = reader.headers().expect("headers").clone();
    assert_eq!(headers, vec!["section", "key", "label", "seconds", "hours"]);

    let rows: Vec<csv::StringRecord> = reader.records().map(|r| r.expect("row")).collect();
    let days: Vec<&csv::StringRecord> = rows.iter().filter(|r| &r[0] == "day").collect();
    assert_eq!(days.len(), 3);
    assert_eq!(&days[1][3], "7200");

    let total = rows.iter().find(|r| &r[0] == "total").expect("a total row");
    assert_eq!(&total[3], "12600");
    assert_eq!(&total[4], "3.5");

    let meta = rows.iter().find(|r| &r[1] == "incomplete").expect("an incomplete marker");
    assert_eq!(&meta[2], "false");
}

#[test]
fn the_default_report_format_comes_from_the_config_file() {
    let h = Harness::with_settings(
        "timezone = \"America/New_York\"\nweek_start = \"monday\"\nreport_format = \"json\"\n",
    );
    h.respond("time-list", "[]");

    let json = h.json(NOW, &["report", "week"]);
    assert_eq!(json["totalSeconds"], 0);
}

#[test]
fn a_week_in_a_different_zone_shifts_its_boundaries() {
    let h = Harness::with_settings("timezone = \"UTC\"\nweek_start = \"monday\"\n");
    // 2026-10-28T03:30Z is Tuesday evening in New York but Wednesday in UTC.
    h.respond(
        "time-list",
        &format!(
            "[{}]",
            entry("c", "8c2a", "Design review", ms("2026-10-28T03:30:00Z"), Some(ms("2026-10-28T04:30:00Z")))
        ),
    );

    let json = h.json(NOW, &["report", "week", "--format", "json"]);
    assert_eq!(json["timezone"], "UTC");
    let days = json["days"].as_array().unwrap();
    assert_eq!(days[2]["seconds"], 3600, "the whole hour lands on Wednesday in UTC");
    assert_eq!(days[1]["seconds"], 0);
}

#[test]
fn a_truncated_result_marks_the_report_incomplete() {
    let h = Harness::new();
    h.respond("time-list", &week_entries());

    let json = h.json(NOW, &["report", "week", "--format", "json", "--limit", "3"]);
    assert_eq!(json["incomplete"], true, "a result that fills the limit is a lower bound");
    assert_eq!(json["runningEntries"], 0, "nothing is running; the limit alone made it partial");
    let warnings = json["warnings"].as_array().expect("warnings");
    assert!(warnings.iter().any(|w| w.as_str().unwrap().contains("may be incomplete")));
}

#[test]
fn a_pagination_cursor_marks_the_report_incomplete() {
    let h = Harness::new();
    h.respond("time-list", r#"{"entries":[],"nextCursor":"abc"}"#);

    let json = h.json(NOW, &["report", "week", "--format", "json"]);
    assert_eq!(json["incomplete"], true, "tt cannot follow the cursor, so totals are partial");
}
