use std::path::{Path, PathBuf};

use jiff::civil::Weekday;
use jiff::tz::TimeZone;
use serde::Deserialize;

use crate::error::{AppError, ErrorKind, Result, config as config_err};
use crate::timeutil::{Precision, parse_weekday, tz_name, weekday_name};

/// Keys recognised in `config.toml`. Anything else is ignored with a warning.
const KNOWN_KEYS: [&str; 4] =
    ["timezone", "week_start", "duration_precision", "default_project"];

#[derive(Debug, Deserialize, Default)]
struct RawConfig {
    timezone: Option<String>,
    week_start: Option<String>,
    duration_precision: Option<String>,
    default_project: Option<String>,
}

/// Effective configuration: every setting resolved, with defaults applied.
#[derive(Debug, Clone)]
pub struct Config {
    pub timezone: TimeZone,
    pub week_start: Weekday,
    pub precision: Precision,
    pub default_project: Option<String>,
    pub path: PathBuf,
    /// Non-fatal problems to report on stderr (unknown keys, and so on).
    pub warnings: Vec<String>,
}

impl Config {
    pub fn timezone_name(&self) -> String {
        tz_name(&self.timezone)
    }

    /// Load configuration, falling back to defaults when the file is absent.
    pub fn load() -> Result<Config> {
        let path = config_path()?;
        Config::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Result<Config> {
        let mut warnings = Vec::new();
        let raw = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => {
                return Err(config_err(format!(
                    "cannot read {}: {err}",
                    path.display()
                )));
            }
        };

        let table: toml::Table = raw.parse().map_err(|e| {
            config_err(format!("{} is not valid TOML: {e}", path.display()))
        })?;
        for key in table.keys() {
            if !KNOWN_KEYS.contains(&key.as_str()) {
                warnings.push(format!(
                    "unknown setting {key:?} in {} was ignored",
                    path.display()
                ));
            }
        }

        let parsed: RawConfig = table.try_into().map_err(|e| {
            config_err(format!("{} has an invalid value: {e}", path.display()))
        })?;

        let timezone = match parsed.timezone.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => TimeZone::get(name).map_err(|_| {
                setting_error(path, "timezone", name, "not a known IANA timezone name")
                    .with_hint("example: timezone = \"America/New_York\"")
            })?,
            _ => {
                let (tz, warning) = system_timezone();
                warnings.extend(warning);
                tz
            }
        };

        let week_start = match parsed.week_start.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => parse_weekday(name).map_err(|_| {
                setting_error(path, "week_start", name, "not a weekday name").with_hint(
                    "use one of: monday, tuesday, wednesday, thursday, friday, saturday, sunday",
                )
            })?,
            _ => Weekday::Monday,
        };

        let precision = match parsed.duration_precision.as_deref().map(str::trim) {
            Some("minute") | None | Some("") => Precision::Minute,
            Some("second") => Precision::Second,
            Some(other) => {
                return Err(setting_error(
                    path,
                    "duration_precision",
                    other,
                    "not a supported precision",
                )
                .with_hint("use \"minute\" or \"second\""));
            }
        };

        let default_project = parsed
            .default_project
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty());

        Ok(Config { timezone, week_start, precision, default_project, path: path.to_path_buf(), warnings })
    }

    /// Validate and persist a single setting, preserving other keys and any
    /// unknown keys already present in the file.
    pub fn set(path: &Path, key: &str, value: &str) -> Result<()> {
        if !KNOWN_KEYS.contains(&key) {
            return Err(config_err(format!("unknown setting {key:?}")).with_hint(format!(
                "known settings: {}",
                KNOWN_KEYS.join(", ")
            )));
        }
        let trimmed = value.trim();

        // Reject bad values before touching the file.
        match key {
            "timezone" => {
                TimeZone::get(trimmed).map_err(|_| {
                    config_err(format!("{trimmed:?} is not a known IANA timezone name"))
                        .with_hint("example: tt config set timezone America/New_York")
                })?;
            }
            "week_start" => {
                parse_weekday(trimmed)?;
            }
            "duration_precision" => {
                if trimmed != "minute" && trimmed != "second" {
                    return Err(config_err(format!(
                        "{trimmed:?} is not a supported duration precision"
                    ))
                    .with_hint("use \"minute\" or \"second\""));
                }
            }
            _ => {}
        }

        let existing = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => {
                return Err(config_err(format!("cannot read {}: {err}", path.display())));
            }
        };
        let mut table: toml::Table = existing.parse().map_err(|e| {
            config_err(format!("{} is not valid TOML: {e}", path.display()))
        })?;

        let stored = match key {
            "week_start" => weekday_name(parse_weekday(trimmed)?).to_string(),
            _ => trimmed.to_string(),
        };
        table.insert(key.to_string(), toml::Value::String(stored));

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                config_err(format!("cannot create {}: {e}", parent.display()))
            })?;
        }
        let rendered = toml::to_string_pretty(&table)
            .map_err(|e| config_err(format!("cannot serialize configuration: {e}")))?;
        std::fs::write(path, rendered)
            .map_err(|e| config_err(format!("cannot write {}: {e}", path.display())))?;
        Ok(())
    }
}

/// The system timezone, named wherever possible.
///
/// `TimeZone::system()` keeps the right rules but cannot always report an IANA
/// name — in containers, `/etc/localtime` is often a symlink out of the usual
/// zoneinfo tree. Since each entry records the zone it was created in, fall
/// back to `$TZ` and `/etc/timezone` to recover a real name, and warn if even
/// those fail so the user knows to set `timezone` explicitly.
fn system_timezone() -> (TimeZone, Option<String>) {
    let system = TimeZone::system();
    if system.iana_name().is_some() {
        return (system, None);
    }

    let from_env = std::env::var("TZ").ok();
    let from_file = std::fs::read_to_string("/etc/timezone").ok();
    for candidate in [from_env, from_file].into_iter().flatten() {
        // `$TZ` may carry a leading colon, and may hold a POSIX rule string
        // rather than a zone name; `TimeZone::get` rejects anything unusable.
        let name = candidate.trim().trim_start_matches(':').trim();
        if name.is_empty() {
            continue;
        }
        if let Ok(named) = TimeZone::get(name) {
            return (named, None);
        }
    }

    (
        system,
        Some(
            "could not determine the name of the system timezone; times are still \
             correct, but entries will not record an IANA zone name. Set it with \
             `tt config set timezone <IANA name>`"
                .to_string(),
        ),
    )
}

fn setting_error(path: &Path, key: &str, value: &str, problem: &str) -> AppError {
    AppError::new(
        ErrorKind::Config,
        format!("setting {key} = {value:?} in {} is {problem}", path.display()),
    )
}

/// Configuration file location. `TT_CONFIG` overrides the OS-standard path.
pub fn config_path() -> Result<PathBuf> {
    if let Ok(raw) = std::env::var("TT_CONFIG")
        && !raw.trim().is_empty()
    {
        return Ok(PathBuf::from(raw));
    }
    let dirs = directories::ProjectDirs::from("", "", "tt").ok_or_else(|| {
        config_err("cannot determine a configuration directory for this platform")
            .with_hint("set TT_CONFIG to an explicit config.toml path")
    })?;
    Ok(dirs.config_dir().join("config.toml"))
}

/// Database location, in precedence order: `--db`, then `TT_DB`, then the
/// OS-standard data directory.
pub fn database_path(flag: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = flag {
        return Ok(path.to_path_buf());
    }
    if let Ok(raw) = std::env::var("TT_DB")
        && !raw.trim().is_empty()
    {
        return Ok(PathBuf::from(raw));
    }
    let dirs = directories::ProjectDirs::from("", "", "tt").ok_or_else(|| {
        config_err("cannot determine a data directory for this platform")
            .with_hint("pass --db PATH or set TT_DB")
    })?;
    Ok(dirs.data_dir().join("tt.db"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("config.toml");
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::load_from(&dir.path().join("absent.toml")).unwrap();
        assert_eq!(cfg.week_start, Weekday::Monday);
        assert_eq!(cfg.precision, Precision::Minute);
        assert!(cfg.default_project.is_none());
        assert!(cfg.warnings.is_empty());
    }

    #[test]
    fn reads_every_setting() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "timezone = \"America/New_York\"\nweek_start = \"sunday\"\n\
             duration_precision = \"second\"\ndefault_project = \"alpha\"\n",
        );
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.timezone_name(), "America/New_York");
        assert_eq!(cfg.week_start, Weekday::Sunday);
        assert_eq!(cfg.precision, Precision::Second);
        assert_eq!(cfg.default_project.as_deref(), Some("alpha"));
    }

    #[test]
    fn invalid_values_name_the_setting_and_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "timezone = \"Mars/Olympus\"\n");
        let err = Config::load_from(&path).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Config);
        assert!(err.message.contains("timezone"), "{}", err.message);
        assert!(err.message.contains("config.toml"), "{}", err.message);

        let path = write(dir.path(), "week_start = \"funday\"\n");
        assert!(Config::load_from(&path).unwrap_err().message.contains("week_start"));

        let path = write(dir.path(), "duration_precision = \"nanosecond\"\n");
        assert!(
            Config::load_from(&path).unwrap_err().message.contains("duration_precision")
        );
    }

    #[test]
    fn unknown_keys_warn_but_do_not_fail() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "week_start = \"monday\"\nnonsense = 1\n");
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.warnings.len(), 1);
        assert!(cfg.warnings[0].contains("nonsense"));
    }

    #[test]
    fn set_validates_before_writing_and_preserves_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "default_project = \"alpha\"\nnonsense = \"keep\"\n");

        assert!(Config::set(&path, "timezone", "Mars/Olympus").is_err());
        assert!(Config::set(&path, "bogus", "x").is_err());
        // A rejected value must not have modified the file.
        assert!(std::fs::read_to_string(&path).unwrap().contains("alpha"));

        Config::set(&path, "timezone", "America/New_York").unwrap();
        Config::set(&path, "week_start", "SUNDAY").unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.timezone_name(), "America/New_York");
        assert_eq!(cfg.week_start, Weekday::Sunday);
        assert_eq!(cfg.default_project.as_deref(), Some("alpha"));
        assert!(std::fs::read_to_string(&path).unwrap().contains("keep"));
    }

    #[test]
    fn set_creates_the_file_and_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        Config::set(&path, "week_start", "friday").unwrap();
        assert_eq!(Config::load_from(&path).unwrap().week_start, Weekday::Friday);
    }

    #[test]
    fn the_system_timezone_is_named_whenever_possible() {
        let (tz, warning) = system_timezone();
        match tz.iana_name() {
            // Either we resolved a real name, or we said so plainly.
            Some(name) => {
                assert!(!name.is_empty());
                assert!(warning.is_none(), "a named zone needs no warning");
                assert!(TimeZone::get(name).is_ok(), "the name must round-trip");
            }
            None => {
                let warning = warning.expect("an unnamed zone must warn");
                assert!(warning.contains("tt config set timezone"), "{warning}");
            }
        }
    }

    #[test]
    fn database_path_precedence_prefers_the_flag() {
        let flag = PathBuf::from("/tmp/explicit.db");
        assert_eq!(database_path(Some(&flag)).unwrap(), flag);
    }
}
