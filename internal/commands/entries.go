package commands

import (
	"fmt"

	"github.com/dsmk/time-tracking/internal/config"
	"github.com/dsmk/time-tracking/internal/ot"
	"github.com/dsmk/time-tracking/internal/output"
	"github.com/dsmk/time-tracking/internal/report"
	"github.com/dsmk/time-tracking/internal/tterr"
)

// `tt log` — list time entries.

// defaultDays is the period `tt log` covers when no dates are given. Explicit
// beats guessing at whatever default `ot time list` applies.
const defaultDays = 7

// LogArgs is what `tt log` was asked to do.
type LogArgs struct {
	Today       bool
	From        string
	To          string
	Task        string
	Format      string
	Limit       int
	LimitSet    bool
	FormatGiven bool
}

// Log lists time entries.
func Log(app *App, args LogArgs) error {
	format := config.FormatText
	if args.FormatGiven {
		parsed, err := config.ParseFormat(args.Format)
		if err != nil {
			return tterr.Usage("%v", err)
		}
		if parsed == config.FormatCSV {
			return tterr.Usage("tt log supports --format text or json").
				WithHint("for CSV, use `tt report range --format csv`")
		}
		format = parsed
	}

	period, err := logPeriod(app, args)
	if err != nil {
		return err
	}
	limit, err := ResolveLimit(args.Limit, args.LimitSet, app.Config)
	if err != nil {
		return err
	}
	fetched, err := app.Fetch(period, args.Task, limit)
	if err != nil {
		return err
	}

	// ot interprets --from/--to in its own timezone, so the window was widened;
	// keep only entries that really touch the requested local days.
	lo, hi := period.Bounds(app.Loc)
	now := fetched.Now
	entries := make([]ot.TimeEntry, 0, len(fetched.Entries))
	for _, entry := range fetched.Entries {
		end := now
		if entry.EndedAt != nil {
			end = *entry.EndedAt
		} else if end.Before(entry.StartedAt) {
			end = entry.StartedAt
		}
		touches := report.OverlapSeconds(entry.StartedAt, end, lo, hi) > 0 ||
			(entry.IsRunning() && entry.StartedAt.Before(hi))
		if touches {
			entries = append(entries, entry)
		}
	}

	for _, warning := range fetched.Warnings {
		output.Warn(warning)
	}

	if format == config.FormatJSON {
		items := make([]output.Entry, 0, len(entries))
		for _, entry := range entries {
			items = append(items, output.EntryJSON(entry, app.Loc, now))
		}
		return output.PrintJSON(items)
	}

	fmt.Printf("%s to %s (%s)\n", period.From, period.To, app.Config.TimezoneName)
	fmt.Println()
	output.PrintEntriesText(entries, app.Loc, now)
	return nil
}

func logPeriod(app *App, args LogArgs) (report.Period, error) {
	today := app.Today()
	if args.Today {
		return report.NewPeriod(today, today)
	}

	switch {
	case args.From == "" && args.To == "":
		return report.NewPeriod(today.AddDays(-(defaultDays - 1)), today)
	case args.To == "":
		from, err := ParseDate(args.From)
		if err != nil {
			return report.Period{}, err
		}
		return report.NewPeriod(from, today)
	case args.From == "":
		to, err := ParseDate(args.To)
		if err != nil {
			return report.Period{}, err
		}
		return report.NewPeriod(to.AddDays(-(defaultDays - 1)), to)
	default:
		from, err := ParseDate(args.From)
		if err != nil {
			return report.Period{}, err
		}
		to, err := ParseDate(args.To)
		if err != nil {
			return report.Period{}, err
		}
		return report.NewPeriod(from, to)
	}
}
