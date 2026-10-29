//! Report arithmetic.
//!
//! OpenTodo has no documented CLI reporting command, so `SPEC.md` §6.5 option 2
//! applies: reports are derived from `ot time list` data. Two consequences shape
//! this module.
//!
//! 1. The CLI has no documented `--tz` flag, so `tt` does not rely on how `ot`
//!    interprets `--from`/`--to`. It asks for a slightly wider window and does
//!    all boundary work itself, in the configured timezone, from the absolute
//!    `startedAt`/`endedAt` timestamps.
//! 2. Durations are computed from those timestamps rather than from OpenTodo's
//!    rounded `minutes`, because entries have to be clipped at period edges.

use jiff::civil::{Date, Weekday};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};

use crate::error::{Error, Kind, Result, usage};
use crate::ot::model::TimeEntry;

/// An inclusive range of local calendar days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period {
    pub from: Date,
    pub to: Date,
}

impl Period {
    pub fn new(from: Date, to: Date) -> Result<Period> {
        if to < from {
            return Err(usage(format!(
                "the range ends before it starts: --from {from} is after --to {to}"
            )));
        }
        Ok(Period { from, to })
    }

    /// The week containing `anchor`, starting on `week_start`.
    pub fn week(anchor: Date, week_start: Weekday) -> Result<Period> {
        let offset = i64::from(
            anchor.weekday().to_monday_zero_offset() - week_start.to_monday_zero_offset(),
        )
        .rem_euclid(7);
        let from = anchor
            .checked_sub(offset.days())
            .map_err(|e| usage(format!("cannot compute the week containing {anchor}: {e}")))?;
        let to = from
            .checked_add(6.days())
            .map_err(|e| usage(format!("cannot compute the week starting {from}: {e}")))?;
        Ok(Period { from, to })
    }

    pub fn days(&self) -> impl Iterator<Item = Date> + use<> {
        let (from, to) = (self.from, self.to);
        std::iter::successors(Some(from), move |d| {
            d.checked_add(1.day()).ok().filter(|next| *next <= to)
        })
    }

    /// Half-open instant bounds of the period in `tz`.
    pub fn bounds(&self, tz: &TimeZone) -> Result<(Timestamp, Timestamp)> {
        let start = day_start(self.from, tz)?;
        let next = self
            .to
            .checked_add(1.day())
            .map_err(|e| usage(format!("cannot compute the end of {}: {e}", self.to)))?;
        Ok((start, day_start(next, tz)?))
    }

    /// The window to request from OpenTodo: one day wider on each side, since
    /// `ot` applies its own (undocumented) timezone to `--from`/`--to` and an
    /// entry near a boundary could otherwise be missed.
    pub fn query_window(&self) -> Result<Period> {
        let from = self
            .from
            .checked_sub(1.day())
            .map_err(|e| usage(format!("cannot widen the period start: {e}")))?;
        let to = self
            .to
            .checked_add(1.day())
            .map_err(|e| usage(format!("cannot widen the period end: {e}")))?;
        Ok(Period { from, to })
    }
}

/// Midnight at the start of `date` in `tz`. On a spring-forward day where
/// midnight does not exist, `compatible()` picks the following valid instant,
/// which keeps days contiguous and makes totals add up.
pub fn day_start(date: Date, tz: &TimeZone) -> Result<Timestamp> {
    let dt = date.to_datetime(jiff::civil::Time::midnight());
    tz.to_ambiguous_zoned(dt)
        .compatible()
        .map(|z| z.timestamp())
        .map_err(|e| Error::new(Kind::Usage, format!("cannot resolve midnight on {date}: {e}")))
}

/// Seconds of `[start, end)` that fall inside `[lo, hi)`.
pub fn overlap_seconds(start: Timestamp, end: Timestamp, lo: Timestamp, hi: Timestamp) -> i64 {
    let begin = start.max(lo);
    let finish = end.min(hi);
    if finish <= begin {
        return 0;
    }
    (finish.as_second() - begin.as_second()).max(0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayTotal {
    pub date: Date,
    pub seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskTotal {
    pub task_id: Option<String>,
    pub label: String,
    pub project: Option<String>,
    pub seconds: i64,
    pub entries: usize,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub period: Period,
    pub timezone: String,
    /// Present for weekly reports; range reports do not have a week start.
    pub week_start: Option<Weekday>,
    pub total_seconds: i64,
    pub days: Vec<DayTotal>,
    pub tasks: Vec<TaskTotal>,
    pub untasked_seconds: i64,
    pub running_entries: usize,
    pub entry_count: usize,
    /// True when the figures are known to be partial: a timer is still running,
    /// or OpenTodo returned as many entries as the limit allowed.
    pub incomplete: bool,
    pub warnings: Vec<String>,
}

/// Build a report from entries OpenTodo returned.
///
/// `now` ends any still-running entry. Pass OpenTodo's `serverTime` when it
/// supplied one, so elapsed time is measured on the same clock that produced
/// the entry. `data_incomplete` says the caller already knows entries may be
/// missing — a filled `--limit`, or a pagination cursor tt cannot follow.
pub fn build(
    period: Period,
    tz: &TimeZone,
    week_start: Option<Weekday>,
    entries: &[TimeEntry],
    now: Timestamp,
    mut warnings: Vec<String>,
    data_incomplete: bool,
) -> Result<Report> {
    let (lo, hi) = period.bounds(tz)?;

    let mut days: Vec<DayTotal> =
        period.days().map(|date| DayTotal { date, seconds: 0 }).collect();
    let mut day_bounds = Vec::with_capacity(days.len());
    for day in &days {
        let start = day_start(day.date, tz)?;
        let next = day
            .date
            .checked_add(1.day())
            .map_err(|e| usage(format!("cannot compute the day after {}: {e}", day.date)))?;
        day_bounds.push((start, day_start(next, tz)?));
    }

    let mut tasks: Vec<TaskTotal> = Vec::new();
    let mut untasked_seconds = 0i64;
    let mut total_seconds = 0i64;
    let mut running_entries = 0usize;
    let mut counted = 0usize;

    for entry in entries {
        let end = match entry.ended_at {
            Some(end) => end,
            None => {
                // A running entry is only counted up to "now"; if the clock is
                // behind the start, count nothing rather than negative time.
                running_entries += 1;
                now.max(entry.started_at)
            }
        };

        let seconds = overlap_seconds(entry.started_at, end, lo, hi);
        if seconds == 0 {
            continue;
        }
        counted += 1;
        total_seconds += seconds;

        for (day, (day_lo, day_hi)) in days.iter_mut().zip(&day_bounds) {
            day.seconds += overlap_seconds(entry.started_at, end, *day_lo, *day_hi);
        }

        match entry.task_key() {
            Some(task_id) => {
                match tasks.iter_mut().find(|t| t.task_id.as_deref() == Some(task_id)) {
                    Some(total) => {
                        total.seconds += seconds;
                        total.entries += 1;
                    }
                    None => tasks.push(TaskTotal {
                        task_id: Some(task_id.to_string()),
                        label: entry
                            .task_title
                            .clone()
                            .unwrap_or_else(|| format!("task {task_id}")),
                        project: entry.project_name.clone(),
                        seconds,
                        entries: 1,
                    }),
                }
            }
            None => untasked_seconds += seconds,
        }
    }

    // Longest first, then by label so equal totals have a stable order.
    tasks.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.label.cmp(&b.label)));

    if running_entries > 0 {
        warnings.push(format!(
            "{running_entries} entr{} still running; time is counted up to {}",
            if running_entries == 1 { "y is" } else { "ies are" },
            now.to_zoned(tz.clone()).strftime("%Y-%m-%d %H:%M %Z")
        ));
    }

    let incomplete = data_incomplete || running_entries > 0;

    Ok(Report {
        period,
        timezone: tz_label(tz),
        week_start,
        total_seconds,
        days,
        tasks,
        untasked_seconds,
        running_entries,
        entry_count: counted,
        incomplete,
        warnings,
    })
}

pub fn tz_label(tz: &TimeZone) -> String {
    tz.iana_name().unwrap_or("system").to_string()
}

/// Render seconds as `7h 25m`, or `0m` for nothing.
pub fn format_duration(seconds: i64) -> String {
    let total_minutes = seconds / 60;
    let (hours, minutes) = (total_minutes / 60, total_minutes % 60);
    if hours > 0 { format!("{hours}h {minutes:02}m") } else { format!("{minutes}m") }
}

/// Decimal hours, for CSV consumers that want to sum a column.
pub fn hours(seconds: i64) -> f64 {
    (seconds as f64 / 36.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn tz() -> TimeZone {
        TimeZone::get("America/New_York").expect("tzdb has New York")
    }

    fn ts(raw: &str) -> Timestamp {
        raw.parse().expect("valid timestamp")
    }

    fn entry(id: &str, task: Option<&str>, start: &str, end: Option<&str>) -> TimeEntry {
        TimeEntry {
            id: id.to_string(),
            task_id: task.map(str::to_string),
            task_title: task.map(|t| format!("Task {t}")),
            project_id: None,
            project_name: Some("Work".to_string()),
            note: None,
            started_at: ts(start),
            ended_at: end.map(ts),
            minutes: None,
        }
    }

    #[test]
    fn a_monday_week_contains_the_anchor() {
        // 2026-10-28 is a Wednesday.
        let period = Period::week(date(2026, 10, 28), Weekday::Monday).expect("week");
        assert_eq!(period.from, date(2026, 10, 26));
        assert_eq!(period.to, date(2026, 11, 1));
    }

    #[test]
    fn a_sunday_week_start_shifts_the_whole_week() {
        let period = Period::week(date(2026, 10, 28), Weekday::Sunday).expect("week");
        assert_eq!(period.from, date(2026, 10, 25));
        assert_eq!(period.to, date(2026, 10, 31));
    }

    #[test]
    fn the_anchors_own_week_start_day_is_the_first_day() {
        let monday = date(2026, 10, 26);
        let period = Period::week(monday, Weekday::Monday).expect("week");
        assert_eq!(period.from, monday);
    }

    #[test]
    fn a_backwards_range_is_a_usage_error() {
        let err = Period::new(date(2026, 10, 10), date(2026, 10, 1)).expect_err("backwards");
        assert_eq!(err.kind, Kind::Usage);
    }

    #[test]
    fn the_query_window_is_one_day_wider_on_each_side() {
        let period = Period::new(date(2026, 10, 20), date(2026, 10, 26)).unwrap();
        let window = period.query_window().unwrap();
        assert_eq!(window.from, date(2026, 10, 19));
        assert_eq!(window.to, date(2026, 10, 27));
    }

    #[test]
    fn entries_are_clipped_at_the_period_boundaries() {
        // 23:00 EDT on the day before through 01:00 on the first day: only the
        // hour inside the period counts.
        let period = Period::new(date(2026, 10, 26), date(2026, 10, 26)).unwrap();
        let entries = [entry("e1", Some("a"), "2026-10-26T03:00:00Z", Some("2026-10-26T05:00:00Z"))];
        let report =
            build(period, &tz(), None, &entries, ts("2026-10-27T00:00:00Z"), vec![], false).unwrap();
        // 03:00Z is 23:00 on the 25th in New York; 05:00Z is 01:00 on the 26th.
        assert_eq!(report.total_seconds, 3600);
        assert_eq!(report.days[0].seconds, 3600);
    }

    #[test]
    fn entries_entirely_outside_the_period_are_ignored() {
        let period = Period::new(date(2026, 10, 26), date(2026, 10, 26)).unwrap();
        let entries = [entry("e1", Some("a"), "2026-10-20T14:00:00Z", Some("2026-10-20T15:00:00Z"))];
        let report =
            build(period, &tz(), None, &entries, ts("2026-10-27T00:00:00Z"), vec![], false).unwrap();
        assert_eq!(report.total_seconds, 0);
        assert_eq!(report.entry_count, 0);
    }

    #[test]
    fn an_entry_spanning_midnight_is_split_across_days() {
        let period = Period::new(date(2026, 10, 26), date(2026, 10, 27)).unwrap();
        // 23:30 to 00:30 local = 03:30Z to 04:30Z.
        let entries = [entry("e1", Some("a"), "2026-10-27T03:30:00Z", Some("2026-10-27T04:30:00Z"))];
        let report =
            build(period, &tz(), None, &entries, ts("2026-10-28T00:00:00Z"), vec![], false).unwrap();
        assert_eq!(report.total_seconds, 3600);
        assert_eq!(report.days[0].seconds, 1800, "26th gets 23:30-24:00");
        assert_eq!(report.days[1].seconds, 1800, "27th gets 00:00-00:30");
    }

    #[test]
    fn daily_totals_sum_to_the_period_total_across_a_dst_change() {
        // 2026-11-01 is the US fall-back day: a 25-hour local day.
        let period = Period::new(date(2026, 10, 31), date(2026, 11, 2)).unwrap();
        let entries = [entry("e1", Some("a"), "2026-10-31T12:00:00Z", Some("2026-11-02T12:00:00Z"))];
        let report =
            build(period, &tz(), None, &entries, ts("2026-11-03T00:00:00Z"), vec![], false).unwrap();
        let summed: i64 = report.days.iter().map(|d| d.seconds).sum();
        assert_eq!(summed, report.total_seconds);
        assert_eq!(report.days[1].seconds, 25 * 3600, "the fall-back day is 25 hours long");
    }

    #[test]
    fn a_running_entry_is_counted_up_to_now_and_flags_the_report() {
        let period = Period::new(date(2026, 10, 26), date(2026, 10, 26)).unwrap();
        let entries = [entry("e1", Some("a"), "2026-10-26T14:00:00Z", None)];
        let report =
            build(period, &tz(), None, &entries, ts("2026-10-26T15:30:00Z"), vec![], false).unwrap();
        assert_eq!(report.total_seconds, 5400);
        assert_eq!(report.running_entries, 1);
        assert!(report.incomplete);
        assert!(report.warnings.iter().any(|w| w.contains("still running")));
    }

    #[test]
    fn entries_without_a_task_are_reported_separately() {
        let period = Period::new(date(2026, 10, 26), date(2026, 10, 26)).unwrap();
        let entries = [
            entry("e1", Some("a"), "2026-10-26T14:00:00Z", Some("2026-10-26T15:00:00Z")),
            entry("e2", None, "2026-10-26T16:00:00Z", Some("2026-10-26T16:30:00Z")),
        ];
        let report =
            build(period, &tz(), None, &entries, ts("2026-10-27T00:00:00Z"), vec![], false).unwrap();
        assert_eq!(report.untasked_seconds, 1800);
        assert_eq!(report.tasks.len(), 1);
        assert_eq!(report.tasks[0].seconds, 3600);
    }

    #[test]
    fn task_totals_merge_entries_and_sort_longest_first() {
        let period = Period::new(date(2026, 10, 26), date(2026, 10, 26)).unwrap();
        let entries = [
            entry("e1", Some("a"), "2026-10-26T14:00:00Z", Some("2026-10-26T14:30:00Z")),
            entry("e2", Some("b"), "2026-10-26T15:00:00Z", Some("2026-10-26T17:00:00Z")),
            entry("e3", Some("a"), "2026-10-26T18:00:00Z", Some("2026-10-26T18:30:00Z")),
        ];
        let report =
            build(period, &tz(), None, &entries, ts("2026-10-27T00:00:00Z"), vec![], false).unwrap();
        assert_eq!(report.tasks[0].task_id.as_deref(), Some("b"));
        assert_eq!(report.tasks[1].seconds, 3600);
        assert_eq!(report.tasks[1].entries, 2);
    }

    #[test]
    fn durations_render_as_hours_and_minutes() {
        assert_eq!(format_duration(0), "0m");
        assert_eq!(format_duration(90), "1m");
        assert_eq!(format_duration(3600), "1h 00m");
        assert_eq!(format_duration(26_700), "7h 25m");
        assert_eq!(hours(5400), 1.5);
    }
}
