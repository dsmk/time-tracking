#![allow(dead_code)]

use std::path::PathBuf;
use std::process::{Command, Output};

use tempfile::TempDir;

/// Integration harness: a temporary directory holding a mock `ot` executable,
/// its canned responses, a `tt` config file pointing at it, and a log of every
/// invocation.
///
/// The mock answers only the commands a test installs a response for. Anything
/// else exits 2 with an `unexpected_command` envelope, so if `tt` ever invents
/// an `ot` subcommand the test that exercises that path fails — which is how
/// `SPEC.md` §11's "unsupported commands are not invoked" is enforced.
pub struct Harness {
    pub dir: TempDir,
    pub mock: PathBuf,
    pub config: PathBuf,
    pub responses: PathBuf,
}

pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

const MOCK: &str = r#"#!/usr/bin/env bash
set -u
dir="${MOCK_OT_DIR:?MOCK_OT_DIR is not set}"

# Record the invocation: one argument per line, terminated by a record mark.
{ for a in "$@"; do printf '%s\n' "$a"; done; printf '\036\n'; } >> "$dir/calls.log"

# Response key: the first two non-flag arguments, e.g. `time list`, `version`.
key=""
count=0
for a in "$@"; do
  case "$a" in -*) continue ;; esac
  if [ "$count" -lt 2 ]; then
    if [ -z "$key" ]; then key="$a"; else key="$key-$a"; fi
    count=$((count + 1))
  fi
done

n=0
[ -f "$dir/$key.count" ] && n=$(cat "$dir/$key.count")
n=$((n + 1))
printf '%s' "$n" > "$dir/$key.count"

out="$dir/$key.$n.out"; [ -f "$out" ] || out="$dir/$key.out"
err="$dir/$key.$n.err"; [ -f "$err" ] || err="$dir/$key.err"
code_file="$dir/$key.$n.code"; [ -f "$code_file" ] || code_file="$dir/$key.code"

if [ ! -f "$out" ] && [ ! -f "$code_file" ]; then
  printf '{"error":{"code":"unexpected_command","message":"mock ot has no response for %s","exit":2}}' "$key"
  printf 'ot: unexpected command: %s\n' "$key" >&2
  exit 2
fi

[ -f "$out" ] && cat "$out"
[ -f "$err" ] && cat "$err" >&2
code=0
[ -f "$code_file" ] && code=$(cat "$code_file")
exit "$code"
"#;

/// A version response the harness installs by default, so tests do not have to
/// restate the probe that runs before every command.
pub const VERSION_JSON: &str = r#"{"output_version":1,"version":"0.13.0"}"#;

impl Harness {
    /// America/New_York with a Monday week start, so timezone and week
    /// behaviour is exercised rather than the host's own settings.
    pub fn new() -> Harness {
        Harness::with_settings("timezone = \"America/New_York\"\nweek_start = \"monday\"\n")
    }

    pub fn with_settings(body: &str) -> Harness {
        let dir = tempfile::tempdir().expect("temp dir");
        let responses = dir.path().join("responses");
        std::fs::create_dir_all(&responses).expect("responses dir");

        let mock = dir.path().join("mock-ot");
        std::fs::write(&mock, MOCK).expect("write mock");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&mock, std::fs::Permissions::from_mode(0o755))
                .expect("chmod mock");
        }

        let config = dir.path().join("config.toml");
        let escaped = mock.display().to_string().replace('\\', "\\\\");
        std::fs::write(&config, format!("ot_command = \"{escaped}\"\n{body}"))
            .expect("write config");

        let harness = Harness { dir, mock, config, responses };
        harness.respond("version", VERSION_JSON);
        harness
    }

    /// Install the response for every call with this key.
    pub fn respond(&self, key: &str, stdout: &str) -> &Harness {
        std::fs::write(self.responses.join(format!("{key}.out")), stdout).expect("write response");
        self
    }

    /// Install the response for the nth call with this key (1-based).
    pub fn respond_nth(&self, key: &str, n: usize, stdout: &str) -> &Harness {
        std::fs::write(self.responses.join(format!("{key}.{n}.out")), stdout)
            .expect("write response");
        self
    }

    /// Install a failing response: exit code plus the documented error envelope.
    pub fn respond_fail(&self, key: &str, code: i32, stdout: &str, stderr: &str) -> &Harness {
        std::fs::write(self.responses.join(format!("{key}.out")), stdout).expect("write stdout");
        std::fs::write(self.responses.join(format!("{key}.err")), stderr).expect("write stderr");
        std::fs::write(self.responses.join(format!("{key}.code")), code.to_string())
            .expect("write code");
        self
    }

    pub fn command(&self, now: &str, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_tt"));
        cmd.args(args)
            .env("TT_CONFIG", &self.config)
            .env("TT_NOW", now)
            .env("MOCK_OT_DIR", &self.responses)
            // Keep the host's own zone from leaking into the test.
            .env_remove("TZ");
        cmd
    }

    pub fn run(&self, now: &str, args: &[&str]) -> Run {
        let out: Output = self.command(now, args).output().expect("run tt");
        Run {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        }
    }

    pub fn ok(&self, now: &str, args: &[&str]) -> String {
        let run = self.run(now, args);
        assert_eq!(
            run.code,
            0,
            "expected `tt {}` to succeed\nstdout: {}\nstderr: {}",
            args.join(" "),
            run.stdout,
            run.stderr
        );
        run.stdout
    }

    pub fn fail(&self, now: &str, args: &[&str]) -> Run {
        let run = self.run(now, args);
        assert_ne!(
            run.code,
            0,
            "expected `tt {}` to fail, but it printed: {}",
            args.join(" "),
            run.stdout
        );
        run
    }

    pub fn json(&self, now: &str, args: &[&str]) -> serde_json::Value {
        let stdout = self.ok(now, args);
        serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("stdout was not valid JSON ({e}): {stdout}"))
    }

    /// Every `ot` invocation the mock saw, in order, as argument vectors.
    pub fn calls(&self) -> Vec<Vec<String>> {
        let path = self.responses.join("calls.log");
        let Ok(text) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        let mut calls = Vec::new();
        let mut current = Vec::new();
        for line in text.lines() {
            if line == "\u{1e}" {
                calls.push(std::mem::take(&mut current));
            } else {
                current.push(line.to_string());
            }
        }
        calls
    }

    /// Invocations whose leading arguments match `prefix`.
    pub fn calls_matching(&self, prefix: &[&str]) -> Vec<Vec<String>> {
        self.calls()
            .into_iter()
            .filter(|call| call.len() >= prefix.len() && call[..prefix.len()] == *prefix)
            .collect()
    }

    pub fn call_count(&self, prefix: &[&str]) -> usize {
        self.calls_matching(prefix).len()
    }
}

/// One closed time entry, in the documented camelCase shape.
pub fn entry(id: &str, task: &str, title: &str, start_ms: i64, end_ms: Option<i64>) -> String {
    let ended = match end_ms {
        Some(ms) => ms.to_string(),
        None => "null".to_string(),
    };
    let minutes = match end_ms {
        Some(ms) => ((ms - start_ms) / 60_000).to_string(),
        None => "null".to_string(),
    };
    format!(
        r#"{{"id":"{id}","taskId":"{task}","userId":"u1","startedAt":{start_ms},"endedAt":{ended},"minutes":{minutes},"note":"n","deviceId":"d1","taskTitle":"{title}","projectId":"p1","projectName":"Work"}}"#
    )
}

/// Unix milliseconds for an RFC 3339 instant, for building fixtures.
pub fn ms(raw: &str) -> i64 {
    raw.parse::<jiff::Timestamp>().expect("valid timestamp").as_millisecond()
}
