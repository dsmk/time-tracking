package integration

import (
	"strings"
	"testing"
)

// Start, stop and status against a mock ot.

const now = "2026-10-28T18:00:00Z" // Wed 14:00 America/New_York

func runningEntry(t *testing.T) string {
	t.Helper()
	return entryJSON("e9", "8c2a", "Design review", ms(t, "2026-10-28T17:00:00Z"), 0)
}

func stoppedEntry(t *testing.T) string {
	t.Helper()
	return entryJSON("e9", "8c2a", "Design review",
		ms(t, "2026-10-28T17:00:00Z"), ms(t, "2026-10-28T18:00:00Z"))
}

func TestStartChecksForARunningTimerThenStartsOne(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "[]")
	h.respond("time-start", runningEntry(t))

	out := h.ok(now, "start", "Design review", "--task", "8c2a")
	for _, want := range []string{"Started", "Design review", "task 8c2a"} {
		if !strings.Contains(out, want) {
			t.Errorf("%q is missing from %q", want, out)
		}
	}

	calls := h.calls()
	wants := [][]string{
		{"version", "--json"},
		{"time", "list", "--json", "--running"},
		{"time", "start", "8c2a", "--note=Design review", "--json"},
	}
	if len(calls) < len(wants) {
		t.Fatalf("got %v", calls)
	}
	for i, want := range wants {
		if strings.Join(calls[i], " ") != strings.Join(want, " ") {
			t.Errorf("call %d: got %v, want %v", i, calls[i], want)
		}
	}
}

func TestStartRefusesToReplaceARunningTimerWithoutSwitch(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "["+runningEntry(t)+"]")

	result := h.fail(now, "start", "Other work", "--task", "3f1d")
	if result.code != 2 {
		t.Errorf("exit: %d", result.code)
	}
	for _, want := range []string{"already running", "--switch"} {
		if !strings.Contains(result.stderr, want) {
			t.Errorf("%q is missing from %q", want, result.stderr)
		}
	}
	if h.callCount("time", "start") != 0 {
		t.Error("no write may be sent")
	}
	if result.stdout != "" {
		t.Errorf("a failure must not print results: %q", result.stdout)
	}
}

func TestStartSwitchWarnsThatOpenTodoWillEndTheCurrentEntry(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "["+runningEntry(t)+"]")
	h.respond("time-start", entryJSON("e10", "3f1d", "Other work", ms(t, now), 0))

	result := h.run(now, "start", "Other work", "--task", "3f1d", "--switch")
	if result.code != 0 {
		t.Fatalf("exit %d: %s", result.code, result.stderr)
	}
	if !strings.Contains(result.stderr, "will end the running entry") {
		t.Errorf("got %q", result.stderr)
	}
	if h.callCount("time", "start") != 1 {
		t.Errorf("start calls: %d", h.callCount("time", "start"))
	}
}

func TestStartWithoutATaskFailsCleanlyWhenThereIsNoTerminal(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "[]")

	result := h.fail(now, "start", "Design review")
	if result.code != 2 {
		t.Errorf("exit: %d", result.code)
	}
	if !strings.Contains(result.stderr, "--task") {
		t.Errorf("got %q", result.stderr)
	}
	if h.callCount("time", "start") != 0 {
		t.Error("no write may be sent")
	}
	if h.callCount("list") != 0 {
		t.Error("no interactive lookup without a terminal")
	}
}

func TestStartJSONReportsTheEntryOpenTodoReturned(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "[]")
	h.respond("time-start", runningEntry(t))

	doc := h.jsonOut(now, "start", "Design review", "--task", "8c2a", "--json")
	checks := map[string]any{
		"id":           "e9",
		"taskId":       "8c2a",
		"running":      true,
		"startedLocal": "2026-10-28T13:00:00-04:00",
	}
	for key, want := range checks {
		if got := jsonField(t, doc, key); got != want {
			t.Errorf("%s: got %v, want %v", key, got, want)
		}
	}
}

func TestStopWithoutARunningTimerChangesNothing(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "[]")

	result := h.fail(now, "stop")
	if result.code != 8 {
		t.Errorf("exit 8 means no active timer, got %d", result.code)
	}
	if !strings.Contains(result.stderr, "no timer is running") {
		t.Errorf("got %q", result.stderr)
	}
	if h.callCount("time", "stop") != 0 {
		t.Error("ot must not be asked to stop nothing")
	}
}

func TestStopReportsTheCompletedEntry(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "["+runningEntry(t)+"]")
	h.respond("time-stop", stoppedEntry(t))

	out := h.ok(now, "stop")
	if !strings.Contains(out, "Stopped") {
		t.Errorf("got %q", out)
	}
	if !strings.Contains(out, "1h 00m") {
		t.Errorf("duration comes from OpenTodo's timestamps: %q", out)
	}
	calls := h.calls()
	if last := strings.Join(calls[len(calls)-1], " "); last != "time stop --json" {
		t.Errorf("last call: %q", last)
	}
}

func TestStopRereadsTheEntryWhenOpenTodoReturnsNoDetails(t *testing.T) {
	h := newHarness(t)
	// 1: the running check. 2: the re-read after the undocumented stop output.
	h.respondNth("time-list", 1, "["+runningEntry(t)+"]")
	h.respondNth("time-list", 2, "["+stoppedEntry(t)+"]")
	h.respond("time-stop", "")

	out := h.ok(now, "stop")
	if !strings.Contains(out, "1h 00m") {
		t.Errorf("got %q", out)
	}
	reread := h.callsMatching("time", "list")[1]
	if !strings.Contains(strings.Join(reread, " "), "--task=8c2a") {
		t.Errorf("the re-read should be scoped to the task: %v", reread)
	}
}

func TestStopDoesNotInventADurationWhenTheEntryCannotBeReread(t *testing.T) {
	h := newHarness(t)
	h.respondNth("time-list", 1, "["+runningEntry(t)+"]")
	h.respondNth("time-list", 2, "[]")
	h.respond("time-stop", "")

	result := h.run(now, "stop")
	if result.code != 0 {
		t.Fatalf("exit %d: %s", result.code, result.stderr)
	}
	if !strings.Contains(result.stdout, "Stopped the timer") {
		t.Errorf("got %q", result.stdout)
	}
	if !strings.Contains(result.stderr, "no entry details") {
		t.Errorf("got %q", result.stderr)
	}
	if strings.Contains(result.stdout, "1h") {
		t.Errorf("no duration may be fabricated: %q", result.stdout)
	}
}

func TestStatusShowsTheRunningTimerAndElapsedTime(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "["+runningEntry(t)+"]")

	out := h.ok(now, "status")
	for _, want := range []string{"Running", "1h 00m", "13:00"} {
		if !strings.Contains(out, want) {
			t.Errorf("%q is missing from %q", want, out)
		}
	}
}

func TestStatusIsConciseAndJSONShapedWhenNothingRuns(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "[]")
	if out := h.ok(now, "status"); !strings.Contains(out, "No timer is running") {
		t.Errorf("got %q", out)
	}

	other := newHarness(t)
	other.respond("time-list", "[]")
	doc := other.jsonOut(now, "status", "--json")
	if jsonField(t, doc, "running") != false {
		t.Error("nothing is running")
	}
	if jsonField(t, doc, "entry") != nil {
		t.Error("no entry may be invented")
	}
}
