package cli

import (
	"bytes"
	"io"
	"strings"
	"testing"

	"github.com/spf13/cobra"

	"github.com/dsmk/time-tracking/internal/tterr"
)

// parse runs the command tree far enough to exercise argument validation. The
// ot client is pointed at a path that cannot exist, so a command that gets as
// far as talking to OpenTodo fails with a client error (exit 7) and a command
// rejected on its arguments fails with a usage error (exit 2) — which is
// exactly the distinction these tests are about.
func parse(t *testing.T, args ...string) error {
	t.Helper()
	root := New()
	root.SetArgs(append([]string{"--ot-command", "/nonexistent/tt-test-ot"}, args...))
	root.SetOut(io.Discard)
	root.SetErr(io.Discard)
	return root.Execute()
}

func TestTheCommandTreeIsValid(t *testing.T) {
	var missing []string
	var walk func(cmd *cobra.Command)
	walk = func(cmd *cobra.Command) {
		if cmd.Short == "" {
			missing = append(missing, cmd.CommandPath())
		}
		for _, child := range cmd.Commands() {
			walk(child)
		}
	}
	walk(New())
	if len(missing) > 0 {
		t.Errorf("commands without a description: %v", missing)
	}
}

func TestEveryDocumentedCommandExists(t *testing.T) {
	want := []string{"start", "stop", "status", "log", "report", "sync"}
	root := New()
	for _, name := range want {
		found := false
		for _, cmd := range root.Commands() {
			found = found || cmd.Name() == name
		}
		if !found {
			t.Errorf("tt %s is missing", name)
		}
	}
}

func TestStartTakesAnOptionalNoteAndATask(t *testing.T) {
	// It reaches the adapter, which is as far as a well-formed command can get
	// without an ot client.
	err := parse(t, "start", "Design review", "--task", "8c2a")
	if tterr.KindOf(err) != tterr.KindClient {
		t.Fatalf("a well-formed start should reach the adapter, got %v", err)
	}
}

func TestStartTakesAtMostOneNote(t *testing.T) {
	err := parse(t, "start", "one", "two")
	if err == nil || tterr.KindOf(err) == tterr.KindClient {
		t.Fatalf("a second positional should be refused, got %v", err)
	}
}

func TestTodayCannotBeCombinedWithAnExplicitRange(t *testing.T) {
	for _, flag := range []string{"--from", "--to"} {
		err := parse(t, "log", "--today", flag, "2026-10-20")
		if tterr.KindOf(err) != tterr.KindUsage {
			t.Errorf("%s with --today should be a usage error, got %v", flag, err)
		}
		if err == nil || !strings.Contains(err.Error(), "today") {
			t.Errorf("the conflict should name the flag: %v", err)
		}
	}
}

func TestReportRangeRequiresBothEnds(t *testing.T) {
	err := parse(t, "report", "range", "--from", "2026-10-01")
	if tterr.KindOf(err) != tterr.KindUsage {
		t.Fatalf("a half range should be a usage error, got %v", err)
	}
	if !strings.Contains(err.Error(), "to") {
		t.Errorf("the missing flag should be named: %v", err)
	}
}

func TestSyncHasAnOptionalStatusSubcommand(t *testing.T) {
	root := New()
	sync, _, err := root.Find([]string{"sync"})
	if err != nil {
		t.Fatal(err)
	}
	if !sync.Runnable() {
		t.Error("`tt sync` must do something of its own")
	}
	status, _, err := root.Find([]string{"sync", "status"})
	if err != nil || status.Name() != "status" {
		t.Fatalf("got %v, %v", status, err)
	}
}

func TestAnUnknownFlagIsAUsageError(t *testing.T) {
	err := parse(t, "log", "--nope")
	if tterr.KindOf(err) != tterr.KindUsage {
		t.Fatalf("got %v", err)
	}
}

func TestHelpAndVersionNeedNoOtClient(t *testing.T) {
	for _, args := range [][]string{{"--version"}, {"--help"}, {"log", "--help"}} {
		root := New()
		var out bytes.Buffer
		root.SetArgs(append([]string{"--ot-command", "/nonexistent/tt-test-ot"}, args...))
		root.SetOut(&out)
		root.SetErr(&out)
		if err := root.Execute(); err != nil {
			t.Errorf("%v: %v", args, err)
		}
		if out.Len() == 0 {
			t.Errorf("%v printed nothing", args)
		}
	}
}

func TestAGroupingCommandRunBareIsAUsageError(t *testing.T) {
	// `tt` and `tt report` do nothing on their own, so running them bare must
	// report that rather than exiting zero having done nothing.
	for _, args := range [][]string{{}, {"report"}} {
		err := parse(t, args...)
		if tterr.KindOf(err) != tterr.KindUsage {
			t.Errorf("tt %v: got %v", args, err)
		}
		if err == nil || !strings.Contains(err.Error(), "needs a subcommand") {
			t.Errorf("tt %v: got %v", args, err)
		}
	}
}

func TestAGroupingCommandNeedsNoOtClient(t *testing.T) {
	// It must fail on its arguments, not on a missing client: the ot_command
	// above does not exist, and a client error would mean tt went looking.
	if err := parse(t); tterr.KindOf(err) == tterr.KindClient {
		t.Errorf("bare tt should not reach for ot: %v", err)
	}
}
