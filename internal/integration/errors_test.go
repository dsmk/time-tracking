package integration

import (
	"strings"
	"testing"
)

// Error handling, capability limits, and sync reporting.
//
// These are the tests that keep tt honest: SPEC.md §10 and §12 turn on never
// reporting success that OpenTodo did not confirm.

func TestAMissingOtClientIsReportedWithInstallationGuidance(t *testing.T) {
	h := harnessWith(t, "timezone = \"UTC\"\n")
	h.rewriteConfig("ot_command = \"tt-no-such-ot-client\"\ntimezone = \"UTC\"\n")

	result := h.fail(now, "status")
	if result.code != 7 {
		t.Errorf("exit: %d", result.code)
	}
	for _, want := range []string{"was not found", "ot_command"} {
		if !strings.Contains(result.stderr, want) {
			t.Errorf("%q is missing from %q", want, result.stderr)
		}
	}
	if result.stdout != "" {
		t.Errorf("a failure must not print results: %q", result.stdout)
	}
}

func TestAnUnreachableServerExitsFourAndSaysTtCannotRecordLocally(t *testing.T) {
	h := newHarness(t)
	h.respondFail("time-list", 4,
		`{"error":{"code":"unreachable","message":"no response from server within 15s","exit":4}}`,
		"ot: no response from server within 15s\n")

	result := h.fail(now, "status")
	if result.code != 4 {
		t.Errorf("tt mirrors ot's exit code, got %d", result.code)
	}
	for _, want := range []string{"no response from server", "online-only", "cannot record this locally"} {
		if !strings.Contains(result.stderr, want) {
			t.Errorf("%q is missing from %q", want, result.stderr)
		}
	}
}

func TestAReadOnlyTokenIsExplainedRatherThanSwallowed(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "[]")
	h.respondFail("time-start", 5,
		`{"error":{"code":"insufficient_scope","message":"token scope read cannot write","exit":5}}`,
		"ot: insufficient scope\n")

	result := h.fail(now, "start", "Design review", "--task", "8c2a")
	if result.code != 5 {
		t.Errorf("exit: %d", result.code)
	}
	for _, want := range []string{"token scope read cannot write", "insufficient_scope", "ot auth status"} {
		if !strings.Contains(result.stderr, want) {
			t.Errorf("%q is missing from %q", want, result.stderr)
		}
	}
}

func TestAnAuthenticationFailurePointsAtOtLogin(t *testing.T) {
	h := newHarness(t)
	h.respondFail("time-list", 3,
		`{"error":{"code":"unauthorized","message":"token rejected","exit":3}}`, "")

	result := h.fail(now, "status")
	if result.code != 3 {
		t.Errorf("exit: %d", result.code)
	}
	if !strings.Contains(result.stderr, "ot login") {
		t.Errorf("got %q", result.stderr)
	}
}

func TestAnUnknownTaskSurfacesOpenTodosOwnErrorCode(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "[]")
	h.respondFail("time-start", 1,
		`{"error":{"code":"not_found","message":"no task with id zzzz","exit":1}}`, "")

	result := h.fail(now, "start", "x", "--task", "zzzz")
	if result.code != 1 {
		t.Errorf("exit: %d", result.code)
	}
	for _, want := range []string{"no task with id zzzz", "not_found"} {
		if !strings.Contains(result.stderr, want) {
			t.Errorf("%q is missing from %q", want, result.stderr)
		}
	}
}

func TestATaskIDThatLooksLikeAFlagNeverReachesOt(t *testing.T) {
	h := newHarness(t)
	h.respond("time-list", "[]")

	result := h.fail(now, "start", "x", "--task", "--server=https://evil.example")
	if result.code != 2 {
		t.Errorf("exit: %d", result.code)
	}
	if h.callCount("time", "start") != 0 {
		t.Error("nothing may be sent to OpenTodo")
	}
}

func TestHumanOrientedOutputIsRejectedInsteadOfReadAsNoEntries(t *testing.T) {
	h := newHarness(t)
	// What `ot time list` prints without --json. Treating this as "no entries"
	// would quietly produce an empty, wrong report.
	h.respond("time-list", "Mon 09:00-09:25  25m  Design review (Work)\n")

	result := h.fail(now, "report", "week")
	if result.code != 1 {
		t.Errorf("exit: %d", result.code)
	}
	for _, want := range []string{"could not read the JSON", "ot schema"} {
		if !strings.Contains(result.stderr, want) {
			t.Errorf("%q is missing from %q", want, result.stderr)
		}
	}
}

func TestAnUnexpectedOtSubcommandWouldFailLoudly(t *testing.T) {
	// The mock answers only what a test installs; nothing is stubbed here, so
	// any ot call at all fails. This is what stops an invented subcommand from
	// passing the suite unnoticed.
	h := newHarness(t)
	result := h.fail(now, "log")
	if result.code != 2 {
		t.Errorf("exit: %d", result.code)
	}
	if !strings.Contains(result.stderr, "unexpected_command") {
		t.Errorf("got %q", result.stderr)
	}
}

func TestAnUnsupportedOutputVersionStopsBeforeAnyDataIsRead(t *testing.T) {
	h := newHarness(t)
	h.respond("version", `{"output_version":2,"version":"1.4.0"}`)

	result := h.fail(now, "status")
	if result.code != 6 {
		t.Errorf("exit: %d", result.code)
	}
	if !strings.Contains(result.stderr, "output_version 2") {
		t.Errorf("got %q", result.stderr)
	}
	if h.callCount("time", "list") != 0 {
		t.Error("no data may be read through parsers that do not fit")
	}
}

func TestAnOtOlderThanTimeTrackingIsRefused(t *testing.T) {
	h := newHarness(t)
	h.respond("version", `{"output_version":1,"version":"0.8.3"}`)

	result := h.fail(now, "status")
	if result.code != 6 {
		t.Errorf("exit: %d", result.code)
	}
	for _, want := range []string{"too old", "0.9.0"} {
		if !strings.Contains(result.stderr, want) {
			t.Errorf("%q is missing from %q", want, result.stderr)
		}
	}
}

func TestAnUnreadableVersionWarnsButDoesNotBlockWork(t *testing.T) {
	h := newHarness(t)
	h.respond("version", `{"output_version":1,"build":"unknown"}`)
	h.respond("time-list", "[]")

	result := h.run(now, "status")
	if result.code != 0 {
		t.Fatalf("exit %d: %s", result.code, result.stderr)
	}
	if !strings.Contains(result.stderr, "could not determine the OpenTodo client version") {
		t.Errorf("got %q", result.stderr)
	}
}

func TestTheVersionProbeCanBeTurnedOff(t *testing.T) {
	h := harnessWith(t, "timezone = \"UTC\"\nversion_check = false\n")
	h.respond("time-list", "[]")

	h.ok(now, "status")
	if h.callCount("version") != 0 {
		t.Error("the probe was asked for and declined")
	}
}

func TestTtSyncReportsThatOpenTodoDocumentsNoSyncCommand(t *testing.T) {
	h := newHarness(t)

	result := h.fail(now, "sync")
	if result.code != 6 {
		t.Errorf("exit 6 means the capability is not supported, got %d", result.code)
	}
	for _, want := range []string{"no documented command", "online-only", "vault sync"} {
		if !strings.Contains(result.stderr, want) {
			t.Errorf("%q is missing from %q", want, result.stderr)
		}
	}
	if h.callCount("sync") != 0 || h.callCount("vault") != 0 {
		t.Error("nothing may be invented and invoked")
	}
}

func TestSyncStatusNeverClaimsSynchronizedJustBecauseACommandWorked(t *testing.T) {
	h := newHarness(t)
	h.respond("auth-status",
		`{"server":"https://todo.example.com","account":"dk@example.com","scope":"full"}`)

	out := h.ok(now, "sync", "status")
	wants := []string{
		"Sync status: unavailable",
		"not the same as synced",
		"https://todo.example.com",
		"token scope full",
		"none possible",
		"OpenTodo app",
	}
	for _, want := range wants {
		if !strings.Contains(out, want) {
			t.Errorf("%q is missing from:\n%s", want, out)
		}
	}
	if strings.Contains(strings.ToLower(out), "synchronized") {
		t.Errorf("the word tt must never print:\n%s", out)
	}
}

func TestSyncStatusJSONSeparatesSyncStateFromReachability(t *testing.T) {
	h := newHarness(t)
	h.respond("auth-status", `{"server":"https://todo.example.com","scope":"read"}`)

	doc := h.jsonOut(now, "sync", "status", "--json")
	checks := map[string]any{
		"syncStatus":          "unavailable",
		"syncStatusKnown":     false,
		"serverReachable":     true,
		"pendingLocalChanges": float64(0),
		"tokenScope":          "read",
	}
	for key, want := range checks {
		if got := jsonField(t, doc, key); got != want {
			t.Errorf("%s: got %v, want %v", key, got, want)
		}
	}
}

func TestSyncStatusReportsOfflineWhenTheServerCannotBeReached(t *testing.T) {
	h := newHarness(t)
	h.respondFail("auth-status", 4,
		`{"error":{"code":"unreachable","message":"no response within 15s","exit":4}}`, "")

	if out := h.ok(now, "sync", "status"); !strings.Contains(out, "Sync status: offline") {
		t.Errorf("got:\n%s", out)
	}

	other := newHarness(t)
	other.respondFail("auth-status", 4, `{"error":{"code":"unreachable","exit":4}}`, "")
	doc := other.jsonOut(now, "sync", "status", "--json")
	if jsonField(t, doc, "syncStatus") != "offline" {
		t.Errorf("syncStatus: %v", jsonField(t, doc, "syncStatus"))
	}
	if jsonField(t, doc, "serverReachable") != false {
		t.Errorf("serverReachable: %v", jsonField(t, doc, "serverReachable"))
	}
}

func TestSyncStatusReportsARejectedTokenAsSignedOut(t *testing.T) {
	h := newHarness(t)
	h.respondFail("auth-status", 3,
		`{"error":{"code":"unauthorized","message":"token rejected","exit":3}}`, "")

	doc := h.jsonOut(now, "sync", "status", "--json")
	if jsonField(t, doc, "syncStatus") != "signed_out" {
		t.Errorf("syncStatus: %v", jsonField(t, doc, "syncStatus"))
	}
	if jsonField(t, doc, "serverReachable") != false {
		t.Errorf("serverReachable: %v", jsonField(t, doc, "serverReachable"))
	}
}

func TestAConfigFileHoldingASecretIsRejected(t *testing.T) {
	h := newHarness(t)
	h.rewriteConfig("token = \"super-secret\"\n")

	result := h.fail(now, "status")
	if result.code != 2 {
		t.Errorf("exit: %d", result.code)
	}
	if !strings.Contains(result.stderr, "credentials") {
		t.Errorf("got %q", result.stderr)
	}
	if strings.Contains(result.stderr, "super-secret") {
		t.Errorf("the value must not be echoed: %q", result.stderr)
	}
}

func TestAnUnknownTimezoneIsActionable(t *testing.T) {
	h := newHarness(t)
	result := h.fail(now, "--timezone", "Mars/Olympus", "status")
	if result.code != 2 {
		t.Errorf("exit: %d", result.code)
	}
	if !strings.Contains(result.stderr, "IANA") {
		t.Errorf("got %q", result.stderr)
	}
}

func TestAGlobalFlagWorksAfterTheSubcommandToo(t *testing.T) {
	h := newHarness(t)
	result := h.fail(now, "status", "--timezone", "Mars/Olympus")
	if result.code != 2 {
		t.Errorf("exit: %d", result.code)
	}
	if !strings.Contains(result.stderr, "IANA") {
		t.Errorf("got %q", result.stderr)
	}
}

func TestABareGroupingCommandExitsTwoWithoutCallingOt(t *testing.T) {
	for _, args := range [][]string{{}, {"report"}} {
		h := newHarness(t)
		result := h.run(now, args...)
		if result.code != 2 {
			t.Errorf("tt %v: exit %d", args, result.code)
		}
		if !strings.Contains(result.stderr, "needs a subcommand") {
			t.Errorf("tt %v: %q", args, result.stderr)
		}
		if len(h.calls()) != 0 {
			t.Errorf("tt %v talked to ot: %v", args, h.calls())
		}
	}
}
