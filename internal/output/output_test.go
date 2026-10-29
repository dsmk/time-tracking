package output

import (
	"encoding/json"
	"io"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/dsmk/time-tracking/internal/civil"
	"github.com/dsmk/time-tracking/internal/ot"
	"github.com/dsmk/time-tracking/internal/report"
)

func newYork(t *testing.T) *time.Location {
	t.Helper()
	loc, err := time.LoadLocation("America/New_York")
	if err != nil {
		t.Fatalf("tzdb: %v", err)
	}
	return loc
}

func instant(t *testing.T, raw string) time.Time {
	t.Helper()
	parsed, err := time.Parse(time.RFC3339, raw)
	if err != nil {
		t.Fatalf("timestamp: %v", err)
	}
	return parsed.UTC()
}

func sample(t *testing.T) ot.TimeEntry {
	t.Helper()
	ended := instant(t, "2026-10-26T13:25:00Z")
	minutes := int64(25)
	return ot.TimeEntry{
		ID:          "e1",
		TaskID:      "8c2a",
		TaskTitle:   "Design review",
		ProjectID:   "p1",
		ProjectName: "Work",
		Note:        "review",
		StartedAt:   instant(t, "2026-10-26T13:00:00Z"),
		EndedAt:     &ended,
		Minutes:     &minutes,
	}
}

func TestAnEntryLineShowsLocalTimes(t *testing.T) {
	line := EntryLine(sample(t), newYork(t), instant(t, "2026-10-26T20:00:00Z"))
	for _, want := range []string{"Mon 2026-10-26", "09:00-09:25", "25m", "[8c2a]"} {
		if !strings.Contains(line, want) {
			t.Errorf("%q is missing from %q", want, line)
		}
	}
}

func TestARunningEntryIsMarkedAndMeasuredToNow(t *testing.T) {
	entry := sample(t)
	entry.EndedAt = nil
	now := instant(t, "2026-10-26T14:00:00Z")

	line := EntryLine(entry, newYork(t), now)
	for _, want := range []string{"09:00-now", "running"} {
		if !strings.Contains(line, want) {
			t.Errorf("%q is missing from %q", want, line)
		}
	}
	if got := report.EntrySeconds(entry, now); got != 3600 {
		t.Errorf("elapsed: %d", got)
	}
}

func TestEntryJSONReportsBothUTCAndLocalStarts(t *testing.T) {
	entry := EntryJSON(sample(t), newYork(t), instant(t, "2026-10-26T20:00:00Z"))
	if entry.StartedAt != "2026-10-26T13:00:00Z" {
		t.Errorf("startedAt: %q", entry.StartedAt)
	}
	if entry.StartedLocal != "2026-10-26T09:00:00-04:00" {
		t.Errorf("startedLocal: %q", entry.StartedLocal)
	}
	if entry.OpenTodoMinutes == nil || *entry.OpenTodoMinutes != 25 {
		t.Errorf("openTodoMinutes: %v", entry.OpenTodoMinutes)
	}
	if entry.Seconds != 1500 {
		t.Errorf("seconds: %d", entry.Seconds)
	}
}

func TestAnAbsentFieldIsJSONNullNotAnEmptyString(t *testing.T) {
	entry := sample(t)
	entry.Note = ""
	entry.TaskID = ""

	text, err := json.Marshal(EntryJSON(entry, newYork(t), instant(t, "2026-10-26T20:00:00Z")))
	if err != nil {
		t.Fatal(err)
	}
	var decoded map[string]any
	if err := json.Unmarshal(text, &decoded); err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{"note", "taskId"} {
		if decoded[key] != nil {
			t.Errorf("%s should be null, got %v", key, decoded[key])
		}
	}
}

func TestReportJSONCarriesTheIncompleteFlagAndWarnings(t *testing.T) {
	entry := sample(t)
	entry.EndedAt = nil
	monday := time.Monday
	period, err := report.NewPeriod(
		civil.New(2026, time.October, 26), civil.New(2026, time.October, 26))
	if err != nil {
		t.Fatal(err)
	}

	built := report.Build(period, newYork(t), "America/New_York", &monday,
		[]ot.TimeEntry{entry}, instant(t, "2026-10-26T14:00:00Z"), nil, false)
	rendered := ReportJSON(built)

	if !rendered.Incomplete {
		t.Error("a running entry makes the report incomplete")
	}
	if rendered.WeekStart == nil || *rendered.WeekStart != "monday" {
		t.Errorf("weekStart: %v", rendered.WeekStart)
	}
	if rendered.TotalSeconds != 3600 {
		t.Errorf("totalSeconds: %d", rendered.TotalSeconds)
	}
	if len(rendered.Warnings) == 0 {
		t.Error("the running entry should be explained")
	}
}

func TestAReportWithNoWarningsRendersAnEmptyArrayNotNull(t *testing.T) {
	period, err := report.NewPeriod(
		civil.New(2026, time.October, 26), civil.New(2026, time.October, 26))
	if err != nil {
		t.Fatal(err)
	}
	built := report.Build(period, newYork(t), "UTC", nil, nil,
		instant(t, "2026-10-27T00:00:00Z"), nil, false)

	text, err := json.Marshal(ReportJSON(built))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(text), `"warnings":[]`) {
		t.Errorf("a consumer should get an array to iterate: %s", text)
	}
}

func TestHoursRenderWithoutATrailingZero(t *testing.T) {
	cases := map[int64]string{12600: "3.5", 3600: "1", 0: "0"}
	for seconds, want := range cases {
		if got := formatHours(seconds); got != want {
			t.Errorf("%d seconds: got %q, want %q", seconds, got, want)
		}
	}
}

func TestPrintJSONDoesNotEscapeOrdinaryPunctuation(t *testing.T) {
	stdout := os.Stdout
	read, write, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	os.Stdout = write
	printErr := PrintJSON(map[string]string{"note": "Design & review <draft>"})
	write.Close()
	os.Stdout = stdout
	if printErr != nil {
		t.Fatal(printErr)
	}

	var captured strings.Builder
	if _, err := io.Copy(&captured, read); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(captured.String(), "Design & review <draft>") {
		t.Errorf("punctuation was escaped: %s", captured.String())
	}
}
