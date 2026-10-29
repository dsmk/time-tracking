// Package cli is tt's command-line surface.
//
// tt is a thin front end: OpenTodo owns the data. Every operation runs through
// the documented ot command-line client, and tt keeps no time-entry database of
// its own.
package cli

import (
	"fmt"
	"strings"

	"github.com/spf13/cobra"

	"github.com/dsmk/time-tracking/internal/clock"
	"github.com/dsmk/time-tracking/internal/commands"
	"github.com/dsmk/time-tracking/internal/config"
	"github.com/dsmk/time-tracking/internal/ot"
	"github.com/dsmk/time-tracking/internal/output"
	"github.com/dsmk/time-tracking/internal/tterr"
)

// Version is the release tt reports, stamped at build time.
var Version = "dev"

// groupCommand marks a command that only groups others. Such a command needs
// no OpenTodo client, and running it bare is a usage error rather than a
// silent success.
const groupCommand = "tt/group"

// globals are the two settings every command can override for one run.
type globals struct {
	otCommand    string
	otCommandSet bool
	timezone     string
	timezoneSet  bool
}

// New builds the whole command tree.
//
// The App is built lazily, inside each command, so that --help and --version
// work without an ot client present and so that a configuration error is
// reported by tt rather than by the flag parser.
func New() *cobra.Command {
	var flags globals
	var app *commands.App

	root := &cobra.Command{
		Use:   "tt",
		Short: "Terminal client for OpenTodo time tracking",
		Long: "Terminal client for OpenTodo time tracking.\n\n" +
			"tt is a thin front end: OpenTodo owns the data. Every operation runs through\n" +
			"the documented `ot` command-line client, and tt keeps no time-entry database\n" +
			"of its own.",
		Version:       Version,
		SilenceUsage:  true,
		SilenceErrors: true,
	}
	root.SetVersionTemplate("tt {{.Version}}\n")
	// Cobra's generated `completion` command would run the pre-run hook below
	// and so demand a working ot client just to print a shell script.
	root.CompletionOptions.DisableDefaultCmd = true

	// Every leaf command needs the App; building it here keeps the wiring in
	// one place and the version probe on one code path.
	root.PersistentPreRunE = func(cmd *cobra.Command, _ []string) error {
		if !cmd.Runnable() || cmd.Annotations[groupCommand] != "" {
			return nil
		}
		// Cobra checks required and mutually exclusive flags only after this
		// hook, which would send a plainly malformed command off to ot before
		// rejecting it. Bad arguments must cost nothing and exit 2.
		if err := cmd.ValidateRequiredFlags(); err != nil {
			return tterr.Usage("%v", err)
		}
		if err := cmd.ValidateFlagGroups(); err != nil {
			return tterr.Usage("%v", err)
		}
		flags.otCommandSet = cmd.Flags().Changed("ot-command")
		flags.timezoneSet = cmd.Flags().Changed("timezone")
		built, err := build(flags)
		if err != nil {
			return err
		}
		app = built
		return nil
	}

	// A bad flag is a usage error with tt's own exit code, not a bare message
	// from the parser.
	root.SetFlagErrorFunc(func(cmd *cobra.Command, err error) error {
		return tterr.Usage("%v", err).
			WithHint("run `%s --help` for the accepted options", cmd.CommandPath())
	})
	root.PersistentFlags().StringVar(&flags.otCommand, "ot-command", "",
		"Path or command name for the OpenTodo CLI (overrides the config file)")
	root.PersistentFlags().StringVar(&flags.timezone, "timezone", "",
		"IANA timezone used for day and week boundaries (overrides the config file)")

	requireSubcommand(root)
	root.AddCommand(
		startCommand(&app),
		stopCommand(&app),
		statusCommand(&app),
		logCommand(&app),
		reportCommand(&app),
		syncCommand(&app),
	)
	return root
}

// build resolves configuration, checks the client, and wires up the adapter.
func build(flags globals) (*commands.App, error) {
	cfg, err := config.Load()
	if err != nil {
		return nil, err
	}
	if flags.otCommandSet {
		if strings.TrimSpace(flags.otCommand) == "" {
			return nil, tterr.Usage("--ot-command must not be empty")
		}
		cfg.OtCommand = flags.otCommand
	}
	if flags.timezoneSet {
		location, err := config.LoadLocation(flags.timezone)
		if err != nil {
			return nil, err
		}
		cfg.Location = location
		cfg.TimezoneName = flags.timezone
		cfg.TimezoneSource = config.FromConfig
	}

	client := ot.New(cfg.OtCommand, ot.NewProcessRunner(cfg.TimeoutSeconds))

	// Fail early and clearly when the client is missing or speaks a JSON
	// version these parsers were not written for.
	if cfg.VersionCheck {
		info, err := client.Version()
		if err != nil {
			return nil, err
		}
		warning, err := ot.CheckVersion(info)
		if err != nil {
			return nil, err
		}
		if warning != "" {
			output.Warn(warning)
		}
	}

	now, err := clock.FromEnv()
	if err != nil {
		return nil, err
	}
	return &commands.App{Config: cfg, Loc: cfg.Location, Clock: now, OT: client}, nil
}

// requireSubcommand makes a grouping command print its usage and exit 2 when
// it is run bare, rather than succeeding while doing nothing.
func requireSubcommand(cmd *cobra.Command) *cobra.Command {
	if cmd.Annotations == nil {
		cmd.Annotations = map[string]string{}
	}
	cmd.Annotations[groupCommand] = "yes"
	cmd.RunE = func(cmd *cobra.Command, _ []string) error {
		fmt.Fprint(cmd.ErrOrStderr(), cmd.UsageString())
		return tterr.Usage("`%s` needs a subcommand", cmd.CommandPath())
	}
	return cmd
}

func startCommand(app **commands.App) *cobra.Command {
	args := commands.StartArgs{}
	cmd := &cobra.Command{
		Use:   "start [NOTE]",
		Short: "Start a timer on an OpenTodo task",
		Long: "Start a timer on an OpenTodo task.\n\n" +
			"OpenTodo attaches time to a task, so NOTE is the entry's note, not a task name.",
		Args: cobra.MaximumNArgs(1),
		RunE: func(_ *cobra.Command, positional []string) error {
			if len(positional) > 0 {
				args.Note = positional[0]
			}
			return commands.Start(*app, args)
		},
	}
	cmd.Flags().StringVar(&args.Task, "task", "",
		"OpenTodo task identifier. Required unless tt can prompt in a terminal")
	cmd.Flags().BoolVar(&args.Switch, "switch", false,
		"Allow OpenTodo to end the entry that is already running")
	cmd.Flags().BoolVar(&args.JSON, "json", false, "Print JSON instead of text")
	return cmd
}

func stopCommand(app **commands.App) *cobra.Command {
	var asJSON bool
	cmd := &cobra.Command{
		Use:   "stop",
		Short: "Stop the running timer",
		Args:  cobra.NoArgs,
		RunE: func(_ *cobra.Command, _ []string) error {
			return commands.Stop(*app, asJSON)
		},
	}
	cmd.Flags().BoolVar(&asJSON, "json", false, "Print JSON instead of text")
	return cmd
}

func statusCommand(app **commands.App) *cobra.Command {
	var asJSON bool
	cmd := &cobra.Command{
		Use:   "status",
		Short: "Show the running timer",
		Args:  cobra.NoArgs,
		RunE: func(_ *cobra.Command, _ []string) error {
			return commands.Status(*app, asJSON)
		},
	}
	cmd.Flags().BoolVar(&asJSON, "json", false, "Print JSON instead of text")
	return cmd
}

func logCommand(app **commands.App) *cobra.Command {
	args := commands.LogArgs{}
	cmd := &cobra.Command{
		Use:   "log",
		Short: "List time entries",
		Args:  cobra.NoArgs,
		RunE: func(cmd *cobra.Command, _ []string) error {
			args.FormatGiven = cmd.Flags().Changed("format")
			args.LimitSet = cmd.Flags().Changed("limit")
			return commands.Log(*app, args)
		},
	}
	cmd.Flags().BoolVar(&args.Today, "today", false,
		"Entries touching today in the selected timezone")
	cmd.Flags().StringVar(&args.From, "from", "", "Inclusive start date, YYYY-MM-DD")
	cmd.Flags().StringVar(&args.To, "to", "", "Inclusive end date, YYYY-MM-DD")
	cmd.Flags().StringVar(&args.Task, "task", "", "Only entries for this OpenTodo task")
	cmd.Flags().StringVar(&args.Format, "format", "text", "Output format: text or json")
	cmd.Flags().IntVar(&args.Limit, "limit", 0,
		"Maximum number of entries to request from OpenTodo")
	cmd.MarkFlagsMutuallyExclusive("today", "from")
	cmd.MarkFlagsMutuallyExclusive("today", "to")
	return cmd
}

func reportCommand(app **commands.App) *cobra.Command {
	cmd := requireSubcommand(&cobra.Command{
		Use:   "report",
		Short: "Summarise tracked time",
		Args:  cobra.NoArgs,
	})
	cmd.AddCommand(reportWeekCommand(app), reportRangeCommand(app))
	return cmd
}

func reportWeekCommand(app **commands.App) *cobra.Command {
	args := commands.WeekArgs{}
	cmd := &cobra.Command{
		Use:   "week",
		Short: "Weekly report",
		Args:  cobra.NoArgs,
		RunE: func(cmd *cobra.Command, _ []string) error {
			args.FormatGiven = cmd.Flags().Changed("format")
			args.LimitSet = cmd.Flags().Changed("limit")
			return commands.ReportWeek(*app, args)
		},
	}
	cmd.Flags().StringVar(&args.Week, "week", "",
		"Any date inside the week to report on. Defaults to today")
	cmd.Flags().StringVar(&args.WeekStart, "week-start", "",
		"First day of the week (overrides the config file)")
	addReportFlags(cmd, &args.Format, &args.Limit)
	return cmd
}

func reportRangeCommand(app **commands.App) *cobra.Command {
	args := commands.RangeArgs{}
	cmd := &cobra.Command{
		Use:   "range",
		Short: "Report over an inclusive date range",
		Args:  cobra.NoArgs,
		RunE: func(cmd *cobra.Command, _ []string) error {
			args.FormatGiven = cmd.Flags().Changed("format")
			args.LimitSet = cmd.Flags().Changed("limit")
			return commands.ReportRange(*app, args)
		},
	}
	cmd.Flags().StringVar(&args.From, "from", "", "Inclusive start date, YYYY-MM-DD")
	cmd.Flags().StringVar(&args.To, "to", "", "Inclusive end date, YYYY-MM-DD")
	_ = cmd.MarkFlagRequired("from")
	_ = cmd.MarkFlagRequired("to")
	addReportFlags(cmd, &args.Format, &args.Limit)
	return cmd
}

func addReportFlags(cmd *cobra.Command, format *string, limit *int) {
	cmd.Flags().StringVar(format, "format", "text", "Output format: text, json or csv")
	cmd.Flags().IntVar(limit, "limit", 0,
		"Maximum number of entries to request from OpenTodo")
}

func syncCommand(app **commands.App) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "sync",
		Short: "Synchronisation: `tt sync status`, or `tt sync`",
		Args:  cobra.NoArgs,
		RunE: func(_ *cobra.Command, _ []string) error {
			return commands.Sync(*app)
		},
	}

	var asJSON bool
	status := &cobra.Command{
		Use:   "status",
		Short: "Report what OpenTodo discloses about synchronisation",
		Args:  cobra.NoArgs,
		RunE: func(_ *cobra.Command, _ []string) error {
			return commands.SyncStatus(*app, asJSON)
		},
	}
	status.Flags().BoolVar(&asJSON, "json", false, "Print JSON instead of text")
	cmd.AddCommand(status)
	return cmd
}
