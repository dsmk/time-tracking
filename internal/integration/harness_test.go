// Package integration runs the real tt binary against a mock ot executable.
//
// The mock answers only the commands a test installs a response for. Anything
// else exits 2 with an `unexpected_command` envelope, so if tt ever invents an
// ot subcommand the test that exercises that path fails — which is how
// SPEC.md §11's "unsupported commands are not invoked" is enforced.
package integration

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"
)

// ttBinary is built once for the whole package by TestMain.
var ttBinary string

func TestMain(m *testing.M) {
	dir, err := os.MkdirTemp("", "tt-integration-")
	if err != nil {
		fmt.Fprintf(os.Stderr, "integration: %v\n", err)
		os.Exit(1)
	}
	defer os.RemoveAll(dir)

	ttBinary = filepath.Join(dir, "tt")
	build := exec.Command("go", "build", "-o", ttBinary, "github.com/dsmk/time-tracking/cmd/tt")
	build.Stdout, build.Stderr = os.Stderr, os.Stderr
	if err := build.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "integration: could not build tt: %v\n", err)
		os.RemoveAll(dir)
		os.Exit(1)
	}

	code := m.Run()
	os.RemoveAll(dir)
	os.Exit(code)
}

const mockOt = `#!/usr/bin/env bash
set -u
dir="${MOCK_OT_DIR:?MOCK_OT_DIR is not set}"

# Record the invocation: one argument per line, terminated by a record mark.
{ for a in "$@"; do printf '%s\n' "$a"; done; printf '\036\n'; } >> "$dir/calls.log"

# Response key: the first two non-flag arguments, e.g. ` + "`time list`, `version`" + `.
key=""
count=0
for a in "$@"; do
  case "$a" in -*) continue ;; esac
  if [ "$count" -lt 2 ]; then
    if [ -z "$key" ]; then key="$a"; else key="$key-$a"; fi
    count=$((count + 1))
  fi
done

n=0
[ -f "$dir/$key.count" ] && n=$(cat "$dir/$key.count")
n=$((n + 1))
printf '%s' "$n" > "$dir/$key.count"

out="$dir/$key.$n.out"; [ -f "$out" ] || out="$dir/$key.out"
err="$dir/$key.$n.err"; [ -f "$err" ] || err="$dir/$key.err"
code_file="$dir/$key.$n.code"; [ -f "$code_file" ] || code_file="$dir/$key.code"

if [ ! -f "$out" ] && [ ! -f "$code_file" ]; then
  printf '{"error":{"code":"unexpected_command","message":"mock ot has no response for %s","exit":2}}' "$key"
  printf 'ot: unexpected command: %s\n' "$key" >&2
  exit 2
fi

[ -f "$out" ] && cat "$out"
[ -f "$err" ] && cat "$err" >&2
code=0
[ -f "$code_file" ] && code=$(cat "$code_file")
exit "$code"
`

// versionJSON is the response the harness installs by default, so tests do not
// have to restate the probe that runs before every command.
const versionJSON = `{"output_version":1,"version":"0.13.0"}`

// harness is a temporary directory holding a mock ot executable, its canned
// responses, a tt config file pointing at it, and a log of every invocation.
type harness struct {
	t         *testing.T
	dir       string
	config    string
	responses string
}

type run struct {
	code   int
	stdout string
	stderr string
}

// newHarness configures America/New_York with a Monday week start, so timezone
// and week behaviour is exercised rather than the host's own settings.
func newHarness(t *testing.T) *harness {
	return harnessWith(t, "timezone = \"America/New_York\"\nweek_start = \"monday\"\n")
}

func harnessWith(t *testing.T, body string) *harness {
	t.Helper()
	dir := t.TempDir()
	responses := filepath.Join(dir, "responses")
	if err := os.MkdirAll(responses, 0o755); err != nil {
		t.Fatal(err)
	}

	mock := filepath.Join(dir, "mock-ot")
	if err := os.WriteFile(mock, []byte(mockOt), 0o755); err != nil {
		t.Fatal(err)
	}

	config := filepath.Join(dir, "config.toml")
	escaped := strings.ReplaceAll(mock, `\`, `\\`)
	if err := os.WriteFile(config,
		[]byte(fmt.Sprintf("ot_command = %q\n%s", escaped, body)), 0o600); err != nil {
		t.Fatal(err)
	}

	h := &harness{t: t, dir: dir, config: config, responses: responses}
	h.respond("version", versionJSON)
	return h
}

// respond installs the response for every call with this key.
func (h *harness) respond(key, stdout string) *harness {
	h.t.Helper()
	h.writeResponse(key+".out", stdout)
	return h
}

// respondNth installs the response for the nth call with this key (1-based).
func (h *harness) respondNth(key string, n int, stdout string) *harness {
	h.t.Helper()
	h.writeResponse(fmt.Sprintf("%s.%d.out", key, n), stdout)
	return h
}

// respondFail installs a failing response: exit code plus the documented error
// envelope.
func (h *harness) respondFail(key string, code int, stdout, stderr string) *harness {
	h.t.Helper()
	h.writeResponse(key+".out", stdout)
	h.writeResponse(key+".err", stderr)
	h.writeResponse(key+".code", strconv.Itoa(code))
	return h
}

func (h *harness) writeResponse(name, body string) {
	h.t.Helper()
	if err := os.WriteFile(filepath.Join(h.responses, name), []byte(body), 0o600); err != nil {
		h.t.Fatal(err)
	}
}

// rewriteConfig replaces the whole config file, for tests about bad settings.
func (h *harness) rewriteConfig(body string) {
	h.t.Helper()
	if err := os.WriteFile(h.config, []byte(body), 0o600); err != nil {
		h.t.Fatal(err)
	}
}

func (h *harness) command(now string, args ...string) *exec.Cmd {
	cmd := exec.Command(ttBinary, args...)
	env := []string{}
	for _, entry := range os.Environ() {
		// Keep the host's own zone from leaking into the test.
		if strings.HasPrefix(entry, "TZ=") || strings.HasPrefix(entry, "TT_") ||
			strings.HasPrefix(entry, "MOCK_OT_DIR=") {
			continue
		}
		env = append(env, entry)
	}
	cmd.Env = append(env,
		"TT_CONFIG="+h.config,
		"TT_NOW="+now,
		"MOCK_OT_DIR="+h.responses,
	)
	return cmd
}

func (h *harness) run(now string, args ...string) run {
	h.t.Helper()
	var stdout, stderr strings.Builder
	cmd := h.command(now, args...)
	cmd.Stdout, cmd.Stderr = &stdout, &stderr

	code := 0
	if err := cmd.Run(); err != nil {
		var exitErr *exec.ExitError
		if !errors.As(err, &exitErr) {
			h.t.Fatalf("could not run tt: %v", err)
		}
		code = exitErr.ExitCode()
	}
	return run{code: code, stdout: stdout.String(), stderr: stderr.String()}
}

// ok runs tt and requires success, returning stdout.
func (h *harness) ok(now string, args ...string) string {
	h.t.Helper()
	result := h.run(now, args...)
	if result.code != 0 {
		h.t.Fatalf("expected `tt %s` to succeed\nstdout: %s\nstderr: %s",
			strings.Join(args, " "), result.stdout, result.stderr)
	}
	return result.stdout
}

// fail runs tt and requires a non-zero exit.
func (h *harness) fail(now string, args ...string) run {
	h.t.Helper()
	result := h.run(now, args...)
	if result.code == 0 {
		h.t.Fatalf("expected `tt %s` to fail, but it printed: %s",
			strings.Join(args, " "), result.stdout)
	}
	return result
}

// jsonOut runs tt and decodes its stdout.
func (h *harness) jsonOut(now string, args ...string) any {
	h.t.Helper()
	stdout := h.ok(now, args...)
	var decoded any
	if err := json.Unmarshal([]byte(stdout), &decoded); err != nil {
		h.t.Fatalf("stdout was not valid JSON (%v): %s", err, stdout)
	}
	return decoded
}

// calls lists every ot invocation the mock saw, in order, as argument vectors.
func (h *harness) calls() [][]string {
	text, err := os.ReadFile(filepath.Join(h.responses, "calls.log"))
	if err != nil {
		return nil
	}
	var all [][]string
	var current []string
	for _, line := range strings.Split(strings.TrimSuffix(string(text), "\n"), "\n") {
		if line == "\x1e" {
			all = append(all, current)
			current = nil
			continue
		}
		current = append(current, line)
	}
	return all
}

// callsMatching lists the invocations whose leading arguments match prefix.
func (h *harness) callsMatching(prefix ...string) [][]string {
	var matched [][]string
	for _, call := range h.calls() {
		if len(call) < len(prefix) {
			continue
		}
		same := true
		for i, want := range prefix {
			same = same && call[i] == want
		}
		if same {
			matched = append(matched, call)
		}
	}
	return matched
}

func (h *harness) callCount(prefix ...string) int {
	return len(h.callsMatching(prefix...))
}

// entryJSON builds one time entry in the documented camelCase shape.
func entryJSON(id, task, title string, startMs int64, endMs int64) string {
	ended, minutes := "null", "null"
	if endMs > 0 {
		ended = strconv.FormatInt(endMs, 10)
		minutes = strconv.FormatInt((endMs-startMs)/60_000, 10)
	}
	return fmt.Sprintf(
		`{"id":%q,"taskId":%q,"userId":"u1","startedAt":%d,"endedAt":%s,"minutes":%s,`+
			`"note":"n","deviceId":"d1","taskTitle":%q,"projectId":"p1","projectName":"Work"}`,
		id, task, startMs, ended, minutes, title)
}

// ms is Unix milliseconds for an RFC 3339 instant, for building fixtures.
func ms(t *testing.T, raw string) int64 {
	t.Helper()
	parsed, err := time.Parse(time.RFC3339, raw)
	if err != nil {
		t.Fatalf("valid timestamp: %v", err)
	}
	return parsed.UnixMilli()
}

// jsonField walks a decoded JSON document, e.g. field(doc, "entry", "id").
func jsonField(t *testing.T, doc any, path ...string) any {
	t.Helper()
	current := doc
	for _, key := range path {
		object, ok := current.(map[string]any)
		if !ok {
			t.Fatalf("%v is not an object while looking for %q", current, key)
		}
		current = object[key]
	}
	return current
}

// jsonNumber reads a numeric field as an int64.
func jsonNumber(t *testing.T, doc any, path ...string) int64 {
	t.Helper()
	value, ok := jsonField(t, doc, path...).(float64)
	if !ok {
		t.Fatalf("%v is not a number", jsonField(t, doc, path...))
	}
	return int64(value)
}

// jsonArray reads an array field.
func jsonArray(t *testing.T, doc any, path ...string) []any {
	t.Helper()
	value, ok := jsonField(t, doc, path...).([]any)
	if !ok {
		t.Fatalf("%v is not an array", jsonField(t, doc, path...))
	}
	return value
}
