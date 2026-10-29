package ot

import (
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/dsmk/time-tracking/internal/civil"
	"github.com/dsmk/time-tracking/internal/tterr"
)

func TestTimeListArgumentsUseOnlyDocumentedFlags(t *testing.T) {
	from, to := civil.New(2026, time.October, 20), civil.New(2026, time.October, 26)
	args := TimeListArgs(EntryFilter{From: &from, To: &to, Task: "8c2a", Limit: 500})
	want := []string{
		"time", "list", "--json",
		"--from=2026-10-20", "--to=2026-10-26", "--task=8c2a", "--limit=500",
	}
	if err := equalArgs(args, want); err != nil {
		t.Fatal(err)
	}
}

func TestRunningFilterQueriesTheDocumentedRunningFlag(t *testing.T) {
	c, runner := client(t)
	runner.push("time list", ok("[]"))
	if _, _, err := c.RunningTimer(); err != nil {
		t.Fatalf("query runs: %v", err)
	}
	if err := equalArgs(runner.call(0), []string{"time", "list", "--json", "--running"}); err != nil {
		t.Fatal(err)
	}
}

func TestANoteIsPassedAsOneArgumentNeverThroughAShell(t *testing.T) {
	c, runner := client(t)
	runner.push("time start", ok("{}"))
	if _, _, err := c.StartTimer("8c2a", "rm -rf / ; echo $(whoami)"); err != nil {
		t.Fatalf("starts: %v", err)
	}
	want := []string{"time", "start", "8c2a", "--note=rm -rf / ; echo $(whoami)", "--json"}
	if err := equalArgs(runner.call(0), want); err != nil {
		t.Fatal(err)
	}
}

func TestANoteThatStartsWithADashCannotBecomeAFlag(t *testing.T) {
	c, runner := client(t)
	runner.push("time start", ok("{}"))
	if _, _, err := c.StartTimer("8c2a", "--server=https://evil.example"); err != nil {
		t.Fatalf("starts: %v", err)
	}
	if got := runner.call(0)[3]; got != "--note=--server=https://evil.example" {
		t.Fatalf("the note became a flag: %q", got)
	}
}

func TestATaskIDThatStartsWithADashIsRefusedBeforeOtRuns(t *testing.T) {
	c, runner := client(t)
	_, _, err := c.StartTimer("--server", "")
	if tterr.KindOf(err) != tterr.KindUsage {
		t.Fatalf("want a usage error, got %v", err)
	}
	if len(runner.calls) != 0 {
		t.Fatalf("ot must not be invoked, but was: %v", runner.calls)
	}
}

func TestAnEmptyNoteIsOmittedRatherThanSentEmpty(t *testing.T) {
	c, runner := client(t)
	runner.push("time start", ok("{}"))
	if _, _, err := c.StartTimer("8c2a", "   "); err != nil {
		t.Fatalf("starts: %v", err)
	}
	if err := equalArgs(runner.call(0), []string{"time", "start", "8c2a", "--json"}); err != nil {
		t.Fatal(err)
	}
}

func TestExitCodesMapToTheDocumentedMeanings(t *testing.T) {
	cases := []struct {
		code int
		kind tterr.Kind
	}{
		{1, tterr.KindServer},
		{2, tterr.KindUsage},
		{3, tterr.KindAuth},
		{4, tterr.KindUnreachable},
		{5, tterr.KindForbidden},
	}
	for _, tc := range cases {
		c, runner := client(t)
		runner.push("time list", fail(tc.code, "", "ot: something went wrong"))
		_, err := c.ListTimeEntries(EntryFilter{})
		if got := tterr.KindOf(err); got != tc.kind {
			t.Errorf("exit %d: got kind %v, want %v", tc.code, got, tc.kind)
		}
		if got := tterr.ExitCodeOf(err); got != tc.code {
			t.Errorf("tt must mirror ot's exit code: got %d, want %d", got, tc.code)
		}
	}
}

func TestTheErrorEnvelopeMessageAndCodeReachTheUser(t *testing.T) {
	c, runner := client(t)
	runner.push("time start", fail(5,
		`{"error":{"code":"insufficient_scope","message":"token is read-only","exit":5}}`,
		"ot: token is read-only"))

	_, _, err := c.StartTimer("8c2a", "")
	var ttError *tterr.Error
	if !errors.As(err, &ttError) {
		t.Fatalf("want a tt error, got %v", err)
	}
	for _, want := range []string{"token is read-only", "insufficient_scope"} {
		if !strings.Contains(ttError.Message, want) {
			t.Errorf("message %q does not mention %q", ttError.Message, want)
		}
	}
	if !strings.Contains(ttError.Hint, "read-only") {
		t.Errorf("hint %q does not explain the scope", ttError.Hint)
	}
}

func TestSecretsInOtErrorsAreRedacted(t *testing.T) {
	c, runner := client(t)
	runner.push("time list", fail(3,
		`{"error":{"code":"unauthorized","message":"bad token ot_9f8e7d6c5b4a3210ffee"}}`, ""))

	_, err := c.ListTimeEntries(EntryFilter{})
	if err == nil || strings.Contains(err.Error(), "ot_9f8e7d6c5b4a3210ffee") {
		t.Fatalf("the token leaked: %v", err)
	}
}

func TestAMissingClientIsReportedWithInstallGuidance(t *testing.T) {
	runner := bareMock(t)
	runner.push("version", runFails(&RunError{Kind: RunNotFound}))

	_, err := New("ot", runner).Version()
	var ttError *tterr.Error
	if !errors.As(err, &ttError) || ttError.Kind != tterr.KindClient {
		t.Fatalf("want a client error, got %v", err)
	}
	if ttError.ExitCode() != 7 {
		t.Errorf("got exit %d, want 7", ttError.ExitCode())
	}
	if !strings.Contains(ttError.Hint, "ot_command") {
		t.Errorf("hint %q does not name the setting", ttError.Hint)
	}
}

func TestATimeoutNeverClaimsTheOperationSucceeded(t *testing.T) {
	runner := bareMock(t)
	runner.push("time stop", runFails(&RunError{Kind: RunTimeout, Timeout: 30 * time.Second}))

	_, _, err := New("ot", runner).StopTimer()
	var ttError *tterr.Error
	if !errors.As(err, &ttError) || ttError.Kind != tterr.KindClient {
		t.Fatalf("want a client error, got %v", err)
	}
	if !strings.Contains(ttError.Hint, "no result was confirmed") {
		t.Errorf("hint %q claims too much", ttError.Hint)
	}
}

func TestSyncIsReportedAsUnsupportedWithoutInvokingAnything(t *testing.T) {
	c, runner := client(t)
	err := c.Sync()
	if tterr.KindOf(err) != tterr.KindUnsupported {
		t.Fatalf("want unsupported, got %v", err)
	}
	if tterr.ExitCodeOf(err) != 6 {
		t.Errorf("got exit %d, want 6", tterr.ExitCodeOf(err))
	}
	if len(runner.calls) != 0 {
		t.Fatalf("nothing may be invented and invoked: %v", runner.calls)
	}
}

func TestUnreachableBecomesAClientStateNotAHardError(t *testing.T) {
	c, runner := client(t)
	runner.push("auth status", fail(4, "", "ot: server unreachable"))

	state, err := c.ClientState()
	if err != nil {
		t.Fatalf("state is reported, not raised: %v", err)
	}
	if state.Kind != StateUnreachable {
		t.Fatalf("got %v, want StateUnreachable", state.Kind)
	}
}

func TestClientStateReadsTheDocumentedFields(t *testing.T) {
	c, runner := client(t)
	runner.push("auth status",
		ok(`{"server":"https://todo.example.com","account":"dk@example.com","scope":"full"}`))

	state, err := c.ClientState()
	if err != nil {
		t.Fatalf("auth status: %v", err)
	}
	if state.Server != "https://todo.example.com" || state.Account != "dk@example.com" ||
		state.Scope != "full" {
		t.Fatalf("got %+v", state)
	}
}

func TestAMissingClientIsAFailureNotASyncState(t *testing.T) {
	runner := bareMock(t)
	runner.push("auth status", runFails(&RunError{Kind: RunNotFound}))

	if _, err := New("ot", runner).ClientState(); tterr.KindOf(err) != tterr.KindClient {
		t.Fatalf("want a client error, got %v", err)
	}
}

func TestVersionCheckRejectsAnUnknownOutputVersion(t *testing.T) {
	two := int64(2)
	_, err := CheckVersion(VersionInfo{OutputVersion: &two, ClientVersion: "1.0.0"})
	if tterr.KindOf(err) != tterr.KindUnsupported {
		t.Fatalf("want unsupported, got %v", err)
	}
}

func TestVersionCheckRejectsAClientOlderThanTimeTracking(t *testing.T) {
	one := int64(1)
	_, err := CheckVersion(VersionInfo{OutputVersion: &one, ClientVersion: "0.8.9"})
	if err == nil || !strings.Contains(err.Error(), "0.9.0") {
		t.Fatalf("want the minimum named, got %v", err)
	}
}

func TestVersionCheckWarnsRatherThanGuessesWhenTheVersionIsAbsent(t *testing.T) {
	one := int64(1)
	warning, err := CheckVersion(VersionInfo{OutputVersion: &one})
	if err != nil {
		t.Fatalf("an unreadable version is not fatal: %v", err)
	}
	if !strings.Contains(warning, "could not determine") {
		t.Fatalf("got %q", warning)
	}
}
