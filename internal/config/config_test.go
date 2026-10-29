package config

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/dsmk/time-tracking/internal/tterr"
)

func write(t *testing.T, body string) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "config.toml")
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestDefaultsAreUsable(t *testing.T) {
	cfg := Default()
	if cfg.OtCommand != "ot" {
		t.Errorf("ot_command: %q", cfg.OtCommand)
	}
	if cfg.WeekStart != time.Monday {
		t.Errorf("week_start: %v", cfg.WeekStart)
	}
	if cfg.ReportFormat != FormatText {
		t.Errorf("report_format: %v", cfg.ReportFormat)
	}
	if cfg.TimezoneSource != FromSystem {
		t.Errorf("timezone source: %v", cfg.TimezoneSource)
	}
	if cfg.Location == nil {
		t.Error("a zone is always resolved")
	}
}

func TestEverySettingIsReadFromTheFile(t *testing.T) {
	cfg, err := FromPath(write(t, `
ot_command = "/opt/bin/ot"
timezone = "America/New_York"
week_start = "sunday"
report_format = "csv"
timeout_seconds = 45
max_entries = 100
version_check = false
`))
	if err != nil {
		t.Fatalf("valid config: %v", err)
	}
	if cfg.OtCommand != "/opt/bin/ot" || cfg.TimezoneName != "America/New_York" ||
		cfg.WeekStart != time.Sunday || cfg.ReportFormat != FormatCSV ||
		cfg.TimeoutSeconds != 45 || cfg.MaxEntries != 100 || cfg.VersionCheck {
		t.Fatalf("got %+v", cfg)
	}
	if cfg.TimezoneSource != FromConfig {
		t.Errorf("timezone source: %v", cfg.TimezoneSource)
	}
}

func TestUnknownKeysAreRejectedSoSecretsCannotBeStashedHere(t *testing.T) {
	_, err := FromPath(write(t, "token = \"super-secret\"\n"))
	if err == nil {
		t.Fatal("unknown keys must not be accepted")
	}
	if !strings.Contains(err.Error(), "token") {
		t.Errorf("the offending key should be named: %v", err)
	}
	if strings.Contains(err.Error(), "super-secret") {
		t.Errorf("the value must not be echoed back: %v", err)
	}
}

func TestBadTimezoneIsActionable(t *testing.T) {
	_, err := FromPath(write(t, "timezone = \"Mars/Olympus\"\n"))
	var ttError *tterr.Error
	if !errors.As(err, &ttError) || ttError.Kind != tterr.KindUsage {
		t.Fatalf("got %v", err)
	}
	if !strings.Contains(ttError.Hint, "IANA") {
		t.Errorf("hint: %q", ttError.Hint)
	}
}

func TestZeroedLimitsAreRejectedRatherThanSilentlyReplaced(t *testing.T) {
	for _, body := range []string{"timeout_seconds = 0\n", "max_entries = 0\n", "ot_command = \"\"\n"} {
		if _, err := FromPath(write(t, body)); tterr.KindOf(err) != tterr.KindUsage {
			t.Errorf("%q: got %v", body, err)
		}
	}
}

func TestAMissingFileAtAnExplicitPathIsAnError(t *testing.T) {
	t.Setenv("TT_CONFIG", filepath.Join(t.TempDir(), "nope.toml"))
	_, err := Load()
	if tterr.KindOf(err) != tterr.KindUsage {
		t.Fatalf("got %v", err)
	}
	if !strings.Contains(err.Error(), "does not exist") {
		t.Errorf("got %v", err)
	}
}

func TestWeekStartAcceptsNamesAndAbbreviations(t *testing.T) {
	for _, name := range []string{"Sunday", " sun ", "SUN"} {
		day, err := ParseWeekday(name)
		if err != nil || day != time.Sunday {
			t.Errorf("%q: got %v, %v", name, day, err)
		}
	}
	if _, err := ParseWeekday("funday"); err == nil {
		t.Error("an invented weekday must be refused")
	}
}

func TestWeekdayNamesRoundTrip(t *testing.T) {
	for day := time.Sunday; day <= time.Saturday; day++ {
		parsed, err := ParseWeekday(WeekdayName(day))
		if err != nil || parsed != day {
			t.Errorf("%v: got %v, %v", day, parsed, err)
		}
	}
}

func TestFormatNamesAreCaseInsensitive(t *testing.T) {
	for _, name := range []string{"JSON", " json ", "json"} {
		format, err := ParseFormat(name)
		if err != nil || format != FormatJSON {
			t.Errorf("%q: got %v, %v", name, format, err)
		}
	}
	if _, err := ParseFormat("yaml"); err == nil {
		t.Error("an unsupported format must be refused")
	}
}
