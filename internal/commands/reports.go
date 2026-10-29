package commands

import (
	"time"

	"github.com/dsmk/time-tracking/internal/config"
	"github.com/dsmk/time-tracking/internal/output"
	"github.com/dsmk/time-tracking/internal/report"
	"github.com/dsmk/time-tracking/internal/tterr"
)

// `tt report week` and `tt report range`.

// WeekArgs is what `tt report week` was asked to do.
type WeekArgs struct {
	Week        string
	WeekStart   string
	Format      string
	FormatGiven bool
	Limit       int
	LimitSet    bool
}

// RangeArgs is what `tt report range` was asked to do.
type RangeArgs struct {
	From        string
	To          string
	Format      string
	FormatGiven bool
	Limit       int
	LimitSet    bool
}

// ReportWeek summarises one week.
func ReportWeek(app *App, args WeekArgs) error {
	anchor := app.Today()
	if args.Week != "" {
		parsed, err := ParseDate(args.Week)
		if err != nil {
			return err
		}
		anchor = parsed
	}

	weekStart := app.Config.WeekStart
	if args.WeekStart != "" {
		parsed, err := config.ParseWeekday(args.WeekStart)
		if err != nil {
			return err
		}
		weekStart = parsed
	}

	return emit(app, report.Week(anchor, weekStart), &weekStart,
		args.Format, args.FormatGiven, args.Limit, args.LimitSet)
}

// ReportRange summarises an inclusive date range.
func ReportRange(app *App, args RangeArgs) error {
	from, err := ParseDate(args.From)
	if err != nil {
		return err
	}
	to, err := ParseDate(args.To)
	if err != nil {
		return err
	}
	period, err := report.NewPeriod(from, to)
	if err != nil {
		return err
	}
	return emit(app, period, nil, args.Format, args.FormatGiven, args.Limit, args.LimitSet)
}

func emit(
	app *App,
	period report.Period,
	weekStart *time.Weekday,
	formatName string,
	formatGiven bool,
	limit int,
	limitSet bool,
) error {
	format := app.Config.ReportFormat
	if formatGiven {
		parsed, err := config.ParseFormat(formatName)
		if err != nil {
			return tterr.Usage("%v", err)
		}
		format = parsed
	}

	resolved, err := ResolveLimit(limit, limitSet, app.Config)
	if err != nil {
		return err
	}
	fetched, err := app.Fetch(period, "", resolved)
	if err != nil {
		return err
	}
	built := report.Build(period, app.Loc, app.Config.TimezoneName, weekStart,
		fetched.Entries, fetched.Now, fetched.Warnings, fetched.Incomplete)

	switch format {
	case config.FormatJSON:
		return output.PrintJSON(output.ReportJSON(built))
	case config.FormatCSV:
		// Warnings would corrupt stdout for a CSV consumer, so they also go to
		// stderr; the file itself carries them as `meta` rows.
		for _, warning := range built.Warnings {
			output.Warn(warning)
		}
		return output.PrintReportCSV(built)
	default:
		output.PrintReportText(built)
		return nil
	}
}
