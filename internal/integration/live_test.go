//go:build live

// Opt-in tests against a **real** OpenTodo server.
//
// SPEC.md §11 says ordinary automated tests must not depend on a live service,
// so this file is behind the `live` build tag: `go test ./...` does not even
// compile it, and CI asserts that. Run them deliberately:
//
//	cd testing && docker compose up -d && ./seed.sh
//	./testing/run-live-tests.sh
//
// They run one at a time because OpenTodo allows one running timer per
// account — that timer is shared state, and tests that start one must leave
// none behind.
//
// **These tests write data.** Point them at a throwaway instance or an `ot`
// profile for a scratch account, never a real one.
//
// Numbers come from testing/.live-env, written by testing/seed.sh, so the
// assertions are exact rather than "something was returned".

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

// live is a tt config pointing at the real ot, in the zone the seed used.
type live struct {
	t      *testing.T
	config string
}

// requireLive refuses to run unless the environment was prepared on purpose.
func requireLive(t *testing.T) *live {
	t.Helper()
	if os.Getenv("TT_LIVE") != "1" {
		t.Fatal("live tests need TT_LIVE=1; run them through testing/run-live-tests.sh")
	}
	otCommand := os.Getenv("OT")
	if otCommand == "" {
		otCommand = "ot"
	}
	config := filepath.Join(t.TempDir(), "config.toml")
	body := fmt.Sprintf("ot_command = %q\ntimezone = %q\nweek_start = \"monday\"\n",
		otCommand, requireVar(t, "TT_LIVE_TZ"))
	if err := os.WriteFile(config, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	return &live{t: t, config: config}
}

// requireVar reads a value testing/seed.sh wrote.
func requireVar(t *testing.T, key string) string {
	t.Helper()
	value := os.Getenv(key)
	if value == "" {
		t.Fatalf("%s is not set.\nRun the live tests through testing/run-live-tests.sh, "+
			"which sources testing/.live-env (written by testing/seed.sh).", key)
	}
	return value
}

func requireSeconds(t *testing.T, key string) int64 {
	t.Helper()
	value, err := strconv.ParseInt(requireVar(t, key), 10, 64)
	if err != nil {
		t.Fatalf("%s is not a number: %v", key, err)
	}
	return value
}

func (l *live) command(args ...string) *exec.Cmd {
	cmd := exec.Command(ttBinary, args...)
	// No TT_NOW: these run against real time on purpose.
	cmd.Env = append(os.Environ(), "TT_CONFIG="+l.config)
	return cmd
}

func (l *live) runCmd(cmd *exec.Cmd) run {
	l.t.Helper()
	var stdout, stderr strings.Builder
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	code := 0
	if err := cmd.Run(); err != nil {
		var exitErr *exec.ExitError
		if !errors.As(err, &exitErr) {
			l.t.Fatalf("could not run tt: %v", err)
		}
		code = exitErr.ExitCode()
	}
	return run{code: code, stdout: stdout.String(), stderr: stderr.String()}
}

func (l *live) run(args ...string) run {
	l.t.Helper()
	return l.runCmd(l.command(args...))
}

func (l *live) ok(args ...string) string {
	l.t.Helper()
	result := l.run(args...)
	if result.code != 0 {
		l.t.Fatalf("`tt %s` failed\nstdout: %s\nstderr: %s",
			strings.Join(args, " "), result.stdout, result.stderr)
	}
	return result.stdout
}

func (l *live) json(args ...string) any {
	l.t.Helper()
	stdout := l.ok(args...)
	var decoded any
	if err := json.Unmarshal([]byte(stdout), &decoded); err != nil {
		l.t.Fatalf("`tt %s` printed invalid JSON (%v): %s",
			strings.Join(args, " "), err, stdout)
	}
	return decoded
}

// ensureStopped leaves no timer running, whatever a test did.
func (l *live) ensureStopped() {
	l.t.Helper()
	if jsonField(l.t, l.json("status", "--json"), "running") == true {
		result := l.run("stop")
		if result.code != 0 && result.code != 8 {
			l.t.Fatalf("could not clean up: %s", result.stderr)
		}
	}
}

// TestLiveRealOtJSONParsesIntoUsableEntries closes compat §11 item 1: whatever
// `ot time list --json` really prints, tt parses it into entries with the
// fields reports depend on.
func TestLiveRealOtJSONParsesIntoUsableEntries(t *testing.T) {
	l := requireLive(t)
	doc := l.json("log",
		"--from", requireVar(t, "TT_LIVE_WEEK_FROM"),
		"--to", requireVar(t, "TT_LIVE_WEEK_TO"),
		"--format", "json")

	items, ok := doc.([]any)
	if !ok {
		t.Fatalf("tt log --format json returns an array, got %T", doc)
	}
	if len(items) != 4 {
		t.Fatalf("seed.sh created four entries, got %d: %v", len(items), items)
	}
	for _, item := range items {
		if _, ok := jsonField(t, item, "id").(string); !ok {
			t.Errorf("every entry needs an id: %v", item)
		}
		if _, ok := jsonField(t, item, "startedAt").(string); !ok {
			t.Errorf("every entry needs startedAt: %v", item)
		}
		if jsonNumber(t, item, "seconds") <= 0 {
			t.Errorf("durations must be real: %v", item)
		}
		if jsonField(t, item, "running") != false {
			t.Errorf("seeded entries are closed: %v", item)
		}
	}
}

// TestLiveReportTotalsMatchTheSeededEntries closes compat §11 item 6 for the
// data path: the totals tt computes match the entries that were created.
func TestLiveReportTotalsMatchTheSeededEntries(t *testing.T) {
	l := requireLive(t)
	doc := l.json("report", "range",
		"--from", requireVar(t, "TT_LIVE_WEEK_FROM"),
		"--to", requireVar(t, "TT_LIVE_WEEK_TO"),
		"--format", "json")

	total := requireSeconds(t, "TT_LIVE_TOTAL_SECONDS")
	if got := jsonNumber(t, doc, "totalSeconds"); got != total {
		t.Errorf("totalSeconds: got %d, want %d", got, total)
	}
	if jsonField(t, doc, "incomplete") != false {
		t.Error("nothing should be running or truncated")
	}
	if got := jsonNumber(t, doc, "untaskedSeconds"); got != 0 {
		t.Errorf("untaskedSeconds: %d", got)
	}
	if got := jsonField(t, doc, "timezone"); got != requireVar(t, "TT_LIVE_TZ") {
		t.Errorf("timezone: %v", got)
	}

	days := jsonArray(t, doc, "days")
	if len(days) != 7 {
		t.Fatalf("got %d days", len(days))
	}
	var summed int64
	for _, day := range days {
		summed += jsonNumber(t, day, "seconds")
	}
	if summed != total {
		t.Errorf("daily totals sum to %d, period total is %d", summed, total)
	}

	on := func(date string) int64 {
		for _, day := range days {
			if jsonField(t, day, "date") == date {
				return jsonNumber(t, day, "seconds")
			}
		}
		t.Fatalf("no row for %s", date)
		return 0
	}
	for _, pair := range [][2]string{
		{"TT_LIVE_MON", "TT_LIVE_MON_SECONDS"},
		{"TT_LIVE_TUE", "TT_LIVE_TUE_SECONDS"},
		{"TT_LIVE_WED", "TT_LIVE_WED_SECONDS"},
		{"TT_LIVE_THU", "TT_LIVE_THU_SECONDS"},
	} {
		if got, want := on(requireVar(t, pair[0])), requireSeconds(t, pair[1]); got != want {
			t.Errorf("%s: got %d, want %d", pair[0], got, want)
		}
	}

	wantTask := requireVar(t, "TT_LIVE_TASK")
	var task any
	for _, candidate := range jsonArray(t, doc, "tasks") {
		if jsonField(t, candidate, "taskId") == wantTask {
			task = candidate
		}
	}
	if task == nil {
		t.Fatalf("the seeded task is missing from the report")
	}
	if got, want := jsonNumber(t, task, "seconds"), requireSeconds(t, "TT_LIVE_TASK_SECONDS"); got != want {
		t.Errorf("two entries merge into one row: got %d, want %d", got, want)
	}
	if got := jsonNumber(t, task, "entries"); got != 2 {
		t.Errorf("task entries: %d", got)
	}
}

// TestLiveALateEveningEntryLandsOnTheRightLocalDay closes compat §11 item 5: a
// 23:00 entry belongs to that local day, not the next one. If ot interpreted
// --from/--to in a distant zone, the one-day widening in Period.QueryWindow
// would show up here.
func TestLiveALateEveningEntryLandsOnTheRightLocalDay(t *testing.T) {
	l := requireLive(t)
	wed, thu := requireVar(t, "TT_LIVE_WED"), requireVar(t, "TT_LIVE_THU")

	onWed := jsonAsArray(t, l.json("log", "--from", wed, "--to", wed, "--format", "json"))
	if len(onWed) != 1 {
		t.Fatalf("the 23:00 entry is Wednesday's, got %d entries: %v", len(onWed), onWed)
	}
	local, _ := jsonField(t, onWed[0], "startedLocal").(string)
	if !strings.HasPrefix(local, wed+"T23:") {
		t.Errorf("local start: %s", local)
	}

	onThu := jsonAsArray(t, l.json("log", "--from", thu, "--to", thu, "--format", "json"))
	if len(onThu) != 1 {
		t.Fatalf("Thursday has only its own entry, got %d", len(onThu))
	}
	if jsonField(t, onThu[0], "id") == jsonField(t, onWed[0], "id") {
		t.Error("the two days returned the same entry")
	}
}

// TestLiveStartStatusStopRoundTrip closes compat §11 items 2 and 6 for writes:
// start, observe, stop — and confirm OpenTodo kept it.
func TestLiveStartStatusStopRoundTrip(t *testing.T) {
	l := requireLive(t)
	l.ensureStopped()

	task := requireVar(t, "TT_LIVE_TASK")
	note := fmt.Sprintf("tt live test %d", os.Getpid())

	started := l.json("start", note, "--task", task, "--json")
	entryID, _ := jsonField(t, started, "id").(string)
	if entryID == "" {
		t.Fatal("start must return an entry id")
	}
	if jsonField(t, started, "taskId") != task {
		t.Errorf("taskId: %v", jsonField(t, started, "taskId"))
	}
	if jsonField(t, started, "running") != true {
		t.Error("the new entry should be running")
	}
	if jsonField(t, started, "endedAt") != nil {
		t.Error("a running entry has no end")
	}

	status := l.json("status", "--json")
	if jsonField(t, status, "running") != true {
		t.Error("status should see the entry we just started")
	}
	if jsonField(t, status, "entry", "id") != entryID {
		t.Errorf("status entry: %v", jsonField(t, status, "entry", "id"))
	}
	if jsonField(t, status, "entry", "note") != note {
		t.Errorf("note: %v", jsonField(t, status, "entry", "note"))
	}

	time.Sleep(2 * time.Second)

	stopped := l.json("stop", "--json")
	if jsonField(t, stopped, "id") != entryID {
		t.Errorf("stop closed a different entry: %v", jsonField(t, stopped, "id"))
	}
	if jsonField(t, stopped, "running") != false {
		t.Error("the entry should be closed")
	}
	if jsonField(t, stopped, "endedAt") == nil {
		t.Error("a stopped entry has an end")
	}
	if got := jsonNumber(t, stopped, "seconds"); got < 1 {
		t.Errorf("elapsed: %d", got)
	}

	// And OpenTodo really kept it.
	found := false
	for _, entry := range jsonAsArray(t, l.json("log", "--today", "--format", "json")) {
		found = found || jsonField(t, entry, "id") == entryID
	}
	if !found {
		t.Error("the stopped entry must be readable back")
	}
	if jsonField(t, l.json("status", "--json"), "running") != false {
		t.Error("nothing should be left running")
	}
}

// TestLiveStopWithoutARunningTimerExitsEight closes compat §11 item 7: tt must
// not send a write that cannot succeed.
func TestLiveStopWithoutARunningTimerExitsEight(t *testing.T) {
	l := requireLive(t)
	l.ensureStopped()

	result := l.run("stop")
	if result.code != 8 {
		t.Fatalf("exit %d\nstdout: %s\nstderr: %s", result.code, result.stdout, result.stderr)
	}
	if !strings.Contains(result.stderr, "no timer is running") {
		t.Errorf("got %q", result.stderr)
	}
	if result.stdout != "" {
		t.Errorf("a failure must not print results: %q", result.stdout)
	}
}

// TestLiveASecondStartIsRefusedWithoutSwitch checks against the real server
// that tt does not let OpenTodo silently close an entry.
func TestLiveASecondStartIsRefusedWithoutSwitch(t *testing.T) {
	l := requireLive(t)
	l.ensureStopped()
	defer l.ensureStopped()

	first := l.json("start", "first", "--task", requireVar(t, "TT_LIVE_TASK"), "--json")
	firstID := jsonField(t, first, "id")

	result := l.run("start", "second", "--task", requireVar(t, "TT_LIVE_PLANNING_TASK"))
	if result.code != 2 {
		t.Fatalf("exit %d: %s", result.code, result.stderr)
	}
	if !strings.Contains(result.stderr, "already running") {
		t.Errorf("got %q", result.stderr)
	}

	// The original entry must be untouched.
	if jsonField(t, l.json("status", "--json"), "entry", "id") != firstID {
		t.Error("the refused start changed something")
	}
}

// TestLiveAnUnreachableServerFailsWithoutRecordingAnything confirms the
// documented online-only behaviour: no queueing, no local record, and the exit
// code that says so (SPEC.md §7).
func TestLiveAnUnreachableServerFailsWithoutRecordingAnything(t *testing.T) {
	l := requireLive(t)

	// Port 1 is reserved and nothing listens there.
	cmd := l.command("start", "offline attempt", "--task", requireVar(t, "TT_LIVE_TASK"))
	cmd.Env = append(cmd.Env, "OPENTODO_URL=http://127.0.0.1:1")
	result := l.runCmd(cmd)

	if result.code != 4 && result.code != 3 {
		t.Fatalf("expected unreachable (4) or a config error (3), got %d: %s",
			result.code, result.stderr)
	}
	if result.stdout != "" {
		t.Errorf("nothing may be reported as recorded: %q", result.stdout)
	}

	// Nothing was queued or invented: the real server has no such entry.
	for _, entry := range jsonAsArray(t, l.json("log", "--today", "--format", "json")) {
		if jsonField(t, entry, "note") == "offline attempt" {
			t.Fatal("an unreachable start left an entry behind")
		}
	}
}

// TestLiveSyncStatusReportsUnavailableAgainstAReachableServer checks that
// `tt sync status` still refuses to claim "synchronized" (SPEC.md §6.7).
func TestLiveSyncStatusReportsUnavailableAgainstAReachableServer(t *testing.T) {
	l := requireLive(t)
	doc := l.json("sync", "status", "--json")

	checks := map[string]any{
		"syncStatus":          "unavailable",
		"syncStatusKnown":     false,
		"serverReachable":     true,
		"pendingLocalChanges": float64(0),
	}
	for key, want := range checks {
		if got := jsonField(t, doc, key); got != want {
			t.Errorf("%s: got %v, want %v", key, got, want)
		}
	}
	if result := l.run("sync"); result.code != 6 {
		t.Errorf("there is no documented sync command to call, got exit %d", result.code)
	}
}

// TestLiveAReadOnlyTokenCannotStartATimer only runs when a read-scoped token is
// provided; otherwise it reports why it did nothing rather than passing
// silently.
func TestLiveAReadOnlyTokenCannotStartATimer(t *testing.T) {
	l := requireLive(t)
	token := os.Getenv("TT_LIVE_READ_TOKEN")
	if token == "" {
		t.Skip("set TT_LIVE_READ_TOKEN to a read-scoped token " +
			"(`ot token create tt-read --scope read`) to exercise exit 5")
	}

	cmd := l.command("start", "read-only attempt", "--task", requireVar(t, "TT_LIVE_TASK"))
	cmd.Env = append(cmd.Env, "OPENTODO_TOKEN="+token)
	result := l.runCmd(cmd)

	if result.code != 5 {
		t.Fatalf("exit %d: %s", result.code, result.stderr)
	}
	if !strings.Contains(result.stderr, "scope") && !strings.Contains(result.stderr, "permitted") {
		t.Errorf("got %q", result.stderr)
	}
	if strings.Contains(result.stderr, token) {
		t.Error("the token leaked into the error")
	}
}

func jsonAsArray(t *testing.T, doc any) []any {
	t.Helper()
	items, ok := doc.([]any)
	if !ok {
		t.Fatalf("expected an array, got %T", doc)
	}
	return items
}
