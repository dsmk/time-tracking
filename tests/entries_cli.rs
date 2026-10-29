//! `tt log` against a mock `ot`.

mod support;

use support::{Harness, entry, ms};

const NOW: &str = "2026-10-28T18:00:00Z"; // Wed 14:00 America/New_York

/// Monday 09:00-10:00, Tuesday 14:00-15:30, and Tuesday 23:30 into Wednesday.
fn week_entries() -> String {
    format!(
        "[{},{},{}]",
        entry("a", "8c2a", "Design review", ms("2026-10-26T13:00:00Z"), Some(ms("2026-10-26T14:00:00Z"))),
        entry("b", "3f1d", "Planning", ms("2026-10-27T18:00:00Z"), Some(ms("2026-10-27T19:30:00Z"))),
        entry("c", "8c2a", "Design review", ms("2026-10-28T03:30:00Z"), Some(ms("2026-10-28T04:30:00Z"))),
    )
}

#[test]
fn log_defaults_to_the_last_seven_days_and_totals_them() {
    let h = Harness::new();
    h.respond("time-list", &week_entries());

    let out = h.ok(NOW, &["log"]);
    assert!(out.contains("2026-10-22 to 2026-10-28"), "{out}");
    assert!(out.contains("America/New_York"), "{out}");
    assert!(out.contains("3 entries, 3h 30m total"), "{out}");
    assert!(out.contains("Mon 2026-10-26  09:00-10:00"), "{out}");
}

#[test]
fn log_asks_opentodo_for_a_window_one_day_wider_on_each_side() {
    let h = Harness::new();
    h.respond("time-list", "[]");

    h.ok(NOW, &["log", "--from", "2026-10-26", "--to", "2026-10-28"]);
    assert_eq!(
        h.calls_matching(&["time", "list"])[0],
        vec!["time", "list", "--json", "--from=2026-10-25", "--to=2026-10-29", "--limit=5000"],
        "the window is widened because ot has no documented --tz flag"
    );
}

#[test]
fn log_today_keeps_only_entries_touching_today_in_the_configured_zone() {
    let h = Harness::new();
    // The widened query returns all three; only the one crossing into
    // Wednesday 00:00-00:30 New York time touches today.
    h.respond("time-list", &week_entries());

    let out = h.ok(NOW, &["log", "--today"]);
    assert!(out.contains("2026-10-28 to 2026-10-28"), "{out}");
    assert!(out.contains("1 entries"), "{out}");
    assert!(out.contains("Tue 2026-10-27  23:30-00:30"), "{out}");
    assert!(!out.contains("Planning"), "Tuesday afternoon is not today: {out}");
}

#[test]
fn log_filters_by_task_through_the_documented_flag() {
    let h = Harness::new();
    h.respond("time-list", "[]");

    h.ok(NOW, &["log", "--today", "--task", "8c2a"]);
    let call = &h.calls_matching(&["time", "list"])[0];
    assert!(call.contains(&"--task=8c2a".to_string()), "{call:?}");
}

#[test]
fn log_json_exposes_both_instants_and_opentodos_own_minutes() {
    let h = Harness::new();
    h.respond("time-list", &week_entries());

    let json = h.json(NOW, &["log", "--format", "json"]);
    let items = json.as_array().expect("an array of entries");
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["id"], "a");
    assert_eq!(items[0]["startedAt"], "2026-10-26T13:00:00Z");
    assert_eq!(items[0]["startedLocal"], "2026-10-26T09:00:00-04:00");
    assert_eq!(items[0]["seconds"], 3600);
    assert_eq!(items[0]["openTodoMinutes"], 60);
    assert_eq!(items[0]["running"], false);
}

#[test]
fn a_result_that_fills_the_limit_is_reported_as_possibly_incomplete() {
    let h = Harness::new();
    h.respond("time-list", &week_entries());

    let run = h.run(NOW, &["log", "--limit", "3"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.contains("may be incomplete"), "{}", run.stderr);
    assert!(run.stderr.contains("larger --limit"), "{}", run.stderr);
}

#[test]
fn a_next_cursor_is_reported_rather_than_silently_dropped() {
    let h = Harness::new();
    h.respond("time-list", r#"{"entries":[],"nextCursor":"abc"}"#);

    let run = h.run(NOW, &["log"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.contains("no documented pagination"), "{}", run.stderr);
}

#[test]
fn log_rejects_csv_and_points_at_the_report_command() {
    let h = Harness::new();
    let run = h.fail(NOW, &["log", "--format", "csv"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("report range --format csv"), "{}", run.stderr);
}

#[test]
fn a_backwards_range_is_refused_before_opentodo_is_called() {
    let h = Harness::new();
    let run = h.fail(NOW, &["log", "--from", "2026-10-28", "--to", "2026-10-01"]);
    assert_eq!(run.code, 2);
    assert_eq!(h.call_count(&["time", "list"]), 0);
}

#[test]
fn an_invalid_date_names_the_expected_form() {
    let h = Harness::new();
    let run = h.fail(NOW, &["log", "--from", "last tuesday"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("YYYY-MM-DD"), "{}", run.stderr);
}
