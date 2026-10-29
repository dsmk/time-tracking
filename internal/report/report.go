// Package report holds tt's report arithmetic.
//
// OpenTodo has no documented CLI reporting command, so SPEC.md §6.5 option 2
// applies: reports are derived from `ot time list` data. Two consequences shape
// this package.
//
//  1. The CLI has no documented --tz flag, so tt does not rely on how ot
//     interprets --from/--to. It asks for a slightly wider window and does all
//     boundary work itself, in the configured timezone, from the absolute
//     startedAt/endedAt timestamps.
//  2. Durations are computed from those timestamps rather than from OpenTodo's
//     rounded `minutes`, because entries have to be clipped at period edges.
package report

import (
	"fmt"
	"math"
	"sort"
	"time"

	"github.com/dsmk/time-tracking/internal/civil"
	"github.com/dsmk/time-tracking/internal/ot"
	"github.com/dsmk/time-tracking/internal/tterr"
)

// Period is an inclusive range of local calendar days.
type Period struct {
	From civil.Date
	To   civil.Date
}

// NewPeriod builds a period, rejecting a range that ends before it starts.
func NewPeriod(from, to civil.Date) (Period, error) {
	if to.Before(from) {
		return Period{}, tterr.Usage(
			"the range ends before it starts: --from %s is after --to %s", from, to)
	}
	return Period{From: from, To: to}, nil
}

// Week is the week containing anchor, starting on weekStart.
func Week(anchor civil.Date, weekStart time.Weekday) Period {
	offset := (mondayZero(anchor.Weekday()) - mondayZero(weekStart) + 7) % 7
	from := anchor.AddDays(-offset)
	return Period{From: from, To: from.AddDays(6)}
}

// mondayZero numbers the weekdays from Monday, so week arithmetic does not
// depend on time.Weekday starting at Sunday.
func mondayZero(day time.Weekday) int { return (int(day) + 6) % 7 }

// Days lists every local calendar day in the period.
func (p Period) Days() []civil.Date {
	var days []civil.Date
	for day := p.From; !day.After(p.To); day = day.AddDays(1) {
		days = append(days, day)
	}
	return days
}

// Bounds are the half-open instant bounds of the period in loc.
func (p Period) Bounds(loc *time.Location) (time.Time, time.Time) {
	return civil.DayStart(p.From, loc), civil.DayStart(p.To.AddDays(1), loc)
}

// QueryWindow is the window to request from OpenTodo: one day wider on each
// side, since ot applies its own (undocumented) timezone to --from/--to and an
// entry near a boundary could otherwise be missed.
func (p Period) QueryWindow() Period {
	return Period{From: p.From.AddDays(-1), To: p.To.AddDays(1)}
}

// OverlapSeconds is the number of seconds of [start, end) that fall inside
// [lo, hi).
func OverlapSeconds(start, end, lo, hi time.Time) int64 {
	begin := start
	if lo.After(begin) {
		begin = lo
	}
	finish := end
	if hi.Before(finish) {
		finish = hi
	}
	if !finish.After(begin) {
		return 0
	}
	return finish.Unix() - begin.Unix()
}

// DayTotal is one row of the by-day breakdown.
type DayTotal struct {
	Date    civil.Date
	Seconds int64
}

// TaskTotal is one row of the by-task breakdown.
type TaskTotal struct {
	TaskID  string
	Label   string
	Project string
	Seconds int64
	Entries int
}

// Report is a finished set of totals for one period.
type Report struct {
	Period   Period
	Timezone string
	// WeekStart is set for weekly reports; range reports have no week start.
	WeekStart       *time.Weekday
	TotalSeconds    int64
	Days            []DayTotal
	Tasks           []TaskTotal
	UntaskedSeconds int64
	RunningEntries  int
	EntryCount      int
	// Incomplete is true when the figures are known to be partial: a timer is
	// still running, or OpenTodo returned as many entries as the limit allowed.
	Incomplete bool
	Warnings   []string
}

// Build turns entries OpenTodo returned into a report.
//
// now ends any still-running entry. Pass OpenTodo's serverTime when it supplied
// one, so elapsed time is measured on the same clock that produced the entry.
// dataIncomplete says the caller already knows entries may be missing — a
// filled --limit, or a pagination cursor tt cannot follow.
func Build(
	period Period,
	loc *time.Location,
	timezone string,
	weekStart *time.Weekday,
	entries []ot.TimeEntry,
	now time.Time,
	warnings []string,
	dataIncomplete bool,
) Report {
	lo, hi := period.Bounds(loc)

	dates := period.Days()
	days := make([]DayTotal, len(dates))
	dayBounds := make([][2]time.Time, len(dates))
	for i, date := range dates {
		days[i] = DayTotal{Date: date}
		dayBounds[i] = [2]time.Time{
			civil.DayStart(date, loc),
			civil.DayStart(date.AddDays(1), loc),
		}
	}

	var (
		tasks          []TaskTotal
		taskIndex      = map[string]int{}
		untasked       int64
		total          int64
		runningEntries int
		counted        int
	)

	for _, entry := range entries {
		end := now
		if entry.EndedAt != nil {
			end = *entry.EndedAt
		} else {
			// A running entry is only counted up to "now"; if the clock is
			// behind the start, count nothing rather than negative time.
			runningEntries++
			if end.Before(entry.StartedAt) {
				end = entry.StartedAt
			}
		}

		seconds := OverlapSeconds(entry.StartedAt, end, lo, hi)
		if seconds == 0 {
			continue
		}
		counted++
		total += seconds

		for i := range days {
			days[i].Seconds += OverlapSeconds(
				entry.StartedAt, end, dayBounds[i][0], dayBounds[i][1])
		}

		if entry.TaskID == "" {
			untasked += seconds
			continue
		}
		if index, ok := taskIndex[entry.TaskID]; ok {
			tasks[index].Seconds += seconds
			tasks[index].Entries++
			continue
		}
		label := entry.TaskTitle
		if label == "" {
			label = "task " + entry.TaskID
		}
		taskIndex[entry.TaskID] = len(tasks)
		tasks = append(tasks, TaskTotal{
			TaskID:  entry.TaskID,
			Label:   label,
			Project: entry.ProjectName,
			Seconds: seconds,
			Entries: 1,
		})
	}

	// Longest first, then by label so equal totals have a stable order.
	sort.SliceStable(tasks, func(a, b int) bool {
		if tasks[a].Seconds != tasks[b].Seconds {
			return tasks[a].Seconds > tasks[b].Seconds
		}
		return tasks[a].Label < tasks[b].Label
	})

	if runningEntries > 0 {
		plural := "ies are"
		if runningEntries == 1 {
			plural = "y is"
		}
		warnings = append(warnings, fmt.Sprintf(
			"%d entr%s still running; time is counted up to %s",
			runningEntries, plural, now.In(loc).Format("2006-01-02 15:04 MST")))
	}

	return Report{
		Period:          period,
		Timezone:        timezone,
		WeekStart:       weekStart,
		TotalSeconds:    total,
		Days:            days,
		Tasks:           tasks,
		UntaskedSeconds: untasked,
		RunningEntries:  runningEntries,
		EntryCount:      counted,
		Incomplete:      dataIncomplete || runningEntries > 0,
		Warnings:        warnings,
	}
}

// FormatDuration renders seconds as `7h 25m`, or `0m` for nothing.
func FormatDuration(seconds int64) string {
	minutes := seconds / 60
	hours, remainder := minutes/60, minutes%60
	if hours > 0 {
		return fmt.Sprintf("%dh %02dm", hours, remainder)
	}
	return fmt.Sprintf("%dm", remainder)
}

// Hours is seconds as decimal hours, for CSV consumers that want to sum a
// column.
func Hours(seconds int64) float64 {
	return math.Round(float64(seconds)/36.0) / 100.0
}

// EntrySeconds is the time an entry covers, measured to now while it is
// running.
func EntrySeconds(entry ot.TimeEntry, now time.Time) int64 {
	end := now
	if entry.EndedAt != nil {
		end = *entry.EndedAt
	} else if end.Before(entry.StartedAt) {
		end = entry.StartedAt
	}
	if seconds := end.Unix() - entry.StartedAt.Unix(); seconds > 0 {
		return seconds
	}
	return 0
}
