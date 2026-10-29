// Package ot is the OpenTodo adapter.
//
// Everything that knows about ot — its subcommands, flags, JSON shapes, exit
// codes and error codes — lives behind this package, as required by SPEC.md §8.
// Only the operations OpenTodo documents are implemented; the rest return an
// Unsupported error naming what is missing.
package ot

import (
	"errors"
	"fmt"
	"strconv"
	"strings"

	"github.com/dsmk/time-tracking/internal/civil"
	"github.com/dsmk/time-tracking/internal/tterr"
)

// DefaultCommand is the OpenTodo CLI tt runs unless configured otherwise.
const DefaultCommand = "ot"

// MinVersion is the lowest ot release with time tracking (SPEC.md §3 targets
// 0.9+).
var MinVersion = Semver{Major: 0, Minor: 9, Patch: 0}

// SupportedOutputVersion is the output_version this adapter's parsers were
// written against.
const SupportedOutputVersion int64 = 1

// EntryFilter says which entries to ask OpenTodo for.
type EntryFilter struct {
	From    *civil.Date
	To      *civil.Date
	Task    string
	Running bool
	// Limit of zero asks for no limit at all.
	Limit int
}

// ClientStateKind is what `ot auth status` could tell us. Deliberately *not* a
// sync state: a command succeeding says nothing about whether devices are in
// sync (SPEC.md §6.7).
type ClientStateKind int

// What the ot client reported about itself.
const (
	// StateConfigured means ot answered; it says nothing about sync.
	StateConfigured ClientStateKind = iota
	// StateUnreachable means the server could not be reached.
	StateUnreachable
	// StateAuthFailure means OpenTodo rejected the stored credentials.
	StateAuthFailure
	// StateUnknown means ot failed in a way that is not a state at all.
	StateUnknown
)

// ClientState is what the ot client disclosed about its own configuration.
type ClientState struct {
	Kind    ClientStateKind
	Server  string
	Account string
	Scope   string
	// Detail carries ot's own message for the non-configured states.
	Detail string
}

// OpenTodo is the set of conceptual operations from SPEC.md §8.
type OpenTodo interface {
	Version() (VersionInfo, error)
	StartTimer(task, note string) (TimeEntry, bool, error)
	StopTimer() (TimeEntry, bool, error)
	RunningTimer() (TimeEntry, bool, error)
	ListTimeEntries(filter EntryFilter) (EntryPage, error)
	ListTasks() ([]Task, error)
	ClientState() (ClientState, error)
	Sync() error
}

// Client implements OpenTodo by running the ot command-line client.
type Client struct {
	Program string
	Runner  Runner
}

// New builds a client that runs program through runner.
func New(program string, runner Runner) *Client {
	return &Client{Program: program, Runner: runner}
}

var _ OpenTodo = (*Client)(nil)

// invoke runs ot with an argument vector and requires a clean exit.
func (c *Client) invoke(args ...string) (Output, error) {
	invocation := Invocation{Program: c.Program, Args: args}
	output, err := c.Runner.Run(invocation)
	if err != nil {
		return Output{}, c.spawnError(err, invocation)
	}
	switch {
	case output.Signaled:
		return Output{}, tterr.New(tterr.KindClient,
			"`%s` was terminated before it finished", invocation.Display()).
			WithHint("no change was confirmed; check OpenTodo before retrying")
	case output.Code != 0:
		return Output{}, c.commandError(output, invocation)
	default:
		return output, nil
	}
}

func (c *Client) spawnError(err error, invocation Invocation) error {
	var runErr *RunError
	if !errors.As(err, &runErr) {
		return tterr.New(tterr.KindClient, "could not run `%s`: %v", invocation.Display(), err)
	}
	switch runErr.Kind {
	case RunNotFound:
		return tterr.New(tterr.KindClient,
			"the OpenTodo client %q was not found", c.Program).
			WithHint("install it with `go install github.com/OpenTodoApp/opentodo/cmd/ot@latest`, " +
				"or set `ot_command` in the tt config file to its full path")
	case RunNotExecutable:
		return tterr.New(tterr.KindClient,
			"the OpenTodo client %q could not be executed: %s", c.Program, runErr.Detail).
			WithHint("check that the file is executable")
	case RunTimeout:
		return tterr.New(tterr.KindClient,
			"`%s` did not finish within %ds", invocation.Display(), int(runErr.Timeout.Seconds())).
			WithHint("the command was killed and no result was confirmed; check OpenTodo " +
				"before retrying, or raise `timeout_seconds` in the tt config file")
	default:
		return tterr.New(tterr.KindClient,
			"could not run `%s`: %s", invocation.Display(), runErr.Detail)
	}
}

// commandError maps a non-zero ot exit into a tt error, preserving OpenTodo's
// own message and error code (redacted) as SPEC.md §8 requires.
func (c *Client) commandError(output Output, invocation Invocation) error {
	envelope, hasEnvelope := ParseError(output.Stdout, output.Stderr)

	detail := ""
	if hasEnvelope {
		detail = strings.TrimSpace(envelope.Message)
	}
	if detail == "" {
		for _, line := range strings.Split(output.Stderr, "\n") {
			if strings.TrimSpace(line) != "" {
				detail = strings.TrimSpace(line)
				break
			}
		}
	}
	if detail != "" {
		detail = Redact(detail)
	}

	kind := tterr.KindServer
	switch output.Code {
	case 1:
		kind = tterr.KindServer
	case 2:
		kind = tterr.KindUsage
	case 3:
		kind = tterr.KindAuth
	case 4:
		kind = tterr.KindUnreachable
	case 5:
		kind = tterr.KindForbidden
	}

	message := fmt.Sprintf("`%s` failed with exit status %d", invocation.Display(), output.Code)
	if detail != "" {
		message = "OpenTodo reported: " + detail
	}
	if envelope.Code != "" {
		message = fmt.Sprintf("%s (%s)", message, envelope.Code)
	}

	err := tterr.New(kind, "%s", message)
	if hint := errorHint(kind, envelope.Code, invocation); hint != "" {
		err = err.WithHint("%s", hint)
	}
	return err
}

func errorHint(kind tterr.Kind, code string, invocation Invocation) string {
	switch code {
	case "insufficient_scope":
		return "this operation writes, and the stored token is read-only; " +
			"`ot auth status` shows the token scope"
	case "read_only":
		return "the ot client is in read-only mode; unset OPENTODO_READ_ONLY"
	case "not_found":
		return "check the identifier; `ot list --json` shows current tasks"
	case "rate_limited":
		return "wait and retry"
	case "conflict", "idempotency_conflict":
		return fmt.Sprintf("OpenTodo reported a %s; open the entry in the OpenTodo web "+
			"interface to see which change won before retrying", code)
	}

	switch kind {
	case tterr.KindUnreachable:
		return "the OpenTodo server could not be reached. The ot client is online-only, so " +
			"tt cannot record this locally; check the network and `ot auth status`"
	case tterr.KindAuth:
		return "sign in again with `ot login <server-url>`, or check `ot auth status`"
	case tterr.KindForbidden:
		return "the stored token is not permitted to do this; check `ot auth status`"
	case tterr.KindUsage:
		return fmt.Sprintf("tt sent `%s`, which this ot version rejected. Compare it with "+
			"`ot schema` and report a tt bug if the command is wrong", invocation.Display())
	default:
		return ""
	}
}

// checkPositional rejects values that a flag parser could mistake for an
// option. Task identifiers are passed positionally, so a leading dash is
// refused outright rather than quietly becoming a flag.
func checkPositional(value, what string) error {
	if strings.TrimSpace(value) == "" {
		return tterr.Usage("%s must not be empty", what)
	}
	if strings.HasPrefix(value, "-") {
		return tterr.Usage(
			"%s %q starts with a dash, which OpenTodo would read as an option", what, value).
			WithHint("pass the identifier OpenTodo shows, for example 8c2a")
	}
	return nil
}

// TimeListArgs builds the argument vector for `ot time list`.
//
// Separate from the client so it can be unit-tested directly: this is the
// surface where an invented flag would do the most damage.
func TimeListArgs(filter EntryFilter) []string {
	args := []string{"time", "list", "--json"}
	if filter.From != nil {
		args = append(args, "--from="+filter.From.String())
	}
	if filter.To != nil {
		args = append(args, "--to="+filter.To.String())
	}
	if filter.Task != "" {
		args = append(args, "--task="+filter.Task)
	}
	if filter.Running {
		args = append(args, "--running")
	}
	if filter.Limit > 0 {
		args = append(args, "--limit="+strconv.Itoa(filter.Limit))
	}
	return args
}

// Version asks ot what it is.
func (c *Client) Version() (VersionInfo, error) {
	output, err := c.invoke("version", "--json")
	if err != nil {
		return VersionInfo{}, err
	}
	return ParseVersion(output.Stdout), nil
}

// StartTimer starts an entry on a task, with an optional note. It reports
// whether OpenTodo described the entry it created.
func (c *Client) StartTimer(task, note string) (TimeEntry, bool, error) {
	if err := checkPositional(task, "a task identifier"); err != nil {
		return TimeEntry{}, false, err
	}
	args := []string{"time", "start", task}
	if trimmed := strings.TrimSpace(note); trimmed != "" {
		// `--flag=value` keeps a note that begins with a dash from being read
		// as an option.
		args = append(args, "--note="+trimmed)
	}
	args = append(args, "--json")

	output, err := c.invoke(args...)
	if err != nil {
		return TimeEntry{}, false, err
	}
	entry, ok := ParseOptionalEntry(output.Stdout)
	return entry, ok, nil
}

// StopTimer ends the running entry. It reports whether OpenTodo described the
// entry it closed.
func (c *Client) StopTimer() (TimeEntry, bool, error) {
	output, err := c.invoke("time", "stop", "--json")
	if err != nil {
		return TimeEntry{}, false, err
	}
	entry, ok := ParseOptionalEntry(output.Stdout)
	return entry, ok, nil
}

// RunningTimer is the entry OpenTodo has open, if any.
func (c *Client) RunningTimer() (TimeEntry, bool, error) {
	page, err := c.ListTimeEntries(EntryFilter{Running: true})
	if err != nil {
		return TimeEntry{}, false, err
	}
	for _, entry := range page.Entries {
		if entry.IsRunning() {
			return entry, true, nil
		}
	}
	return TimeEntry{}, false, nil
}

// ListTimeEntries queries OpenTodo for entries matching the filter.
func (c *Client) ListTimeEntries(filter EntryFilter) (EntryPage, error) {
	if filter.Task != "" {
		if err := checkPositional(filter.Task, "a task identifier"); err != nil {
			return EntryPage{}, err
		}
	}
	output, err := c.invoke(TimeListArgs(filter)...)
	if err != nil {
		return EntryPage{}, err
	}
	return ParseEntryPage(output.Stdout)
}

// ListTasks reads the tasks the interactive picker offers.
func (c *Client) ListTasks() ([]Task, error) {
	output, err := c.invoke("list", "--json")
	if err != nil {
		return nil, err
	}
	return ParseTasks(output.Stdout)
}

// ClientState reports what `ot auth status` disclosed. A reachability or
// credential failure is a state rather than an error; a missing or broken
// client is a real failure.
func (c *Client) ClientState() (ClientState, error) {
	output, err := c.invoke("auth", "status", "--json")
	if err == nil {
		state := ClientState{Kind: StateConfigured}
		if fields, ok := newObject([]byte(strings.TrimSpace(output.Stdout))); ok {
			state.Server = fields.str("server", "url", "serverUrl")
			state.Account = fields.str("account", "email", "user", "userEmail")
			state.Scope = fields.str("scope", "tokenScope")
		}
		return state, nil
	}

	detail := err.Error()
	var ttError *tterr.Error
	if errors.As(err, &ttError) {
		detail = ttError.Message
	}
	switch tterr.KindOf(err) {
	case tterr.KindUnreachable:
		return ClientState{Kind: StateUnreachable, Detail: detail}, nil
	case tterr.KindAuth, tterr.KindForbidden:
		return ClientState{Kind: StateAuthFailure, Detail: detail}, nil
	case tterr.KindClient:
		return ClientState{}, err
	default:
		return ClientState{Kind: StateUnknown, Detail: detail}, nil
	}
}

// Sync always fails: OpenTodo has no documented command to synchronise tasks or
// time entries. See docs/opentodo-compatibility.md §6.
func (c *Client) Sync() error {
	return tterr.Unsupported(
		"OpenTodo has no documented command to synchronise tasks or time entries").
		WithHint("the ot client is online-only: every write goes straight to the server, so " +
			"there is nothing to push. `ot vault sync` and `ot issues sync` are unrelated features")
}

// CheckVersion compares the client against what this adapter can parse.
//
// It returns a warning when the version could not be determined, because
// skipping the check silently would hide exactly the mismatch it exists to
// catch.
func CheckVersion(info VersionInfo) (string, error) {
	if info.OutputVersion != nil && *info.OutputVersion != SupportedOutputVersion {
		return "", tterr.New(tterr.KindUnsupported,
			"this OpenTodo client reports output_version %d, but tt understands only version %d",
			*info.OutputVersion, SupportedOutputVersion).
			WithHint("upgrade tt, or run an ot release whose --json output is version 1")
	}

	found, ok := ParseSemver(info.ClientVersion)
	switch {
	case !ok:
		return "could not determine the OpenTodo client version from `ot version --json`; " +
			"continuing without the minimum-version check", nil
	case found.Less(MinVersion):
		return "", tterr.New(tterr.KindUnsupported,
			"OpenTodo client version %s is too old; tt needs %d.%d.%d or later",
			info.ClientVersion, MinVersion.Major, MinVersion.Minor, MinVersion.Patch).
			WithHint("time tracking was added in OpenTodo 0.9.0")
	default:
		return "", nil
	}
}
