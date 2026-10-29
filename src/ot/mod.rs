//! The OpenTodo adapter.
//!
//! Everything that knows about `ot` — its subcommands, flags, JSON shapes, exit
//! codes and error codes — lives behind this module, as required by
//! `SPEC.md` §8. Only the operations OpenTodo documents are implemented; the
//! rest return an `Unsupported` error naming what is missing.

pub mod model;
pub mod redact;
pub mod runner;

use jiff::civil::Date;

use crate::error::{Error, Kind, Result, unsupported};
use model::{EntryPage, Task, TimeEntry, VersionInfo};
use runner::{Invocation, Output, RunError, Runner};

/// Lowest `ot` release with time tracking (`SPEC.md` §3 targets 0.9+).
pub const MIN_OT_VERSION: (u64, u64, u64) = (0, 9, 0);
/// The `output_version` this adapter's parsers were written against.
pub const SUPPORTED_OUTPUT_VERSION: i64 = 1;

/// Which entries to ask OpenTodo for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntryFilter {
    pub from: Option<Date>,
    pub to: Option<Date>,
    pub task: Option<String>,
    pub running: bool,
    pub limit: Option<u32>,
}

/// What `ot auth status` could tell us. Deliberately *not* a sync state: a
/// command succeeding says nothing about whether devices are in sync
/// (`SPEC.md` §6.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientState {
    Configured { server: Option<String>, account: Option<String>, scope: Option<String> },
    Unreachable(String),
    AuthFailure(String),
    Unknown(String),
}

/// The conceptual operations from `SPEC.md` §8.
pub trait OpenTodo {
    fn version(&self) -> Result<VersionInfo>;
    fn start_timer(&self, task: &str, note: Option<&str>) -> Result<Option<TimeEntry>>;
    fn stop_timer(&self) -> Result<Option<TimeEntry>>;
    fn get_running_timer(&self) -> Result<Option<TimeEntry>>;
    fn list_time_entries(&self, filter: &EntryFilter) -> Result<EntryPage>;
    fn list_tasks(&self) -> Result<Vec<Task>>;
    fn client_state(&self) -> Result<ClientState>;

    /// OpenTodo has no documented sync command for tasks or time entries, so
    /// this always fails. See `docs/opentodo-compatibility.md` §6.
    fn sync(&self) -> Result<()> {
        Err(unsupported(
            "OpenTodo has no documented command to synchronise tasks or time entries",
        )
        .with_hint(
            "the ot client is online-only: every write goes straight to the server, so there \
             is nothing to push. `ot vault sync` and `ot issues sync` are unrelated features",
        ))
    }
}

/// `OpenTodo` implemented by running the `ot` command-line client.
pub struct OtCli<R: Runner> {
    program: String,
    runner: R,
}

impl<R: Runner> OtCli<R> {
    pub fn new(program: impl Into<String>, runner: R) -> OtCli<R> {
        OtCli { program: program.into(), runner }
    }

    /// Run `ot` with an argument vector and require a clean exit.
    fn invoke(&self, args: Vec<String>) -> Result<Output> {
        let invocation = Invocation::new(self.program.clone(), args);
        let output = self.runner.run(&invocation).map_err(|e| self.spawn_error(e, &invocation))?;
        match output.code {
            Some(0) => Ok(output),
            Some(code) => Err(self.command_error(code, &output, &invocation)),
            None => Err(Error::new(
                Kind::Client,
                format!("`{}` was terminated before it finished", invocation.display()),
            )
            .with_hint("no change was confirmed; check OpenTodo before retrying")),
        }
    }

    fn spawn_error(&self, err: RunError, invocation: &Invocation) -> Error {
        match err {
            RunError::NotFound => Error::new(
                Kind::Client,
                format!("the OpenTodo client {:?} was not found", self.program),
            )
            .with_hint(
                "install it with `go install github.com/OpenTodoApp/opentodo/cmd/ot@latest`, or set \
                 `ot_command` in the tt config file to its full path",
            ),
            RunError::NotExecutable(detail) => Error::new(
                Kind::Client,
                format!("the OpenTodo client {:?} could not be executed: {detail}", self.program),
            )
            .with_hint("check that the file is executable"),
            RunError::Timeout(d) => Error::new(
                Kind::Client,
                format!("`{}` did not finish within {}s", invocation.display(), d.as_secs()),
            )
            .with_hint(
                "the command was killed and no result was confirmed; check OpenTodo before \
                 retrying, or raise `timeout_seconds` in the tt config file",
            ),
            RunError::Io(detail) => Error::new(
                Kind::Client,
                format!("could not run `{}`: {detail}", invocation.display()),
            ),
        }
    }

    /// Map a non-zero `ot` exit into a `tt` error, preserving OpenTodo's own
    /// message and error code (redacted) as `SPEC.md` §8 requires.
    fn command_error(&self, code: i32, output: &Output, invocation: &Invocation) -> Error {
        let envelope = model::parse_error(&output.stdout, &output.stderr);
        let ot_code = envelope.as_ref().and_then(|e| e.code.clone());
        let detail = envelope
            .as_ref()
            .and_then(|e| e.message.clone())
            .filter(|m| !m.trim().is_empty())
            .or_else(|| {
                let line = output.stderr.lines().find(|l| !l.trim().is_empty())?;
                Some(line.trim().to_string())
            })
            .map(|m| redact::redact(&m));

        let kind = match code {
            1 => Kind::Server,
            2 => Kind::Usage,
            3 => Kind::Auth,
            4 => Kind::Unreachable,
            5 => Kind::Forbidden,
            _ => Kind::Server,
        };

        let what = match &detail {
            Some(message) => format!("OpenTodo reported: {message}"),
            None => format!("`{}` failed with exit status {code}", invocation.display()),
        };
        let message = match &ot_code {
            Some(c) => format!("{what} ({c})"),
            None => what,
        };

        let hint = match (kind, ot_code.as_deref()) {
            (_, Some("insufficient_scope")) => Some(
                "this operation writes, and the stored token is read-only; \
                 `ot auth status` shows the token scope"
                    .to_string(),
            ),
            (_, Some("read_only")) => {
                Some("the ot client is in read-only mode; unset OPENTODO_READ_ONLY".to_string())
            }
            (_, Some("not_found")) => {
                Some("check the identifier; `ot list --json` shows current tasks".to_string())
            }
            (_, Some("rate_limited")) => Some("wait and retry".to_string()),
            (_, Some(c @ ("conflict" | "idempotency_conflict"))) => Some(format!(
                "OpenTodo reported a {c}; open the entry in the OpenTodo web interface to see \
                 which change won before retrying"
            )),
            (Kind::Unreachable, _) => Some(
                "the OpenTodo server could not be reached. The ot client is online-only, so tt \
                 cannot record this locally; check the network and `ot auth status`"
                    .to_string(),
            ),
            (Kind::Auth, _) => Some(
                "sign in again with `ot login <server-url>`, or check `ot auth status`".to_string(),
            ),
            (Kind::Forbidden, _) => {
                Some("the stored token is not permitted to do this; check `ot auth status`".to_string())
            }
            (Kind::Usage, _) => Some(format!(
                "tt sent `{}`, which this ot version rejected. Compare it with `ot schema` and \
                 report a tt bug if the command is wrong",
                invocation.display()
            )),
            _ => None,
        };

        let error = Error::new(kind, message);
        match hint {
            Some(hint) => error.with_hint(hint),
            None => error,
        }
    }

    /// Reject values that a flag parser could mistake for an option. Task
    /// identifiers are passed positionally, so a leading dash is refused
    /// outright rather than quietly becoming a flag.
    fn check_positional(value: &str, what: &str) -> Result<()> {
        if value.trim().is_empty() {
            return Err(crate::error::usage(format!("{what} must not be empty")));
        }
        if value.starts_with('-') {
            return Err(crate::error::usage(format!(
                "{what} {value:?} starts with a dash, which OpenTodo would read as an option"
            ))
            .with_hint("pass the identifier OpenTodo shows, for example 8c2a"));
        }
        Ok(())
    }
}

/// Build the argument vector for `ot time list`.
///
/// Separate from the client so it can be unit-tested directly: this is the
/// surface where an invented flag would do the most damage.
pub fn time_list_args(filter: &EntryFilter) -> Vec<String> {
    let mut args = vec!["time".to_string(), "list".to_string(), "--json".to_string()];
    if let Some(from) = filter.from {
        args.push(format!("--from={from}"));
    }
    if let Some(to) = filter.to {
        args.push(format!("--to={to}"));
    }
    if let Some(task) = &filter.task {
        args.push(format!("--task={task}"));
    }
    if filter.running {
        args.push("--running".to_string());
    }
    if let Some(limit) = filter.limit {
        args.push(format!("--limit={limit}"));
    }
    args
}

impl<R: Runner> OpenTodo for OtCli<R> {
    fn version(&self) -> Result<VersionInfo> {
        let output = self.invoke(vec!["version".to_string(), "--json".to_string()])?;
        Ok(model::parse_version(&output.stdout))
    }

    fn start_timer(&self, task: &str, note: Option<&str>) -> Result<Option<TimeEntry>> {
        Self::check_positional(task, "a task identifier")?;
        let mut args = vec!["time".to_string(), "start".to_string(), task.to_string()];
        if let Some(note) = note.map(str::trim).filter(|n| !n.is_empty()) {
            // `--flag=value` keeps a note that begins with a dash from being
            // read as an option.
            args.push(format!("--note={note}"));
        }
        args.push("--json".to_string());
        let output = self.invoke(args)?;
        Ok(model::parse_optional_entry(&output.stdout))
    }

    fn stop_timer(&self) -> Result<Option<TimeEntry>> {
        let output =
            self.invoke(vec!["time".to_string(), "stop".to_string(), "--json".to_string()])?;
        Ok(model::parse_optional_entry(&output.stdout))
    }

    fn get_running_timer(&self) -> Result<Option<TimeEntry>> {
        let filter = EntryFilter { running: true, ..EntryFilter::default() };
        let page = self.list_time_entries(&filter)?;
        Ok(page.entries.into_iter().find(TimeEntry::is_running))
    }

    fn list_time_entries(&self, filter: &EntryFilter) -> Result<EntryPage> {
        if let Some(task) = &filter.task {
            Self::check_positional(task, "a task identifier")?;
        }
        let output = self.invoke(time_list_args(filter))?;
        model::parse_entry_page(&output.stdout)
    }

    fn list_tasks(&self) -> Result<Vec<Task>> {
        let output = self.invoke(vec!["list".to_string(), "--json".to_string()])?;
        model::parse_tasks(&output.stdout)
    }

    fn client_state(&self) -> Result<ClientState> {
        let args = vec!["auth".to_string(), "status".to_string(), "--json".to_string()];
        match self.invoke(args) {
            Ok(output) => {
                let value: serde_json::Value =
                    serde_json::from_str(output.stdout.trim()).unwrap_or(serde_json::Value::Null);
                let pick = |keys: &[&str]| -> Option<String> {
                    let map = value.as_object()?;
                    keys.iter()
                        .find_map(|k| map.get(*k).and_then(|v| v.as_str()))
                        .map(|s| s.to_string())
                };
                Ok(ClientState::Configured {
                    server: pick(&["server", "url", "serverUrl", "server_url"]),
                    account: pick(&["account", "email", "user", "userEmail"]),
                    scope: pick(&["scope", "tokenScope", "token_scope"]),
                })
            }
            Err(err) => match err.kind {
                Kind::Unreachable => Ok(ClientState::Unreachable(err.message().to_string())),
                Kind::Auth | Kind::Forbidden => {
                    Ok(ClientState::AuthFailure(err.message().to_string()))
                }
                // A missing or broken client is a real failure, not a state.
                Kind::Client => Err(err),
                _ => Ok(ClientState::Unknown(err.message().to_string())),
            },
        }
    }
}

/// Check the client against what this adapter can parse.
///
/// Returns a warning string when the version could not be determined, because
/// skipping the check silently would hide exactly the mismatch it exists to
/// catch.
pub fn check_version(info: &VersionInfo) -> Result<Option<String>> {
    if let Some(output_version) = info.output_version
        && output_version != SUPPORTED_OUTPUT_VERSION
    {
        return Err(Error::new(
            Kind::Unsupported,
            format!(
                "this OpenTodo client reports output_version {output_version}, but tt understands \
                 only version {SUPPORTED_OUTPUT_VERSION}"
            ),
        )
        .with_hint("upgrade tt, or run an ot release whose --json output is version 1"));
    }

    match info.client_version.as_deref().and_then(model::parse_semver) {
        Some(found) if found < MIN_OT_VERSION => {
            let (ma, mi, pa) = MIN_OT_VERSION;
            Err(Error::new(
                Kind::Unsupported,
                format!(
                    "OpenTodo client version {} is too old; tt needs {ma}.{mi}.{pa} or later",
                    info.client_version.as_deref().unwrap_or("?")
                ),
            )
            .with_hint("time tracking was added in OpenTodo 0.9.0"))
        }
        Some(_) => Ok(None),
        None => Ok(Some(
            "could not determine the OpenTodo client version from `ot version --json`; \
             continuing without the minimum-version check"
                .to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::runner::mock::{MockRunner, fail, ok};
    use super::*;
    use jiff::civil::date;

    fn client() -> OtCli<MockRunner> {
        OtCli::new("ot", MockRunner::new())
    }

    #[test]
    fn time_list_arguments_use_only_documented_flags() {
        let filter = EntryFilter {
            from: Some(date(2026, 10, 20)),
            to: Some(date(2026, 10, 26)),
            task: Some("8c2a".into()),
            running: false,
            limit: Some(500),
        };
        assert_eq!(
            time_list_args(&filter),
            vec![
                "time",
                "list",
                "--json",
                "--from=2026-10-20",
                "--to=2026-10-26",
                "--task=8c2a",
                "--limit=500"
            ]
        );
    }

    #[test]
    fn running_filter_queries_the_documented_running_flag() {
        let client = client();
        client.runner.push("time list", ok("[]"));
        client.get_running_timer().expect("query runs");
        assert_eq!(client.runner.call(0), vec!["time", "list", "--json", "--running"]);
    }

    #[test]
    fn a_note_is_passed_as_one_argument_never_through_a_shell() {
        let client = client();
        client.runner.push("time start", ok("{}"));
        client.start_timer("8c2a", Some("rm -rf / ; echo $(whoami)")).expect("starts");
        assert_eq!(
            client.runner.call(0),
            vec!["time", "start", "8c2a", "--note=rm -rf / ; echo $(whoami)", "--json"]
        );
    }

    #[test]
    fn a_note_that_starts_with_a_dash_cannot_become_a_flag() {
        let client = client();
        client.runner.push("time start", ok("{}"));
        client.start_timer("8c2a", Some("--server=https://evil.example")).expect("starts");
        let args = client.runner.call(0);
        assert_eq!(args[3], "--note=--server=https://evil.example");
    }

    #[test]
    fn a_task_id_that_starts_with_a_dash_is_refused_before_ot_runs() {
        let client = client();
        let err = client.start_timer("--server", None).expect_err("must refuse");
        assert_eq!(err.kind, Kind::Usage);
        assert!(client.runner.calls().is_empty(), "ot must not be invoked");
    }

    #[test]
    fn an_empty_note_is_omitted_rather_than_sent_empty() {
        let client = client();
        client.runner.push("time start", ok("{}"));
        client.start_timer("8c2a", Some("   ")).expect("starts");
        assert_eq!(client.runner.call(0), vec!["time", "start", "8c2a", "--json"]);
    }

    #[test]
    fn exit_codes_map_to_the_documented_meanings() {
        for (code, kind) in [
            (1, Kind::Server),
            (2, Kind::Usage),
            (3, Kind::Auth),
            (4, Kind::Unreachable),
            (5, Kind::Forbidden),
        ] {
            let client = client();
            client.runner.push("time list", fail(code, "", "ot: something went wrong"));
            let err = client.list_time_entries(&EntryFilter::default()).expect_err("fails");
            assert_eq!(err.kind, kind, "exit {code}");
            assert_eq!(err.exit_code(), code, "tt mirrors ot's exit code");
        }
    }

    #[test]
    fn the_error_envelope_message_and_code_reach_the_user() {
        let client = client();
        client.runner.push(
            "time start",
            fail(
                5,
                r#"{"error":{"code":"insufficient_scope","message":"token is read-only","exit":5}}"#,
                "ot: token is read-only",
            ),
        );
        let err = client.start_timer("8c2a", None).expect_err("fails");
        assert!(err.message().contains("token is read-only"), "{err}");
        assert!(err.message().contains("insufficient_scope"), "{err}");
        assert!(err.hint().unwrap().contains("read-only"), "{err}");
    }

    #[test]
    fn secrets_in_ot_errors_are_redacted() {
        let client = client();
        client.runner.push(
            "time list",
            fail(3, r#"{"error":{"code":"unauthorized","message":"bad token ot_9f8e7d6c5b4a3210ffee"}}"#, ""),
        );
        let err = client.list_time_entries(&EntryFilter::default()).expect_err("fails");
        assert!(!err.message().contains("ot_9f8e7d6c5b4a3210ffee"), "{err}");
    }

    #[test]
    fn a_missing_client_is_reported_with_install_guidance() {
        let runner = MockRunner::bare();
        runner.push("version", Err(RunError::NotFound));
        let client = OtCli::new("ot", runner);
        let err = client.version().expect_err("not installed");
        assert_eq!(err.kind, Kind::Client);
        assert_eq!(err.exit_code(), 7);
        assert!(err.hint().unwrap().contains("ot_command"), "{err}");
    }

    #[test]
    fn a_timeout_never_claims_the_operation_succeeded() {
        let runner = MockRunner::bare();
        runner.push("time stop", Err(RunError::Timeout(std::time::Duration::from_secs(30))));
        let client = OtCli::new("ot", runner);
        let err = client.stop_timer().expect_err("timed out");
        assert_eq!(err.kind, Kind::Client);
        assert!(err.hint().unwrap().contains("no result was confirmed"), "{err}");
    }

    #[test]
    fn sync_is_reported_as_unsupported_without_invoking_anything() {
        let client = client();
        let err = client.sync().expect_err("no documented sync command");
        assert_eq!(err.kind, Kind::Unsupported);
        assert_eq!(err.exit_code(), 6);
        assert!(client.runner.calls().is_empty());
    }

    #[test]
    fn unreachable_becomes_a_client_state_not_a_hard_error() {
        let client = client();
        client.runner.push("auth status", fail(4, "", "ot: server unreachable"));
        let state = client.client_state().expect("state is reported");
        assert!(matches!(state, ClientState::Unreachable(_)), "{state:?}");
    }

    #[test]
    fn version_check_rejects_an_unknown_output_version() {
        let info = VersionInfo { output_version: Some(2), client_version: Some("1.0.0".into()) };
        let err = check_version(&info).expect_err("unsupported");
        assert_eq!(err.kind, Kind::Unsupported);
    }

    #[test]
    fn version_check_rejects_a_client_older_than_time_tracking() {
        let info = VersionInfo { output_version: Some(1), client_version: Some("0.8.9".into()) };
        let err = check_version(&info).expect_err("too old");
        assert!(err.message().contains("0.9.0"), "{err}");
    }

    #[test]
    fn version_check_warns_rather_than_guesses_when_the_version_is_absent() {
        let info = VersionInfo { output_version: Some(1), client_version: None };
        let warning = check_version(&info).expect("not fatal").expect("warns");
        assert!(warning.contains("could not determine"), "{warning}");
    }
}
