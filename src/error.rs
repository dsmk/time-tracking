use std::fmt;

/// Error categories. Each maps to a distinct process exit status so that
/// scripts can tell failure modes apart (see `ErrorKind::exit_code`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Bad user input: malformed times, empty descriptions, bad flag combinations.
    Validation,
    /// An entry (or id prefix) could not be resolved.
    NotFound,
    /// The request conflicts with existing data: overlap, timer already running.
    Conflict,
    /// The configuration file is missing a valid value or cannot be written.
    Config,
    /// The database could not be opened, migrated, or queried.
    Database,
}

impl ErrorKind {
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorKind::Validation => 2,
            ErrorKind::NotFound => 3,
            ErrorKind::Conflict => 4,
            ErrorKind::Config => 5,
            ErrorKind::Database => 6,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ErrorKind::Validation => "invalid input",
            ErrorKind::NotFound => "not found",
            ErrorKind::Conflict => "conflict",
            ErrorKind::Config => "configuration error",
            ErrorKind::Database => "database error",
        }
    }
}

#[derive(Debug)]
pub struct AppError {
    pub kind: ErrorKind,
    pub message: String,
    /// Optional second line with a concrete suggestion for the user.
    pub hint: Option<String>,
}

impl AppError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        AppError { kind, message: message.into(), hint: None }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.label(), self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, "\nhint: {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for AppError {}

pub type Result<T> = std::result::Result<T, AppError>;

pub fn validation(message: impl Into<String>) -> AppError {
    AppError::new(ErrorKind::Validation, message)
}

pub fn not_found(message: impl Into<String>) -> AppError {
    AppError::new(ErrorKind::NotFound, message)
}

pub fn conflict(message: impl Into<String>) -> AppError {
    AppError::new(ErrorKind::Conflict, message)
}

pub fn config(message: impl Into<String>) -> AppError {
    AppError::new(ErrorKind::Config, message)
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        AppError::new(ErrorKind::Database, err.to_string())
    }
}
