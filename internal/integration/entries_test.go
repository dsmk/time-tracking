package integration

import (
	"strings"
	"testing"
)

// `tt log` against a mock ot.

// weekEntries covers Monday 09:00-10:00, Tuesday 14:00-15:30, and Tuesday
// 23:30 into Wednesday.
func weekEntries(t *testing.T) string {
	t.Helper()
	return "[" + strings.Join([]string{
		entryJSON("a", "8c2a", "Design review",
			ms(t, "2026-10-26T13:00:00Z"), ms(t, "2026-10-26T14:00:00Z")),
		entryJSON("b", "3f1d", "Planning",
			ms(t, "2026-10-27T18:00:00Z"), ms(t, "2026-10-27T19:30:00Z")),
		entryJSON("c", "8c2a", "Design review",
			ms(t, "2026-10-28T03:30:00Z"), ms(t, "2026-10-28T04:30:00Z")),
	}, ",") + "]"
}

func TestLogDefaultsToTheLastSevenDaysAndTotalsThem(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", weekEntries(t))

	out := h.ok(now, "log")
	wants := []string{
		"2026-10-22 to 2026-10-28",
		"America/New_York",
		"3 entries, 3h 30m total",
		"Mon 2026-10-26  09:00-10:00",
	}
	for _, want := range wants {
		if !strings.Contains(out, want) {
			t.Errorf("%q is missing from:\n%s", want, out)
		}
	}
}

func TestLogAsksOpenTodoForAWindowOneDayWiderOnEachSide(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "[]")

	h.ok(now, "log", "--from", "2026-10-26", "--to", "2026-10-28")
	got := strings.Join(h.callsMatching("time", "list")[0], " ")
	want := "time list --json --from=2026-10-25 --to=2026-10-29 --limit=5000"
	if got != want {
		t.Errorf("the window is widened because ot has no documented --tz flag\ngot:  %s\nwant: %s",
			got, want)
	}
}

func TestLogTodayKeepsOnlyEntriesTouchingTodayInTheConfiguredZone(t *testing.T) {
	h := newHarness(t)
	// The widened query returns all three; only the one crossing into
	// Wednesday 00:00-00:30 New York time touches today.
	h.respond("time-list", weekEntries(t))

	out := h.ok(now, "log", "--today")
	for _, want := range []string{"2026-10-28 to 2026-10-28", "1 entries", "Tue 2026-10-27  23:30-00:30"} {
		if !strings.Contains(out, want) {
			t.Errorf("%q is missing from:\n%s", want, out)
		}
	}
	if strings.Contains(out, "Planning") {
		t.Errorf("Tuesday afternoon is not today:\n%s", out)
	}
}

func TestLogFiltersByTaskThroughTheDocumentedFlag(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "[]")

	h.ok(now, "log", "--today", "--task", "8c2a")
	call := strings.Join(h.callsMatching("time", "list")[0], " ")
	if !strings.Contains(call, "--task=8c2a") {
		t.Errorf("got %q", call)
	}
}

func TestLogJSONExposesBothInstantsAndOpenTodosOwnMinutes(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", weekEntries(t))

	doc := h.jsonOut(now, "log", "--format", "json")
	items, ok := doc.([]any)
	if !ok {
		t.Fatalf("expected an array of entries, got %T", doc)
	}
	if len(items) != 3 {
		t.Fatalf("got %d entries", len(items))
	}
	checks := map[string]any{
		"id":              "a",
		"startedAt":       "2026-10-26T13:00:00Z",
		"startedLocal":    "2026-10-26T09:00:00-04:00",
		"seconds":         float64(3600),
		"openTodoMinutes": float64(60),
		"running":         false,
	}
	for key, want := range checks {
		if got := jsonField(t, items[0], key); got != want {
			t.Errorf("%s: got %v, want %v", key, got, want)
		}
	}
}

func TestAResultThatFillsTheLimitIsReportedAsPossiblyIncomplete(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", weekEntries(t))

	result := h.run(now, "log", "--limit", "3")
	if result.code != 0 {
		t.Fatalf("exit %d: %s", result.code, result.stderr)
	}
	for _, want := range []string{"may be incomplete", "larger --limit"} {
		if !strings.Contains(result.stderr, want) {
			t.Errorf("%q is missing from %q", want, result.stderr)
		}
	}
}

func TestANextCursorIsReportedRatherThanSilentlyDropped(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", `{"entries":[],"nextCursor":"abc"}`)

	result := h.run(now, "log")
	if result.code != 0 {
		t.Fatalf("exit %d: %s", result.code, result.stderr)
	}
	if !strings.Contains(result.stderr, "no documented pagination") {
		t.Errorf("got %q", result.stderr)
	}
}

func TestLogRejectsCSVAndPointsAtTheReportCommand(t *testing.T) {
	h := newHarness(t)
	result := h.fail(now, "log", "--format", "csv")
	if result.code != 2 {
		t.Errorf("exit: %d", result.code)
	}
	if !strings.Contains(result.stderr, "report range --format csv") {
		t.Errorf("got %q", result.stderr)
	}
}

func TestABackwardsRangeIsRefusedBeforeOpenTodoIsCalled(t *testing.T) {
	h := newHarness(t)
	result := h.fail(now, "log", "--from", "2026-10-28", "--to", "2026-10-01")
	if result.code != 2 {
		t.Errorf("exit: %d", result.code)
	}
	if h.callCount("time", "list") != 0 {
		t.Error("nothing may be asked of OpenTodo")
	}
}

func TestAnInvalidDateNamesTheExpectedForm(t *testing.T) {
	h := newHarness(t)
	result := h.fail(now, "log", "--from", "last tuesday")
	if result.code != 2 {
		t.Errorf("exit: %d", result.code)
	}
	if !strings.Contains(result.stderr, "YYYY-MM-DD") {
		t.Errorf("got %q", result.stderr)
	}
}

func TestAnUnknownLogFormatIsRefused(t *testing.T) {
	h := newHarness(t)
	result := h.fail(now, "log", "--format", "yaml")
	if result.code != 2 {
		t.Errorf("exit: %d", result.code)
	}
	if !strings.Contains(result.stderr, "text") {
		t.Errorf("the accepted formats should be named: %q", result.stderr)
	}
}
