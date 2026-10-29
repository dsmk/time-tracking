package civil

import (
	"testing"
	"time"
)

func TestParseAcceptsOnlyFullISODates(t *testing.T) {
	date, err := Parse("2026-10-27")
	if err != nil {
		t.Fatalf("a plain date parses: %v", err)
	}
	if date.String() != "2026-10-27" {
		t.Errorf("got %s", date)
	}
	for _, bad := range []string{"last tuesday", "27/10/2026", "2026-13-01", "2026-10-27T00:00:00Z", ""} {
		if _, err := Parse(bad); err == nil {
			t.Errorf("%q should not parse as a date", bad)
		}
	}
}

func TestAddDaysCrossesMonthAndYearBoundaries(t *testing.T) {
	cases := []struct {
		from string
		days int
		want string
	}{
		{"2026-10-31", 1, "2026-11-01"},
		{"2026-01-01", -1, "2025-12-31"},
		{"2024-02-28", 1, "2024-02-29"},
		{"2026-10-26", 6, "2026-11-01"},
	}
	for _, tc := range cases {
		from, err := Parse(tc.from)
		if err != nil {
			t.Fatal(err)
		}
		if got := from.AddDays(tc.days).String(); got != tc.want {
			t.Errorf("%s + %d: got %s, want %s", tc.from, tc.days, got, tc.want)
		}
	}
}

func TestCompareOrdersDates(t *testing.T) {
	earlier, later := New(2026, time.October, 26), New(2026, time.November, 1)
	if !earlier.Before(later) || !later.After(earlier) {
		t.Error("October comes before November")
	}
	if earlier.Compare(earlier) != 0 {
		t.Error("a date equals itself")
	}
}

func TestOfReadsTheLocalDateNotTheUTCOne(t *testing.T) {
	loc, err := time.LoadLocation("America/New_York")
	if err != nil {
		t.Skipf("tzdb: %v", err)
	}
	// 03:30Z on the 28th is 23:30 on the 27th in New York.
	instant, err := time.Parse(time.RFC3339, "2026-10-28T03:30:00Z")
	if err != nil {
		t.Fatal(err)
	}
	if got := Of(instant, loc); got.String() != "2026-10-27" {
		t.Errorf("got %s", got)
	}
	if got := Of(instant, time.UTC); got.String() != "2026-10-28" {
		t.Errorf("got %s", got)
	}
}

func TestDayStartIsLocalMidnightOnAnOrdinaryDay(t *testing.T) {
	loc, err := time.LoadLocation("America/New_York")
	if err != nil {
		t.Skipf("tzdb: %v", err)
	}
	start := DayStart(New(2026, time.October, 27), loc)
	if got := start.UTC().Format(time.RFC3339); got != "2026-10-27T04:00:00Z" {
		t.Errorf("got %s", got)
	}
}

func TestDayStartPicksTheEarlierMidnightOnAFallBackDay(t *testing.T) {
	// Lord Howe and a few others fold midnight; Brazil used to. Use a zone the
	// tzdb reliably carries: Cuba moved its clocks back at midnight.
	loc, err := time.LoadLocation("America/Havana")
	if err != nil {
		t.Skipf("tzdb: %v", err)
	}
	start := DayStart(New(2018, time.November, 4), loc)
	// 00:00 CDT (-04:00) is the first of the two, an hour before 00:00 CST.
	if got := start.UTC().Format(time.RFC3339); got != "2018-11-04T04:00:00Z" {
		t.Errorf("the earlier midnight should win, got %s", got)
	}
}
