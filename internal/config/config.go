// Package config holds tt's own settings.
//
// Deliberately small: anything about *which* OpenTodo instance to talk to, or
// how to authenticate to it, stays in ot's own configuration (see
// docs/opentodo-compatibility.md §8). Unknown keys are rejected so a stray
// `token = "..."` fails loudly instead of sitting in a plaintext file.
package config

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/BurntSushi/toml"

	"github.com/dsmk/time-tracking/internal/ot"
	"github.com/dsmk/time-tracking/internal/tterr"
)

// Format is the output format for reports and listings.
type Format string

// The formats tt can render.
const (
	FormatText Format = "text"
	FormatJSON Format = "json"
	FormatCSV  Format = "csv"
)

// ParseFormat reads a format name.
func ParseFormat(text string) (Format, error) {
	switch Format(strings.ToLower(strings.TrimSpace(text))) {
	case FormatText:
		return FormatText, nil
	case FormatJSON:
		return FormatJSON, nil
	case FormatCSV:
		return FormatCSV, nil
	default:
		return "", fmt.Errorf("unknown format %q, expected text, json, or csv", text)
	}
}

// Source records where the timezone came from, for report footers.
type Source int

// Where a setting was resolved from.
const (
	FromSystem Source = iota
	FromConfig
)

// raw is the file as written. Pointers mark "absent" so a key set to its zero
// value is still validated rather than silently replaced by the default.
type raw struct {
	OtCommand      *string `toml:"ot_command"`
	Timezone       *string `toml:"timezone"`
	WeekStart      *string `toml:"week_start"`
	ReportFormat   *string `toml:"report_format"`
	TimeoutSeconds *int    `toml:"timeout_seconds"`
	MaxEntries     *int    `toml:"max_entries"`
	VersionCheck   *bool   `toml:"version_check"`
}

// Config is the resolved settings for one run.
type Config struct {
	OtCommand string
	// Location is the zone used for day and week boundaries.
	Location *time.Location
	// TimezoneName is what reports print; the IANA name where one is known.
	TimezoneName   string
	TimezoneSource Source
	WeekStart      time.Weekday
	ReportFormat   Format
	// TimeoutSeconds is how long to wait for an ot invocation before killing
	// it. Above ot's own documented 15 s server timeout so its error wins the
	// race.
	TimeoutSeconds int
	// MaxEntries is the --limit passed to `ot time list`. Results that hit it
	// are reported as incomplete rather than quietly truncated.
	MaxEntries int
	// VersionCheck runs `ot version --json` before other calls.
	VersionCheck bool
}

// Default is the configuration tt uses when no file is present.
func Default() Config {
	location, name, source := systemZone()
	return Config{
		OtCommand:      ot.DefaultCommand,
		Location:       location,
		TimezoneName:   name,
		TimezoneSource: source,
		WeekStart:      time.Monday,
		ReportFormat:   FormatText,
		TimeoutSeconds: 30,
		MaxEntries:     5000,
		VersionCheck:   true,
	}
}

// Load resolves the config path, reads it if present, and validates it.
//
// TT_CONFIG overrides the path; a missing file is not an error, but a missing
// file at an explicitly requested path is.
func Load() (Config, error) {
	if path, ok := os.LookupEnv("TT_CONFIG"); ok && path != "" {
		if _, err := os.Stat(path); err != nil {
			return Config{}, tterr.Usage("config file %s does not exist", path).
				WithHint("TT_CONFIG points at this path; unset it to use the default")
		}
		return FromPath(path)
	}
	path, err := defaultPath()
	if err != nil {
		return Default(), nil
	}
	if _, err := os.Stat(path); err != nil {
		return Default(), nil
	}
	return FromPath(path)
}

// FromPath reads and validates one config file.
func FromPath(path string) (Config, error) {
	text, err := os.ReadFile(path)
	if err != nil {
		return Config{}, tterr.Usage("cannot read config file %s: %v", path, err)
	}

	var file raw
	meta, err := toml.Decode(string(text), &file)
	if err != nil {
		// The error text can quote the offending source line, which would echo
		// back a credential the user should not have put here.
		return Config{}, invalidConfig(path, ot.Redact(err.Error()))
	}
	if undecoded := meta.Undecoded(); len(undecoded) > 0 {
		keys := make([]string, 0, len(undecoded))
		for _, key := range undecoded {
			keys = append(keys, key.String())
		}
		sort.Strings(keys)
		// Only the key names, never the values: an unknown key is exactly where
		// a secret would have been written.
		return Config{}, invalidConfig(path, "unknown key "+strings.Join(keys, ", "))
	}
	return fromRaw(file)
}

func invalidConfig(path, detail string) *tterr.Error {
	return tterr.Usage("config file %s is not valid: %s", path, detail).
		WithHint("supported keys: ot_command, timezone, week_start, report_format, " +
			"timeout_seconds, max_entries, version_check. OpenTodo credentials and " +
			"server URL belong to `ot`, not to tt")
}

func fromRaw(file raw) (Config, error) {
	config := Default()

	if file.Timezone != nil {
		location, err := LoadLocation(*file.Timezone)
		if err != nil {
			return Config{}, err
		}
		config.Location = location
		config.TimezoneName = *file.Timezone
		config.TimezoneSource = FromConfig
	}
	if file.WeekStart != nil {
		day, err := ParseWeekday(*file.WeekStart)
		if err != nil {
			return Config{}, err
		}
		config.WeekStart = day
	}
	if file.ReportFormat != nil {
		format, err := ParseFormat(*file.ReportFormat)
		if err != nil {
			return Config{}, tterr.Usage("invalid report_format: %v", err)
		}
		config.ReportFormat = format
	}
	if file.OtCommand != nil {
		if strings.TrimSpace(*file.OtCommand) == "" {
			return Config{}, tterr.Usage("ot_command must not be empty").
				WithHint("set ot_command to `ot` or to the full path of the OpenTodo CLI")
		}
		config.OtCommand = *file.OtCommand
	}
	if file.TimeoutSeconds != nil {
		if *file.TimeoutSeconds <= 0 {
			return Config{}, tterr.Usage("timeout_seconds must be greater than zero")
		}
		config.TimeoutSeconds = *file.TimeoutSeconds
	}
	if file.MaxEntries != nil {
		if *file.MaxEntries <= 0 {
			return Config{}, tterr.Usage("max_entries must be greater than zero")
		}
		config.MaxEntries = *file.MaxEntries
	}
	if file.VersionCheck != nil {
		config.VersionCheck = *file.VersionCheck
	}
	return config, nil
}

// LoadLocation resolves an IANA timezone name, with an actionable error.
func LoadLocation(name string) (*time.Location, error) {
	location, err := time.LoadLocation(name)
	if err != nil {
		return nil, tterr.Usage("unknown timezone %q: %v", name, err).
			WithHint("use an IANA name such as America/New_York")
	}
	return location, nil
}

// systemZone is the documented fallback: OpenTodo's CLI exposes no timezone
// setting, so tt uses the host's (docs/opentodo-compatibility.md §9). The IANA
// name is recovered where the host records one, because reports print it.
func systemZone() (*time.Location, string, Source) {
	if name, ok := os.LookupEnv("TZ"); ok && name != "" {
		if location, err := time.LoadLocation(name); err == nil {
			return location, name, FromSystem
		}
	}
	if target, err := os.Readlink("/etc/localtime"); err == nil {
		if name, ok := zoneNameFromPath(target); ok {
			if location, err := time.LoadLocation(name); err == nil {
				return location, name, FromSystem
			}
		}
	}
	name := time.Local.String()
	if name == "" || name == "Local" {
		name = "system"
	}
	return time.Local, name, FromSystem
}

// zoneNameFromPath pulls "America/New_York" out of a zoneinfo path.
func zoneNameFromPath(path string) (string, bool) {
	path = filepath.ToSlash(path)
	const marker = "/zoneinfo/"
	index := strings.LastIndex(path, marker)
	if index < 0 {
		return "", false
	}
	name := strings.TrimPrefix(path[index+len(marker):], "posix/")
	if name == "" {
		return "", false
	}
	return name, true
}

func defaultPath() (string, error) {
	dir, err := os.UserConfigDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "tt", "config.toml"), nil
}

var weekdays = map[string]time.Weekday{
	"monday": time.Monday, "mon": time.Monday,
	"tuesday": time.Tuesday, "tue": time.Tuesday,
	"wednesday": time.Wednesday, "wed": time.Wednesday,
	"thursday": time.Thursday, "thu": time.Thursday,
	"friday": time.Friday, "fri": time.Friday,
	"saturday": time.Saturday, "sat": time.Saturday,
	"sunday": time.Sunday, "sun": time.Sunday,
}

// ParseWeekday reads a weekday name or three-letter abbreviation.
func ParseWeekday(name string) (time.Weekday, error) {
	if day, ok := weekdays[strings.ToLower(strings.TrimSpace(name))]; ok {
		return day, nil
	}
	return 0, tterr.Usage("unknown week_start %q", strings.TrimSpace(name)).
		WithHint("use a weekday name, for example monday or sunday")
}

// WeekdayName is the lowercase name tt prints for a weekday.
func WeekdayName(day time.Weekday) string {
	return strings.ToLower(day.String())
}
