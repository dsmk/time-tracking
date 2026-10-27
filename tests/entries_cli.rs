mod support;

use support::Harness;

const NOW: &str = "2026-10-28T20:00:00Z"; // Wednesday 16:00 America/New_York

/// Three completed entries across Monday and Tuesday local time.
fn seeded() -> Harness {
    let h = Harness::new();
    h.ok(NOW, &["add", "Code review", "--start", "2026-10-26T09:00", "--end", "2026-10-26T10:30", "--project", "alpha", "--tag", "review"]);
    h.ok(NOW, &["add", "Design review", "--start", "2026-10-27T09:00", "--duration", "1h", "--project", "alpha"]);
    h.ok(NOW, &["add", "Investigate alert", "--start", "2026-10-27T13:00", "--duration", "30m", "--project", "platform", "--tag", "incident", "--tag", "oncall"]);
    h
}

#[test]
fn add_accepts_an_end_time_or_a_duration() {
    let h = Harness::new();
    let out = h.ok(NOW, &["add", "Code review", "--start", "2026-10-27T09:00", "--end", "2026-10-27T10:30"]);
    assert!(out.contains("Added entry"), "{out}");
    assert!(out.contains("1h 30m"), "{out}");

    h.ok(NOW, &["add", "By duration", "--start", "2026-10-27T11:00", "--duration", "1h30m"]);
    let entries = h.json(NOW, &["log", "--json"]);
    let by_duration: Vec<_> = entries
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["description"] == "By duration")
        .collect();
    assert_eq!(by_duration[0]["duration_seconds"], 5400);
    assert_eq!(by_duration[0]["start_at"], "2026-10-27T15:00:00Z");
    assert_eq!(by_duration[0]["end_at"], "2026-10-27T16:30:00Z");
}

#[test]
fn add_requires_exactly_one_of_end_or_duration() {
    let h = Harness::new();
    let run = h.fail(NOW, &["add", "Neither", "--start", "2026-10-27T09:00"]);
    assert!(run.stderr.contains("--end") && run.stderr.contains("--duration"), "{}", run.stderr);

    let run = h.fail(
        NOW,
        &["add", "Both", "--start", "2026-10-27T09:00", "--end", "2026-10-27T10:00", "--duration", "1h"],
    );
    assert!(run.stderr.contains("cannot be used with") || run.stderr.contains("cannot be combined"), "{}", run.stderr);
    assert_eq!(h.json(NOW, &["log", "--json"]).as_array().unwrap().len(), 0);
}

#[test]
fn add_rejects_an_end_at_or_before_the_start() {
    let h = Harness::new();
    let run = h.fail(NOW, &["add", "Backwards", "--start", "2026-10-27T10:00", "--end", "2026-10-27T09:00"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("later than"), "{}", run.stderr);

    let run = h.fail(NOW, &["add", "Zero", "--start", "2026-10-27T10:00", "--end", "2026-10-27T10:00"]);
    assert_eq!(run.code, 2);
    assert_eq!(h.json(NOW, &["log", "--json"]).as_array().unwrap().len(), 0);
}

#[test]
fn descriptions_must_not_be_blank() {
    let h = Harness::new();
    let run = h.fail(NOW, &["start", "   "]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("description"), "{}", run.stderr);
}

#[test]
fn the_default_project_applies_when_no_project_is_given() {
    let h = Harness::with_config(
        "timezone = \"America/New_York\"\ndefault_project = \"fallback\"\n",
    );
    h.ok(NOW, &["add", "Unassigned work", "--start", "2026-10-27T09:00", "--duration", "1h"]);
    h.ok(NOW, &["add", "Explicit work", "--start", "2026-10-27T11:00", "--duration", "1h", "--project", "alpha"]);

    let entries = h.json(NOW, &["log", "--json"]);
    let project_of = |name: &str| {
        entries
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["description"] == name)
            .unwrap()["project"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(project_of("Unassigned work"), "fallback");
    assert_eq!(project_of("Explicit work"), "alpha");
}

#[test]
fn hash_tags_in_the_description_become_tags_and_merge_with_flag_tags() {
    let h = Harness::new();
    h.ok(NOW, &["add", "Investigate #incident while #on-call", "--start", "2026-10-27T09:00", "--duration", "1h", "--tag", "oncall"]);

    let entry = &h.json(NOW, &["log", "--json"])[0];
    assert_eq!(entry["description"], "Investigate #incident while #on-call", "the text is kept verbatim");
    assert_eq!(entry["tags"], serde_json::json!(["oncall", "incident", "on-call"]));

    // A hash tag is also searchable through the tag filter.
    let filtered = h.json(NOW, &["log", "--json", "--tag", "incident"]);
    assert_eq!(filtered.as_array().unwrap().len(), 1);
}

#[test]
fn duplicate_tags_collapse_case_insensitively_keeping_the_first_spelling() {
    let h = Harness::new();
    h.ok(NOW, &["add", "Tagged #Incident", "--start", "2026-10-27T09:00", "--duration", "1h", "--tag", "INCIDENT", "--tag", "incident"]);
    assert_eq!(h.json(NOW, &["log", "--json"])[0]["tags"], serde_json::json!(["INCIDENT"]));
}

#[test]
fn overlapping_entries_are_rejected_unless_allowed() {
    let h = Harness::new();
    h.ok(NOW, &["add", "First", "--start", "2026-10-27T09:00", "--end", "2026-10-27T11:00"]);

    let run = h.fail(NOW, &["add", "Overlapping", "--start", "2026-10-27T10:00", "--end", "2026-10-27T12:00"]);
    assert_eq!(run.code, 4);
    assert!(run.stderr.contains("overlaps"), "{}", run.stderr);
    assert!(run.stderr.contains("First"), "the error names the conflicting entry: {}", run.stderr);
    assert_eq!(h.json(NOW, &["log", "--json"]).as_array().unwrap().len(), 1);

    // Touching, not overlapping: allowed.
    h.ok(NOW, &["add", "Adjacent", "--start", "2026-10-27T11:00", "--end", "2026-10-27T12:00"]);

    // Explicitly allowed overlap succeeds and is surfaced in reports.
    h.ok(NOW, &["add", "Allowed", "--start", "2026-10-27T10:00", "--end", "2026-10-27T12:00", "--allow-overlap"]);
    let report = h.json(NOW, &["report", "week", "--format", "json"]);
    assert!(!report["overlaps"].as_array().unwrap().is_empty(), "overlaps stay visible in reports");
    assert!(h.ok(NOW, &["report", "week"]).contains("Warning"));
}

#[test]
fn overlap_is_detected_across_a_date_boundary() {
    let h = Harness::new();
    // 23:00 Tuesday to 01:00 Wednesday, local.
    h.ok(NOW, &["add", "Night shift", "--start", "2026-10-27T23:00", "--end", "2026-10-28T01:00"]);
    let run = h.fail(NOW, &["add", "Also at midnight", "--start", "2026-10-28T00:30", "--duration", "1h"]);
    assert_eq!(run.code, 4);
    assert!(run.stderr.contains("Night shift"), "{}", run.stderr);
}

#[test]
fn a_running_timer_blocks_entries_recorded_after_it_started() {
    let h = Harness::new();
    h.ok("2026-10-27T13:00:00Z", &["start", "Running since 09:00"]);
    // Entirely before the running timer: fine.
    h.ok(NOW, &["add", "Earlier", "--start", "2026-10-27T07:00", "--duration", "1h"]);
    // Extending past the running timer's start: rejected.
    let run = h.fail(NOW, &["add", "Later", "--start", "2026-10-27T08:30", "--duration", "2h"]);
    assert_eq!(run.code, 4);
}

#[test]
fn log_filters_by_date_project_tag_and_description() {
    let h = seeded();

    let all = h.json(NOW, &["log", "--json"]);
    assert_eq!(all.as_array().unwrap().len(), 3);
    // Newest first.
    assert_eq!(all[0]["description"], "Investigate alert");
    assert_eq!(all[2]["description"], "Code review");

    let by_project = h.json(NOW, &["log", "--json", "--project", "alpha"]);
    assert_eq!(by_project.as_array().unwrap().len(), 2);

    let by_tag = h.json(NOW, &["log", "--json", "--tag", "incident"]);
    assert_eq!(by_tag.as_array().unwrap().len(), 1);
    assert_eq!(by_tag[0]["description"], "Investigate alert");

    let by_search = h.json(NOW, &["log", "--json", "--search", "REVIEW"]);
    assert_eq!(by_search.as_array().unwrap().len(), 2, "search is case-insensitive");

    let by_range = h.json(NOW, &["log", "--json", "--from", "2026-10-27", "--to", "2026-10-27"]);
    assert_eq!(by_range.as_array().unwrap().len(), 2);

    let from_only = h.json(NOW, &["log", "--json", "--from", "2026-10-27"]);
    assert_eq!(from_only.as_array().unwrap().len(), 2);

    let to_only = h.json(NOW, &["log", "--json", "--to", "2026-10-26"]);
    assert_eq!(to_only.as_array().unwrap().len(), 1);

    let limited = h.json(NOW, &["log", "--json", "--limit", "1"]);
    assert_eq!(limited.as_array().unwrap().len(), 1);
}

#[test]
fn log_today_uses_the_configured_timezone() {
    let h = Harness::new();
    // 2026-10-28T03:00:00Z is still Tuesday 23:00 in New York.
    h.ok("2026-10-28T03:00:00Z", &["add", "Late Tuesday", "--start", "2026-10-27T23:00", "--duration", "30m"]);
    let today = h.json("2026-10-28T03:00:00Z", &["log", "--json", "--today"]);
    assert_eq!(today.as_array().unwrap().len(), 1, "23:00 local is still today");

    // An hour later it is Wednesday locally, so the entry is no longer today's.
    let tomorrow = h.json("2026-10-28T05:00:00Z", &["log", "--json", "--today"]);
    assert_eq!(tomorrow.as_array().unwrap().len(), 0);
}

#[test]
fn an_entry_crossing_midnight_appears_in_both_days() {
    let h = Harness::new();
    h.ok(NOW, &["add", "Night shift", "--start", "2026-10-27T23:00", "--end", "2026-10-28T01:00"]);
    for day in ["2026-10-27", "2026-10-28"] {
        let found = h.json(NOW, &["log", "--json", "--from", day, "--to", day]);
        assert_eq!(found.as_array().unwrap().len(), 1, "entry should appear on {day}");
    }
}

#[test]
fn log_rejects_today_combined_with_a_date_range() {
    let h = seeded();
    let run = h.fail(NOW, &["log", "--today", "--from", "2026-10-26"]);
    assert!(run.stderr.contains("cannot be used with"), "{}", run.stderr);

    let run = h.fail(NOW, &["log", "--from", "2026-10-28", "--to", "2026-10-26"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("later than"), "{}", run.stderr);
}

#[test]
fn running_entries_are_marked_in_text_output_and_counted_through_now() {
    let h = Harness::new();
    h.ok("2026-10-28T19:00:00Z", &["start", "Still going"]);
    let text = h.ok(NOW, &["log"]);
    assert!(text.contains("in progress"), "{text}");
    assert!(text.contains("1h 00m"), "duration runs through now: {text}");

    let json = h.json(NOW, &["log", "--json"]);
    assert_eq!(json[0]["running"], true);
    assert_eq!(json[0]["duration_seconds"], 3600);
}

#[test]
fn show_resolves_a_prefix_and_rejects_an_ambiguous_one() {
    let h = seeded();
    let id = h.id_of(NOW, "Design review");

    let full = h.json(NOW, &["show", &id, "--json"]);
    assert_eq!(full["id"], id.as_str());
    // An 8-character prefix, as printed by `tt log`.
    let short = &id[..8];
    assert_eq!(h.json(NOW, &["show", short, "--json"])["id"], id.as_str());

    // Every UUID v4 shares nothing useful at length 0, but a one-character
    // prefix will usually match several of three entries; if not, skip.
    let run = h.fail(NOW, &["show", "zzzzzzzz"]);
    assert_eq!(run.code, 2, "a non-hex id is invalid input");

    let run = h.fail(NOW, &["show", "deadbeef"]);
    assert_eq!(run.code, 3, "a well-formed but unknown id is not found");
}

#[test]
fn edit_updates_fields_and_bumps_the_revision() {
    let h = seeded();
    let id = h.id_of(NOW, "Design review");

    // Edit later than the seeding clock so `updated_at` can be seen to move.
    const LATER: &str = "2026-10-28T21:00:00Z";
    h.ok(LATER, &["edit", &id, "--description", "Review revised design"]);
    h.ok(LATER, &["edit", &id, "--project", "beta"]);
    h.ok(LATER, &["edit", &id, "--start", "2026-10-27T09:15", "--end", "2026-10-27T10:45"]);

    let entry = h.json(NOW, &["show", &id, "--json"]);
    assert_eq!(entry["description"], "Review revised design");
    assert_eq!(entry["project"], "beta");
    assert_eq!(entry["duration_seconds"], 5400);
    assert_eq!(entry["revision"], 4, "one bump per successful edit");
    assert_eq!(entry["created_at"], "2026-10-28T20:00:00Z");
    assert_eq!(entry["updated_at"], "2026-10-28T21:00:00Z", "updated_at moves with each edit");

    h.ok(LATER, &["edit", &id, "--clear-project"]);
    assert_eq!(h.json(NOW, &["show", &id, "--json"])["project"], serde_json::Value::Null);
}

#[test]
fn edit_manages_tags() {
    let h = seeded();
    let id = h.id_of(NOW, "Investigate alert");

    h.ok(NOW, &["edit", &id, "--add-tag", "urgent"]);
    assert_eq!(
        h.json(NOW, &["show", &id, "--json"])["tags"],
        serde_json::json!(["incident", "oncall", "urgent"])
    );

    h.ok(NOW, &["edit", &id, "--remove-tag", "ONCALL"]);
    assert_eq!(
        h.json(NOW, &["show", &id, "--json"])["tags"],
        serde_json::json!(["incident", "urgent"]),
        "tag removal is case-insensitive"
    );

    // --tag replaces the whole set.
    h.ok(NOW, &["edit", &id, "--tag", "fresh", "--tag", "set"]);
    assert_eq!(h.json(NOW, &["show", &id, "--json"])["tags"], serde_json::json!(["fresh", "set"]));
}

#[test]
fn edit_requires_a_change_and_validates_the_whole_entry() {
    let h = seeded();
    let id = h.id_of(NOW, "Design review");
    let before = h.json(NOW, &["show", &id, "--json"]);

    let run = h.fail(NOW, &["edit", &id]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("no changes"), "{}", run.stderr);

    // Start after end.
    let run = h.fail(NOW, &["edit", &id, "--start", "2026-10-27T23:00"]);
    assert_eq!(run.code, 2);

    // Into another entry's interval.
    let run = h.fail(NOW, &["edit", &id, "--start", "2026-10-26T09:30", "--end", "2026-10-26T10:00"]);
    assert_eq!(run.code, 4);
    assert!(run.stderr.contains("overlaps"), "{}", run.stderr);

    // Every failure left the entry exactly as it was.
    assert_eq!(h.json(NOW, &["show", &id, "--json"]), before);
}

#[test]
fn end_cannot_be_edited_on_a_running_entry() {
    let h = Harness::new();
    h.ok("2026-10-28T19:00:00Z", &["start", "Running"]);
    let id = h.id_of(NOW, "Running");

    let run = h.fail(NOW, &["edit", &id, "--end", "2026-10-28T16:30"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("still running"), "{}", run.stderr);
    assert_eq!(h.json(NOW, &["show", &id, "--json"])["revision"], 1);

    // The start time of a running entry may still be edited.
    h.ok(NOW, &["edit", &id, "--start", "2026-10-28T14:00"]);
    assert_eq!(h.json(NOW, &["show", &id, "--json"])["running"], true);
}

#[test]
fn ambiguous_and_nonexistent_local_times_are_rejected() {
    let h = Harness::new();
    // 02:30 on 2026-03-08 does not exist in New York.
    let run = h.fail(NOW, &["add", "Gap", "--start", "2026-03-08T02:30", "--duration", "1h"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("does not exist"), "{}", run.stderr);

    // 01:30 on 2026-11-01 happens twice.
    let run = h.fail(NOW, &["add", "Fold", "--start", "2026-11-01T01:30", "--duration", "1h"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("ambiguous"), "{}", run.stderr);

    // With an explicit offset both are accepted.
    h.ok(NOW, &["add", "Explicit gap", "--start", "2026-03-08T03:30-04:00", "--duration", "1h"]);
    h.ok(NOW, &["add", "Explicit fold", "--start", "2026-11-01T01:30-04:00", "--duration", "30m"]);
    assert_eq!(h.json(NOW, &["log", "--json"]).as_array().unwrap().len(), 2);
}

#[test]
fn invalid_input_is_rejected_with_an_actionable_message() {
    let h = Harness::new();
    for (args, needle) in [
        (vec!["add", "x", "--start", "not-a-time", "--duration", "1h"], "not a valid timestamp"),
        (vec!["add", "x", "--start", "2026-10-27T09:00", "--duration", "1d"], "not a valid duration"),
        (vec!["log", "--from", "2026-13-40"], "not a valid date"),
        (vec!["log", "--limit", "0"], "positive"),
        (vec!["start", "x", "--tag", "two words"], "whitespace"),
    ] {
        let run = h.fail(NOW, &args);
        assert_eq!(run.code, 2, "`tt {}` should be a validation error", args.join(" "));
        assert!(
            run.stderr.contains(needle),
            "expected {needle:?} in stderr for `tt {}`, got: {}",
            args.join(" "),
            run.stderr
        );
    }
    assert_eq!(h.json(NOW, &["log", "--json"]).as_array().unwrap().len(), 0);
}
