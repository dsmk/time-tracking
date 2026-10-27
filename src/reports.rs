use jiff::Timestamp;
use jiff::civil::{Date, Weekday};
use jiff::tz::TimeZone;
use serde::Serialize;

use crate::error::{Result, validation};
use crate::model::Entry;
use crate::timeutil::{add_days, start_of_day, to_storage, week_start_date};

/// The half-open instant window `[start, end)` a report covers, together with
/// the local calendar days that produced it.
#[derive(Debug, Clone)]
pub struct Period {
    pub start: Timestamp,
    pub end: Timestamp,
    /// First local day included.
    pub first_day: Date,
    /// Last local day included (inclusive).
    pub last_day: Date,
}

impl Period {
    /// Build the period covering the inclusive local date range
    /// `[first_day, last_day]`, as the half-open instant range from midnight
    /// at the start of `first_day` to midnight after `last_day`.
    pub fn from_dates(first_day: Date, last_day: Date, tz: &TimeZone) -> Result<Period> {
        if last_day < first_day {
            return Err(validation(format!(
                "--from {first_day} is later than --to {last_day}"
            )));
        }
        let start = start_of_day(first_day, tz)?;
        let end = start_of_day(add_days(last_day, 1)?, tz)?;
        Ok(Period { start, end, first_day, last_day })
    }

    /// The period for the week containing `date`, honouring `week_start`.
    pub fn for_week(date: Date, week_start: Weekday, tz: &TimeZone) -> Result<Period> {
        let first = week_start_date(date, week_start);
        Period::from_dates(first, add_days(first, 6)?, tz)
    }

    pub fn days(&self) -> Result<Vec<Date>> {
        let mut days = Vec::new();
        let mut day = self.first_day;
        while day <= self.last_day {
            days.push(day);
            day = add_days(day, 1)?;
        }
        Ok(days)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub seconds: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DayTotal {
    pub date: String,
    pub weekday: String,
    pub seconds: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverlapPair {
    pub first: String,
    pub second: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// "week" or "range".
    pub kind: String,
    pub title: String,
    pub timezone: String,
    pub period_start: String,
    pub period_end: String,
    pub first_day: String,
    pub last_day: String,
    pub total_seconds: i64,
    pub entry_count: usize,
    pub by_project: Vec<Group>,
    pub by_day: Vec<DayTotal>,
    pub by_tag: Vec<Group>,
    pub overlaps: Vec<OverlapPair>,
}

/// Seconds shared by two half-open intervals.
fn overlap_seconds(
    a_start: Timestamp,
    a_end: Timestamp,
    b_start: Timestamp,
    b_end: Timestamp,
) -> i64 {
    let start = a_start.max(b_start).as_second();
    let end = a_end.min(b_end).as_second();
    (end - start).max(0)
}

/// The portion of `entry` that falls inside `period`. A running entry is
/// measured through `now`.
pub fn clipped_seconds(entry: &Entry, period: &Period, now: Timestamp) -> i64 {
    overlap_seconds(entry.start_at, entry.effective_end(now), period.start, period.end)
}

/// Aggregate entries into a report. Nothing is rounded here; display rounding
/// happens only when text output is rendered.
pub fn build(
    kind: &str,
    title: String,
    entries: &[Entry],
    period: &Period,
    tz: &TimeZone,
    now: Timestamp,
) -> Result<Report> {
    let contributing: Vec<&Entry> =
        entries.iter().filter(|e| clipped_seconds(e, period, now) > 0).collect();

    let mut total = 0i64;
    let mut projects: Vec<Group> = Vec::new();
    let mut tags: Vec<Group> = Vec::new();

    for entry in &contributing {
        let seconds = clipped_seconds(entry, period, now);
        total += seconds;

        let project = entry.project.clone().unwrap_or_else(|| "Unassigned".to_string());
        match projects.iter_mut().find(|g| g.name.eq_ignore_ascii_case(&project)) {
            Some(group) => group.seconds += seconds,
            None => projects.push(Group { name: project, seconds }),
        }

        // An entry contributes its full in-period time to each of its tags, so
        // tag totals can exceed the report total.
        for tag in &entry.tags {
            match tags.iter_mut().find(|g| g.name.eq_ignore_ascii_case(tag)) {
                Some(group) => group.seconds += seconds,
                None => tags.push(Group { name: tag.clone(), seconds }),
            }
        }
    }

    // Largest first, ties broken by name so output is deterministic.
    projects.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.name.cmp(&b.name)));
    tags.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.name.cmp(&b.name)));

    let mut by_day = Vec::new();
    for day in period.days()? {
        let day_start = start_of_day(day, tz)?.max(period.start);
        let day_end = start_of_day(add_days(day, 1)?, tz)?.min(period.end);
        let seconds: i64 = contributing
            .iter()
            .map(|e| overlap_seconds(e.start_at, e.effective_end(now), day_start, day_end))
            .sum();
        by_day.push(DayTotal {
            date: day.to_string(),
            weekday: format!("{:?}", day.weekday()),
            seconds,
        });
    }

    let owned: Vec<Entry> = contributing.iter().map(|e| (*e).clone()).collect();
    let overlaps = crate::entries::overlap_pairs(&owned, now)
        .into_iter()
        .map(|(first, second)| OverlapPair { first, second })
        .collect();

    Ok(Report {
        kind: kind.to_string(),
        title,
        timezone: crate::timeutil::tz_name(tz),
        period_start: to_storage(period.start),
        period_end: to_storage(period.end),
        first_day: period.first_day.to_string(),
        last_day: period.last_day.to_string(),
        total_seconds: total,
        entry_count: contributing.len(),
        by_project: projects,
        by_day,
        by_tag: tags,
        overlaps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(raw: &str) -> Timestamp {
        raw.parse().unwrap()
    }

    fn ny() -> TimeZone {
        TimeZone::get("America/New_York").unwrap()
    }

    fn entry(id: &str, start: &str, end: Option<&str>, project: Option<&str>, tags: &[&str]) -> Entry {
        Entry {
            id: id.to_string(),
            start_at: ts(start),
            end_at: end.map(ts),
            description: format!("entry {id}"),
            project: project.map(String::from),
            created_at: ts(start),
            updated_at: ts(start),
            revision: 1,
            deleted_at: None,
            timezone: "America/New_York".into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
        }
    }

    #[test]
    fn week_period_honours_the_configured_first_day() {
        let tz = ny();
        // 2026-10-27 is a Tuesday.
        let date: Date = "2026-10-27".parse().unwrap();

        let monday = Period::for_week(date, Weekday::Monday, &tz).unwrap();
        assert_eq!(monday.first_day.to_string(), "2026-10-26");
        assert_eq!(monday.last_day.to_string(), "2026-11-01");
        assert_eq!(to_storage(monday.start), "2026-10-26T04:00:00Z");

        let sunday = Period::for_week(date, Weekday::Sunday, &tz).unwrap();
        assert_eq!(sunday.first_day.to_string(), "2026-10-25");
        assert_eq!(sunday.last_day.to_string(), "2026-10-31");
    }

    #[test]
    fn a_week_containing_a_dst_change_is_not_exactly_168_hours() {
        let tz = ny();
        // The week of 2026-11-01 gains an hour when clocks fall back.
        let period = Period::for_week("2026-11-01".parse().unwrap(), Weekday::Sunday, &tz).unwrap();
        let elapsed = period.end.as_second() - period.start.as_second();
        assert_eq!(elapsed, 7 * 24 * 3600 + 3600);
    }

    #[test]
    fn range_period_is_half_open_and_rejects_reversed_dates() {
        let tz = ny();
        let period = Period::from_dates(
            "2026-10-01".parse().unwrap(),
            "2026-10-31".parse().unwrap(),
            &tz,
        )
        .unwrap();
        assert_eq!(to_storage(period.start), "2026-10-01T04:00:00Z");
        // Midnight *after* the last day. New York is still on EDT at that
        // instant; the fall-back transition does not happen until 02:00 local.
        assert_eq!(to_storage(period.end), "2026-11-01T04:00:00Z");

        let err = Period::from_dates(
            "2026-10-31".parse().unwrap(),
            "2026-10-01".parse().unwrap(),
            &tz,
        )
        .unwrap_err();
        assert!(err.message.contains("later than"), "{}", err.message);
    }

    #[test]
    fn entries_crossing_the_boundary_contribute_only_the_inside_portion() {
        let tz = ny();
        let period = Period::from_dates(
            "2026-10-27".parse().unwrap(),
            "2026-10-27".parse().unwrap(),
            &tz,
        )
        .unwrap();
        // 23:00 on the 26th to 01:00 on the 27th, local: one hour lands inside.
        let crossing = entry("a", "2026-10-27T03:00:00Z", Some("2026-10-27T05:00:00Z"), None, &[]);
        assert_eq!(clipped_seconds(&crossing, &period, ts("2026-10-27T20:00:00Z")), 3600);

        // Entirely outside contributes nothing.
        let outside = entry("b", "2026-10-20T13:00:00Z", Some("2026-10-20T14:00:00Z"), None, &[]);
        assert_eq!(clipped_seconds(&outside, &period, ts("2026-10-27T20:00:00Z")), 0);
    }

    #[test]
    fn a_running_entry_contributes_through_now_and_is_clipped() {
        let tz = ny();
        let period = Period::from_dates(
            "2026-10-27".parse().unwrap(),
            "2026-10-27".parse().unwrap(),
            &tz,
        )
        .unwrap();
        let running = entry("a", "2026-10-27T13:00:00Z", None, None, &[]);
        assert_eq!(clipped_seconds(&running, &period, ts("2026-10-27T14:30:00Z")), 5400);
        // A "now" past the end of the period is clipped to the period end.
        assert_eq!(
            clipped_seconds(&running, &period, ts("2026-10-29T14:30:00Z")),
            period.end.as_second() - ts("2026-10-27T13:00:00Z").as_second()
        );
    }

    #[test]
    fn aggregates_by_project_day_and_tag() {
        let tz = ny();
        let now = ts("2026-10-28T20:00:00Z");
        let period =
            Period::for_week("2026-10-27".parse().unwrap(), Weekday::Monday, &tz).unwrap();
        let entries = vec![
            // Tuesday 09:00-10:30 local, project alpha, tags a+b
            entry("1", "2026-10-27T13:00:00Z", Some("2026-10-27T14:30:00Z"), Some("alpha"), &["a", "b"]),
            // Tuesday 11:00-12:00 local, no project
            entry("2", "2026-10-27T15:00:00Z", Some("2026-10-27T16:00:00Z"), None, &[]),
            // Wednesday 09:00-10:00 local, project alpha, tag A (different case)
            entry("3", "2026-10-28T13:00:00Z", Some("2026-10-28T14:00:00Z"), Some("alpha"), &["A"]),
        ];

        let report = build("week", "Week".into(), &entries, &period, &tz, now).unwrap();
        assert_eq!(report.total_seconds, 5400 + 3600 + 3600);
        assert_eq!(report.entry_count, 3);

        assert_eq!(
            report.by_project,
            vec![
                Group { name: "alpha".into(), seconds: 9000 },
                Group { name: "Unassigned".into(), seconds: 3600 },
            ]
        );

        // Tag totals are case-insensitive and keep the first spelling seen.
        assert_eq!(
            report.by_tag,
            vec![
                Group { name: "a".into(), seconds: 9000 },
                Group { name: "b".into(), seconds: 5400 },
            ]
        );

        // Seven days, Monday through Sunday, with totals on Tue and Wed only.
        assert_eq!(report.by_day.len(), 7);
        assert_eq!(report.by_day[0].date, "2026-10-26");
        assert_eq!(report.by_day[0].seconds, 0);
        assert_eq!(report.by_day[1].seconds, 9000);
        assert_eq!(report.by_day[2].seconds, 3600);
        // Daily totals must reconstruct the report total exactly.
        assert_eq!(report.by_day.iter().map(|d| d.seconds).sum::<i64>(), report.total_seconds);
    }

    #[test]
    fn an_entry_spanning_midnight_is_split_across_both_days() {
        let tz = ny();
        let now = ts("2026-10-28T20:00:00Z");
        let period = Period::from_dates(
            "2026-10-26".parse().unwrap(),
            "2026-10-28".parse().unwrap(),
            &tz,
        )
        .unwrap();
        // 2026-10-27 23:00 to 2026-10-28 01:00 local.
        let entries =
            vec![entry("1", "2026-10-28T03:00:00Z", Some("2026-10-28T05:00:00Z"), None, &[])];
        let report = build("range", "Range".into(), &entries, &period, &tz, now).unwrap();
        assert_eq!(report.by_day[1].seconds, 3600, "one hour on the 27th");
        assert_eq!(report.by_day[2].seconds, 3600, "one hour on the 28th");
        assert_eq!(report.total_seconds, 7200);
    }

    #[test]
    fn a_dst_day_can_hold_twenty_five_elapsed_hours() {
        let tz = ny();
        let now = ts("2026-11-02T20:00:00Z");
        let period = Period::from_dates(
            "2026-11-01".parse().unwrap(),
            "2026-11-01".parse().unwrap(),
            &tz,
        )
        .unwrap();
        // Midnight to midnight local on the fall-back day: 25 elapsed hours.
        let entries =
            vec![entry("1", "2026-11-01T04:00:00Z", Some("2026-11-02T05:00:00Z"), None, &[])];
        let report = build("range", "Range".into(), &entries, &period, &tz, now).unwrap();
        assert_eq!(report.total_seconds, 25 * 3600);
        assert_eq!(report.by_day[0].seconds, 25 * 3600);
    }

    #[test]
    fn overlapping_contributors_are_reported() {
        let tz = ny();
        let now = ts("2026-10-27T20:00:00Z");
        let period = Period::from_dates(
            "2026-10-27".parse().unwrap(),
            "2026-10-27".parse().unwrap(),
            &tz,
        )
        .unwrap();
        let entries = vec![
            entry("1", "2026-10-27T13:00:00Z", Some("2026-10-27T15:00:00Z"), None, &[]),
            entry("2", "2026-10-27T14:00:00Z", Some("2026-10-27T16:00:00Z"), None, &[]),
        ];
        let report = build("range", "Range".into(), &entries, &period, &tz, now).unwrap();
        assert_eq!(report.overlaps.len(), 1);
        assert_eq!(report.overlaps[0].first, "1");

        let apart = vec![
            entry("1", "2026-10-27T13:00:00Z", Some("2026-10-27T14:00:00Z"), None, &[]),
            entry("2", "2026-10-27T14:00:00Z", Some("2026-10-27T15:00:00Z"), None, &[]),
        ];
        let report = build("range", "Range".into(), &apart, &period, &tz, now).unwrap();
        assert!(report.overlaps.is_empty(), "touching intervals do not overlap");
    }

    #[test]
    fn aggregation_is_not_rounded() {
        let tz = ny();
        let now = ts("2026-10-27T20:00:00Z");
        let period = Period::from_dates(
            "2026-10-27".parse().unwrap(),
            "2026-10-27".parse().unwrap(),
            &tz,
        )
        .unwrap();
        // Three 29-second entries: each rounds to 0m for display, but the
        // report total must still be 87 seconds.
        let entries = vec![
            entry("1", "2026-10-27T13:00:00Z", Some("2026-10-27T13:00:29Z"), None, &[]),
            entry("2", "2026-10-27T14:00:00Z", Some("2026-10-27T14:00:29Z"), None, &[]),
            entry("3", "2026-10-27T15:00:00Z", Some("2026-10-27T15:00:29Z"), None, &[]),
        ];
        let report = build("range", "Range".into(), &entries, &period, &tz, now).unwrap();
        assert_eq!(report.total_seconds, 87);
    }
}
