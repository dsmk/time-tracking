package ot

import (
	"fmt"
	"strings"
	"testing"
)

// mockRunner is an in-process stand-in for ot. Responses are keyed by the
// invocation's leading non-flag arguments, for example `time list` or
// `auth status`.
type mockRunner struct {
	t         *testing.T
	responses map[string][]response
	calls     []Invocation
}

type response struct {
	output Output
	err    error
}

// newMock builds a mock that already answers the version probe every command
// path begins with.
func newMock(t *testing.T) *mockRunner {
	t.Helper()
	runner := bareMock(t)
	runner.push("version", ok(`{"output_version":1,"version":"0.13.0"}`))
	return runner
}

// bareMock has no canned version response, for version-probe tests.
func bareMock(t *testing.T) *mockRunner {
	t.Helper()
	return &mockRunner{t: t, responses: map[string][]response{}}
}

func (m *mockRunner) push(key string, r response) *mockRunner {
	m.responses[key] = append(m.responses[key], r)
	return m
}

// call is the argument vector of the nth invocation, for assertions on command
// construction.
func (m *mockRunner) call(index int) []string {
	m.t.Helper()
	if index >= len(m.calls) {
		m.t.Fatalf("no call %d; ot was invoked %d times", index, len(m.calls))
	}
	return m.calls[index].Args
}

func (m *mockRunner) Run(invocation Invocation) (Output, error) {
	m.calls = append(m.calls, invocation)
	key := keyFor(invocation.Args)
	queued := m.responses[key]
	if len(queued) == 0 {
		m.t.Fatalf("mock ot has no response for %q (%s)", key, invocation.Display())
	}
	m.responses[key] = queued[1:]
	return queued[0].output, queued[0].err
}

// keyFor is the first two non-flag arguments, which is what identifies an ot
// subcommand.
func keyFor(args []string) string {
	var parts []string
	for _, arg := range args {
		if strings.HasPrefix(arg, "-") {
			continue
		}
		if parts = append(parts, arg); len(parts) == 2 {
			break
		}
	}
	return strings.Join(parts, " ")
}

func ok(stdout string) response {
	return response{output: Output{Code: 0, Stdout: stdout}}
}

func fail(code int, stdout, stderr string) response {
	return response{output: Output{Code: code, Stdout: stdout, Stderr: stderr}}
}

func runFails(err error) response { return response{err: err} }

// client wires a mock runner to an ot client.
func client(t *testing.T) (*Client, *mockRunner) {
	t.Helper()
	runner := newMock(t)
	return New("ot", runner), runner
}

func equalArgs(got, want []string) error {
	if len(got) != len(want) {
		return fmt.Errorf("got %v, want %v", got, want)
	}
	for i := range got {
		if got[i] != want[i] {
			return fmt.Errorf("got %v, want %v", got, want)
		}
	}
	return nil
}
