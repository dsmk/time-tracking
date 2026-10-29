// Package output renders tt's results.
//
// Human output goes to stdout; warnings and anything uncertain go to stderr so
// that --format json output stays machine-readable.
package output

import (
	"encoding/csv"
	"encoding/json"
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/dsmk/time-tracking/internal/config"
	"github.com/dsmk/time-tracking/internal/ot"
	"github.com/dsmk/time-tracking/internal/report"
	"github.com/dsmk/time-tracking/internal/tterr"
)

// Layouts tt prints times with. localLayout is RFC 3339 without the Z
// shorthand, so a local time always carries a numeric offset.
const (
	instantLayout = time.RFC3339
	localLayout   = "2006-01-02T15:04:05-07:00"
)

// Warn prints a note to stderr, where it cannot corrupt machine-readable
// stdout.
func Warn(message string) {
	fmt.Fprintf(os.Stderr, "tt: note: %s\n", message)
}

// nullable renders an absent string as JSON null rather than "".
func nullable(value string) *string {
	if value == "" {
		return nil
	}
	return &value
}

// Entry is one time entry as tt prints it.
type Entry struct {
	ID           string  `json:"id"`
	TaskID       *string `json:"taskId"`
	TaskTitle    *string `json:"taskTitle"`
	ProjectID    *string `json:"projectId"`
	ProjectName  *string `json:"projectName"`
	Note         *string `json:"note"`
	StartedAt    string  `json:"startedAt"`
	StartedLocal string  `json:"startedLocal"`
	EndedAt      *string `json:"endedAt"`
	Running      bool    `json:"running"`
	Seconds      int64   `json:"seconds"`
	// OpenTodoMinutes is OpenTodo's own rounded duration, passed through
	// unchanged.
	OpenTodoMinutes *int64 `json:"openTodoMinutes"`
}

// EntryJSON converts an entry into its printable form.
func EntryJSON(entry ot.TimeEntry, loc *time.Location, now time.Time) Entry {
	var ended *string
	if entry.EndedAt != nil {
		text := entry.EndedAt.UTC().Format(instantLayout)
		ended = &text
	}
	return Entry{
		ID:              entry.ID,
		TaskID:          nullable(entry.TaskID),
		TaskTitle:       nullable(entry.TaskTitle),
		ProjectID:       nullable(entry.ProjectID),
		ProjectName:     nullable(entry.ProjectName),
		Note:            nullable(entry.Note),
		StartedAt:       entry.StartedAt.UTC().Format(instantLayout),
		StartedLocal:    entry.StartedAt.In(loc).Format(localLayout),
		EndedAt:         ended,
		Running:         entry.IsRunning(),
		Seconds:         report.EntrySeconds(entry, now),
		OpenTodoMinutes: entry.Minutes,
	}
}

// EntryLine is one line per entry:
// `Mon 2026-10-26  09:00-09:25  25m  label  [task]`.
func EntryLine(entry ot.TimeEntry, loc *time.Location, now time.Time) string {
	start := entry.StartedAt.In(loc)
	end := "now"
	if entry.EndedAt != nil {
		end = entry.EndedAt.In(loc).Format("15:04")
	}
	line := fmt.Sprintf("%s  %s-%s  %7s  %s",
		start.Format("Mon 2006-01-02"),
		start.Format("15:04"),
		end,
		report.FormatDuration(report.EntrySeconds(entry, now)),
		entry.Label())
	if entry.ProjectName != "" {
		line += "  (" + entry.ProjectName + ")"
	}
	if entry.TaskID != "" {
		line += "  [" + entry.TaskID + "]"
	}
	if entry.IsRunning() {
		line += "  <- running"
	}
	return line
}

// PrintEntriesText lists entries for a human, with a total.
func PrintEntriesText(entries []ot.TimeEntry, loc *time.Location, now time.Time) {
	if len(entries) == 0 {
		fmt.Println("No time entries.")
		return
	}
	var total int64
	for _, entry := range entries {
		fmt.Println(EntryLine(entry, loc, now))
		total += report.EntrySeconds(entry, now)
	}
	fmt.Println()
	fmt.Printf("%d entries, %s total\n", len(entries), report.FormatDuration(total))
}

// PrintJSON writes a value as indented JSON on stdout.
//
// HTML escaping is off: a note or a server URL containing `&`, `<` or `>` is
// data tt is passing through from OpenTodo, and turning it into `\u0026` would
// make the output harder to read without making it any safer.
func PrintJSON(value any) error {
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetEscapeHTML(false)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(value); err != nil {
		return tterr.New(tterr.KindServer, "could not render JSON: %v", err)
	}
	return nil
}

// Day is one row of a report's by-day breakdown, as printed.
type Day struct {
	Date    string  `json:"date"`
	Weekday string  `json:"weekday"`
	Seconds int64   `json:"seconds"`
	Hours   float64 `json:"hours"`
}

// Task is one row of a report's by-task breakdown, as printed.
type Task struct {
	TaskID  *string `json:"taskId"`
	Title   string  `json:"title"`
	Project *string `json:"project"`
	Seconds int64   `json:"seconds"`
	Hours   float64 `json:"hours"`
	Entries int     `json:"entries"`
}

// Report is a finished report in its printable form.
type Report struct {
	From         string  `json:"from"`
	To           string  `json:"to"`
	Timezone     string  `json:"timezone"`
	WeekStart    *string `json:"weekStart"`
	TotalSeconds int64   `json:"totalSeconds"`
	TotalHours   float64 `json:"totalHours"`
	EntryCount   int     `json:"entryCount"`
	Days         []Day   `json:"days"`
	Tasks        []Task  `json:"tasks"`
	// UntaskedSeconds is time OpenTodo recorded against no task.
	UntaskedSeconds int64 `json:"untaskedSeconds"`
	RunningEntries  int   `json:"runningEntries"`
	// Incomplete is never silently dropped: a consumer can refuse to invoice an
	// incomplete week.
	Incomplete bool     `json:"incomplete"`
	Warnings   []string `json:"warnings"`
}

// ReportJSON converts a report into its printable form.
func ReportJSON(built report.Report) Report {
	var weekStart *string
	if built.WeekStart != nil {
		name := config.WeekdayName(*built.WeekStart)
		weekStart = &name
	}

	days := make([]Day, 0, len(built.Days))
	for _, day := range built.Days {
		days = append(days, Day{
			Date:    day.Date.String(),
			Weekday: day.Date.Format("Monday"),
			Seconds: day.Seconds,
			Hours:   report.Hours(day.Seconds),
		})
	}

	tasks := make([]Task, 0, len(built.Tasks))
	for _, task := range built.Tasks {
		tasks = append(tasks, Task{
			TaskID:  nullable(task.TaskID),
			Title:   task.Label,
			Project: nullable(task.Project),
			Seconds: task.Seconds,
			Hours:   report.Hours(task.Seconds),
			Entries: task.Entries,
		})
	}

	warnings := built.Warnings
	if warnings == nil {
		warnings = []string{}
	}

	return Report{
		From:            built.Period.From.String(),
		To:              built.Period.To.String(),
		Timezone:        built.Timezone,
		WeekStart:       weekStart,
		TotalSeconds:    built.TotalSeconds,
		TotalHours:      report.Hours(built.TotalSeconds),
		EntryCount:      built.EntryCount,
		Days:            days,
		Tasks:           tasks,
		UntaskedSeconds: built.UntaskedSeconds,
		RunningEntries:  built.RunningEntries,
		Incomplete:      built.Incomplete,
		Warnings:        warnings,
	}
}

// PrintReportText renders a report for a terminal.
func PrintReportText(built report.Report) {
	heading := fmt.Sprintf("%s to %s (%s)",
		built.Period.From, built.Period.To, built.Timezone)
	if built.WeekStart != nil {
		heading = fmt.Sprintf("Week of %s to %s (week starts %s, %s)",
			built.Period.From, built.Period.To,
			config.WeekdayName(*built.WeekStart), built.Timezone)
	}
	fmt.Println(heading)
	fmt.Println(strings.Repeat("-", len(heading)))
	fmt.Printf("Total: %s\n", report.FormatDuration(built.TotalSeconds))
	fmt.Println()

	fmt.Println("By day")
	for _, day := range built.Days {
		fmt.Printf("  %s  %8s\n",
			day.Date.Format("Mon 2006-01-02"), report.FormatDuration(day.Seconds))
	}
	fmt.Println()

	fmt.Println("By task")
	if len(built.Tasks) == 0 && built.UntaskedSeconds == 0 {
		fmt.Println("  (nothing tracked)")
	}
	for _, task := range built.Tasks {
		project := ""
		if task.Project != "" {
			project = " (" + task.Project + ")"
		}
		id := task.TaskID
		if id == "" {
			id = "-"
		}
		fmt.Printf("  %8s  %s%s  [%s]\n",
			report.FormatDuration(task.Seconds), task.Label, project, id)
	}
	if built.UntaskedSeconds > 0 {
		fmt.Printf("  %8s  (no task)\n", report.FormatDuration(built.UntaskedSeconds))
	}

	if len(built.Warnings) > 0 {
		fmt.Println()
		fmt.Println("Notes")
		for _, warning := range built.Warnings {
			fmt.Printf("  - %s\n", warning)
		}
	}
}

// PrintReportCSV writes a tidy CSV: one row per day, one per task, one total.
// The `section` column lets a spreadsheet filter the parts apart.
func PrintReportCSV(built report.Report) error {
	writer := csv.NewWriter(os.Stdout)
	rows := [][]string{{"section", "key", "label", "seconds", "hours"}}

	for _, day := range built.Days {
		rows = append(rows, []string{
			"day", day.Date.String(), day.Date.Format("Monday"),
			strconv.FormatInt(day.Seconds, 10), formatHours(day.Seconds),
		})
	}
	for _, task := range built.Tasks {
		rows = append(rows, []string{
			"task", task.TaskID, task.Label,
			strconv.FormatInt(task.Seconds, 10), formatHours(task.Seconds),
		})
	}
	if built.UntaskedSeconds > 0 {
		rows = append(rows, []string{
			"task", "", "(no task)",
			strconv.FormatInt(built.UntaskedSeconds, 10), formatHours(built.UntaskedSeconds),
		})
	}
	rows = append(rows, []string{
		"total", built.Period.From.String(), built.Period.To.String(),
		strconv.FormatInt(built.TotalSeconds, 10), formatHours(built.TotalSeconds),
	})
	// An incomplete report says so inside the file, not only on the terminal.
	rows = append(rows, []string{
		"meta", "incomplete", strconv.FormatBool(built.Incomplete), "", "",
	})
	for _, warning := range built.Warnings {
		rows = append(rows, []string{"meta", "warning", warning, "", ""})
	}

	if err := writer.WriteAll(rows); err != nil {
		return tterr.New(tterr.KindServer, "could not write CSV: %v", err)
	}
	writer.Flush()
	if err := writer.Error(); err != nil {
		return tterr.New(tterr.KindServer, "could not write CSV: %v", err)
	}
	return nil
}

func formatHours(seconds int64) string {
	return strconv.FormatFloat(report.Hours(seconds), 'f', -1, 64)
}
