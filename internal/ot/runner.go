package ot

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"strings"
	"time"
)

// Invocation is a single ot call, always as a program plus an argument vector.
//
// SPEC.md §4 forbids building a shell string: descriptions, notes and task
// identifiers arrive from the user and must never be re-parsed by a shell.
// Keeping the arguments in a []string all the way to exec.Command means there
// is no point at which quoting could be got wrong.
type Invocation struct {
	Program string
	Args    []string
}

// Display is a human-readable form for error messages only — never executed.
func (i Invocation) Display() string {
	parts := make([]string, 0, len(i.Args)+1)
	parts = append(parts, quote(i.Program))
	for _, arg := range i.Args {
		parts = append(parts, quote(arg))
	}
	return strings.Join(parts, " ")
}

func quote(arg string) string {
	if arg == "" {
		return `""`
	}
	for _, r := range arg {
		safe := (r >= 'a' && r <= 'z') || (r >= 'A' && r <= 'Z') || (r >= '0' && r <= '9') ||
			strings.ContainsRune("-_./:=,", r)
		if !safe {
			return fmt.Sprintf("%q", arg)
		}
	}
	return arg
}

// Output is what one ot invocation produced.
type Output struct {
	// Code is the exit status. Signaled is true instead when the process was
	// terminated by a signal, in which case Code is meaningless.
	Code     int
	Signaled bool
	Stdout   string
	Stderr   string
}

// RunErrorKind says why an invocation never produced an exit status.
type RunErrorKind int

// Why ot could not be run to completion.
const (
	// RunNotFound means the program is not on PATH or at the configured path.
	RunNotFound RunErrorKind = iota
	// RunNotExecutable means it was found but could not be executed.
	RunNotExecutable
	// RunTimeout means it was killed after exceeding the configured timeout.
	RunTimeout
	// RunIO covers everything else.
	RunIO
)

// RunError is a failure to run ot at all, as opposed to ot reporting failure.
type RunError struct {
	Kind    RunErrorKind
	Detail  string
	Timeout time.Duration
}

func (e *RunError) Error() string {
	switch e.Kind {
	case RunNotFound:
		return "program not found"
	case RunNotExecutable:
		return "program could not be executed: " + e.Detail
	case RunTimeout:
		return fmt.Sprintf("timed out after %s", e.Timeout)
	default:
		return e.Detail
	}
}

// Runner executes an ot invocation. ProcessRunner is the real one; unit tests
// substitute a mock so command construction and output handling can be tested
// without ot installed.
type Runner interface {
	Run(invocation Invocation) (Output, error)
}

// ProcessRunner runs ot as a child process.
type ProcessRunner struct {
	Timeout time.Duration
}

// NewProcessRunner builds a runner that kills ot after timeoutSeconds.
func NewProcessRunner(timeoutSeconds int) ProcessRunner {
	return ProcessRunner{Timeout: time.Duration(timeoutSeconds) * time.Second}
}

// Run executes the invocation, capturing both streams separately.
func (r ProcessRunner) Run(invocation Invocation) (Output, error) {
	ctx, cancel := context.WithTimeout(context.Background(), r.Timeout)
	defer cancel()

	cmd := exec.CommandContext(ctx, invocation.Program, invocation.Args...)
	var stdout, stderr bytes.Buffer
	cmd.Stdin = nil
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	// Do not wait on a grandchild holding the pipes open once ot itself is gone.
	cmd.WaitDelay = time.Second

	err := cmd.Run()
	output := Output{Stdout: stdout.String(), Stderr: stderr.String()}

	if ctx.Err() == context.DeadlineExceeded {
		return Output{}, &RunError{Kind: RunTimeout, Timeout: r.Timeout}
	}
	if err == nil {
		output.Code = cmd.ProcessState.ExitCode()
		return output, nil
	}

	var exitErr *exec.ExitError
	if errors.As(err, &exitErr) {
		// ExitCode reports -1 when a signal killed the process, which is the
		// one case where "it failed with status N" would be a lie.
		if code := exitErr.ExitCode(); code >= 0 {
			output.Code = code
			return output, nil
		}
		output.Signaled = true
		return output, nil
	}
	return Output{}, startError(err)
}

func startError(err error) *RunError {
	if errors.Is(err, exec.ErrNotFound) || errors.Is(err, fs.ErrNotExist) {
		return &RunError{Kind: RunNotFound}
	}
	if errors.Is(err, fs.ErrPermission) || errors.Is(err, os.ErrPermission) {
		return &RunError{Kind: RunNotExecutable, Detail: err.Error()}
	}
	// A file that exists and is executable but is not a program at all (a
	// directory, a broken interpreter line) lands here.
	var pathErr *fs.PathError
	if errors.As(err, &pathErr) {
		return &RunError{Kind: RunNotExecutable, Detail: err.Error()}
	}
	return &RunError{Kind: RunIO, Detail: err.Error()}
}
