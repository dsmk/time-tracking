package commands

import (
	"bufio"
	"fmt"
	"os"
	"strconv"
	"strings"

	"github.com/dsmk/time-tracking/internal/civil"
	"github.com/dsmk/time-tracking/internal/ot"
	"github.com/dsmk/time-tracking/internal/output"
	"github.com/dsmk/time-tracking/internal/report"
	"github.com/dsmk/time-tracking/internal/tterr"
)

// `tt start`, `tt stop`, `tt status`.

// StartArgs is what `tt start` was asked to do.
type StartArgs struct {
	Note   string
	Task   string
	Switch bool
	JSON   bool
}

// Start begins a timer on an OpenTodo task.
func Start(app *App, args StartArgs) error {
	task, err := resolveTask(app, args.Task)
	if err != nil {
		return err
	}

	// OpenTodo ends the current entry when another task is started, so tt makes
	// that explicit rather than letting a typo silently close an entry.
	running, isRunning, err := app.OT.RunningTimer()
	if err != nil {
		return err
	}
	if isRunning {
		if !args.Switch {
			return tterr.New(tterr.KindUsage,
				"a timer is already running: %s (since %s)",
				running.Label(), running.StartedAt.In(app.Loc).Format("15:04")).
				WithHint("stop it with `tt stop`, or pass --switch to let OpenTodo end it")
		}
		output.Warn(fmt.Sprintf(
			"OpenTodo will end the running entry for %s before starting the new one",
			running.Label()))
	}

	entry, described, err := app.OT.StartTimer(task, args.Note)
	if err != nil {
		return err
	}
	// `ot time start --json` output is undocumented: when it tells us nothing,
	// ask OpenTodo what is running rather than reporting a made-up entry.
	if !described {
		entry, described, err = app.OT.RunningTimer()
		if err != nil {
			return err
		}
	}

	switch {
	case described && args.JSON:
		return output.PrintJSON(output.EntryJSON(entry, app.Loc, app.Now()))
	case described:
		fmt.Printf("Started %s at %s\n",
			entry.Label(), entry.StartedAt.In(app.Loc).Format("15:04 MST"))
		if entry.TaskID != "" {
			fmt.Printf("  task %s  entry %s\n", entry.TaskID, entry.ID)
		}
		return nil
	case args.JSON:
		return output.PrintJSON(map[string]any{
			"started": true,
			"task":    task,
			"entry":   nil,
			"warning": "OpenTodo accepted the start but returned no entry details",
		})
	default:
		fmt.Printf("OpenTodo accepted the start for task %s.\n", task)
		output.Warn("OpenTodo returned no entry details; check `tt status`")
		return nil
	}
}

// Stop ends the running timer.
func Stop(app *App, asJSON bool) error {
	// Check before writing: `ot time stop` uses exit 1 both for "nothing runs"
	// and for a server failure, and a failed write is worth not sending at all.
	running, isRunning, err := app.OT.RunningTimer()
	if err != nil {
		return err
	}
	if !isRunning {
		return tterr.New(tterr.KindNoTimer, "no timer is running, so nothing was stopped").
			WithHint("start one with `tt start --task <TASK_ID>`")
	}

	entry, described, err := app.OT.StopTimer()
	if err != nil {
		return err
	}
	if !described {
		// Undocumented output: re-read the entry instead of computing a
		// duration tt cannot verify.
		entry, described, err = refetch(app, running)
		if err != nil {
			return err
		}
	}

	switch {
	case described && asJSON:
		return output.PrintJSON(output.EntryJSON(entry, app.Loc, app.Now()))
	case described:
		fmt.Printf("Stopped %s\n", entry.Label())
		fmt.Printf("  %s\n", output.EntryLine(entry, app.Loc, app.Now()))
		return nil
	case asJSON:
		return output.PrintJSON(map[string]any{
			"stopped": true,
			"entryId": running.ID,
			"entry":   nil,
			"warning": "OpenTodo accepted the stop but returned no entry details",
		})
	default:
		fmt.Printf("Stopped the timer for %s.\n", running.Label())
		output.Warn("OpenTodo returned no entry details, so the recorded duration is not " +
			"shown; check `tt log --today`")
		return nil
	}
}

// Status shows the running timer, if there is one.
func Status(app *App, asJSON bool) error {
	entry, isRunning, err := app.OT.RunningTimer()
	if err != nil {
		return err
	}
	now := app.Now()

	switch {
	case isRunning && asJSON:
		return output.PrintJSON(map[string]any{
			"running": true,
			"entry":   output.EntryJSON(entry, app.Loc, now),
		})
	case isRunning:
		fmt.Printf("Running: %s for %s (since %s)\n",
			entry.Label(),
			report.FormatDuration(report.EntrySeconds(entry, now)),
			entry.StartedAt.In(app.Loc).Format("15:04 MST"))
		if entry.TaskID != "" {
			fmt.Printf("  task %s  entry %s\n", entry.TaskID, entry.ID)
		}
		return nil
	case asJSON:
		return output.PrintJSON(map[string]any{"running": false, "entry": nil})
	default:
		fmt.Println("No timer is running.")
		return nil
	}
}

// refetch re-reads an entry OpenTodo did not describe, by listing the days
// around it.
func refetch(app *App, running ot.TimeEntry) (ot.TimeEntry, bool, error) {
	started := civil.Of(running.StartedAt, app.Loc)
	from, to := started.AddDays(-1), started.AddDays(1)
	page, err := app.OT.ListTimeEntries(ot.EntryFilter{
		From:  &from,
		To:    &to,
		Task:  running.TaskID,
		Limit: 100,
	})
	if err != nil {
		return ot.TimeEntry{}, false, err
	}
	for _, entry := range page.Entries {
		if entry.ID == running.ID {
			return entry, true, nil
		}
	}
	return ot.TimeEntry{}, false, nil
}

// resolveTask finds the task to track against: the flag, or an interactive
// choice.
//
// OpenTodo attaches every time entry to a task and documents no way to start
// one without a task, so there is no "just start something" path here.
func resolveTask(app *App, explicit string) (string, error) {
	if explicit != "" {
		return explicit, nil
	}
	if !isTerminal(os.Stdin) || !isTerminal(os.Stderr) {
		return "", tterr.Usage(
			"no task was given, and tt cannot prompt when it is not run from a terminal").
			WithHint("pass --task <TASK_ID>; `ot list --json` shows the available tasks")
	}

	tasks, err := app.OT.ListTasks()
	if err != nil {
		return "", err
	}
	if len(tasks) == 0 {
		return "", tterr.Usage("OpenTodo returned no tasks to choose from").
			WithHint("create one with `ot add \"...\"`, then pass --task")
	}

	// Prompts go to stderr so stdout stays clean for piped output.
	shown := tasks
	if len(shown) > 20 {
		shown = shown[:20]
	}
	fmt.Fprintln(os.Stderr, "Select a task:")
	for i, task := range shown {
		title := task.Title
		if title == "" {
			title = "(untitled)"
		}
		project := ""
		if task.ProjectName != "" {
			project = " (" + task.ProjectName + ")"
		}
		fmt.Fprintf(os.Stderr, "  %2d. %s%s  [%s]\n", i+1, title, project, task.ID)
	}
	if len(tasks) > len(shown) {
		fmt.Fprintf(os.Stderr,
			"  ... and %d more; pass --task to choose one of those\n", len(tasks)-len(shown))
	}
	fmt.Fprint(os.Stderr, "Number: ")

	line, err := bufio.NewReader(os.Stdin).ReadString('\n')
	if err != nil && line == "" {
		return "", tterr.Usage("could not read your choice: %v", err)
	}
	line = strings.TrimSpace(line)
	choice, convErr := strconv.Atoi(line)
	if convErr != nil || choice < 1 || choice > len(shown) {
		return "", tterr.Usage("%q is not one of the listed numbers", line)
	}
	return shown[choice-1].ID, nil
}

// isTerminal reports whether a file is attached to a character device, which is
// as much as the standard library can tell us about being run interactively.
func isTerminal(file *os.File) bool {
	info, err := file.Stat()
	return err == nil && info.Mode()&os.ModeCharDevice != 0
}
