//! Error handling, capability limits, and sync reporting.
//!
//! These are the tests that keep `tt` honest: `SPEC.md` §10 and §12 turn on
//! never reporting success that OpenTodo did not confirm.

mod support;

use support::Harness;

const NOW: &str = "2026-10-28T18:00:00Z";

#[test]
fn a_missing_ot_client_is_reported_with_installation_guidance() {
    let h = Harness::with_settings("timezone = \"UTC\"\n");
    std::fs::write(&h.config, "ot_command = \"tt-no-such-ot-client\"\ntimezone = \"UTC\"\n")
        .expect("rewrite config");

    let run = h.fail(NOW, &["status"]);
    assert_eq!(run.code, 7);
    assert!(run.stderr.contains("was not found"), "{}", run.stderr);
    assert!(run.stderr.contains("ot_command"), "{}", run.stderr);
    assert!(run.stdout.is_empty());
}

#[test]
fn an_unreachable_server_exits_four_and_says_tt_cannot_record_locally() {
    let h = Harness::new();
    h.respond_fail(
        "time-list",
        4,
        r#"{"error":{"code":"unreachable","message":"no response from server within 15s","exit":4}}"#,
        "ot: no response from server within 15s\n",
    );

    let run = h.fail(NOW, &["status"]);
    assert_eq!(run.code, 4, "tt mirrors ot's exit code");
    assert!(run.stderr.contains("no response from server"), "{}", run.stderr);
    assert!(run.stderr.contains("online-only"), "{}", run.stderr);
    assert!(run.stderr.contains("cannot record this locally"), "{}", run.stderr);
}

#[test]
fn a_read_only_token_is_explained_rather_than_swallowed() {
    let h = Harness::new();
    h.respond("time-list", "[]");
    h.respond_fail(
        "time-start",
        5,
        r#"{"error":{"code":"insufficient_scope","message":"token scope read cannot write","exit":5}}"#,
        "ot: insufficient scope\n",
    );

    let run = h.fail(NOW, &["start", "Design review", "--task", "8c2a"]);
    assert_eq!(run.code, 5);
    assert!(run.stderr.contains("token scope read cannot write"), "{}", run.stderr);
    assert!(run.stderr.contains("insufficient_scope"), "{}", run.stderr);
    assert!(run.stderr.contains("ot auth status"), "{}", run.stderr);
}

#[test]
fn an_authentication_failure_points_at_ot_login() {
    let h = Harness::new();
    h.respond_fail(
        "time-list",
        3,
        r#"{"error":{"code":"unauthorized","message":"token rejected","exit":3}}"#,
        "",
    );

    let run = h.fail(NOW, &["status"]);
    assert_eq!(run.code, 3);
    assert!(run.stderr.contains("ot login"), "{}", run.stderr);
}

#[test]
fn an_unknown_task_surfaces_opentodos_own_error_code() {
    let h = Harness::new();
    h.respond("time-list", "[]");
    h.respond_fail(
        "time-start",
        1,
        r#"{"error":{"code":"not_found","message":"no task with id zzzz","exit":1}}"#,
        "",
    );

    let run = h.fail(NOW, &["start", "x", "--task", "zzzz"]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("no task with id zzzz"), "{}", run.stderr);
    assert!(run.stderr.contains("not_found"), "{}", run.stderr);
}

#[test]
fn a_task_id_that_looks_like_a_flag_never_reaches_ot() {
    let h = Harness::new();
    h.respond("time-list", "[]");

    let run = h.fail(NOW, &["start", "x", "--task", "--server=https://evil.example"]);
    assert_eq!(run.code, 2);
    assert_eq!(h.call_count(&["time", "start"]), 0);
}

#[test]
fn human_oriented_output_is_rejected_instead_of_read_as_no_entries() {
    let h = Harness::new();
    // What `ot time list` prints without --json. Treating this as "no entries"
    // would quietly produce an empty, wrong report.
    h.respond("time-list", "Mon 09:00-09:25  25m  Design review (Work)\n");

    let run = h.fail(NOW, &["report", "week"]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("could not read the JSON"), "{}", run.stderr);
    assert!(run.stderr.contains("ot schema"), "{}", run.stderr);
}

#[test]
fn an_unexpected_ot_subcommand_would_fail_loudly() {
    // The mock answers only what a test installs; nothing is stubbed here, so
    // any ot call at all fails. This is what stops an invented subcommand from
    // passing the suite unnoticed.
    let h = Harness::new();
    let run = h.fail(NOW, &["log"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("unexpected_command"), "{}", run.stderr);
}

#[test]
fn an_unsupported_output_version_stops_before_any_data_is_read() {
    let h = Harness::new();
    h.respond("version", r#"{"output_version":2,"version":"1.4.0"}"#);

    let run = h.fail(NOW, &["status"]);
    assert_eq!(run.code, 6);
    assert!(run.stderr.contains("output_version 2"), "{}", run.stderr);
    assert_eq!(h.call_count(&["time", "list"]), 0);
}

#[test]
fn an_ot_older_than_time_tracking_is_refused() {
    let h = Harness::new();
    h.respond("version", r#"{"output_version":1,"version":"0.8.3"}"#);

    let run = h.fail(NOW, &["status"]);
    assert_eq!(run.code, 6);
    assert!(run.stderr.contains("too old"), "{}", run.stderr);
    assert!(run.stderr.contains("0.9.0"), "{}", run.stderr);
}

#[test]
fn an_unreadable_version_warns_but_does_not_block_work() {
    let h = Harness::new();
    h.respond("version", r#"{"output_version":1,"build":"unknown"}"#);
    h.respond("time-list", "[]");

    let run = h.run(NOW, &["status"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stderr.contains("could not determine the OpenTodo client version"), "{}", run.stderr);
}

#[test]
fn the_version_probe_can_be_turned_off() {
    let h = Harness::with_settings("timezone = \"UTC\"\nversion_check = false\n");
    h.respond("time-list", "[]");

    h.ok(NOW, &["status"]);
    assert_eq!(h.call_count(&["version"]), 0);
}

#[test]
fn tt_sync_reports_that_opentodo_documents_no_sync_command() {
    let h = Harness::new();

    let run = h.fail(NOW, &["sync"]);
    assert_eq!(run.code, 6, "exit 6 means the capability is not supported");
    assert!(run.stderr.contains("no documented command"), "{}", run.stderr);
    assert!(run.stderr.contains("online-only"), "{}", run.stderr);
    assert!(run.stderr.contains("vault sync"), "unrelated commands are named: {}", run.stderr);
    assert_eq!(h.call_count(&["sync"]), 0, "nothing may be invented and invoked");
    assert_eq!(h.call_count(&["vault"]), 0);
}

#[test]
fn sync_status_never_claims_synchronized_just_because_a_command_worked() {
    let h = Harness::new();
    h.respond(
        "auth-status",
        r#"{"server":"https://todo.example.com","account":"dk@example.com","scope":"full"}"#,
    );

    let out = h.ok(NOW, &["sync", "status"]);
    assert!(out.contains("Sync status: unavailable"), "{out}");
    assert!(!out.to_lowercase().contains("synchronized"), "{out}");
    assert!(out.contains("not the same as synced"), "{out}");
    assert!(out.contains("https://todo.example.com"), "{out}");
    assert!(out.contains("token scope full"), "{out}");
    assert!(out.contains("none possible"), "pending changes are explained: {out}");
    assert!(out.contains("OpenTodo app"), "the user is told where real status lives: {out}");
}

#[test]
fn sync_status_json_separates_sync_state_from_reachability() {
    let h = Harness::new();
    h.respond("auth-status", r#"{"server":"https://todo.example.com","scope":"read"}"#);

    let json = h.json(NOW, &["sync", "status", "--json"]);
    assert_eq!(json["syncStatus"], "unavailable");
    assert_eq!(json["syncStatusKnown"], false);
    assert_eq!(json["serverReachable"], true);
    assert_eq!(json["pendingLocalChanges"], 0);
    assert_eq!(json["tokenScope"], "read");
}

#[test]
fn sync_status_reports_offline_when_the_server_cannot_be_reached() {
    let h = Harness::new();
    h.respond_fail(
        "auth-status",
        4,
        r#"{"error":{"code":"unreachable","message":"no response within 15s","exit":4}}"#,
        "",
    );

    let out = h.ok(NOW, &["sync", "status"]);
    assert!(out.contains("Sync status: offline"), "{out}");

    let h2 = Harness::new();
    h2.respond_fail("auth-status", 4, r#"{"error":{"code":"unreachable","exit":4}}"#, "");
    let json = h2.json(NOW, &["sync", "status", "--json"]);
    assert_eq!(json["syncStatus"], "offline");
    assert_eq!(json["serverReachable"], false);
}

#[test]
fn sync_status_reports_a_rejected_token_as_signed_out() {
    let h = Harness::new();
    h.respond_fail(
        "auth-status",
        3,
        r#"{"error":{"code":"unauthorized","message":"token rejected","exit":3}}"#,
        "",
    );

    let json = h.json(NOW, &["sync", "status", "--json"]);
    assert_eq!(json["syncStatus"], "signed_out");
    assert_eq!(json["serverReachable"], false);
}

#[test]
fn a_config_file_holding_a_secret_is_rejected() {
    let h = Harness::new();
    std::fs::write(&h.config, "token = \"super-secret\"\n").expect("rewrite config");

    let run = h.fail(NOW, &["status"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("credentials"), "{}", run.stderr);
    assert!(!run.stderr.contains("super-secret"), "the value must not be echoed: {}", run.stderr);
}

#[test]
fn an_unknown_timezone_is_actionable() {
    let h = Harness::new();
    let run = h.fail(NOW, &["--timezone", "Mars/Olympus", "status"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("IANA"), "{}", run.stderr);
}
