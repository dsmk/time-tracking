//! Parsing of the documented OpenTodo JSON shapes.
//!
//! Field names and types come from `docs/opentodo-compatibility.md` §3, which
//! records which parts are documented and which are defensive. Where the docs
//! are silent (bare array vs `{entries,…}` envelope, snake_case vs camelCase in
//! the CLI) the parser accepts both rather than guessing one and failing hard.

use jiff::Timestamp;
use serde::Deserialize;

use crate::error::{Error, Kind, Result};

/// One OpenTodo time entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeEntry {
    pub id: String,
    pub task_id: Option<String>,
    pub task_title: Option<String>,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub note: Option<String>,
    pub started_at: Timestamp,
    /// `None` while the entry is running (documented: `endedAt` is null).
    pub ended_at: Option<Timestamp>,
    /// OpenTodo's own whole-minute duration. Reported, but never used for
    /// report arithmetic: reports clip at period boundaries and need the
    /// timestamps.
    pub minutes: Option<i64>,
}

impl TimeEntry {
    pub fn is_running(&self) -> bool {
        self.ended_at.is_none()
    }

    /// Description for human output: the note, else the task title, else the id.
    pub fn label(&self) -> String {
        match (&self.task_title, &self.note) {
            (Some(title), Some(note)) if !note.is_empty() => format!("{title} — {note}"),
            (Some(title), _) => title.clone(),
            (None, Some(note)) if !note.is_empty() => note.clone(),
            _ => format!("entry {}", self.id),
        }
    }

    /// Stable key used to group report rows by task.
    pub fn task_key(&self) -> Option<&str> {
        self.task_id.as_deref()
    }
}

/// A page of entries plus whatever envelope metadata was present.
#[derive(Debug, Clone, Default)]
pub struct EntryPage {
    pub entries: Vec<TimeEntry>,
    pub server_time: Option<Timestamp>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEntry {
    id: Option<String>,
    #[serde(alias = "task_id")]
    task_id: Option<String>,
    #[serde(alias = "task_title")]
    task_title: Option<String>,
    #[serde(alias = "project_id")]
    project_id: Option<String>,
    #[serde(alias = "project_name")]
    project_name: Option<String>,
    note: Option<String>,
    #[serde(alias = "started_at")]
    started_at: Option<OtTime>,
    #[serde(alias = "ended_at")]
    ended_at: Option<OtTime>,
    minutes: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEnvelope {
    entries: Option<Vec<RawEntry>>,
    #[serde(alias = "next_cursor")]
    next_cursor: Option<String>,
    #[serde(alias = "server_time")]
    server_time: Option<OtTime>,
}

/// A documented Unix-millisecond timestamp. RFC 3339 strings and whole seconds
/// are also accepted: cheap insurance against an undocumented shape, and the
/// alternative is a hard parse failure on data we could have understood.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OtTime(Timestamp);

impl<'de> Deserialize<'de> for OtTime {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> std::result::Result<OtTime, D::Error> {
        use serde::de::Error as _;
        let value = serde_json::Value::deserialize(de)?;
        match value {
            serde_json::Value::Number(n) => {
                let raw = n.as_i64().ok_or_else(|| {
                    D::Error::custom(format!("timestamp {n} is not an integer"))
                })?;
                // Values small enough to be seconds since the epoch (before
                // 1970-01-01 + ~115 days in ms) are treated as seconds.
                let ts = if raw.abs() < 10_000_000_000 {
                    Timestamp::from_second(raw)
                } else {
                    Timestamp::from_millisecond(raw)
                };
                ts.map(OtTime).map_err(|e| D::Error::custom(format!("timestamp {raw}: {e}")))
            }
            serde_json::Value::String(s) => s
                .parse::<Timestamp>()
                .map(OtTime)
                .map_err(|e| D::Error::custom(format!("timestamp {s:?}: {e}"))),
            other => Err(D::Error::custom(format!("unexpected timestamp value: {other}"))),
        }
    }
}

fn bad_output(detail: impl std::fmt::Display) -> Error {
    Error::new(Kind::Server, format!("could not read the JSON OpenTodo returned: {detail}"))
        .with_hint(
            "tt expects the documented `--json` output of ot 0.9+ (output_version 1); \
             run `ot schema` and compare, or report this as a tt bug",
        )
}

impl RawEntry {
    fn into_entry(self) -> Result<TimeEntry> {
        let id = self.id.ok_or_else(|| bad_output("a time entry has no `id`"))?;
        let started_at = self
            .started_at
            .ok_or_else(|| bad_output(format!("time entry {id} has no `startedAt`")))?
            .0;
        let ended_at = self.ended_at.map(|t| t.0);
        if let Some(end) = ended_at
            && end < started_at
        {
            return Err(bad_output(format!("time entry {id} ends before it starts")));
        }
        Ok(TimeEntry {
            id,
            task_id: self.task_id,
            task_title: self.task_title,
            project_id: self.project_id,
            project_name: self.project_name,
            note: self.note.filter(|n| !n.is_empty()),
            started_at,
            ended_at,
            minutes: self.minutes,
        })
    }
}

/// Parse `ot time list --json` output.
///
/// Accepts a bare array (what the CLI docs imply), the HTTP envelope
/// `{entries, nextCursor?, serverTime}`, or a single entity.
pub fn parse_entry_page(stdout: &str) -> Result<EntryPage> {
    let text = stdout.trim();
    if text.is_empty() {
        return Ok(EntryPage::default());
    }
    let value: serde_json::Value = serde_json::from_str(text).map_err(bad_output)?;
    match value {
        serde_json::Value::Array(items) => {
            let raw: Vec<RawEntry> =
                serde_json::from_value(serde_json::Value::Array(items)).map_err(bad_output)?;
            Ok(EntryPage {
                entries: raw.into_iter().map(RawEntry::into_entry).collect::<Result<_>>()?,
                ..EntryPage::default()
            })
        }
        value @ serde_json::Value::Object(_) => {
            let envelope: RawEnvelope =
                serde_json::from_value(value.clone()).map_err(bad_output)?;
            match envelope.entries {
                Some(raw) => Ok(EntryPage {
                    entries: raw.into_iter().map(RawEntry::into_entry).collect::<Result<_>>()?,
                    server_time: envelope.server_time.map(|t| t.0),
                    next_cursor: envelope.next_cursor,
                }),
                // A lone entity, which is what `time start`/`time stop` are
                // most likely to print.
                None => {
                    let raw: RawEntry = serde_json::from_value(value).map_err(bad_output)?;
                    Ok(EntryPage { entries: vec![raw.into_entry()?], ..EntryPage::default() })
                }
            }
        }
        serde_json::Value::Null => Ok(EntryPage::default()),
        other => Err(bad_output(format!("expected an array or object, got {other}"))),
    }
}

/// A single entry, for commands documented to return one entity.
/// Returns `Ok(None)` when the output carries no entry at all, which the
/// caller handles by re-querying rather than inventing data.
pub fn parse_optional_entry(stdout: &str) -> Option<TimeEntry> {
    parse_entry_page(stdout).ok().and_then(|page| page.entries.into_iter().next())
}

/// Documented error envelope: `{"error":{"code":…,"message":…,"exit":N}}`.
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
pub struct OtError {
    pub code: Option<String>,
    pub message: Option<String>,
    pub exit: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: OtError,
}

pub fn parse_error(stdout: &str, stderr: &str) -> Option<OtError> {
    for text in [stdout, stderr] {
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if let Ok(envelope) = serde_json::from_str::<ErrorEnvelope>(text) {
            return Some(envelope.error);
        }
    }
    None
}

/// A task, as used by the interactive picker. Only `id` and `title` are relied
/// on; the project name is shown when present.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, alias = "project_name")]
    pub project_name: Option<String>,
}

pub fn parse_tasks(stdout: &str) -> Result<Vec<Task>> {
    let text = stdout.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let value: serde_json::Value = serde_json::from_str(text).map_err(bad_output)?;
    let items = match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(mut map) => map
            .remove("tasks")
            .and_then(|v| match v {
                serde_json::Value::Array(items) => Some(items),
                _ => None,
            })
            .ok_or_else(|| bad_output("task list output has no array of tasks"))?,
        other => return Err(bad_output(format!("expected a task array, got {other}"))),
    };
    serde_json::from_value(serde_json::Value::Array(items)).map_err(bad_output)
}

/// What `ot version --json` told us. Both fields are optional: only
/// `output_version` is documented, so the client version is best-effort.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionInfo {
    pub output_version: Option<i64>,
    pub client_version: Option<String>,
}

const VERSION_KEYS: [&str; 5] = ["version", "ot_version", "otVersion", "client_version", "cli_version"];

pub fn parse_version(stdout: &str) -> VersionInfo {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout.trim()) else {
        return VersionInfo::default();
    };
    let Some(map) = value.as_object() else {
        return VersionInfo::default();
    };

    let output_version = map
        .get("output_version")
        .or_else(|| map.get("outputVersion"))
        .and_then(|v| v.as_i64());

    let mut client_version = find_version(map);
    if client_version.is_none() {
        // One level of nesting, e.g. {"ot":{"version":"0.13.0"}}.
        for nested in map.values().filter_map(|v| v.as_object()) {
            client_version = find_version(nested);
            if client_version.is_some() {
                break;
            }
        }
    }

    VersionInfo { output_version, client_version }
}

fn find_version(map: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    VERSION_KEYS
        .iter()
        .find_map(|key| map.get(*key).and_then(|v| v.as_str()))
        .map(|s| s.to_string())
}

/// Parse a dotted version into comparable parts. Returns `None` when the string
/// is not recognisable, in which case the caller must skip the check rather
/// than guess.
pub fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let text = raw.trim().trim_start_matches(['v', 'V']);
    let core = text.split(['-', '+', ' ']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str = r#"{
        "id": "e1", "taskId": "8c2a", "userId": "u1",
        "startedAt": 1793109600000, "endedAt": 1793113200000,
        "minutes": 60, "note": "review", "deviceId": "d1",
        "taskTitle": "Design review", "projectId": "p1", "projectName": "Work"
    }"#;

    #[test]
    fn parses_the_documented_camel_case_entry() {
        let page = parse_entry_page(&format!("[{ENTRY}]")).expect("parses");
        let entry = &page.entries[0];
        assert_eq!(entry.id, "e1");
        assert_eq!(entry.task_id.as_deref(), Some("8c2a"));
        assert_eq!(entry.task_title.as_deref(), Some("Design review"));
        assert_eq!(entry.project_name.as_deref(), Some("Work"));
        assert_eq!(entry.note.as_deref(), Some("review"));
        assert_eq!(entry.minutes, Some(60));
        assert_eq!(entry.started_at.to_string(), "2026-10-27T14:00:00Z");
        assert_eq!(entry.ended_at.unwrap().to_string(), "2026-10-27T15:00:00Z");
        assert!(!entry.is_running());
    }

    #[test]
    fn accepts_the_http_envelope_too() {
        let json = format!(
            r#"{{"entries":[{ENTRY}],"nextCursor":"abc","serverTime":1793113200000}}"#
        );
        let page = parse_entry_page(&json).expect("parses");
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.next_cursor.as_deref(), Some("abc"));
        assert_eq!(page.server_time.unwrap().to_string(), "2026-10-27T15:00:00Z");
    }

    #[test]
    fn a_null_ended_at_means_running() {
        let json = r#"[{"id":"e1","taskId":"8c2a","startedAt":1793109600000,"endedAt":null}]"#;
        let page = parse_entry_page(json).expect("parses");
        assert!(page.entries[0].is_running());
    }

    #[test]
    fn empty_output_is_an_empty_page_not_an_error() {
        assert!(parse_entry_page("  ").expect("parses").entries.is_empty());
        assert!(parse_entry_page("[]").expect("parses").entries.is_empty());
    }

    #[test]
    fn snake_case_and_rfc3339_are_tolerated() {
        let json = r#"[{"id":"e1","task_id":"8c2a","started_at":"2026-10-27T14:00:00Z"}]"#;
        let entry = &parse_entry_page(json).expect("parses").entries[0];
        assert_eq!(entry.task_id.as_deref(), Some("8c2a"));
        assert_eq!(entry.started_at.to_string(), "2026-10-27T14:00:00Z");
    }

    #[test]
    fn garbage_output_is_not_silently_treated_as_no_entries() {
        let err = parse_entry_page("Mon 09:00-09:25  25m  Design review").expect_err("not JSON");
        assert_eq!(err.kind, Kind::Server);
        assert!(err.hint().unwrap().contains("ot schema"));
    }

    #[test]
    fn an_entry_without_a_start_is_rejected() {
        let err = parse_entry_page(r#"[{"id":"e1"}]"#).expect_err("needs startedAt");
        assert!(err.message().contains("startedAt"), "{err}");
    }

    #[test]
    fn parses_the_documented_error_envelope() {
        let json = r#"{"error":{"code":"insufficient_scope","message":"read token","exit":5}}"#;
        let err = parse_error(json, "").expect("envelope");
        assert_eq!(err.code.as_deref(), Some("insufficient_scope"));
        assert_eq!(err.exit, Some(5));
    }

    #[test]
    fn version_output_is_read_leniently() {
        let v = parse_version(r#"{"output_version":1,"version":"0.13.0"}"#);
        assert_eq!(v.output_version, Some(1));
        assert_eq!(v.client_version.as_deref(), Some("0.13.0"));

        let nested = parse_version(r#"{"output_version":1,"ot":{"version":"v0.9.1-rc2"}}"#);
        assert_eq!(nested.client_version.as_deref(), Some("v0.9.1-rc2"));

        // Undocumented shape: no panic, no guess.
        let unknown = parse_version(r#"{"output_version":1}"#);
        assert_eq!(unknown.client_version, None);
    }

    #[test]
    fn semver_parsing_handles_prefixes_and_prereleases() {
        assert_eq!(parse_semver("0.13.0"), Some((0, 13, 0)));
        assert_eq!(parse_semver("v0.9.1-rc2"), Some((0, 9, 1)));
        assert_eq!(parse_semver("0.9"), Some((0, 9, 0)));
        assert_eq!(parse_semver("nightly"), None);
    }

    #[test]
    fn tasks_parse_from_an_array_or_a_wrapper() {
        let tasks = parse_tasks(r#"[{"id":"8c2a","title":"Design review","projectName":"Work"}]"#)
            .expect("parses");
        assert_eq!(tasks[0].title.as_deref(), Some("Design review"));
        let wrapped = parse_tasks(r#"{"tasks":[{"id":"1","title":"x"}]}"#).expect("parses");
        assert_eq!(wrapped.len(), 1);
    }
}
