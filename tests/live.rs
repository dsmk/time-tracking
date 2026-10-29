//! Opt-in tests against a **real** OpenTodo server.
//!
//! `SPEC.md` §11 says ordinary automated tests must not depend on a live
//! service, so every test here is `#[ignore]`d: `cargo test` skips them and CI
//! asserts that it did. Run them deliberately:
//!
//! ```text
//! cd testing && docker compose up -d && ./seed.sh
//! ./testing/run-live-tests.sh
//! ```
//!
//! They run single-threaded because OpenTodo allows one running timer per
//! account — that timer is shared state, and tests that start one must leave
//! none behind.
//!
//! **These tests write data.** Point them at a throwaway instance or an `ot`
//! profile for a scratch account, never a real one.
//!
//! Numbers come from `testing/.live-env`, written by `testing/seed.sh`, so the
//! assertions are exact rather than "something was returned".

use std::path::PathBuf;
use std::process::{Command, Output};

use tempfile::TempDir;

struct Live {
    _dir: TempDir,
    config: PathBuf,
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// A required value from `testing/.live-env`.
fn var(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| {
        panic!(
            "{key} is not set.\n\
             Run the live tests through testing/run-live-tests.sh, which sources \
             testing/.live-env (written by testing/seed.sh)."
        )
    })
}

fn seconds(key: &str) -> i64 {
    var(key).parse().unwrap_or_else(|e| panic!("{key} is not a number: {e}"))
}

impl Live {
    /// Build a tt config pointing at the real `ot`, in the zone the seed used.
    fn require() -> Live {
        assert_eq!(
            std::env::var("TT_LIVE").as_deref(),
            Ok("1"),
            "live tests need TT_LIVE=1; run them through testing/run-live-tests.sh"
        );
        let dir = tempfile::tempdir().expect("temp dir");
        let config = dir.path().join("config.toml");
        let ot = std::env::var("OT").unwrap_or_else(|_| "ot".to_string());
        std::fs::write(
            &config,
            format!(
                "ot_command = \"{}\"\ntimezone = \"{}\"\nweek_start = \"monday\"\n",
                ot.replace('\\', "\\\\"),
                var("TT_LIVE_TZ")
            ),
        )
        .expect("write config");
        Live { _dir: dir, config }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_tt"));
        // No TT_NOW: these run against real time on purpose.
        cmd.args(args).env("TT_CONFIG", &self.config);
        cmd
    }

    fn run(&self, args: &[&str]) -> Run {
        let out: Output = self.command(args).output().expect("run tt");
        Run {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        }
    }

    fn ok(&self, args: &[&str]) -> String {
        let run = self.run(args);
        assert_eq!(
            run.code,
            0,
            "`tt {}` failed\nstdout: {}\nstderr: {}",
            args.join(" "),
            run.stdout,
            run.stderr
        );
        run.stdout
    }

    fn json(&self, args: &[&str]) -> serde_json::Value {
        let stdout = self.ok(args);
        serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("`tt {}` printed invalid JSON ({e}): {stdout}", args.join(" ")))
    }

    /// Leave no timer running, whatever a test did.
    fn ensure_stopped(&self) {
        let status = self.json(&["status", "--json"]);
        if status["running"] == serde_json::Value::Bool(true) {
            let run = self.run(&["stop"]);
            assert!(run.code == 0 || run.code == 8, "could not clean up: {}", run.stderr);
        }
    }
}

/// Closes compat §11 item 1: whatever `ot time list --json` really prints, tt
/// parses it into entries with the fields reports depend on.
#[test]
#[ignore = "requires a live OpenTodo server; see testing/README.md"]
fn real_ot_json_parses_into_usable_entries() {
    let live = Live::require();
    let entries = live.json(&[
        "log",
        "--from",
        &var("TT_LIVE_WEEK_FROM"),
        "--to",
        &var("TT_LIVE_WEEK_TO"),
        "--format",
        "json",
    ]);
    let items = entries.as_array().expect("tt log --format json returns an array");
    assert_eq!(items.len(), 4, "seed.sh created four entries: {items:#?}");

    for item in items {
        assert!(item["id"].is_string(), "every entry needs an id: {item}");
        assert!(item["startedAt"].is_string(), "every entry needs startedAt: {item}");
        assert!(item["seconds"].as_i64().unwrap_or(0) > 0, "durations must be real: {item}");
        assert_eq!(item["running"], false);
    }
}

/// Closes compat §11 item 6 for the data path: the totals tt computes match
/// the entries that were actually created.
#[test]
#[ignore = "requires a live OpenTodo server; see testing/README.md"]
fn report_totals_match_the_seeded_entries() {
    let live = Live::require();
    let report = live.json(&[
        "report",
        "range",
        "--from",
        &var("TT_LIVE_WEEK_FROM"),
        "--to",
        &var("TT_LIVE_WEEK_TO"),
        "--format",
        "json",
    ]);

    assert_eq!(report["totalSeconds"], seconds("TT_LIVE_TOTAL_SECONDS"));
    assert_eq!(report["incomplete"], false, "nothing should be running or truncated");
    assert_eq!(report["untaskedSeconds"], 0);
    assert_eq!(report["timezone"], var("TT_LIVE_TZ"));

    let days = report["days"].as_array().expect("days");
    assert_eq!(days.len(), 7);
    let summed: i64 = days.iter().map(|d| d["seconds"].as_i64().unwrap()).sum();
    assert_eq!(summed, seconds("TT_LIVE_TOTAL_SECONDS"), "daily totals must sum to the total");

    let on = |date: &str| -> i64 {
        days.iter()
            .find(|d| d["date"] == date)
            .unwrap_or_else(|| panic!("no row for {date}"))["seconds"]
            .as_i64()
            .unwrap()
    };
    assert_eq!(on(&var("TT_LIVE_MON")), seconds("TT_LIVE_MON_SECONDS"));
    assert_eq!(on(&var("TT_LIVE_TUE")), seconds("TT_LIVE_TUE_SECONDS"));
    assert_eq!(on(&var("TT_LIVE_WED")), seconds("TT_LIVE_WED_SECONDS"));
    assert_eq!(on(&var("TT_LIVE_THU")), seconds("TT_LIVE_THU_SECONDS"));

    let task = report["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .find(|t| t["taskId"] == var("TT_LIVE_TASK"))
        .expect("the seeded task appears in the report");
    assert_eq!(task["seconds"], seconds("TT_LIVE_TASK_SECONDS"), "two entries merge into one row");
    assert_eq!(task["entries"], 2);
}

/// Closes compat §11 item 5: a 23:00 entry belongs to that local day, not the
/// next one. If `ot` interpreted `--from`/`--to` in a distant zone, the
/// one-day widening in `Period::query_window` would show up here.
#[test]
#[ignore = "requires a live OpenTodo server; see testing/README.md"]
fn a_late_evening_entry_lands_on_the_right_local_day() {
    let live = Live::require();
    let wed = var("TT_LIVE_WED");
    let thu = var("TT_LIVE_THU");

    let on_wed = live.json(&["log", "--from", &wed, "--to", &wed, "--format", "json"]);
    let wed_items = on_wed.as_array().expect("array");
    assert_eq!(wed_items.len(), 1, "the 23:00 entry is Wednesday's: {wed_items:#?}");
    assert!(
        wed_items[0]["startedLocal"].as_str().unwrap().starts_with(&format!("{wed}T23:")),
        "local start: {}",
        wed_items[0]["startedLocal"]
    );

    let on_thu = live.json(&["log", "--from", &thu, "--to", &thu, "--format", "json"]);
    let thu_items = on_thu.as_array().expect("array");
    assert_eq!(thu_items.len(), 1, "Thursday has only its own entry");
    assert_ne!(thu_items[0]["id"], wed_items[0]["id"]);
}

/// Closes compat §11 items 2 and 6 for writes: start, observe, stop — and
/// confirm OpenTodo kept it.
#[test]
#[ignore = "requires a live OpenTodo server; see testing/README.md"]
fn start_status_stop_round_trip() {
    let live = Live::require();
    live.ensure_stopped();

    let task = var("TT_LIVE_TASK");
    let note = format!("tt live test {}", std::process::id());

    let started = live.json(&["start", &note, "--task", &task, "--json"]);
    let entry_id = started["id"].as_str().expect("start returns an entry id").to_string();
    assert_eq!(started["taskId"], task);
    assert_eq!(started["running"], true);
    assert!(started["endedAt"].is_null(), "a running entry has no end");

    let status = live.json(&["status", "--json"]);
    assert_eq!(status["running"], true);
    assert_eq!(status["entry"]["id"], entry_id, "status sees the entry we just started");
    assert_eq!(status["entry"]["note"], note);

    std::thread::sleep(std::time::Duration::from_secs(2));

    let stopped = live.json(&["stop", "--json"]);
    assert_eq!(stopped["id"], entry_id);
    assert_eq!(stopped["running"], false);
    assert!(!stopped["endedAt"].is_null(), "a stopped entry has an end");
    assert!(stopped["seconds"].as_i64().unwrap() >= 1, "elapsed: {}", stopped["seconds"]);

    // And OpenTodo really kept it.
    let today = live.json(&["log", "--today", "--format", "json"]);
    assert!(
        today.as_array().unwrap().iter().any(|e| e["id"] == entry_id.as_str()),
        "the stopped entry must be readable back: {today:#?}"
    );

    assert_eq!(live.json(&["status", "--json"])["running"], false);
}

/// Closes compat §11 item 7: tt must not send a write that cannot succeed.
#[test]
#[ignore = "requires a live OpenTodo server; see testing/README.md"]
fn stop_without_a_running_timer_exits_eight() {
    let live = Live::require();
    live.ensure_stopped();

    let run = live.run(&["stop"]);
    assert_eq!(run.code, 8, "stdout: {}\nstderr: {}", run.stdout, run.stderr);
    assert!(run.stderr.contains("no timer is running"), "{}", run.stderr);
    assert!(run.stdout.is_empty());
}

/// OpenTodo ends the current entry when another task is started. tt refuses to
/// let that happen silently — verified against the real server, not a mock.
#[test]
#[ignore = "requires a live OpenTodo server; see testing/README.md"]
fn a_second_start_is_refused_without_switch() {
    let live = Live::require();
    live.ensure_stopped();

    let first = live.json(&["start", "first", "--task", &var("TT_LIVE_TASK"), "--json"]);
    let first_id = first["id"].as_str().expect("id").to_string();

    let run = live.run(&["start", "second", "--task", &var("TT_LIVE_PLANNING_TASK")]);
    assert_eq!(run.code, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.contains("already running"), "{}", run.stderr);

    // The original entry must be untouched.
    let status = live.json(&["status", "--json"]);
    assert_eq!(status["entry"]["id"], first_id, "the refused start changed nothing");

    live.ensure_stopped();
}

/// Confirms the documented online-only behaviour: no queueing, no local
/// record, and the exit code that says so (`SPEC.md` §7).
#[test]
#[ignore = "requires a live OpenTodo server; see testing/README.md"]
fn an_unreachable_server_fails_without_recording_anything() {
    let live = Live::require();
    // Port 1 is reserved and nothing listens there.
    let mut cmd = live.command(&["start", "offline attempt", "--task", &var("TT_LIVE_TASK")]);
    cmd.env("OPENTODO_URL", "http://127.0.0.1:1");
    let out = cmd.output().expect("run tt");
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        code == 4 || code == 3,
        "expected unreachable (4) or a config error (3), got {code}: {stderr}"
    );
    assert!(out.stdout.is_empty(), "nothing may be reported as recorded");

    // Nothing was queued or invented: the real server has no such entry.
    let today = live.json(&["log", "--today", "--format", "json"]);
    assert!(
        !today
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["note"] == "offline attempt"),
        "an unreachable start must leave no entry behind: {today:#?}"
    );
}

/// `tt sync status` against a real server still refuses to claim "synchronized"
/// (`SPEC.md` §6.7).
#[test]
#[ignore = "requires a live OpenTodo server; see testing/README.md"]
fn sync_status_reports_unavailable_against_a_reachable_server() {
    let live = Live::require();
    let json = live.json(&["sync", "status", "--json"]);
    assert_eq!(json["syncStatus"], "unavailable");
    assert_eq!(json["syncStatusKnown"], false);
    assert_eq!(json["serverReachable"], true);
    assert_eq!(json["pendingLocalChanges"], 0);

    let run = live.run(&["sync"]);
    assert_eq!(run.code, 6, "there is no documented sync command to call");
}

/// Only runs when a read-scoped token is provided; otherwise it reports why it
/// did nothing rather than passing silently.
#[test]
#[ignore = "requires a live OpenTodo server; see testing/README.md"]
fn a_read_only_token_cannot_start_a_timer() {
    let live = Live::require();
    let Ok(token) = std::env::var("TT_LIVE_READ_TOKEN") else {
        eprintln!(
            "skipped: set TT_LIVE_READ_TOKEN to a read-scoped token \
             (`ot token create tt-read --scope read`) to exercise exit 5"
        );
        return;
    };

    let mut cmd = live.command(&["start", "read-only attempt", "--task", &var("TT_LIVE_TASK")]);
    cmd.env("OPENTODO_TOKEN", token);
    let out = cmd.output().expect("run tt");
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert_eq!(out.status.code(), Some(5), "stderr: {stderr}");
    assert!(stderr.contains("scope") || stderr.contains("permitted"), "{stderr}");
    assert!(!stderr.contains(&std::env::var("TT_LIVE_READ_TOKEN").unwrap()), "the token leaked");
}
