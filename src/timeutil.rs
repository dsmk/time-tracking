use jiff::civil::{Date, DateTime, Weekday};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::{Timestamp, ToSpan, Zoned};

use crate::error::{Result, validation};

/// How precisely durations are rendered in text output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    Minute,
    Second,
}

/// Upper sentinel used when a running entry is treated as an open-ended
/// interval in SQL comparisons. Sorts above every real timestamp because all
/// stored timestamps use the same fixed-width RFC 3339 UTC format.
pub const MAX_TIMESTAMP_TEXT: &str = "9999-12-31T23:59:59Z";

/// Canonical on-disk timestamp format: RFC 3339, UTC, whole seconds.
pub fn to_storage(ts: Timestamp) -> String {
    // Truncate to second precision so stored values are fixed width and can be
    // compared lexicographically by SQLite.
    let secs = ts.as_second();
    Timestamp::from_second(secs)
        .expect("second-truncated timestamp is in range")
        .to_string()
}

pub fn from_storage(raw: &str) -> Result<Timestamp> {
    raw.parse::<Timestamp>()
        .map_err(|e| validation(format!("stored timestamp {raw:?} is not valid RFC 3339: {e}")))
}

/// Parse a user-supplied duration such as `90m`, `1h30m`, or `45s`.
/// Returns whole seconds; the result is always strictly positive.
pub fn parse_duration(input: &str) -> Result<i64> {
    let text = input.trim();
    if text.is_empty() {
        return Err(bad_duration(input));
    }

    let mut total: i64 = 0;
    let mut digits = String::new();
    let mut saw_unit = false;

    for ch in text.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            continue;
        }
        let unit_seconds = match ch.to_ascii_lowercase() {
            'h' => 3600,
            'm' => 60,
            's' => 1,
            _ => return Err(bad_duration(input)),
        };
        if digits.is_empty() {
            return Err(bad_duration(input));
        }
        let value: i64 = digits.parse().map_err(|_| bad_duration(input))?;
        total = total
            .checked_add(value.checked_mul(unit_seconds).ok_or_else(|| bad_duration(input))?)
            .ok_or_else(|| bad_duration(input))?;
        digits.clear();
        saw_unit = true;
    }

    if !digits.is_empty() || !saw_unit {
        return Err(bad_duration(input));
    }
    if total <= 0 {
        return Err(validation(format!("duration {input:?} must be greater than zero")));
    }
    Ok(total)
}

fn bad_duration(input: &str) -> crate::error::AppError {
    validation(format!("{input:?} is not a valid duration"))
        .with_hint("use integers with h, m, or s units, for example 90m, 1h30m, or 45s")
}

/// Render a duration for text output. Hours are always shown so that columns
/// line up; minutes and seconds are zero-padded.
pub fn format_duration(seconds: i64, precision: Precision) -> String {
    let negative = seconds < 0;
    let mut secs = seconds.abs();
    if precision == Precision::Minute {
        // Round to the nearest minute for display only; stored instants and
        // aggregates are never rounded.
        secs = (secs + 30) / 60 * 60;
    }
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let sign = if negative { "-" } else { "" };
    match precision {
        Precision::Minute => format!("{sign}{hours}h {minutes:02}m"),
        Precision::Second => format!("{sign}{hours}h {minutes:02}m {:02}s", secs % 60),
    }
}

/// Parse a calendar date in `YYYY-MM-DD` form.
pub fn parse_date(input: &str) -> Result<Date> {
    input.trim().parse::<Date>().map_err(|e| {
        validation(format!("{input:?} is not a valid date: {e}"))
            .with_hint("dates use the form YYYY-MM-DD, for example 2026-10-27")
    })
}

/// Parse a user-supplied timestamp.
///
/// A value carrying `Z` or an explicit UTC offset is absolute and is accepted
/// as-is. A value without an offset is interpreted in `tz`; if that local time
/// is ambiguous or nonexistent because of a daylight-saving transition, the
/// value is rejected with an explanation.
pub fn parse_timestamp(input: &str, tz: &TimeZone) -> Result<Timestamp> {
    let text = input.trim();
    if text.is_empty() {
        return Err(bad_timestamp(input, "the value is empty"));
    }
    // Accept a space in place of the RFC 3339 `T` separator for convenience.
    let normalized = normalize_separator(text);

    if has_explicit_offset(&normalized) {
        return normalized
            .parse::<Timestamp>()
            .map_err(|e| bad_timestamp(input, &e.to_string()));
    }

    let civil: DateTime = normalized
        .parse()
        .map_err(|e: jiff::Error| bad_timestamp(input, &e.to_string()))?;

    match tz.to_ambiguous_zoned(civil).offset() {
        AmbiguousOffset::Unambiguous { .. } => {}
        AmbiguousOffset::Gap { before, after } => {
            return Err(validation(format!(
                "local time {civil} does not exist in {}: clocks jump from {before} to {after} \
                 for daylight saving time",
                tz_name(tz)
            ))
            .with_hint("choose a time outside the gap, or give an explicit offset, for \
                        example 2026-03-08T03:30-04:00"));
        }
        AmbiguousOffset::Fold { before, after } => {
            return Err(validation(format!(
                "local time {civil} is ambiguous in {}: it occurs twice, at offset {before} and \
                 again at {after}, because of a daylight-saving transition",
                tz_name(tz)
            ))
            .with_hint(format!(
                "repeat the value with an explicit offset, for example {civil}{}",
                format_offset(before)
            )));
        }
    }

    let zoned = tz
        .to_ambiguous_zoned(civil)
        .unambiguous()
        .map_err(|e| bad_timestamp(input, &e.to_string()))?;
    Ok(zoned.timestamp())
}

fn normalize_separator(text: &str) -> String {
    match text.find(' ') {
        Some(idx) if text[..idx].len() == 10 => {
            let mut out = text.to_string();
            out.replace_range(idx..idx + 1, "T");
            out
        }
        _ => text.to_string(),
    }
}

/// True when the value ends in `Z` or carries a `+HH:MM` / `-HH:MM` offset.
/// The scan starts after the date part so that the dashes in `YYYY-MM-DD` are
/// not mistaken for a negative offset.
fn has_explicit_offset(text: &str) -> bool {
    let Some(sep) = text.find(['T', 't']) else {
        return text.ends_with('Z') || text.ends_with('z');
    };
    let time_part = &text[sep + 1..];
    time_part.ends_with('Z')
        || time_part.ends_with('z')
        || time_part.contains('+')
        || time_part.contains('-')
}

fn bad_timestamp(input: &str, detail: &str) -> crate::error::AppError {
    validation(format!("{input:?} is not a valid timestamp: {detail}")).with_hint(
        "accepted forms: 2026-10-27T09:30, 2026-10-27T09:30:00, 2026-10-27T09:30-04:00, \
         2026-10-27T13:30:00Z",
    )
}

/// Render a UTC offset as `+HH:MM` / `-HH:MM`. jiff's own `Display` prints
/// `-04`, which is not an RFC 3339 offset, so a hint built from it would not
/// parse if the user copied it back.
fn format_offset(offset: jiff::tz::Offset) -> String {
    let total = offset.seconds();
    let sign = if total < 0 { '-' } else { '+' };
    let total = total.abs();
    format!("{sign}{:02}:{:02}", total / 3600, (total % 3600) / 60)
}

pub fn tz_name(tz: &TimeZone) -> String {
    tz.iana_name().unwrap_or("local").to_string()
}

/// First instant of the given local calendar day. If midnight itself does not
/// exist because of a daylight-saving transition, the first instant that does
/// exist is used, so report periods never lose a day.
pub fn start_of_day(date: Date, tz: &TimeZone) -> Result<Timestamp> {
    let zoned: Zoned = date
        .to_zoned(tz.clone())
        .map_err(|e| validation(format!("cannot resolve midnight on {date}: {e}")))?;
    Ok(zoned.timestamp())
}

/// The local calendar date an instant falls on.
pub fn local_date(ts: Timestamp, tz: &TimeZone) -> Date {
    ts.to_zoned(tz.clone()).date()
}

/// The first day of the week containing `date`, given the configured week start.
pub fn week_start_date(date: Date, week_start: Weekday) -> Date {
    let offset = (date.weekday().to_monday_zero_offset() as i64
        - week_start.to_monday_zero_offset() as i64)
        .rem_euclid(7);
    date.checked_sub((offset as i32).days())
        .expect("subtracting at most six days stays in range")
}

pub fn add_days(date: Date, days: i32) -> Result<Date> {
    date.checked_add(days.days())
        .map_err(|e| validation(format!("date {date} plus {days} days is out of range: {e}")))
}

pub fn format_local_datetime(ts: Timestamp, tz: &TimeZone, precision: Precision) -> String {
    let fmt = match precision {
        Precision::Minute => "%Y-%m-%d %H:%M",
        Precision::Second => "%Y-%m-%d %H:%M:%S",
    };
    jiff::fmt::strtime::format(fmt, &ts.to_zoned(tz.clone()))
        .unwrap_or_else(|_| ts.to_string())
}

pub fn format_local_time(ts: Timestamp, tz: &TimeZone, precision: Precision) -> String {
    let fmt = match precision {
        Precision::Minute => "%H:%M",
        Precision::Second => "%H:%M:%S",
    };
    jiff::fmt::strtime::format(fmt, &ts.to_zoned(tz.clone()))
        .unwrap_or_else(|_| ts.to_string())
}

pub fn format_local_date(ts: Timestamp, tz: &TimeZone) -> String {
    ts.to_zoned(tz.clone()).date().to_string()
}

pub fn parse_weekday(name: &str) -> Result<Weekday> {
    match name.trim().to_ascii_lowercase().as_str() {
        "monday" | "mon" => Ok(Weekday::Monday),
        "tuesday" | "tue" => Ok(Weekday::Tuesday),
        "wednesday" | "wed" => Ok(Weekday::Wednesday),
        "thursday" | "thu" => Ok(Weekday::Thursday),
        "friday" | "fri" => Ok(Weekday::Friday),
        "saturday" | "sat" => Ok(Weekday::Saturday),
        "sunday" | "sun" => Ok(Weekday::Sunday),
        other => Err(validation(format!("{other:?} is not a weekday name")).with_hint(
            "use one of: monday, tuesday, wednesday, thursday, friday, saturday, sunday",
        )),
    }
}

pub fn weekday_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Monday => "monday",
        Weekday::Tuesday => "tuesday",
        Weekday::Wednesday => "wednesday",
        Weekday::Thursday => "thursday",
        Weekday::Friday => "friday",
        Weekday::Saturday => "saturday",
        Weekday::Sunday => "sunday",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ny() -> TimeZone {
        TimeZone::get("America/New_York").unwrap()
    }

    #[test]
    fn parses_duration_units_and_combinations() {
        assert_eq!(parse_duration("45s").unwrap(), 45);
        assert_eq!(parse_duration("90m").unwrap(), 5400);
        assert_eq!(parse_duration("2h").unwrap(), 7200);
        assert_eq!(parse_duration("1h30m").unwrap(), 5400);
        assert_eq!(parse_duration("1h30m15s").unwrap(), 5415);
        assert_eq!(parse_duration("  1h 30m ".replace(' ', "").as_str()).unwrap(), 5400);
    }

    #[test]
    fn rejects_malformed_durations() {
        for bad in ["", "90", "h", "1d", "m30", "1h30", "-5m", "1.5h", "1h30x"] {
            assert!(parse_duration(bad).is_err(), "expected {bad:?} to be rejected");
        }
        assert!(parse_duration("0m").is_err());
    }

    #[test]
    fn formats_durations_at_each_precision() {
        assert_eq!(format_duration(5400, Precision::Minute), "1h 30m");
        assert_eq!(format_duration(5415, Precision::Minute), "1h 30m");
        assert_eq!(format_duration(5415, Precision::Second), "1h 30m 15s");
        assert_eq!(format_duration(0, Precision::Minute), "0h 00m");
        assert_eq!(format_duration(45, Precision::Minute), "0h 01m", "rounds to nearest minute");
        assert_eq!(format_duration(29, Precision::Minute), "0h 00m");
        assert_eq!(format_duration(45, Precision::Second), "0h 00m 45s");
    }

    #[test]
    fn parses_absolute_timestamps_unchanged() {
        let tz = ny();
        let utc = parse_timestamp("2026-10-27T13:30:00Z", &tz).unwrap();
        let offset = parse_timestamp("2026-10-27T09:30-04:00", &tz).unwrap();
        assert_eq!(utc, offset);
    }

    #[test]
    fn interprets_naive_timestamps_in_the_configured_zone() {
        let ts = parse_timestamp("2026-10-27T09:30", &ny()).unwrap();
        assert_eq!(to_storage(ts), "2026-10-27T13:30:00Z");
        // The same civil time in another zone is a different instant.
        let utc = parse_timestamp("2026-10-27T09:30", &TimeZone::UTC).unwrap();
        assert_eq!(to_storage(utc), "2026-10-27T09:30:00Z");
    }

    #[test]
    fn accepts_a_space_separator_and_optional_seconds() {
        let tz = ny();
        assert_eq!(
            parse_timestamp("2026-10-27 09:30", &tz).unwrap(),
            parse_timestamp("2026-10-27T09:30:00", &tz).unwrap()
        );
    }

    #[test]
    fn rejects_nonexistent_local_times() {
        // 2026-03-08 02:30 does not exist in New York: clocks jump 02:00 -> 03:00.
        let err = parse_timestamp("2026-03-08T02:30", &ny()).unwrap_err();
        assert!(err.message.contains("does not exist"), "{}", err.message);
    }

    #[test]
    fn rejects_ambiguous_local_times() {
        // 2026-11-01 01:30 occurs twice in New York.
        let err = parse_timestamp("2026-11-01T01:30", &ny()).unwrap_err();
        assert!(err.message.contains("ambiguous"), "{}", err.message);
    }

    #[test]
    fn the_ambiguity_hint_contains_a_timestamp_that_parses() {
        let tz = ny();
        let err = parse_timestamp("2026-11-01T01:30", &tz).unwrap_err();
        let hint = err.hint.expect("ambiguous times carry a hint");
        let suggestion = hint.rsplit(", for example ").next().unwrap();
        assert_eq!(suggestion, "2026-11-01T01:30:00-04:00");
        // The suggested value must be accepted verbatim.
        parse_timestamp(suggestion, &tz).expect("the hint must round-trip");
    }

    #[test]
    fn the_gap_hint_contains_a_timestamp_that_parses() {
        let tz = ny();
        let err = parse_timestamp("2026-03-08T02:30", &tz).unwrap_err();
        let hint = err.hint.expect("nonexistent times carry a hint");
        let suggestion = hint.rsplit("example ").next().unwrap();
        parse_timestamp(suggestion, &tz).expect("the hint must round-trip");
    }

    #[test]
    fn formats_offsets_as_rfc_3339() {
        let tz = ny();
        let winter = parse_timestamp("2026-01-15T12:00:00Z", &tz).unwrap();
        let summer = parse_timestamp("2026-07-15T12:00:00Z", &tz).unwrap();
        assert_eq!(format_offset(tz.to_offset(winter)), "-05:00");
        assert_eq!(format_offset(tz.to_offset(summer)), "-04:00");
        assert_eq!(format_offset(TimeZone::UTC.to_offset(winter)), "+00:00");
    }

    #[test]
    fn accepts_dst_edge_times_with_an_explicit_offset() {
        let tz = ny();
        let first = parse_timestamp("2026-11-01T01:30-04:00", &tz).unwrap();
        let second = parse_timestamp("2026-11-01T01:30-05:00", &tz).unwrap();
        assert_ne!(first, second);
        assert_eq!(second.as_second() - first.as_second(), 3600);
    }

    #[test]
    fn computes_week_start_for_every_configured_first_day() {
        // 2026-10-27 is a Tuesday.
        let date = parse_date("2026-10-27").unwrap();
        let cases = [
            (Weekday::Monday, "2026-10-26"),
            (Weekday::Tuesday, "2026-10-27"),
            (Weekday::Wednesday, "2026-10-21"),
            (Weekday::Thursday, "2026-10-22"),
            (Weekday::Friday, "2026-10-23"),
            (Weekday::Saturday, "2026-10-24"),
            (Weekday::Sunday, "2026-10-25"),
        ];
        for (start, expected) in cases {
            assert_eq!(
                week_start_date(date, start).to_string(),
                expected,
                "week starting {start:?}"
            );
        }
    }

    #[test]
    fn week_start_is_idempotent_on_the_first_day() {
        let date = parse_date("2026-10-26").unwrap(); // Monday
        assert_eq!(week_start_date(date, Weekday::Monday), date);
    }

    #[test]
    fn start_of_day_uses_the_local_zone() {
        let date = parse_date("2026-10-27").unwrap();
        let ts = start_of_day(date, &ny()).unwrap();
        assert_eq!(to_storage(ts), "2026-10-27T04:00:00Z");
    }

    #[test]
    fn a_dst_day_has_twenty_three_elapsed_hours() {
        let tz = ny();
        let spring = parse_date("2026-03-08").unwrap();
        let begin = start_of_day(spring, &tz).unwrap();
        let end = start_of_day(add_days(spring, 1).unwrap(), &tz).unwrap();
        assert_eq!(end.as_second() - begin.as_second(), 23 * 3600);

        let fall = parse_date("2026-11-01").unwrap();
        let begin = start_of_day(fall, &tz).unwrap();
        let end = start_of_day(add_days(fall, 1).unwrap(), &tz).unwrap();
        assert_eq!(end.as_second() - begin.as_second(), 25 * 3600);
    }

    #[test]
    fn storage_format_is_fixed_width_and_sorts_lexicographically() {
        let a = to_storage(parse_timestamp("2026-01-02T03:04:05Z", &TimeZone::UTC).unwrap());
        let b = to_storage(parse_timestamp("2026-10-27T13:30:00Z", &TimeZone::UTC).unwrap());
        assert_eq!(a.len(), b.len());
        assert!(a < b);
        assert!(b.as_str() < MAX_TIMESTAMP_TEXT);
    }

    #[test]
    fn weekday_names_round_trip() {
        for name in ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"] {
            assert_eq!(weekday_name(parse_weekday(name).unwrap()), name);
        }
        assert!(parse_weekday("funday").is_err());
    }
}
