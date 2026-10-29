//! Start, stop and status against a mock `ot`.

mod support;

use support::{Harness, entry, ms};

const NOW: &str = "2026-10-28T18:00:00Z"; // Wed 14:00 America/New_York

fn running_entry() -> String {
    entry("e9", "8c2a", "Design review", ms("2026-10-28T17:00:00Z"), None)
}

fn stopped_entry() -> String {
    entry(
        "e9",
        "8c2a",
        "Design review",
        ms("2026-10-28T17:00:00Z"),
        Some(ms("2026-10-28T18:00:00Z")),
    )
}

#[test]
fn start_checks_for_a_running_timer_then_starts_one() {
    let h = Harness::new();
    h.respond("time-list", "[]");
    h.respond("time-start", &running_entry());

    let out = h.ok(NOW, &["start", "Design review", "--task", "8c2a"]);
    assert!(out.contains("Started"), "{out}");
    assert!(out.contains("Design review"), "{out}");
    assert!(out.contains("task 8c2a"), "{out}");

    let calls = h.calls();
    assert_eq!(calls[0], vec!["version", "--json"]);
    assert_eq!(calls[1], vec!["time", "list", "--json", "--running"]);
    assert_eq!(
        calls[2],
        vec!["time", "start", "8c2a", "--note=Design review", "--json"],
        "only documented ot arguments, passed as a vector"
    );
}

#[test]
fn start_refuses_to_replace_a_running_timer_without_switch() {
    let h = Harness::new();
    h.respond("time-list", &format!("[{}]", running_entry()));

    let run = h.fail(NOW, &["start", "Other work", "--task", "3f1d"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("already running"), "{}", run.stderr);
    assert!(run.stderr.contains("--switch"), "{}", run.stderr);
    assert_eq!(h.call_count(&["time", "start"]), 0, "no write may be sent");
    assert!(run.stdout.is_empty(), "a failure must not print results");
}

#[test]
fn start_switch_warns_that_opentodo_will_end_the_current_entry() {
    let h = Harness::new();
    h.respond("time-list", &format!("[{}]", running_entry()));
    h.respond("time-start", &entry("e10", "3f1d", "Other work", ms(NOW), None));

    let run = h.run(NOW, &["start", "Other work", "--task", "3f1d", "--switch"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.contains("will end the running entry"), "{}", run.stderr);
    assert_eq!(h.call_count(&["time", "start"]), 1);
}

#[test]
fn start_without_a_task_fails_cleanly_when_there_is_no_terminal() {
    let h = Harness::new();
    h.respond("time-list", "[]");

    let run = h.fail(NOW, &["start", "Design review"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("--task"), "{}", run.stderr);
    assert_eq!(h.call_count(&["time", "start"]), 0);
    assert_eq!(h.call_count(&["list"]), 0, "no interactive lookup without a terminal");
}

#[test]
fn start_json_reports_the_entry_opentodo_returned() {
    let h = Harness::new();
    h.respond("time-list", "[]");
    h.respond("time-start", &running_entry());

    let json = h.json(NOW, &["start", "Design review", "--task", "8c2a", "--json"]);
    assert_eq!(json["id"], "e9");
    assert_eq!(json["taskId"], "8c2a");
    assert_eq!(json["running"], true);
    assert_eq!(json["startedLocal"], "2026-10-28T13:00:00-04:00");
}

#[test]
fn stop_without_a_running_timer_changes_nothing() {
    let h = Harness::new();
    h.respond("time-list", "[]");

    let run = h.fail(NOW, &["stop"]);
    assert_eq!(run.code, 8, "exit 8 means no active timer");
    assert!(run.stderr.contains("no timer is running"), "{}", run.stderr);
    assert_eq!(h.call_count(&["time", "stop"]), 0, "ot must not be asked to stop nothing");
}

#[test]
fn stop_reports_the_completed_entry() {
    let h = Harness::new();
    h.respond("time-list", &format!("[{}]", running_entry()));
    h.respond("time-stop", &stopped_entry());

    let out = h.ok(NOW, &["stop"]);
    assert!(out.contains("Stopped"), "{out}");
    assert!(out.contains("1h 00m"), "duration comes from OpenTodo's timestamps: {out}");
    assert_eq!(h.calls().last().unwrap(), &vec!["time", "stop", "--json"]);
}

#[test]
fn stop_rereads_the_entry_when_opentodo_returns_no_details() {
    let h = Harness::new();
    // 1: the running check. 2: the re-read after the undocumented stop output.
    h.respond_nth("time-list", 1, &format!("[{}]", running_entry()));
    h.respond_nth("time-list", 2, &format!("[{}]", stopped_entry()));
    h.respond("time-stop", "");

    let out = h.ok(NOW, &["stop"]);
    assert!(out.contains("1h 00m"), "{out}");
    let reread = &h.calls_matching(&["time", "list"])[1];
    assert!(reread.contains(&"--task=8c2a".to_string()), "{reread:?}");
}

#[test]
fn stop_does_not_invent_a_duration_when_the_entry_cannot_be_reread() {
    let h = Harness::new();
    h.respond_nth("time-list", 1, &format!("[{}]", running_entry()));
    h.respond_nth("time-list", 2, "[]");
    h.respond("time-stop", "");

    let run = h.run(NOW, &["stop"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("Stopped the timer"), "{}", run.stdout);
    assert!(run.stderr.contains("no entry details"), "{}", run.stderr);
    assert!(!run.stdout.contains("1h"), "no duration may be fabricated: {}", run.stdout);
}

#[test]
fn status_shows_the_running_timer_and_elapsed_time() {
    let h = Harness::new();
    h.respond("time-list", &format!("[{}]", running_entry()));

    let out = h.ok(NOW, &["status"]);
    assert!(out.contains("Running"), "{out}");
    assert!(out.contains("1h 00m"), "{out}");
    assert!(out.contains("13:00"), "local start time: {out}");
}

#[test]
fn status_is_concise_and_json_shaped_when_nothing_runs() {
    let h = Harness::new();
    h.respond("time-list", "[]");

    assert!(h.ok(NOW, &["status"]).contains("No timer is running"));

    let h2 = Harness::new();
    h2.respond("time-list", "[]");
    let json = h2.json(NOW, &["status", "--json"]);
    assert_eq!(json["running"], false);
    assert!(json["entry"].is_null());
}
