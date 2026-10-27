mod support;

use support::Harness;

const NOW: &str = "2026-10-28T20:00:00Z"; // Wednesday 16:00 America/New_York

/// Monday 1h30m (alpha), Tuesday 1h (alpha) + 30m (platform).
fn seeded() -> Harness {
    let h = Harness::new();
    h.ok(NOW, &["add", "Code review", "--start", "2026-10-26T09:00", "--end", "2026-10-26T10:30", "--project", "alpha", "--tag", "review"]);
    h.ok(NOW, &["add", "Design review", "--start", "2026-10-27T09:00", "--duration", "1h", "--project", "alpha"]);
    h.ok(NOW, &["add", "Investigate alert", "--start", "2026-10-27T13:00", "--duration", "30m", "--project", "platform", "--tag", "incident"]);
    h
}

#[test]
fn weekly_report_covers_the_current_week_in_the_configured_timezone() {
    let h = seeded();
    let report = h.json(NOW, &["report", "week", "--format", "json"]);

    assert_eq!(report["kind"], "week");
    assert_eq!(report["timezone"], "America/New_York");
    assert_eq!(report["first_day"], "2026-10-26");
    assert_eq!(report["last_day"], "2026-11-01");
    // Local midnight Monday, expressed in UTC.
    assert_eq!(report["period_start"], "2026-10-26T04:00:00Z");
    assert_eq!(report["total_seconds"], 5400 + 3600 + 1800);
    assert_eq!(report["entry_count"], 3);

    assert_eq!(
        report["by_project"],
        serde_json::json!([
            {"name": "alpha", "seconds": 9000},
            {"name": "platform", "seconds": 1800},
        ])
    );
    assert_eq!(report["by_day"].as_array().unwrap().len(), 7);
    assert_eq!(report["by_day"][0]["date"], "2026-10-26");
    assert_eq!(report["by_day"][0]["seconds"], 5400);
    assert_eq!(report["by_day"][1]["seconds"], 5400);
    assert_eq!(report["by_day"][6]["seconds"], 0);
    assert_eq!(
        report["by_tag"],
        serde_json::json!([
            {"name": "review", "seconds": 5400},
            {"name": "incident", "seconds": 1800},
        ])
    );
    assert!(report["overlaps"].as_array().unwrap().is_empty());
}

#[test]
fn week_start_configuration_moves_the_period() {
    let h = Harness::with_config("timezone = \"America/New_York\"\nweek_start = \"sunday\"\n");
    h.ok(NOW, &["add", "Sunday work", "--start", "2026-10-25T09:00", "--duration", "1h"]);
    h.ok(NOW, &["add", "Tuesday work", "--start", "2026-10-27T09:00", "--duration", "1h"]);

    let sunday = h.json(NOW, &["report", "week", "--format", "json"]);
    assert_eq!(sunday["first_day"], "2026-10-25");
    assert_eq!(sunday["last_day"], "2026-10-31");
    assert_eq!(sunday["total_seconds"], 7200, "a Sunday week includes both entries");

    // The same data under a Monday week start excludes the Sunday entry.
    std::fs::write(&h.config, "timezone = \"America/New_York\"\nweek_start = \"monday\"\n").unwrap();
    let monday = h.json(NOW, &["report", "week", "--format", "json"]);
    assert_eq!(monday["first_day"], "2026-10-26");
    assert_eq!(monday["total_seconds"], 3600);
}

#[test]
fn week_option_selects_the_week_containing_a_date() {
    let h = seeded();
    let previous = h.json(NOW, &["report", "week", "--week", "2026-10-20", "--format", "json"]);
    assert_eq!(previous["first_day"], "2026-10-19");
    assert_eq!(previous["total_seconds"], 0);

    let current = h.json(NOW, &["report", "week", "--week", "2026-11-01", "--format", "json"]);
    assert_eq!(current["first_day"], "2026-10-26", "Sunday belongs to the Monday week");
    assert_eq!(current["total_seconds"], 10800);
}

#[test]
fn reports_can_be_filtered_by_project_and_tag() {
    let h = seeded();
    let alpha = h.json(NOW, &["report", "week", "--project", "alpha", "--format", "json"]);
    assert_eq!(alpha["total_seconds"], 9000);
    assert_eq!(alpha["by_project"].as_array().unwrap().len(), 1);

    let tagged = h.json(NOW, &["report", "week", "--tag", "incident", "--format", "json"]);
    assert_eq!(tagged["total_seconds"], 1800);
}

#[test]
fn range_report_is_inclusive_and_validates_its_dates() {
    let h = seeded();
    let one_day =
        h.json(NOW, &["report", "range", "--from", "2026-10-27", "--to", "2026-10-27", "--format", "json"]);
    assert_eq!(one_day["kind"], "range");
    assert_eq!(one_day["total_seconds"], 5400);
    assert_eq!(one_day["by_day"].as_array().unwrap().len(), 1);

    let whole = h.json(NOW, &["report", "range", "--from", "2026-10-01", "--to", "2026-10-31", "--format", "json"]);
    assert_eq!(whole["total_seconds"], 10800);
    assert_eq!(whole["by_day"].as_array().unwrap().len(), 31);

    let run = h.fail(NOW, &["report", "range", "--from", "2026-10-31", "--to", "2026-10-01"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("later than"), "{}", run.stderr);

    let run = h.fail(NOW, &["report", "range", "--from", "nonsense", "--to", "2026-10-01"]);
    assert_eq!(run.code, 2);
}

#[test]
fn entries_crossing_the_period_boundary_contribute_only_their_inside_portion() {
    let h = Harness::new();
    // 23:00 Sunday to 01:00 Monday, local: one hour falls in the Monday week.
    h.ok(NOW, &["add", "Across the boundary", "--start", "2026-10-25T23:00", "--end", "2026-10-26T01:00"]);
    let report = h.json(NOW, &["report", "week", "--format", "json"]);
    assert_eq!(report["total_seconds"], 3600);
    assert_eq!(report["by_day"][0]["seconds"], 3600);
}

#[test]
fn a_running_entry_contributes_through_now() {
    let h = Harness::new();
    h.ok("2026-10-28T19:00:00Z", &["start", "Still going", "--project", "alpha"]);
    let report = h.json(NOW, &["report", "week", "--format", "json"]);
    assert_eq!(report["total_seconds"], 3600, "one hour elapsed so far");
    assert_eq!(report["by_project"][0]["name"], "alpha");
}

#[test]
fn a_week_with_a_dst_change_counts_actual_elapsed_time() {
    let h = Harness::new();
    // Local midnight to midnight on the fall-back day is 25 elapsed hours.
    h.ok("2026-11-02T20:00:00Z", &["add", "All of Sunday", "--start", "2026-11-01T00:00", "--end", "2026-11-02T00:00"]);
    let report = h.json("2026-11-02T20:00:00Z", &["report", "week", "--week", "2026-11-01", "--format", "json"]);
    assert_eq!(report["total_seconds"], 25 * 3600);
    let sunday = report["by_day"].as_array().unwrap().last().unwrap();
    assert_eq!(sunday["date"], "2026-11-01");
    assert_eq!(sunday["seconds"], 25 * 3600);
}

#[test]
fn text_reports_round_only_for_display() {
    let h = Harness::new();
    // 89 seconds: displays as 0h 01m but aggregates exactly.
    h.ok(NOW, &["add", "Short", "--start", "2026-10-27T09:00", "--duration", "89s"]);
    let text = h.ok(NOW, &["report", "week"]);
    assert!(text.contains("Weekly report"), "{text}");
    assert!(text.contains("2026-10-26 through 2026-11-01"), "{text}");
    assert!(text.contains("Total: 0h 01m"), "{text}");
    assert!(text.contains("By project"), "{text}");
    assert!(text.contains("Unassigned"), "{text}");
    assert!(text.contains("By day"), "{text}");

    assert_eq!(h.json(NOW, &["report", "week", "--format", "json"])["total_seconds"], 89);
}

#[test]
fn second_precision_configuration_changes_display_only() {
    let h = Harness::with_config(
        "timezone = \"America/New_York\"\nduration_precision = \"second\"\n",
    );
    h.ok(NOW, &["add", "Short", "--start", "2026-10-27T09:00", "--duration", "89s"]);
    let text = h.ok(NOW, &["report", "week"]);
    assert!(text.contains("Total: 0h 01m 29s"), "{text}");
    assert!(h.ok(NOW, &["log"]).contains("09:00:00"), "times gain seconds too");
}

#[test]
fn csv_output_is_importable_and_covers_every_group() {
    let h = seeded();
    let csv = h.ok(NOW, &["report", "week", "--format", "csv"]);
    let mut lines = csv.lines();
    assert_eq!(
        lines.next().unwrap(),
        "period_start,period_end,group_type,group_name,seconds"
    );
    assert_eq!(lines.next().unwrap(), "2026-10-26,2026-11-01,total,total,10800");

    let rows: Vec<&str> = csv.lines().collect();
    assert!(rows.contains(&"2026-10-26,2026-11-01,project,alpha,9000"), "{csv}");
    assert!(rows.contains(&"2026-10-26,2026-11-01,project,platform,1800"), "{csv}");
    assert!(rows.contains(&"2026-10-26,2026-11-01,day,2026-10-26,5400"), "{csv}");
    assert!(rows.contains(&"2026-10-26,2026-11-01,tag,incident,1800"), "{csv}");
    // 1 header + 1 total + 2 projects + 7 days + 2 tags
    assert_eq!(rows.len(), 13, "{csv}");
}

#[test]
fn csv_quotes_values_containing_separators() {
    let h = Harness::new();
    h.ok(NOW, &["add", "Work", "--start", "2026-10-27T09:00", "--duration", "1h", "--project", "a,b"]);
    let csv = h.ok(NOW, &["report", "week", "--format", "csv"]);
    assert!(csv.contains("project,\"a,b\",3600"), "{csv}");
}

#[test]
fn json_output_is_machine_readable_and_free_of_decoration() {
    let h = seeded();
    // Every JSON command must emit nothing but the JSON document.
    for args in [
        vec!["log", "--json"],
        vec!["status", "--json"],
        vec!["report", "week", "--format", "json"],
    ] {
        let run = h.run(NOW, &args);
        assert_eq!(run.code, 0);
        serde_json::from_str::<serde_json::Value>(&run.stdout)
            .unwrap_or_else(|e| panic!("`tt {}` emitted invalid JSON ({e}): {}", args.join(" "), run.stdout));
    }

    let entry = &h.json(NOW, &["log", "--json"])[0];
    // Full UUID, not the short id.
    assert_eq!(entry["id"].as_str().unwrap().len(), 36);
    // UTC timestamps carry Z; local ones carry an explicit offset.
    assert!(entry["start_at"].as_str().unwrap().ends_with('Z'));
    assert!(entry["start_local"].as_str().unwrap().contains("-04:00"));
    assert!(entry["duration_seconds"].is_i64());
}

#[test]
fn json_array_order_is_deterministic() {
    let h = seeded();
    let first = h.ok(NOW, &["log", "--json"]);
    for _ in 0..3 {
        assert_eq!(h.ok(NOW, &["log", "--json"]), first);
    }
    let report = h.ok(NOW, &["report", "week", "--format", "json"]);
    for _ in 0..3 {
        assert_eq!(h.ok(NOW, &["report", "week", "--format", "json"]), report);
    }
}

#[test]
fn config_show_reports_effective_settings_and_paths() {
    let h = seeded();
    let out = h.ok(NOW, &["config", "show"]);
    assert!(out.contains("timezone            = \"America/New_York\""), "{out}");
    assert!(out.contains("week_start          = \"monday\""), "{out}");
    assert!(out.contains("duration_precision  = \"minute\""), "{out}");
    assert!(out.contains(&h.config.display().to_string()), "{out}");
    assert!(out.contains(&h.db.display().to_string()), "{out}");
}

#[test]
fn config_set_validates_and_persists() {
    let h = Harness::with_config("timezone = \"America/New_York\"\n");

    let run = h.fail(NOW, &["config", "set", "timezone", "Mars/Olympus"]);
    assert_eq!(run.code, 5);
    // The rejected value was never written.
    assert!(std::fs::read_to_string(&h.config).unwrap().contains("America/New_York"));

    h.ok(NOW, &["config", "set", "week_start", "sunday"]);
    h.ok(NOW, &["config", "set", "timezone", "UTC"]);
    let out = h.ok(NOW, &["config", "show"]);
    assert!(out.contains("week_start          = \"sunday\""), "{out}");
    assert!(out.contains("timezone            = \"UTC\""), "{out}");

    let run = h.fail(NOW, &["config", "set", "nonsense", "1"]);
    assert_eq!(run.code, 5);
}

#[test]
fn invalid_configuration_names_the_setting_and_the_file() {
    let h = Harness::with_config("timezone = \"Mars/Olympus\"\n");
    let run = h.fail(NOW, &["log"]);
    assert_eq!(run.code, 5);
    assert!(run.stderr.contains("timezone"), "{}", run.stderr);
    assert!(run.stderr.contains("config.toml"), "{}", run.stderr);
}

#[test]
fn unknown_configuration_keys_warn_on_stderr_without_failing() {
    let h = Harness::with_config("timezone = \"UTC\"\nnonsense = 1\n");
    let run = h.run(NOW, &["log"]);
    assert_eq!(run.code, 0);
    assert!(run.stderr.contains("nonsense"), "{}", run.stderr);
    // The warning must not contaminate JSON output on stdout.
    let run = h.run(NOW, &["log", "--json"]);
    serde_json::from_str::<serde_json::Value>(&run.stdout).expect("stdout is pure JSON");
}

#[test]
fn the_db_flag_overrides_the_environment_variable() {
    let h = seeded();
    let other = h.dir.path().join("other.db");
    // A different database starts empty.
    let out = h.ok(NOW, &["log", "--db", other.to_str().unwrap(), "--json"]);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&out).unwrap().as_array().unwrap().len(), 0);
    // While TT_DB still has the seeded entries.
    assert_eq!(h.json(NOW, &["log", "--json"]).as_array().unwrap().len(), 3);
}

#[test]
fn an_unwritable_database_path_reports_a_useful_error() {
    let h = Harness::new();
    let run = h.fail(NOW, &["log", "--db", "/proc/version/tt.db"]);
    assert_eq!(run.code, 6, "database errors exit with 6");
    assert!(
        run.stderr.contains("database") || run.stderr.contains("cannot"),
        "{}",
        run.stderr
    );
}
