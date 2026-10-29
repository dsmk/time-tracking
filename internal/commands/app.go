// Package commands implements tt's commands on top of the OpenTodo adapter.
package commands

import (
	"fmt"
	"sort"
	"time"

	"github.com/dsmk/time-tracking/internal/civil"
	"github.com/dsmk/time-tracking/internal/clock"
	"github.com/dsmk/time-tracking/internal/config"
	"github.com/dsmk/time-tracking/internal/ot"
	"github.com/dsmk/time-tracking/internal/report"
	"github.com/dsmk/time-tracking/internal/tterr"
)

// App is everything a command needs: resolved settings, a clock, and the
// adapter.
type App struct {
	Config config.Config
	Loc    *time.Location
	Clock  clock.Clock
	OT     ot.OpenTodo
}

// Fetched is entries plus the metadata a report needs to stay honest about
// them.
type Fetched struct {
	Entries []ot.TimeEntry
	// Now is OpenTodo's clock, when it reported one; otherwise tt's.
	Now      time.Time
	Warnings []string
	// Incomplete means OpenTodo may have had more entries than it returned, so
	// any total built from these is a lower bound.
	Incomplete bool
}

// Now is the current instant.
func (a *App) Now() time.Time { return a.Clock.Now() }

// Today is the current local calendar date.
func (a *App) Today() civil.Date { return civil.Of(a.Now(), a.Loc) }

// Fetch asks OpenTodo for every entry that could touch period.
//
// The query window is widened by a day on each side because ot applies its own
// timezone to --from/--to (docs/opentodo-compatibility.md §9); callers clip to
// the real boundaries themselves.
func (a *App) Fetch(period report.Period, task string, limit int) (Fetched, error) {
	window := period.QueryWindow()
	page, err := a.OT.ListTimeEntries(ot.EntryFilter{
		From:  &window.From,
		To:    &window.To,
		Task:  task,
		Limit: limit,
	})
	if err != nil {
		return Fetched{}, err
	}

	var warnings []string
	incomplete := false
	if len(page.Entries) >= limit {
		incomplete = true
		warnings = append(warnings, fmt.Sprintf(
			"OpenTodo returned %d entries, which is the requested limit, so this report may "+
				"be incomplete; re-run with a larger --limit", len(page.Entries)))
	}
	if page.NextCursor != "" {
		incomplete = true
		warnings = append(warnings,
			"OpenTodo reported more entries beyond this page and the ot CLI has no "+
				"documented pagination flag, so this report may be incomplete")
	}

	entries := page.Entries
	sort.SliceStable(entries, func(i, j int) bool {
		if !entries[i].StartedAt.Equal(entries[j].StartedAt) {
			return entries[i].StartedAt.Before(entries[j].StartedAt)
		}
		return entries[i].ID < entries[j].ID
	})

	now := a.Now()
	if page.ServerTime != nil {
		now = *page.ServerTime
	}
	return Fetched{Entries: entries, Now: now, Warnings: warnings, Incomplete: incomplete}, nil
}

// ParseDate reads a YYYY-MM-DD date from the command line.
func ParseDate(raw string) (civil.Date, error) {
	date, err := civil.Parse(raw)
	if err != nil {
		return civil.Date{}, tterr.Usage("%q is not a date: %v", raw, err).
			WithHint("use the form YYYY-MM-DD")
	}
	return date, nil
}

// ResolveLimit picks the --limit to send. provided says whether the user asked
// for one; otherwise the configured maximum applies.
func ResolveLimit(requested int, provided bool, cfg config.Config) (int, error) {
	if !provided {
		return cfg.MaxEntries, nil
	}
	if requested <= 0 {
		return 0, tterr.Usage("--limit must be greater than zero")
	}
	return requested, nil
}
