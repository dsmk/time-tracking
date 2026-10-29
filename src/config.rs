use std::path::{Path, PathBuf};

use jiff::civil::Weekday;
use jiff::tz::TimeZone;
use serde::Deserialize;

use crate::error::{Result, usage};

/// Default output format for reports and listings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Text,
    Json,
    Csv,
}

impl std::str::FromStr for Format {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Format, String> {
        match s.to_ascii_lowercase().as_str() {
            "text" => Ok(Format::Text),
            "json" => Ok(Format::Json),
            "csv" => Ok(Format::Csv),
            other => Err(format!("unknown format {other:?}, expected text, json, or csv")),
        }
    }
}

/// `tt` settings. Deliberately small: anything about *which* OpenTodo instance
/// to talk to, or how to authenticate to it, stays in `ot`'s own configuration
/// (see `docs/opentodo-compatibility.md` §8). Unknown keys are rejected so a
/// stray `token = "..."` fails loudly instead of sitting in a plaintext file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RawConfig {
    pub ot_command: String,
    pub timezone: Option<String>,
    pub week_start: String,
    pub report_format: String,
    /// Seconds to wait for an `ot` invocation before killing it. Above `ot`'s
    /// own documented 15 s server timeout so its error wins the race.
    pub timeout_seconds: u64,
    /// `--limit` passed to `ot time list`. Results that hit it are reported as
    /// incomplete rather than quietly truncated.
    pub max_entries: u32,
    /// Run `ot version --json` before other calls.
    pub version_check: bool,
}

impl Default for RawConfig {
    fn default() -> RawConfig {
        RawConfig {
            ot_command: "ot".to_string(),
            timezone: None,
            week_start: "monday".to_string(),
            report_format: "text".to_string(),
            timeout_seconds: 30,
            max_entries: 5000,
            version_check: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub ot_command: String,
    pub timezone: TimeZone,
    /// Where the timezone came from, for report footers.
    pub timezone_source: TimezoneSource,
    pub week_start: Weekday,
    pub report_format: Format,
    pub timeout_seconds: u64,
    pub max_entries: u32,
    pub version_check: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimezoneSource {
    Config,
    System,
}

impl Config {
    /// Resolve the config path, read it if present, and validate it.
    ///
    /// `TT_CONFIG` overrides the path; a missing file is not an error, but a
    /// missing file at an explicitly requested path is.
    pub fn load() -> Result<Config> {
        match std::env::var_os("TT_CONFIG") {
            Some(path) if !path.is_empty() => {
                let path = PathBuf::from(path);
                if !path.exists() {
                    return Err(usage(format!(
                        "config file {} does not exist",
                        path.display()
                    ))
                    .with_hint("TT_CONFIG points at this path; unset it to use the default"));
                }
                Config::from_path(&path)
            }
            _ => match default_path() {
                Some(path) if path.exists() => Config::from_path(&path),
                _ => Config::from_raw(RawConfig::default()),
            },
        }
    }

    pub fn from_path(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            usage(format!("cannot read config file {}: {e}", path.display()))
        })?;
        let raw: RawConfig = toml::from_str(&text).map_err(|e| {
            // `e.to_string()` quotes the offending source line, which would
            // echo back a credential the user should not have put here.
            let detail = crate::ot::redact::redact(e.message());
            usage(format!("config file {} is not valid: {detail}", path.display())).with_hint(
                "supported keys: ot_command, timezone, week_start, report_format, \
                 timeout_seconds, max_entries, version_check. OpenTodo credentials and \
                 server URL belong to `ot`, not to tt",
            )
        })?;
        Config::from_raw(raw)
    }

    pub fn from_raw(raw: RawConfig) -> Result<Config> {
        let (timezone, timezone_source) = match &raw.timezone {
            Some(name) => {
                let tz = TimeZone::get(name).map_err(|e| {
                    usage(format!("unknown timezone {name:?}: {e}"))
                        .with_hint("use an IANA name such as America/New_York")
                })?;
                (tz, TimezoneSource::Config)
            }
            // OpenTodo's CLI exposes no timezone setting, so the system zone is
            // the documented fallback (docs/opentodo-compatibility.md §9).
            None => (TimeZone::system(), TimezoneSource::System),
        };

        let week_start = parse_weekday(&raw.week_start)?;
        let report_format: Format = raw.report_format.parse().map_err(|e: String| {
            usage(format!("invalid report_format: {e}"))
        })?;

        if raw.ot_command.trim().is_empty() {
            return Err(usage("ot_command must not be empty")
                .with_hint("set ot_command to `ot` or to the full path of the OpenTodo CLI"));
        }
        if raw.timeout_seconds == 0 {
            return Err(usage("timeout_seconds must be greater than zero"));
        }
        if raw.max_entries == 0 {
            return Err(usage("max_entries must be greater than zero"));
        }

        Ok(Config {
            ot_command: raw.ot_command,
            timezone,
            timezone_source,
            week_start,
            report_format,
            timeout_seconds: raw.timeout_seconds,
            max_entries: raw.max_entries,
            version_check: raw.version_check,
        })
    }
}

fn default_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "tt")
        .map(|dirs| dirs.config_dir().join("config.toml"))
}

pub fn parse_weekday(name: &str) -> Result<Weekday> {
    match name.trim().to_ascii_lowercase().as_str() {
        "monday" | "mon" => Ok(Weekday::Monday),
        "tuesday" | "tue" => Ok(Weekday::Tuesday),
        "wednesday" | "wed" => Ok(Weekday::Wednesday),
        "thursday" | "thu" => Ok(Weekday::Thursday),
        "friday" | "fri" => Ok(Weekday::Friday),
        "saturday" | "sat" => Ok(Weekday::Saturday),
        "sunday" | "sun" => Ok(Weekday::Sunday),
        other => Err(usage(format!("unknown week_start {other:?}"))
            .with_hint("use a weekday name, for example monday or sunday")),
    }
}

pub fn weekday_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Monday => "monday",
        Weekday::Tuesday => "tuesday",
        Weekday::Wednesday => "wednesday",
        Weekday::Thursday => "thursday",
        Weekday::Friday => "friday",
        Weekday::Saturday => "saturday",
        Weekday::Sunday => "sunday",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_usable() {
        let config = Config::from_raw(RawConfig::default()).expect("defaults are valid");
        assert_eq!(config.ot_command, "ot");
        assert_eq!(config.week_start, Weekday::Monday);
        assert_eq!(config.report_format, Format::Text);
        assert_eq!(config.timezone_source, TimezoneSource::System);
    }

    #[test]
    fn unknown_keys_are_rejected_so_secrets_cannot_be_stashed_here() {
        let err = toml::from_str::<RawConfig>("token = \"super-secret\"\n")
            .expect_err("unknown keys must not deserialize");
        assert!(err.to_string().contains("token"), "{err}");
    }

    #[test]
    fn bad_timezone_is_actionable() {
        let raw = RawConfig { timezone: Some("Mars/Olympus".into()), ..Default::default() };
        let err = Config::from_raw(raw).expect_err("unknown zone");
        assert_eq!(err.kind, crate::error::Kind::Usage);
        assert!(err.hint().unwrap().contains("IANA"));
    }

    #[test]
    fn week_start_accepts_names_and_abbreviations() {
        assert_eq!(parse_weekday("Sunday").unwrap(), Weekday::Sunday);
        assert_eq!(parse_weekday(" sun ").unwrap(), Weekday::Sunday);
        assert!(parse_weekday("funday").is_err());
    }
}
