use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A single `ot` invocation, always as a program plus an argument vector.
///
/// `SPEC.md` §4 forbids building a shell string: descriptions, notes and task
/// identifiers arrive from the user and must never be re-parsed by a shell.
/// Keeping the arguments in a `Vec<String>` all the way to `Command::args`
/// means there is no point at which quoting could be got wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: String,
    pub args: Vec<String>,
}

impl Invocation {
    pub fn new(program: impl Into<String>, args: impl IntoIterator<Item = String>) -> Invocation {
        Invocation { program: program.into(), args: args.into_iter().collect() }
    }

    /// Human-readable form **for error messages only** — never executed.
    pub fn display(&self) -> String {
        let mut out = quote(&self.program);
        for arg in &self.args {
            out.push(' ');
            out.push_str(&quote(arg));
        }
        out
    }
}

fn quote(arg: &str) -> String {
    let safe = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '=' | ','));
    if safe { arg.to_string() } else { format!("{arg:?}") }
}

#[derive(Debug, Clone)]
pub struct Output {
    /// `None` when the process was terminated by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone)]
pub enum RunError {
    /// The program could not be found on `PATH` or at the configured path.
    NotFound,
    /// Found, but could not be executed (permissions, not an executable, ...).
    NotExecutable(String),
    /// Killed after exceeding the configured timeout.
    Timeout(Duration),
    Io(String),
}

/// Anything that can execute an `ot` invocation. The process implementation is
/// the real one; unit tests substitute a mock so command construction and
/// output handling can be tested without `ot` installed.
pub trait Runner {
    fn run(&self, invocation: &Invocation) -> Result<Output, RunError>;
}

pub struct ProcessRunner {
    timeout: Duration,
}

impl ProcessRunner {
    pub fn new(timeout_seconds: u64) -> ProcessRunner {
        ProcessRunner { timeout: Duration::from_secs(timeout_seconds) }
    }
}

impl Runner for ProcessRunner {
    fn run(&self, invocation: &Invocation) -> Result<Output, RunError> {
        let mut child = Command::new(&invocation.program)
            .args(&invocation.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => RunError::NotFound,
                std::io::ErrorKind::PermissionDenied => RunError::NotExecutable(e.to_string()),
                _ => RunError::Io(e.to_string()),
            })?;

        // Drain both pipes on their own threads: a child that fills one pipe
        // while we wait on the other would deadlock.
        let mut stdout_pipe = child.stdout.take().expect("stdout is piped");
        let mut stderr_pipe = child.stderr.take().expect("stderr is piped");
        let stdout_reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout_pipe.read_to_end(&mut buf);
            buf
        });
        let stderr_reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stderr_pipe.read_to_end(&mut buf);
            buf
        });

        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(RunError::Io(e.to_string())),
            }
        };

        let stdout = String::from_utf8_lossy(&stdout_reader.join().unwrap_or_default()).into_owned();
        let stderr = String::from_utf8_lossy(&stderr_reader.join().unwrap_or_default()).into_owned();

        match status {
            Some(status) => Ok(Output { code: status.code(), stdout, stderr }),
            None => Err(RunError::Timeout(self.timeout)),
        }
    }
}

#[cfg(test)]
pub mod mock {
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};

    use super::{Invocation, Output, RunError, Runner};

    /// In-process stand-in for `ot`. Responses are keyed by the invocation's
    /// leading non-flag arguments, for example `time list` or `auth status`.
    #[derive(Default)]
    pub struct MockRunner {
        responses: RefCell<HashMap<String, VecDeque<Result<Output, RunError>>>>,
        pub calls: RefCell<Vec<Invocation>>,
    }

    impl MockRunner {
        pub fn new() -> MockRunner {
            let runner = MockRunner::default();
            // Every command path begins with a version probe.
            runner.push("version", ok(r#"{"output_version":1,"version":"0.13.0"}"#));
            runner
        }

        /// A mock with no canned version response, for version-probe tests.
        pub fn bare() -> MockRunner {
            MockRunner::default()
        }

        pub fn push(&self, key: &str, response: Result<Output, RunError>) -> &MockRunner {
            self.responses.borrow_mut().entry(key.to_string()).or_default().push_back(response);
            self
        }

        pub fn calls(&self) -> Vec<Invocation> {
            self.calls.borrow().clone()
        }

        /// Arguments of the nth call, for assertions on command construction.
        pub fn call(&self, index: usize) -> Vec<String> {
            self.calls.borrow()[index].args.clone()
        }
    }

    pub fn key_for(args: &[String]) -> String {
        args.iter()
            .filter(|a| !a.starts_with('-'))
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn ok(stdout: &str) -> Result<Output, RunError> {
        Ok(Output { code: Some(0), stdout: stdout.to_string(), stderr: String::new() })
    }

    pub fn fail(code: i32, stdout: &str, stderr: &str) -> Result<Output, RunError> {
        Ok(Output { code: Some(code), stdout: stdout.to_string(), stderr: stderr.to_string() })
    }

    impl Runner for MockRunner {
        fn run(&self, invocation: &Invocation) -> Result<Output, RunError> {
            self.calls.borrow_mut().push(invocation.clone());
            let key = key_for(&invocation.args);
            let mut responses = self.responses.borrow_mut();
            match responses.get_mut(&key).and_then(|queue| queue.pop_front()) {
                Some(response) => response,
                None => panic!("MockRunner has no response for {key:?} ({})", invocation.display()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_quotes_arguments_that_need_it() {
        let inv = Invocation::new(
            "ot",
            ["time".into(), "start".into(), "8c2a".into(), "--note".into(), "a b; rm -rf /".into()],
        );
        let shown = inv.display();
        assert!(shown.contains(r#""a b; rm -rf /""#), "{shown}");
    }

    #[test]
    fn missing_program_is_reported_as_not_found() {
        let runner = ProcessRunner::new(5);
        let err = runner
            .run(&Invocation::new("tt-no-such-program-exists", []))
            .expect_err("program does not exist");
        assert!(matches!(err, RunError::NotFound), "{err:?}");
    }

    #[test]
    fn output_streams_and_exit_code_are_captured_separately() {
        let runner = ProcessRunner::new(10);
        let out = runner
            .run(&Invocation::new(
                "/bin/sh",
                ["-c".into(), "printf out; printf err >&2; exit 3".into()],
            ))
            .expect("sh runs");
        assert_eq!(out.code, Some(3));
        assert_eq!(out.stdout, "out");
        assert_eq!(out.stderr, "err");
    }

    #[test]
    fn a_hung_child_is_killed_at_the_timeout() {
        let runner = ProcessRunner { timeout: Duration::from_millis(150) };
        let err = runner
            .run(&Invocation::new("/bin/sh", ["-c".into(), "sleep 30".into()]))
            .expect_err("must time out");
        assert!(matches!(err, RunError::Timeout(_)), "{err:?}");
    }

    #[test]
    fn termination_by_signal_is_visible_as_a_missing_exit_code() {
        let runner = ProcessRunner::new(10);
        let out = runner
            .run(&Invocation::new("/bin/sh", ["-c".into(), "kill -TERM $$".into()]))
            .expect("sh runs");
        assert_eq!(out.code, None);
    }
}
