use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug)]
pub enum EvaluateError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Search(crate::search::SearchError),
    Tape(crate::tape::TapeError),
    InvalidConfig(String),
    InvalidManifest(String),
    InvalidResult(String),
    NativeResult(String),
    Launch(std::io::Error),
    Timeout(Duration),
    Cancelled,
    Infrastructure {
        faults: usize,
        completed: usize,
        planned: usize,
        evidence: PathBuf,
    },
    EmptyLeaderboard,
}

impl fmt::Display for EvaluateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "search evaluator I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "invalid search evaluator JSON: {error}"),
            Self::Search(error) => error.fmt(formatter),
            Self::Tape(error) => error.fmt(formatter),
            Self::InvalidConfig(message) => {
                write!(formatter, "invalid evaluator config: {message}")
            }
            Self::InvalidManifest(message) => write!(formatter, "invalid population: {message}"),
            Self::InvalidResult(message) => write!(formatter, "invalid search result: {message}"),
            Self::NativeResult(message) => {
                write!(formatter, "invalid native milestone result: {message}")
            }
            Self::Launch(error) => write!(formatter, "could not launch Dusklight: {error}"),
            Self::Timeout(duration) => write!(
                formatter,
                "Dusklight timed out after {} ms",
                duration.as_millis()
            ),
            Self::Cancelled => {
                formatter.write_str("trial cancelled after another infrastructure fault")
            }
            Self::Infrastructure {
                faults,
                completed,
                planned,
                evidence,
            } => write!(
                formatter,
                "population evaluation failed: {faults} infrastructure fault(s), {completed}/{planned} attempts completed; evidence: {}",
                evidence.display()
            ),
            Self::EmptyLeaderboard => formatter.write_str("search produced an empty leaderboard"),
        }
    }
}

impl Error for EvaluateError {}

impl From<std::io::Error> for EvaluateError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for EvaluateError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<crate::search::SearchError> for EvaluateError {
    fn from(value: crate::search::SearchError) -> Self {
        Self::Search(value)
    }
}

impl From<crate::tape::TapeError> for EvaluateError {
    fn from(value: crate::tape::TapeError) -> Self {
        Self::Tape(value)
    }
}

impl From<crate::continuous_search::ContinuousSearchError> for EvaluateError {
    fn from(value: crate::continuous_search::ContinuousSearchError) -> Self {
        Self::InvalidConfig(value.to_string())
    }
}

impl From<crate::bayesian_search::BayesianError> for EvaluateError {
    fn from(value: crate::bayesian_search::BayesianError) -> Self {
        Self::InvalidConfig(value.to_string())
    }
}
