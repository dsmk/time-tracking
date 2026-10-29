package ot

import (
	"errors"
	"strings"
	"testing"
	"time"
)

func TestDisplayQuotesArgumentsThatNeedIt(t *testing.T) {
	shown := Invocation{
		Program: "ot",
		Args:    []string{"time", "start", "8c2a", "--note", "a b; rm -rf /"},
	}.Display()
	if !strings.Contains(shown, `"a b; rm -rf /"`) {
		t.Fatalf("got %q", shown)
	}
}

func TestMissingProgramIsReportedAsNotFound(t *testing.T) {
	_, err := NewProcessRunner(5).Run(Invocation{Program: "tt-no-such-program-exists"})
	var runErr *RunError
	if !errors.As(err, &runErr) || runErr.Kind != RunNotFound {
		t.Fatalf("got %v", err)
	}
}

func TestOutputStreamsAndExitCodeAreCapturedSeparately(t *testing.T) {
	out, err := NewProcessRunner(10).Run(Invocation{
		Program: "/bin/sh",
		Args:    []string{"-c", "printf out; printf err >&2; exit 3"},
	})
	if err != nil {
		t.Fatalf("sh runs: %v", err)
	}
	if out.Code != 3 || out.Stdout != "out" || out.Stderr != "err" {
		t.Fatalf("got %+v", out)
	}
}

func TestAHungChildIsKilledAtTheTimeout(t *testing.T) {
	_, err := ProcessRunner{Timeout: 150 * time.Millisecond}.Run(Invocation{
		Program: "/bin/sh",
		Args:    []string{"-c", "sleep 30"},
	})
	var runErr *RunError
	if !errors.As(err, &runErr) || runErr.Kind != RunTimeout {
		t.Fatalf("got %v", err)
	}
}

func TestTerminationBySignalIsVisibleRatherThanAnExitCode(t *testing.T) {
	out, err := NewProcessRunner(10).Run(Invocation{
		Program: "/bin/sh",
		Args:    []string{"-c", "kill -TERM $$"},
	})
	if err != nil {
		t.Fatalf("sh runs: %v", err)
	}
	if !out.Signaled {
		t.Fatalf("a signalled process must not look like a clean exit: %+v", out)
	}
}

func TestASignalledChildIsNeverReportedAsSuccess(t *testing.T) {
	runner := NewProcessRunner(10)
	c := New("/bin/sh", signalRunner{runner})
	_, err := c.Version()
	if err == nil || !strings.Contains(err.Error(), "terminated before it finished") {
		t.Fatalf("got %v", err)
	}
}

// signalRunner replaces whatever it is asked to run with a process that kills
// itself, so the client's handling of a signalled child can be exercised.
type signalRunner struct{ inner ProcessRunner }

func (s signalRunner) Run(Invocation) (Output, error) {
	return s.inner.Run(Invocation{Program: "/bin/sh", Args: []string{"-c", "kill -TERM $$"}})
}
