#![allow(dead_code)]

use std::path::PathBuf;
use std::process::{Command, Output};

use tempfile::TempDir;

/// A throwaway database and config file, plus helpers for running the real
/// `tt` binary against them with a pinned clock.
pub struct Harness {
    pub dir: TempDir,
    pub db: PathBuf,
    pub config: PathBuf,
}

pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Harness {
    /// New harness configured for America/New_York with a Monday week start,
    /// so timezone and week-boundary behavior is exercised rather than
    /// whatever the host happens to be set to.
    pub fn new() -> Harness {
        Harness::with_config("timezone = \"America/New_York\"\nweek_start = \"monday\"\n")
    }

    pub fn with_config(body: &str) -> Harness {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = dir.path().join("data").join("tt.db");
        let config = dir.path().join("config.toml");
        std::fs::write(&config, body).expect("write config");
        Harness { dir, db, config }
    }

    pub fn command(&self, now: &str, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_tt"));
        cmd.args(args)
            .env("TT_DB", &self.db)
            .env("TT_CONFIG", &self.config)
            .env("TT_NOW", now)
            // Keep the host's own settings from leaking into the test.
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

    /// Run a command that must succeed, returning stdout.
    pub fn ok(&self, now: &str, args: &[&str]) -> String {
        let run = self.run(now, args);
        assert_eq!(
            run.code, 0,
            "expected `tt {}` to succeed\nstdout: {}\nstderr: {}",
            args.join(" "),
            run.stdout,
            run.stderr
        );
        run.stdout
    }

    /// Run a command that must fail, returning the exit code and stderr.
    pub fn fail(&self, now: &str, args: &[&str]) -> Run {
        let run = self.run(now, args);
        assert_ne!(
            run.code, 0,
            "expected `tt {}` to fail, but it printed: {}",
            args.join(" "),
            run.stdout
        );
        assert!(
            run.stdout.trim().is_empty(),
            "a failing command must not write results to stdout, got: {}",
            run.stdout
        );
        run
    }

    pub fn json(&self, now: &str, args: &[&str]) -> serde_json::Value {
        let stdout = self.ok(now, args);
        serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("stdout was not valid JSON ({e}): {stdout}"))
    }

    /// Full id of the single entry matching a description, via JSON output.
    pub fn id_of(&self, now: &str, description: &str) -> String {
        let entries = self.json(now, &["log", "--json", "--limit", "500"]);
        let matches: Vec<&serde_json::Value> = entries
            .as_array()
            .expect("log --json returns an array")
            .iter()
            .filter(|e| e["description"] == description)
            .collect();
        assert_eq!(matches.len(), 1, "expected one entry named {description:?}");
        matches[0]["id"].as_str().expect("id is a string").to_string()
    }
}
