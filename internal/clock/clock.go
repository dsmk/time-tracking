// Package clock is the source of "now".
//
// Commands take a Clock rather than calling time.Now so tests can pin the
// current instant.
package clock

import (
	"os"
	"strings"
	"time"

	"github.com/dsmk/time-tracking/internal/tterr"
)

// Clock reports the current instant.
type Clock interface {
	Now() time.Time
}

// System reads the host clock.
type System struct{}

// Now is the current instant.
func (System) Now() time.Time { return time.Now().UTC() }

// Fixed is a frozen instant, for tests.
type Fixed struct{ Instant time.Time }

// Now is the frozen instant.
func (f Fixed) Now() time.Time { return f.Instant }

// FromEnv builds the clock for a run. TT_NOW, when set to an RFC 3339
// timestamp, freezes the current instant; the integration tests use it to make
// default report periods and elapsed times deterministic.
func FromEnv() (Clock, error) {
	raw := strings.TrimSpace(os.Getenv("TT_NOW"))
	if raw == "" {
		return System{}, nil
	}
	instant, err := time.Parse(time.RFC3339, raw)
	if err != nil {
		return nil, tterr.Usage("TT_NOW is not a valid RFC 3339 timestamp: %v", err).
			WithHint("example: TT_NOW=2026-10-27T13:30:00Z")
	}
	return Fixed{Instant: instant.UTC()}, nil
}
