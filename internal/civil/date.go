// Package civil holds a calendar date with no time and no zone.
//
// Go's time.Time always carries both, which makes it easy to compare a date
// against an instant by accident. A report's period is made of local calendar
// days, so those days get their own type and only become instants through
// DayStart, where the timezone is explicit.
package civil

import (
	"fmt"
	"time"
)

// Layout is the only date form tt reads or writes.
const Layout = "2006-01-02"

// Date is a local calendar date.
type Date struct {
	Year  int
	Month time.Month
	Day   int
}

// New builds a date. The parts are normalised the way time.Date normalises
// them, so New(2026, 1, 32) is 2026-02-01.
func New(year int, month time.Month, day int) Date {
	t := time.Date(year, month, day, 0, 0, 0, 0, time.UTC)
	return Date{t.Year(), t.Month(), t.Day()}
}

// Parse reads a YYYY-MM-DD date.
func Parse(text string) (Date, error) {
	t, err := time.Parse(Layout, text)
	if err != nil {
		return Date{}, err
	}
	return Date{t.Year(), t.Month(), t.Day()}, nil
}

// Of is the local calendar date of an instant.
func Of(t time.Time, loc *time.Location) Date {
	local := t.In(loc)
	return Date{local.Year(), local.Month(), local.Day()}
}

func (d Date) String() string {
	return fmt.Sprintf("%04d-%02d-%02d", d.Year, int(d.Month), d.Day)
}

// AddDays returns the date n days later, or earlier when n is negative.
func (d Date) AddDays(n int) Date {
	return New(d.Year, d.Month, d.Day+n)
}

// Weekday is the day of the week this date falls on.
func (d Date) Weekday() time.Weekday {
	return time.Date(d.Year, d.Month, d.Day, 0, 0, 0, 0, time.UTC).Weekday()
}

// Compare orders two dates: negative when d is earlier, zero when equal.
func (d Date) Compare(other Date) int {
	switch {
	case d.Year != other.Year:
		return sign(d.Year - other.Year)
	case d.Month != other.Month:
		return sign(int(d.Month) - int(other.Month))
	default:
		return sign(d.Day - other.Day)
	}
}

// Before reports whether d is earlier than other.
func (d Date) Before(other Date) bool { return d.Compare(other) < 0 }

// After reports whether d is later than other.
func (d Date) After(other Date) bool { return d.Compare(other) > 0 }

// Format renders the date through a time layout, for weekday names and such.
func (d Date) Format(layout string) string {
	return time.Date(d.Year, d.Month, d.Day, 0, 0, 0, 0, time.UTC).Format(layout)
}

// DayStart is the first instant of this date in loc.
//
// On a spring-forward day where local midnight does not exist, this is the
// instant the gap ends, which keeps days contiguous and makes daily totals add
// up to the period total. On a fall-back day where midnight happens twice, it
// is the earlier one, for the same reason.
func DayStart(d Date, loc *time.Location) time.Time {
	t := time.Date(d.Year, d.Month, d.Day, 0, 0, 0, 0, loc)
	if y, m, day := t.Date(); y == d.Year && m == d.Month && day == d.Day &&
		t.Hour() == 0 && t.Minute() == 0 && t.Second() == 0 && t.Nanosecond() == 0 {
		return t
	}

	// Midnight was swallowed by a DST gap, and time.Date's normalisation of a
	// nonexistent local time is explicitly unspecified. Search instead for the
	// earliest instant whose local date has reached d: that is where the day
	// begins, whatever the zone did. A day's worth of slack on each side covers
	// any real transition.
	lo := t.Add(-26 * time.Hour).Truncate(time.Second)
	hi := t.Add(26 * time.Hour).Truncate(time.Second)
	for lo.Before(hi) {
		mid := lo.Add(hi.Sub(lo) / 2).Truncate(time.Second)
		if Of(mid, loc).Before(d) {
			lo = mid.Add(time.Second)
		} else {
			hi = mid
		}
	}
	return lo.In(loc)
}

func sign(n int) int {
	switch {
	case n < 0:
		return -1
	case n > 0:
		return 1
	default:
		return 0
	}
}
