package integration

import (
	"encoding/csv"
	"fmt"
	"strings"
	"testing"
)

// `tt report week` and `tt report range` against a mock ot.

func TestTheWeeklyReportCoversTheConfiguredWeekAndStatesItsRules(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", weekEntries(t))

	out := h.ok(now, "report", "week")
	wants := []string{
		"Week of 2026-10-26 to 2026-11-01 (week starts monday, America/New_York)",
		"Total: 3h 30m",
		"Mon 2026-10-26    1h 00m",
		"Tue 2026-10-27    2h 00m",
		"Wed 2026-10-28       30m",
		"Sun 2026-11-01",
	}
	for _, want := range wants {
		if !strings.Contains(out, want) {
			t.Errorf("%q is missing from:\n%s", want, out)
		}
	}
}

func TestTaskTotalsMergeEntriesAndNameTheProject(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", weekEntries(t))

	out := h.ok(now, "report", "week")
	_, tasks, found := strings.Cut(out, "By task")
	if !found {
		t.Fatalf("no task section in:\n%s", out)
	}
	var row string
	for _, line := range strings.Split(tasks, "\n") {
		if strings.Contains(line, "Design review") {
			row = line
			break
		}
	}
	for _, want := range []string{"2h 00m", "(Work)", "[8c2a]"} {
		if !strings.Contains(row, want) {
			t.Errorf("%q is missing from %q", want, row)
		}
	}
}

func TestTheWeekStartCanBeOverriddenOnTheCommandLine(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "[]")

	out := h.ok(now, "report", "week", "--week-start", "sunday")
	if !strings.Contains(out, "Week of 2026-10-25 to 2026-10-31 (week starts sunday") {
		t.Errorf("got:\n%s", out)
	}
}

func TestAPastWeekCanBeSelectedByAnyDateInsideIt(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "[]")

	out := h.ok(now, "report", "week", "--week", "2026-10-21")
	if !strings.Contains(out, "Week of 2026-10-19 to 2026-10-25") {
		t.Errorf("got:\n%s", out)
	}
	got := strings.Join(h.callsMatching("time", "list")[0], " ")
	want := "time list --json --from=2026-10-18 --to=2026-10-26 --limit=5000"
	if got != want {
		t.Errorf("got  %s\nwant %s", got, want)
	}
}

func TestTheJSONReportCarriesTotalsDaysTasksAndHonestyFlags(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", weekEntries(t))

	doc := h.jsonOut(now, "report", "week", "--format", "json")
	checks := map[string]any{
		"from":            "2026-10-26",
		"to":              "2026-11-01",
		"timezone":        "America/New_York",
		"weekStart":       "monday",
		"totalSeconds":    float64(12600),
		"totalHours":      3.5,
		"incomplete":      false,
		"untaskedSeconds": float64(0),
	}
	for key, want := range checks {
		if got := jsonField(t, doc, key); got != want {
			t.Errorf("%s: got %v, want %v", key, got, want)
		}
	}

	days := jsonArray(t, doc, "days")
	if len(days) != 7 {
		t.Fatalf("got %d days", len(days))
	}
	if got := jsonNumber(t, days[1], "seconds"); got != 7200 {
		t.Errorf("Tuesday: %d", got)
	}
	if got := jsonField(t, days[1], "weekday"); got != "Tuesday" {
		t.Errorf("weekday: %v", got)
	}
	var summed int64
	for _, day := range days {
		summed += jsonNumber(t, day, "seconds")
	}
	if summed != 12600 {
		t.Errorf("daily totals must sum to the period total, got %d", summed)
	}

	tasks := jsonArray(t, doc, "tasks")
	if jsonField(t, tasks[0], "taskId") != "8c2a" {
		t.Errorf("tasks: %v", tasks)
	}
	if got := jsonNumber(t, tasks[0], "seconds"); got != 7200 {
		t.Errorf("task seconds: %d", got)
	}
	if got := jsonNumber(t, tasks[0], "entries"); got != 2 {
		t.Errorf("task entries: %d", got)
	}
}

func TestARunningEntryMakesTheReportSayItIsIncomplete(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list",
		"["+entryJSON("r", "8c2a", "Design review", ms(t, "2026-10-28T17:00:00Z"), 0)+"]")

	doc := h.jsonOut(now, "report", "week", "--format", "json")
	if jsonField(t, doc, "incomplete") != true {
		t.Error("a running entry makes the report incomplete")
	}
	if got := jsonNumber(t, doc, "runningEntries"); got != 1 {
		t.Errorf("runningEntries: %d", got)
	}
	if got := jsonNumber(t, doc, "totalSeconds"); got != 3600 {
		t.Errorf("counted up to now: %d", got)
	}
	warnings := jsonArray(t, doc, "warnings")
	found := false
	for _, warning := range warnings {
		text, _ := warning.(string)
		found = found || strings.Contains(text, "still running")
	}
	if !found {
		t.Errorf("warnings: %v", warnings)
	}
}

func TestAnUntaskedEntryIsReportedSeparately(t *testing.T) {
	untasked := fmt.Sprintf(`[{"id":"u1","taskId":null,"startedAt":%d,"endedAt":%d,"minutes":30}]`,
		ms(t, "2026-10-26T13:00:00Z"), ms(t, "2026-10-26T13:30:00Z"))

	h := newHarness(t)
	h.respond("time-list", untasked)
	if out := h.ok(now, "report", "week"); !strings.Contains(out, "30m  (no task)") {
		t.Errorf("got:\n%s", out)
	}

	other := newHarness(t)
	other.respond("time-list", untasked)
	doc := other.jsonOut(now, "report", "week", "--format", "json")
	if got := jsonNumber(t, doc, "untaskedSeconds"); got != 1800 {
		t.Errorf("untaskedSeconds: %d", got)
	}
}

func TestTheRangeReportUsesInclusiveLocalDates(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", weekEntries(t))

	out := h.ok(now, "report", "range", "--from", "2026-10-26", "--to", "2026-10-27")
	if !strings.Contains(out, "2026-10-26 to 2026-10-27 (America/New_York)") {
		t.Errorf("got:\n%s", out)
	}
	// Wednesday's half hour falls outside the range and is clipped away.
	if !strings.Contains(out, "Total: 3h 00m") {
		t.Errorf("got:\n%s", out)
	}
}

func TestCSVOutputIsValidAndCarriesTheIncompleteMarker(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", weekEntries(t))

	text := h.ok(now, "report", "range",
		"--from", "2026-10-26", "--to", "2026-10-28", "--format", "csv")
	rows, err := csv.NewReader(strings.NewReader(text)).ReadAll()
	if err != nil {
		t.Fatalf("invalid CSV: %v\n%s", err, text)
	}
	header := strings.Join(rows[0], ",")
	if header != "section,key,label,seconds,hours" {
		t.Fatalf("header: %q", header)
	}

	var days [][]string
	var total, meta []string
	for _, row := range rows[1:] {
		switch {
		case row[0] == "day":
			days = append(days, row)
		case row[0] == "total":
			total = row
		case row[1] == "incomplete":
			meta = row
		}
	}
	if len(days) != 3 {
		t.Errorf("got %d day rows", len(days))
	}
	if days[1][3] != "7200" {
		t.Errorf("Tuesday: %q", days[1][3])
	}
	if total == nil || total[3] != "12600" || total[4] != "3.5" {
		t.Errorf("total row: %v", total)
	}
	if meta == nil || meta[2] != "false" {
		t.Errorf("incomplete marker: %v", meta)
	}
}

func TestTheDefaultReportFormatComesFromTheConfigFile(t *testing.T) {
	h := harnessWith(t,
		"timezone = \"America/New_York\"\nweek_start = \"monday\"\nreport_format = \"json\"\n")
	h.respond("time-list", "[]")

	doc := h.jsonOut(now, "report", "week")
	if got := jsonNumber(t, doc, "totalSeconds"); got != 0 {
		t.Errorf("totalSeconds: %d", got)
	}
}

func TestAWeekInADifferentZoneShiftsItsBoundaries(t *testing.T) {
	h := harnessWith(t, "timezone = \"UTC\"\nweek_start = \"monday\"\n")
	// 2026-10-28T03:30Z is Tuesday evening in New York but Wednesday in UTC.
	h.respond("time-list", "["+entryJSON("c", "8c2a", "Design review",
		ms(t, "2026-10-28T03:30:00Z"), ms(t, "2026-10-28T04:30:00Z"))+"]")

	doc := h.jsonOut(now, "report", "week", "--format", "json")
	if jsonField(t, doc, "timezone") != "UTC" {
		t.Errorf("timezone: %v", jsonField(t, doc, "timezone"))
	}
	days := jsonArray(t, doc, "days")
	if got := jsonNumber(t, days[2], "seconds"); got != 3600 {
		t.Errorf("the whole hour lands on Wednesday in UTC, got %d", got)
	}
	if got := jsonNumber(t, days[1], "seconds"); got != 0 {
		t.Errorf("Tuesday: %d", got)
	}
}

func TestATruncatedResultMarksTheReportIncomplete(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", weekEntries(t))

	doc := h.jsonOut(now, "report", "week", "--format", "json", "--limit", "3")
	if jsonField(t, doc, "incomplete") != true {
		t.Error("a result that fills the limit is a lower bound")
	}
	if got := jsonNumber(t, doc, "runningEntries"); got != 0 {
		t.Errorf("nothing is running; the limit alone made it partial, got %d", got)
	}
	warnings := jsonArray(t, doc, "warnings")
	found := false
	for _, warning := range warnings {
		text, _ := warning.(string)
		found = found || strings.Contains(text, "may be incomplete")
	}
	if !found {
		t.Errorf("warnings: %v", warnings)
	}
}

func TestAPaginationCursorMarksTheReportIncomplete(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", `{"entries":[],"nextCursor":"abc"}`)

	doc := h.jsonOut(now, "report", "week", "--format", "json")
	if jsonField(t, doc, "incomplete") != true {
		t.Error("tt cannot follow the cursor, so totals are partial")
	}
}
