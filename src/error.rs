use std::fmt;

/// Why a command failed. The discriminants drive `tt`'s exit codes: 1-5 mirror
/// the documented `ot` exit codes so a script wrapping `tt` can treat them the
/// same way, and 6-8 cover conditions that are `tt`'s own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `ot` exit 1: the server rejected the request or failed.
    Server,
    /// `ot` exit 2, bad input, or an unusable config file.
    Usage,
    /// `ot` exit 3: authentication or configuration problem.
    Auth,
    /// `ot` exit 4: the OpenTodo server could not be reached.
    Unreachable,
    /// `ot` exit 5: the token or instance does not permit the operation.
    Forbidden,
    /// The operation has no documented OpenTodo equivalent.
    Unsupported,
    /// The `ot` client is missing, not executable, or did not finish.
    Client,
    /// No timer is running.
    NoTimer,
}

impl Kind {
    pub fn exit_code(self) -> i32 {
        match self {
            Kind::Server => 1,
            Kind::Usage => 2,
            Kind::Auth => 3,
            Kind::Unreachable => 4,
            Kind::Forbidden => 5,
            Kind::Unsupported => 6,
            Kind::Client => 7,
            Kind::NoTimer => 8,
        }
    }
}

/// An error with an optional actionable hint. Every error surfaced to the user
/// is one of these, so `SPEC.md` §10's "actionable errors" requirement is a
/// property of the type rather than of each call site remembering.
#[derive(Debug, Clone)]
pub struct Error {
    pub kind: Kind,
    message: String,
    hint: Option<String>,
}

impl Error {
    pub fn new(kind: Kind, message: impl Into<String>) -> Error {
        Error { kind, message: message.into(), hint: None }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Error {
        self.hint = Some(hint.into());
        self
    }

    pub fn exit_code(&self) -> i32 {
        self.kind.exit_code()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// Used by tests and by callers that render hints themselves.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, "\n  hint: {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub fn usage(message: impl Into<String>) -> Error {
    Error::new(Kind::Usage, message)
}

pub fn unsupported(message: impl Into<String>) -> Error {
    Error::new(Kind::Unsupported, message)
}
