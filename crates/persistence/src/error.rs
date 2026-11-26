use std::fmt;

/// Errors produced while reading or writing persisted state or journal files.
#[derive(Debug)]
pub enum PersistenceError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Csv(csv::Error),
}

impl From<std::io::Error> for PersistenceError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for PersistenceError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<csv::Error> for PersistenceError {
    fn from(value: csv::Error) -> Self {
        Self::Csv(value)
    }
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Json(err) => write!(f, "JSON serialization error: {err}"),
            Self::Csv(err) => write!(f, "CSV journal error: {err}"),
        }
    }
}

impl std::error::Error for PersistenceError {}
