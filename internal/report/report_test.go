package report

import (
	"strings"
	"testing"
	"time"

	"github.com/dsmk/time-tracking/internal/civil"
	"github.com/dsmk/time-tracking/internal/ot"
	"github.com/dsmk/time-tracking/internal/tterr"
)

func newYork(t *testing.T) *time.Location {
	t.Helper()
	loc, err := time.LoadLocation("America/New_York")
	if err != nil {
		t.Fatalf("tzdb has New York: %v", err)
	}
	return loc
}

func instant(t *testing.T, raw string) time.Time {
	t.Helper()
	parsed, err := time.Parse(time.RFC3339, raw)
	if err != nil {
		t.Fatalf("valid timestamp: %v", err)
	}
	return parsed.UTC()
}

func entry(t *testing.T, id, task, start string, end string) ot.TimeEntry {
	t.Helper()
	e := ot.TimeEntry{
		ID:          id,
		TaskID:      task,
		ProjectName: "Work",
		StartedAt:   instant(t, start),
	}
	if task != "" {
		e.TaskTitle = "Task " + task
	}
	if end != "" {
		ended := instant(t, end)
		e.EndedAt = &ended
	}
	return e
}

func build(t *testing.T, period Period, entries []ot.TimeEntry, now string) Report {
	t.Helper()
	return Build(period, newYork(t), "America/New_York", nil, entries, instant(t, now), nil, false)
}

func period(t *testing.T, from, to string) Period {
	t.Helper()
	start, err := civil.Parse(from)
	if err != nil {
		t.Fatal(err)
	}
	end, err := civil.Parse(to)
	if err != nil {
		t.Fatal(err)
	}
	p, err := NewPeriod(start, end)
	if err != nil {
		t.Fatal(err)
	}
	return p
}

func TestAMondayWeekContainsTheAnchor(t *testing.T) {
	// 2026-10-28 is a Wednesday.
	week := Week(civil.New(2026, time.October, 28), time.Monday)
	if week.From.String() != "2026-10-26" || week.To.String() != "2026-11-01" {
		t.Fatalf("got %s to %s", week.From, week.To)
	}
}

func TestASundayWeekStartShiftsTheWholeWeek(t *testing.T) {
	week := Week(civil.New(2026, time.October, 28), time.Sunday)
	if week.From.String() != "2026-10-25" || week.To.String() != "2026-10-31" {
		t.Fatalf("got %s to %s", week.From, week.To)
	}
}

func TestTheAnchorsOwnWeekStartDayIsTheFirstDay(t *testing.T) {
	monday := civil.New(2026, time.October, 26)
	if got := Week(monday, time.Monday).From; got != monday {
		t.Fatalf("got %s, want %s", got, monday)
	}
}

func TestABackwardsRangeIsAUsageError(t *testing.T) {
	_, err := NewPeriod(civil.New(2026, time.October, 10), civil.New(2026, time.October, 1))
	if tterr.KindOf(err) != tterr.KindUsage {
		t.Fatalf("got %v", err)
	}
}

func TestTheQueryWindowIsOneDayWiderOnEachSide(t *testing.T) {
	window := period(t, "2026-10-20", "2026-10-26").QueryWindow()
	if window.From.String() != "2026-10-19" || window.To.String() != "2026-10-27" {
		t.Fatalf("got %s to %s", window.From, window.To)
	}
}

func TestEntriesAreClippedAtThePeriodBoundaries(t *testing.T) {
	// 23:00 EDT on the day before through 01:00 on the first day: only the hour
	// inside the period counts.
	built := build(t, period(t, "2026-10-26", "2026-10-26"),
		[]ot.TimeEntry{entry(t, "e1", "a", "2026-10-26T03:00:00Z", "2026-10-26T05:00:00Z")},
		"2026-10-27T00:00:00Z")
	if built.TotalSeconds != 3600 {
		t.Fatalf("total: %d", built.TotalSeconds)
	}
	if built.Days[0].Seconds != 3600 {
		t.Fatalf("day: %d", built.Days[0].Seconds)
	}
}

func TestEntriesEntirelyOutsideThePeriodAreIgnored(t *testing.T) {
	built := build(t, period(t, "2026-10-26", "2026-10-26"),
		[]ot.TimeEntry{entry(t, "e1", "a", "2026-10-20T14:00:00Z", "2026-10-20T15:00:00Z")},
		"2026-10-27T00:00:00Z")
	if built.TotalSeconds != 0 || built.EntryCount != 0 {
		t.Fatalf("got %d seconds over %d entries", built.TotalSeconds, built.EntryCount)
	}
}

func TestAnEntrySpanningMidnightIsSplitAcrossDays(t *testing.T) {
	// 23:30 to 00:30 local = 03:30Z to 04:30Z.
	built := build(t, period(t, "2026-10-26", "2026-10-27"),
		[]ot.TimeEntry{entry(t, "e1", "a", "2026-10-27T03:30:00Z", "2026-10-27T04:30:00Z")},
		"2026-10-28T00:00:00Z")
	if built.TotalSeconds != 3600 {
		t.Fatalf("total: %d", built.TotalSeconds)
	}
	if built.Days[0].Seconds != 1800 {
		t.Errorf("the 26th gets 23:30-24:00, got %d", built.Days[0].Seconds)
	}
	if built.Days[1].Seconds != 1800 {
		t.Errorf("the 27th gets 00:00-00:30, got %d", built.Days[1].Seconds)
	}
}

func TestDailyTotalsSumToThePeriodTotalAcrossADSTChange(t *testing.T) {
	// 2026-11-01 is the US fall-back day: a 25-hour local day.
	built := build(t, period(t, "2026-10-31", "2026-11-02"),
		[]ot.TimeEntry{entry(t, "e1", "a", "2026-10-31T12:00:00Z", "2026-11-02T12:00:00Z")},
		"2026-11-03T00:00:00Z")
	var summed int64
	for _, day := range built.Days {
		summed += day.Seconds
	}
	if summed != built.TotalSeconds {
		t.Fatalf("daily totals sum to %d, period total is %d", summed, built.TotalSeconds)
	}
	if built.Days[1].Seconds != 25*3600 {
		t.Errorf("the fall-back day is 25 hours long, got %d", built.Days[1].Seconds)
	}
}

func TestDailyTotalsSumToThePeriodTotalAcrossASpringForward(t *testing.T) {
	// 2026-03-08 is the US spring-forward day: a 23-hour local day.
	built := build(t, period(t, "2026-03-07", "2026-03-09"),
		[]ot.TimeEntry{entry(t, "e1", "a", "2026-03-07T12:00:00Z", "2026-03-09T12:00:00Z")},
		"2026-03-10T00:00:00Z")
	var summed int64
	for _, day := range built.Days {
		summed += day.Seconds
	}
	if summed != built.TotalSeconds {
		t.Fatalf("daily totals sum to %d, period total is %d", summed, built.TotalSeconds)
	}
	if built.Days[1].Seconds != 23*3600 {
		t.Errorf("the spring-forward day is 23 hours long, got %d", built.Days[1].Seconds)
	}
}

func TestARunningEntryIsCountedUpToNowAndFlagsTheReport(t *testing.T) {
	built := build(t, period(t, "2026-10-26", "2026-10-26"),
		[]ot.TimeEntry{entry(t, "e1", "a", "2026-10-26T14:00:00Z", "")},
		"2026-10-26T15:30:00Z")
	if built.TotalSeconds != 5400 || built.RunningEntries != 1 || !built.Incomplete {
		t.Fatalf("got %+v", built)
	}
	found := false
	for _, warning := range built.Warnings {
		found = found || strings.Contains(warning, "still running")
	}
	if !found {
		t.Fatalf("warnings: %v", built.Warnings)
	}
}

func TestEntriesWithoutATaskAreReportedSeparately(t *testing.T) {
	built := build(t, period(t, "2026-10-26", "2026-10-26"), []ot.TimeEntry{
		entry(t, "e1", "a", "2026-10-26T14:00:00Z", "2026-10-26T15:00:00Z"),
		entry(t, "e2", "", "2026-10-26T16:00:00Z", "2026-10-26T16:30:00Z"),
	}, "2026-10-27T00:00:00Z")
	if built.UntaskedSeconds != 1800 {
		t.Errorf("untasked: %d", built.UntaskedSeconds)
	}
	if len(built.Tasks) != 1 || built.Tasks[0].Seconds != 3600 {
		t.Errorf("tasks: %+v", built.Tasks)
	}
}

func TestTaskTotalsMergeEntriesAndSortLongestFirst(t *testing.T) {
	built := build(t, period(t, "2026-10-26", "2026-10-26"), []ot.TimeEntry{
		entry(t, "e1", "a", "2026-10-26T14:00:00Z", "2026-10-26T14:30:00Z"),
		entry(t, "e2", "b", "2026-10-26T15:00:00Z", "2026-10-26T17:00:00Z"),
		entry(t, "e3", "a", "2026-10-26T18:00:00Z", "2026-10-26T18:30:00Z"),
	}, "2026-10-27T00:00:00Z")
	if built.Tasks[0].TaskID != "b" {
		t.Errorf("longest first: %+v", built.Tasks)
	}
	if built.Tasks[1].Seconds != 3600 || built.Tasks[1].Entries != 2 {
		t.Errorf("entries merge: %+v", built.Tasks[1])
	}
}

func TestDurationsRenderAsHoursAndMinutes(t *testing.T) {
	cases := map[int64]string{0: "0m", 90: "1m", 3600: "1h 00m", 26700: "7h 25m"}
	for seconds, want := range cases {
		if got := FormatDuration(seconds); got != want {
			t.Errorf("%d seconds: got %q, want %q", seconds, got, want)
		}
	}
	if got := Hours(5400); got != 1.5 {
		t.Errorf("hours: %v", got)
	}
}

func TestDayStartHandlesAZoneWhoseMidnightDisappears(t *testing.T) {
	// Asia/Beirut moved its clocks forward at midnight on 2023-03-26, so
	// 00:00 on that date never happened there.
	loc, err := time.LoadLocation("Asia/Beirut")
	if err != nil {
		t.Skipf("tzdb has no Beirut: %v", err)
	}
	start := civil.DayStart(civil.New(2023, time.March, 26), loc)
	if got := start.In(loc).Format("2006-01-02 15:04"); got != "2023-03-26 01:00" {
		t.Fatalf("the day begins when the gap ends, got %s", got)
	}
	// Days stay contiguous across the gap, and the day that lost an hour is
	// 23 hours long rather than being skipped or double-counted.
	previous := civil.DayStart(civil.New(2023, time.March, 25), loc)
	if start.Sub(previous) != 24*time.Hour {
		t.Errorf("the day before the change is %s long", start.Sub(previous))
	}
	next := civil.DayStart(civil.New(2023, time.March, 27), loc)
	if next.Sub(start) != 23*time.Hour {
		t.Errorf("the short day is %s long", next.Sub(start))
	}
}
