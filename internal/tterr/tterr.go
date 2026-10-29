// Package tterr carries the one error type tt shows a user.
package tterr

import (
	"errors"
	"fmt"
)

// Kind says why a command failed. The values are tt's exit codes: 1-5 mirror
// the documented ot exit codes so a script wrapping tt can treat them the same
// way, and 6-8 cover conditions that are tt's own.
type Kind int

const (
	// KindServer is ot exit 1: the server rejected the request or failed.
	KindServer Kind = 1
	// KindUsage is ot exit 2, bad input, or an unusable config file.
	KindUsage Kind = 2
	// KindAuth is ot exit 3: authentication or configuration problem.
	KindAuth Kind = 3
	// KindUnreachable is ot exit 4: the OpenTodo server could not be reached.
	KindUnreachable Kind = 4
	// KindForbidden is ot exit 5: the token or instance does not permit this.
	KindForbidden Kind = 5
	// KindUnsupported means the operation has no documented OpenTodo equivalent.
	KindUnsupported Kind = 6
	// KindClient means the ot client is missing, not executable, or did not finish.
	KindClient Kind = 7
	// KindNoTimer means no timer is running.
	KindNoTimer Kind = 8
)

// ExitCode is the process exit status for this kind.
func (k Kind) ExitCode() int { return int(k) }

// Error is a failure with an optional actionable hint. Every error surfaced to
// the user is one of these, so SPEC.md §10's "actionable errors" requirement is
// a property of the type rather than of each call site remembering.
type Error struct {
	Kind    Kind
	Message string
	Hint    string
}

func (e *Error) Error() string {
	if e.Hint == "" {
		return e.Message
	}
	return e.Message + "\n  hint: " + e.Hint
}

// ExitCode is the process exit status for this error.
func (e *Error) ExitCode() int { return e.Kind.ExitCode() }

// WithHint returns the error with an actionable hint attached.
func (e *Error) WithHint(format string, args ...any) *Error {
	e.Hint = sprint(format, args)
	return e
}

// New builds an error of the given kind.
func New(kind Kind, format string, args ...any) *Error {
	return &Error{Kind: kind, Message: sprint(format, args)}
}

// Usage builds a KindUsage error: bad input, or a config tt cannot use.
func Usage(format string, args ...any) *Error {
	return New(KindUsage, format, args...)
}

// Unsupported builds a KindUnsupported error: the operation has no documented
// OpenTodo equivalent.
func Unsupported(format string, args ...any) *Error {
	return New(KindUnsupported, format, args...)
}

// ExitCodeOf reports the exit status for any error. Anything that is not a
// *Error is treated as a server-side failure, which is the conservative
// reading: it is never mistaken for success.
func ExitCodeOf(err error) int {
	if err == nil {
		return 0
	}
	var e *Error
	if errors.As(err, &e) {
		return e.ExitCode()
	}
	return int(KindServer)
}

// KindOf reports the kind of any error, defaulting to KindServer.
func KindOf(err error) Kind {
	var e *Error
	if errors.As(err, &e) {
		return e.Kind
	}
	return KindServer
}

// sprint formats only when there are arguments, so a bare message containing a
// percent sign survives unchanged.
func sprint(format string, args []any) string {
	if len(args) == 0 {
		return format
	}
	return fmt.Sprintf(format, args...)
}
