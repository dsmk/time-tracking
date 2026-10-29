package ot

import (
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
	"time"

	"github.com/dsmk/time-tracking/internal/tterr"
)

// Parsing of the documented OpenTodo JSON shapes.
//
// Field names and types come from docs/opentodo-compatibility.md §3, which
// records which parts are documented and which are defensive. Where the docs
// are silent (bare array vs {entries,…} envelope, snake_case vs camelCase in
// the CLI) the parser accepts both rather than guessing one and failing hard.

// TimeEntry is one OpenTodo time entry. The optional strings are empty when
// OpenTodo did not supply them.
type TimeEntry struct {
	ID          string
	TaskID      string
	TaskTitle   string
	ProjectID   string
	ProjectName string
	Note        string
	StartedAt   time.Time
	// EndedAt is nil while the entry is running (documented: endedAt is null).
	EndedAt *time.Time
	// Minutes is OpenTodo's own whole-minute duration. Reported, but never used
	// for report arithmetic: reports clip at period boundaries and need the
	// timestamps.
	Minutes *int64
}

// IsRunning reports whether OpenTodo has not ended this entry yet.
func (e TimeEntry) IsRunning() bool { return e.EndedAt == nil }

// Label is the description for human output: the note, else the task title,
// else the id.
func (e TimeEntry) Label() string {
	switch {
	case e.TaskTitle != "" && e.Note != "":
		return e.TaskTitle + " — " + e.Note
	case e.TaskTitle != "":
		return e.TaskTitle
	case e.Note != "":
		return e.Note
	default:
		return "entry " + e.ID
	}
}

// EntryPage is a page of entries plus whatever envelope metadata was present.
type EntryPage struct {
	Entries    []TimeEntry
	ServerTime *time.Time
	NextCursor string
}

// Task is used by the interactive picker. Only ID and Title are relied on; the
// project name is shown when present.
type Task struct {
	ID          string
	Title       string
	ProjectName string
}

// VersionInfo is what `ot version --json` told us. Both fields are optional:
// only OutputVersion is documented, so the client version is best-effort.
type VersionInfo struct {
	OutputVersion *int64
	ClientVersion string
}

// OtError is the documented error envelope:
// {"error":{"code":…,"message":…,"exit":N}}.
type OtError struct {
	Code    string
	Message string
	Exit    *int
}

func badOutput(format string, args ...any) *tterr.Error {
	return tterr.New(tterr.KindServer,
		"could not read the JSON OpenTodo returned: "+fmt.Sprintf(format, args...)).
		WithHint("tt expects the documented `--json` output of ot 0.9+ (output_version 1); " +
			"run `ot schema` and compare, or report this as a tt bug")
}

// object is a JSON object whose keys are matched without regard to case or
// separators, so the documented camelCase and the snake_case the CLI might
// print both resolve to the same field.
type object map[string]json.RawMessage

func newObject(data []byte) (object, bool) {
	var decoded map[string]json.RawMessage
	if err := json.Unmarshal(data, &decoded); err != nil {
		return nil, false
	}
	normalised := make(object, len(decoded))
	for key, value := range decoded {
		normalised[normaliseKey(key)] = value
	}
	return normalised, true
}

func normaliseKey(key string) string {
	var out strings.Builder
	for _, r := range strings.ToLower(key) {
		if r != '_' && r != '-' {
			out.WriteRune(r)
		}
	}
	return out.String()
}

func (o object) raw(name string) (json.RawMessage, bool) {
	value, ok := o[normaliseKey(name)]
	if !ok || isJSONNull(value) {
		return nil, false
	}
	return value, true
}

// str reads a string field, ignoring a value of the wrong type rather than
// failing: an unexpected shape in a field tt only displays is not worth
// refusing the whole response over.
func (o object) str(names ...string) string {
	for _, name := range names {
		raw, ok := o.raw(name)
		if !ok {
			continue
		}
		var value string
		if json.Unmarshal(raw, &value) == nil {
			return value
		}
	}
	return ""
}

func (o object) int64(names ...string) *int64 {
	for _, name := range names {
		raw, ok := o.raw(name)
		if !ok {
			continue
		}
		var value int64
		if json.Unmarshal(raw, &value) == nil {
			return &value
		}
	}
	return nil
}

func isJSONNull(raw json.RawMessage) bool {
	return len(raw) == 0 || strings.TrimSpace(string(raw)) == "null"
}

// timestamp reads a documented Unix-millisecond timestamp. RFC 3339 strings and
// whole seconds are also accepted: cheap insurance against an undocumented
// shape, and the alternative is a hard parse failure on data we could have
// understood.
func (o object) timestamp(names ...string) (*time.Time, error) {
	for _, name := range names {
		raw, ok := o.raw(name)
		if !ok {
			continue
		}
		text := strings.TrimSpace(string(raw))

		if strings.HasPrefix(text, `"`) {
			var value string
			if err := json.Unmarshal(raw, &value); err != nil {
				return nil, badOutput("timestamp %s: %v", name, err)
			}
			parsed, err := time.Parse(time.RFC3339, value)
			if err != nil {
				return nil, badOutput("timestamp %q: %v", value, err)
			}
			parsed = parsed.UTC()
			return &parsed, nil
		}

		number, err := strconv.ParseInt(text, 10, 64)
		if err != nil {
			// A fractional epoch value is still a number tt can use.
			seconds, floatErr := strconv.ParseFloat(text, 64)
			if floatErr != nil {
				return nil, badOutput("unexpected timestamp value: %s", text)
			}
			number = int64(seconds)
		}
		var parsed time.Time
		// Values small enough to be seconds since the epoch (before
		// 1970-01-01 + ~115 days in ms) are treated as seconds.
		if number < 10_000_000_000 && number > -10_000_000_000 {
			parsed = time.Unix(number, 0).UTC()
		} else {
			parsed = time.UnixMilli(number).UTC()
		}
		return &parsed, nil
	}
	return nil, nil
}

func entryFromObject(o object) (TimeEntry, error) {
	id := o.str("id")
	if id == "" {
		return TimeEntry{}, badOutput("a time entry has no `id`")
	}
	started, err := o.timestamp("startedAt")
	if err != nil {
		return TimeEntry{}, err
	}
	if started == nil {
		return TimeEntry{}, badOutput("time entry %s has no `startedAt`", id)
	}
	ended, err := o.timestamp("endedAt")
	if err != nil {
		return TimeEntry{}, err
	}
	if ended != nil && ended.Before(*started) {
		return TimeEntry{}, badOutput("time entry %s ends before it starts", id)
	}
	return TimeEntry{
		ID:          id,
		TaskID:      o.str("taskId"),
		TaskTitle:   o.str("taskTitle"),
		ProjectID:   o.str("projectId"),
		ProjectName: o.str("projectName"),
		Note:        o.str("note"),
		StartedAt:   *started,
		EndedAt:     ended,
		Minutes:     o.int64("minutes"),
	}, nil
}

// ParseEntryPage reads `ot time list --json` output.
//
// It accepts a bare array (what the CLI docs imply), the HTTP envelope
// {entries, nextCursor?, serverTime}, or a single entity.
func ParseEntryPage(stdout string) (EntryPage, error) {
	text := strings.TrimSpace(stdout)
	if text == "" || text == "null" {
		return EntryPage{}, nil
	}
	if !json.Valid([]byte(text)) {
		return EntryPage{}, badOutput("the output is not JSON")
	}

	switch text[0] {
	case '[':
		entries, err := entriesFromArray(json.RawMessage(text))
		if err != nil {
			return EntryPage{}, err
		}
		return EntryPage{Entries: entries}, nil

	case '{':
		envelope, ok := newObject([]byte(text))
		if !ok {
			return EntryPage{}, badOutput("the output is not a JSON object")
		}
		items, hasEntries := envelope.raw("entries")
		if !hasEntries {
			// A lone entity, which is what `time start`/`time stop` are most
			// likely to print.
			entry, err := entryFromObject(envelope)
			if err != nil {
				return EntryPage{}, err
			}
			return EntryPage{Entries: []TimeEntry{entry}}, nil
		}
		entries, err := entriesFromArray(items)
		if err != nil {
			return EntryPage{}, err
		}
		serverTime, err := envelope.timestamp("serverTime")
		if err != nil {
			return EntryPage{}, err
		}
		return EntryPage{
			Entries:    entries,
			ServerTime: serverTime,
			NextCursor: envelope.str("nextCursor"),
		}, nil

	default:
		return EntryPage{}, badOutput("expected an array or object, got %s", text)
	}
}

func entriesFromArray(raw json.RawMessage) ([]TimeEntry, error) {
	var items []json.RawMessage
	if err := json.Unmarshal(raw, &items); err != nil {
		return nil, badOutput("%v", err)
	}
	entries := make([]TimeEntry, 0, len(items))
	for _, item := range items {
		o, ok := newObject(item)
		if !ok {
			return nil, badOutput("a time entry is not a JSON object")
		}
		entry, err := entryFromObject(o)
		if err != nil {
			return nil, err
		}
		entries = append(entries, entry)
	}
	return entries, nil
}

// ParseOptionalEntry reads a single entry, for commands documented to return
// one entity. It reports no entry at all when the output carries none, which
// the caller handles by re-querying rather than inventing data.
func ParseOptionalEntry(stdout string) (TimeEntry, bool) {
	page, err := ParseEntryPage(stdout)
	if err != nil || len(page.Entries) == 0 {
		return TimeEntry{}, false
	}
	return page.Entries[0], true
}

// ParseError reads the documented error envelope out of either stream.
func ParseError(stdout, stderr string) (OtError, bool) {
	for _, text := range []string{stdout, stderr} {
		text = strings.TrimSpace(text)
		if text == "" || !strings.HasPrefix(text, "{") {
			continue
		}
		envelope, ok := newObject([]byte(text))
		if !ok {
			continue
		}
		raw, ok := envelope.raw("error")
		if !ok {
			continue
		}
		inner, ok := newObject(raw)
		if !ok {
			continue
		}
		result := OtError{Code: inner.str("code"), Message: inner.str("message")}
		if exit := inner.int64("exit"); exit != nil {
			code := int(*exit)
			result.Exit = &code
		}
		return result, true
	}
	return OtError{}, false
}

// ParseTasks reads `ot list --json` output, as an array or a {tasks:[…]}
// wrapper.
func ParseTasks(stdout string) ([]Task, error) {
	text := strings.TrimSpace(stdout)
	if text == "" || text == "null" {
		return nil, nil
	}
	if !json.Valid([]byte(text)) {
		return nil, badOutput("the task list output is not JSON")
	}

	items := json.RawMessage(text)
	if text[0] == '{' {
		wrapper, ok := newObject([]byte(text))
		if !ok {
			return nil, badOutput("task list output has no array of tasks")
		}
		inner, ok := wrapper.raw("tasks")
		if !ok {
			return nil, badOutput("task list output has no array of tasks")
		}
		items = inner
	} else if text[0] != '[' {
		return nil, badOutput("expected a task array, got %s", text)
	}

	var raw []json.RawMessage
	if err := json.Unmarshal(items, &raw); err != nil {
		return nil, badOutput("%v", err)
	}
	tasks := make([]Task, 0, len(raw))
	for _, item := range raw {
		o, ok := newObject(item)
		if !ok {
			return nil, badOutput("a task is not a JSON object")
		}
		tasks = append(tasks, Task{
			ID:          o.str("id"),
			Title:       o.str("title"),
			ProjectName: o.str("projectName"),
		})
	}
	return tasks, nil
}

// versionKeys are the places a client version has been seen, in preference
// order. Only output_version is documented, so this is best-effort.
var versionKeys = []string{"version", "ot_version", "otVersion", "client_version", "cli_version"}

// ParseVersion reads `ot version --json` leniently: an unreadable version is
// reported as absent, never guessed.
func ParseVersion(stdout string) VersionInfo {
	root, ok := newObject([]byte(strings.TrimSpace(stdout)))
	if !ok {
		return VersionInfo{}
	}
	info := VersionInfo{
		OutputVersion: root.int64("outputVersion"),
		ClientVersion: root.str(versionKeys...),
	}
	if info.ClientVersion == "" {
		// One level of nesting, e.g. {"ot":{"version":"0.13.0"}}.
		for _, raw := range root {
			nested, ok := newObject(raw)
			if !ok {
				continue
			}
			if version := nested.str(versionKeys...); version != "" {
				info.ClientVersion = version
				break
			}
		}
	}
	return info
}

// Semver is a dotted version, comparable part by part.
type Semver struct{ Major, Minor, Patch uint64 }

// Less orders two versions.
func (v Semver) Less(other Semver) bool {
	switch {
	case v.Major != other.Major:
		return v.Major < other.Major
	case v.Minor != other.Minor:
		return v.Minor < other.Minor
	default:
		return v.Patch < other.Patch
	}
}

// ParseSemver reads a dotted version. It reports failure when the string is not
// recognisable, in which case the caller must skip the check rather than guess.
func ParseSemver(text string) (Semver, bool) {
	trimmed := strings.TrimLeft(strings.TrimSpace(text), "vV")
	core := strings.FieldsFunc(trimmed, func(r rune) bool {
		return r == '-' || r == '+' || r == ' '
	})
	if len(core) == 0 {
		return Semver{}, false
	}
	parts := strings.Split(core[0], ".")
	numbers := [3]uint64{}
	for i := range numbers {
		if i >= len(parts) {
			continue
		}
		value, err := strconv.ParseUint(parts[i], 10, 64)
		if err != nil {
			// The major must parse; a missing minor or patch defaults to zero,
			// but a non-numeric one means this is not a version at all.
			return Semver{}, false
		}
		numbers[i] = value
	}
	return Semver{numbers[0], numbers[1], numbers[2]}, true
}
