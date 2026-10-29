package ot

import (
	"errors"
	"strings"
	"testing"

	"github.com/dsmk/time-tracking/internal/tterr"
)

const sampleEntry = `{
	"id": "e1", "taskId": "8c2a", "userId": "u1",
	"startedAt": 1793109600000, "endedAt": 1793113200000,
	"minutes": 60, "note": "review", "deviceId": "d1",
	"taskTitle": "Design review", "projectId": "p1", "projectName": "Work"
}`

func TestParsesTheDocumentedCamelCaseEntry(t *testing.T) {
	page, err := ParseEntryPage("[" + sampleEntry + "]")
	if err != nil {
		t.Fatalf("parses: %v", err)
	}
	entry := page.Entries[0]
	checks := []struct{ got, want string }{
		{entry.ID, "e1"},
		{entry.TaskID, "8c2a"},
		{entry.TaskTitle, "Design review"},
		{entry.ProjectName, "Work"},
		{entry.Note, "review"},
		{entry.StartedAt.Format("2006-01-02T15:04:05Z"), "2026-10-27T14:00:00Z"},
		{entry.EndedAt.Format("2006-01-02T15:04:05Z"), "2026-10-27T15:00:00Z"},
	}
	for _, check := range checks {
		if check.got != check.want {
			t.Errorf("got %q, want %q", check.got, check.want)
		}
	}
	if entry.Minutes == nil || *entry.Minutes != 60 {
		t.Errorf("minutes: %v", entry.Minutes)
	}
	if entry.IsRunning() {
		t.Error("a closed entry is not running")
	}
}

func TestAcceptsTheHTTPEnvelopeToo(t *testing.T) {
	page, err := ParseEntryPage(
		`{"entries":[` + sampleEntry + `],"nextCursor":"abc","serverTime":1793113200000}`)
	if err != nil {
		t.Fatalf("parses: %v", err)
	}
	if len(page.Entries) != 1 {
		t.Fatalf("got %d entries", len(page.Entries))
	}
	if page.NextCursor != "abc" {
		t.Errorf("cursor: %q", page.NextCursor)
	}
	if page.ServerTime == nil ||
		page.ServerTime.Format("2006-01-02T15:04:05Z") != "2026-10-27T15:00:00Z" {
		t.Errorf("server time: %v", page.ServerTime)
	}
}

func TestANullEndedAtMeansRunning(t *testing.T) {
	page, err := ParseEntryPage(
		`[{"id":"e1","taskId":"8c2a","startedAt":1793109600000,"endedAt":null}]`)
	if err != nil {
		t.Fatalf("parses: %v", err)
	}
	if !page.Entries[0].IsRunning() {
		t.Error("a null endedAt means the entry is still running")
	}
}

func TestEmptyOutputIsAnEmptyPageNotAnError(t *testing.T) {
	for _, text := range []string{"  ", "[]"} {
		page, err := ParseEntryPage(text)
		if err != nil {
			t.Fatalf("%q: %v", text, err)
		}
		if len(page.Entries) != 0 {
			t.Errorf("%q produced %d entries", text, len(page.Entries))
		}
	}
}

func TestSnakeCaseAndRFC3339AreTolerated(t *testing.T) {
	page, err := ParseEntryPage(
		`[{"id":"e1","task_id":"8c2a","started_at":"2026-10-27T14:00:00Z"}]`)
	if err != nil {
		t.Fatalf("parses: %v", err)
	}
	entry := page.Entries[0]
	if entry.TaskID != "8c2a" {
		t.Errorf("task: %q", entry.TaskID)
	}
	if got := entry.StartedAt.Format("2006-01-02T15:04:05Z"); got != "2026-10-27T14:00:00Z" {
		t.Errorf("start: %q", got)
	}
}

func TestGarbageOutputIsNotSilentlyTreatedAsNoEntries(t *testing.T) {
	_, err := ParseEntryPage("Mon 09:00-09:25  25m  Design review")
	if tterr.KindOf(err) != tterr.KindServer {
		t.Fatalf("want a server error, got %v", err)
	}
	var ttError *tterr.Error
	if !errors.As(err, &ttError) || !strings.Contains(ttError.Hint, "ot schema") {
		t.Errorf("hint does not point at ot schema: %v", err)
	}
}

func TestAnEntryWithoutAStartIsRejected(t *testing.T) {
	_, err := ParseEntryPage(`[{"id":"e1"}]`)
	if err == nil || !strings.Contains(err.Error(), "startedAt") {
		t.Fatalf("want the missing field named, got %v", err)
	}
}

func TestAnEntryThatEndsBeforeItStartsIsRejected(t *testing.T) {
	_, err := ParseEntryPage(
		`[{"id":"e1","startedAt":1793113200000,"endedAt":1793109600000}]`)
	if err == nil || !strings.Contains(err.Error(), "ends before it starts") {
		t.Fatalf("got %v", err)
	}
}

func TestParsesTheDocumentedErrorEnvelope(t *testing.T) {
	envelope, ok := ParseError(
		`{"error":{"code":"insufficient_scope","message":"read token","exit":5}}`, "")
	if !ok {
		t.Fatal("no envelope")
	}
	if envelope.Code != "insufficient_scope" {
		t.Errorf("code: %q", envelope.Code)
	}
	if envelope.Exit == nil || *envelope.Exit != 5 {
		t.Errorf("exit: %v", envelope.Exit)
	}
}

func TestVersionOutputIsReadLeniently(t *testing.T) {
	info := ParseVersion(`{"output_version":1,"version":"0.13.0"}`)
	if info.OutputVersion == nil || *info.OutputVersion != 1 {
		t.Errorf("output version: %v", info.OutputVersion)
	}
	if info.ClientVersion != "0.13.0" {
		t.Errorf("client version: %q", info.ClientVersion)
	}

	nested := ParseVersion(`{"output_version":1,"ot":{"version":"v0.9.1-rc2"}}`)
	if nested.ClientVersion != "v0.9.1-rc2" {
		t.Errorf("nested version: %q", nested.ClientVersion)
	}

	// Undocumented shape: no panic, no guess.
	if unknown := ParseVersion(`{"output_version":1}`); unknown.ClientVersion != "" {
		t.Errorf("a version was invented: %q", unknown.ClientVersion)
	}
}

func TestSemverParsingHandlesPrefixesAndPrereleases(t *testing.T) {
	cases := []struct {
		text string
		want Semver
		ok   bool
	}{
		{"0.13.0", Semver{0, 13, 0}, true},
		{"v0.9.1-rc2", Semver{0, 9, 1}, true},
		{"0.9", Semver{0, 9, 0}, true},
		{"nightly", Semver{}, false},
		{"dev", Semver{}, false},
	}
	for _, tc := range cases {
		got, ok := ParseSemver(tc.text)
		if ok != tc.ok || got != tc.want {
			t.Errorf("%q: got (%v, %v), want (%v, %v)", tc.text, got, ok, tc.want, tc.ok)
		}
	}
}

func TestTasksParseFromAnArrayOrAWrapper(t *testing.T) {
	tasks, err := ParseTasks(`[{"id":"8c2a","title":"Design review","projectName":"Work"}]`)
	if err != nil {
		t.Fatalf("parses: %v", err)
	}
	if tasks[0].Title != "Design review" || tasks[0].ProjectName != "Work" {
		t.Errorf("got %+v", tasks[0])
	}

	wrapped, err := ParseTasks(`{"tasks":[{"id":"1","title":"x"}]}`)
	if err != nil || len(wrapped) != 1 {
		t.Fatalf("got %v, %v", wrapped, err)
	}
}
