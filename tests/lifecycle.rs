mod support;

use support::Harness;

/// Tuesday 09:00 America/New_York.
const T0900: &str = "2026-10-27T13:00:00Z";
const T1030: &str = "2026-10-27T14:30:00Z";
const T1200: &str = "2026-10-27T16:00:00Z";

#[test]
fn init_creates_the_database_and_is_safe_to_repeat() {
    let h = Harness::new();
    let first = h.ok(T0900, &["init"]);
    assert!(first.contains("schema version 1"), "{first}");
    assert!(h.db.exists(), "init must create the database file");

    // Put a row in, then re-initialize: nothing may be lost.
    h.ok(T0900, &["add", "Keep me", "--start", "2026-10-27T09:00", "--duration", "30m"]);
    let second = h.ok(T1030, &["init"]);
    assert!(second.contains("schema version 1"), "{second}");
    assert_eq!(h.json(T1030, &["log", "--json"]).as_array().unwrap().len(), 1);
}

#[test]
fn the_database_is_created_on_demand_without_init() {
    let h = Harness::new();
    h.ok(T0900, &["start", "No init first"]);
    assert!(h.db.exists());
}

#[test]
fn start_status_stop_lifecycle_survives_process_exit() {
    let h = Harness::new();

    let started = h.ok(T0900, &["start", "Implement login", "--project", "app", "--tag", "focus"]);
    assert!(started.contains("Started timer"), "{started}");

    // A separate process must still see the running timer: nothing essential
    // is held in memory.
    let status = h.json(T1030, &["status", "--json"]);
    assert_eq!(status["description"], "Implement login");
    assert_eq!(status["project"], "app");
    assert_eq!(status["running"], true);
    assert_eq!(status["end_at"], serde_json::Value::Null);
    assert_eq!(status["duration_seconds"], 5400, "running time counts through now");
    assert_eq!(status["start_at"], "2026-10-27T13:00:00Z");
    assert_eq!(status["timezone"], "America/New_York");

    let stopped = h.ok(T1030, &["stop"]);
    assert!(stopped.contains("Stopped timer"), "{stopped}");
    assert!(stopped.contains("1h 30m"), "completed duration is shown: {stopped}");

    // After stopping, no timer is running.
    assert_eq!(h.ok(T1200, &["status"]).trim(), "No timer running");
    assert_eq!(h.json(T1200, &["status", "--json"]), serde_json::Value::Null);

    let entry = &h.json(T1200, &["log", "--json"])[0];
    assert_eq!(entry["running"], false);
    assert_eq!(entry["end_at"], "2026-10-27T14:30:00Z");
    assert_eq!(entry["duration_seconds"], 5400);
    assert_eq!(entry["revision"], 2, "stopping is a mutation");
}

#[test]
fn starting_a_second_timer_is_rejected_and_leaves_the_first_alone() {
    let h = Harness::new();
    h.ok(T0900, &["start", "First"]);

    let run = h.fail(T1030, &["start", "Second"]);
    assert_eq!(run.code, 4, "conflicts exit with 4");
    assert!(run.stderr.contains("already running"), "{}", run.stderr);
    assert!(run.stderr.contains("First"), "the error names the running entry: {}", run.stderr);

    // The original timer is untouched, and no second entry was created.
    let entries = h.json(T1030, &["log", "--json"]);
    assert_eq!(entries.as_array().unwrap().len(), 1);
    assert_eq!(h.json(T1030, &["status", "--json"])["description"], "First");
}

#[test]
fn stopping_without_a_running_timer_fails_clearly() {
    let h = Harness::new();
    let run = h.fail(T0900, &["stop"]);
    assert_eq!(run.code, 4);
    assert!(run.stderr.contains("no timer is running"), "{}", run.stderr);
}

#[test]
fn stop_must_be_later_than_start_and_leaves_the_timer_running() {
    let h = Harness::new();
    h.ok(T1030, &["start", "Running"]);

    let run = h.fail(T1200, &["stop", "--at", "2026-10-27T09:00"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("later than"), "{}", run.stderr);

    // Still running, still revision 1.
    let status = h.json(T1200, &["status", "--json"]);
    assert_eq!(status["running"], true);
    assert_eq!(status["revision"], 1);
}

#[test]
fn start_at_records_a_past_start_and_the_timer_keeps_running() {
    let h = Harness::new();
    h.ok(T1200, &["start", "Backdated", "--at", "2026-10-27T09:00"]);
    let status = h.json(T1200, &["status", "--json"]);
    assert_eq!(status["start_at"], "2026-10-27T13:00:00Z");
    assert_eq!(status["running"], true);
    assert_eq!(status["duration_seconds"], 3 * 3600);
}

#[test]
fn concurrent_starts_cannot_create_two_running_timers() {
    let h = Harness::new();
    // Initialize first so the race is over the timer, not over schema creation.
    h.ok(T0900, &["init"]);

    let codes: Vec<i32> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let h = &h;
                let label = format!("race {i}");
                scope.spawn(move || {
                    h.command(T0900, &["start", &label])
                        .output()
                        .expect("spawn tt")
                        .status
                        .code()
                        .unwrap_or(-1)
                })
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().unwrap()).collect()
    });

    let winners = codes.iter().filter(|c| **c == 0).count();
    assert_eq!(winners, 1, "exactly one concurrent start may win, got codes {codes:?}");

    // And the database agrees: one entry, one running timer.
    let entries = h.json(T1030, &["log", "--json"]);
    assert_eq!(entries.as_array().unwrap().len(), 1, "only one entry may exist");
    assert_eq!(h.json(T1030, &["status", "--json"])["running"], true);
}

#[test]
fn delete_requires_confirmation_when_stdin_is_not_a_terminal() {
    let h = Harness::new();
    h.ok(T0900, &["add", "Disposable", "--start", "2026-10-27T09:00", "--duration", "30m"]);
    let id = h.id_of(T1200, "Disposable");

    let run = h.fail(T1200, &["delete", &id]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("--yes"), "{}", run.stderr);

    // Still present and unmodified.
    let entries = h.json(T1200, &["log", "--json"]);
    assert_eq!(entries.as_array().unwrap().len(), 1);
    assert_eq!(entries[0]["revision"], 1);
}

#[test]
fn soft_delete_hides_the_entry_but_keeps_the_row() {
    let h = Harness::new();
    h.ok(T0900, &["add", "Disposable", "--start", "2026-10-27T09:00", "--duration", "30m"]);
    let id = h.id_of(T1200, "Disposable");

    let out = h.ok(T1200, &["delete", &id, "--yes"]);
    assert!(out.contains("Deleted entry"), "{out}");

    // Gone from normal listings, reports, and show.
    assert_eq!(h.json(T1200, &["log", "--json"]).as_array().unwrap().len(), 0);
    let report = h.json(T1200, &["report", "week", "--format", "json"]);
    assert_eq!(report["total_seconds"], 0);
    let run = h.fail(T1200, &["show", &id]);
    assert_eq!(run.code, 3);
    assert!(run.stderr.contains("deleted"), "{}", run.stderr);

    // The row survives as a tombstone, with a bumped revision.
    let shown = h.json(T1200, &["show", &id, "--json", "--include-deleted"]);
    assert_eq!(shown["id"], id.as_str());
    assert_ne!(shown["deleted_at"], serde_json::Value::Null);
    assert_eq!(shown["revision"], 2);
}

#[test]
fn deleting_a_running_timer_says_the_time_is_discarded() {
    let h = Harness::new();
    h.ok(T0900, &["start", "Mistake"]);
    let id = h.id_of(T1030, "Mistake");

    let out = h.ok(T1030, &["delete", &id, "--yes"]);
    assert!(out.contains("Discarded the running timer"), "{out}");
    assert!(out.contains("not recorded"), "{out}");

    // The timer slot is free again.
    assert_eq!(h.json(T1030, &["status", "--json"]), serde_json::Value::Null);
    h.ok(T1030, &["start", "Fresh start"]);
}
