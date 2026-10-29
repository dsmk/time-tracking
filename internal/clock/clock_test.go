package clock

import (
	"testing"
	"time"

	"github.com/dsmk/time-tracking/internal/tterr"
)

func TestFromEnvFreezesTheClockWhenTTNowIsSet(t *testing.T) {
	t.Setenv("TT_NOW", "2026-10-27T13:30:00Z")
	frozen, err := FromEnv()
	if err != nil {
		t.Fatal(err)
	}
	if got := frozen.Now().Format(time.RFC3339); got != "2026-10-27T13:30:00Z" {
		t.Errorf("got %s", got)
	}
	if !frozen.Now().Equal(frozen.Now()) {
		t.Error("a frozen clock must not move")
	}
}

func TestFromEnvFallsBackToTheHostClock(t *testing.T) {
	for _, value := range []string{"", "   "} {
		t.Setenv("TT_NOW", value)
		now, err := FromEnv()
		if err != nil {
			t.Fatalf("%q: %v", value, err)
		}
		if _, frozen := now.(Fixed); frozen {
			t.Errorf("%q should not freeze the clock", value)
		}
	}
}

func TestFromEnvRejectsAnUnreadableTimestamp(t *testing.T) {
	t.Setenv("TT_NOW", "yesterday")
	_, err := FromEnv()
	if tterr.KindOf(err) != tterr.KindUsage {
		t.Fatalf("got %v", err)
	}
}
