//! Durable binary ownership for the authoritative state graph.

use crate::state_graph::{StateGraph, StateGraphError};
use dusklight_automation_contracts::artifact::Digest;
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const STATE_GRAPH_FILE_EXTENSION: &str = "dsg";
const MAXIMUM_STATE_GRAPH_BYTES: u64 = 16 * 1024 * 1024 * 1024;
static TEMPORARY_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredStateGraph {
    pub path: PathBuf,
    pub graph_sha256: Digest,
}

pub fn persist_state_graph(
    directory: &Path,
    graph: &StateGraph,
) -> Result<StoredStateGraph, PersistenceError> {
    graph.validate()?;
    let bytes = graph.encode()?;
    let graph_sha256 = graph.content_sha256()?;
    fs::create_dir_all(directory).map_err(PersistenceError::Io)?;
    let path = directory.join(format!(
        "{}.{STATE_GRAPH_FILE_EXTENSION}",
        lower_hex(graph_sha256)
    ));
    if path.exists() {
        let existing = read_state_graph(&path, graph_sha256)?;
        if existing != *graph {
            return Err(PersistenceError::Collision);
        }
        return Ok(StoredStateGraph { path, graph_sha256 });
    }

    let sequence = TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(
        ".{}.{}.{}.tmp",
        lower_hex(graph_sha256),
        std::process::id(),
        sequence
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(PersistenceError::Io)?;
    if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(PersistenceError::Io(error));
    }
    drop(file);
    match fs::rename(&temporary, &path) {
        Ok(()) => {}
        Err(_) if path.exists() => {
            fs::remove_file(&temporary).map_err(PersistenceError::Io)?;
            let existing = read_state_graph(&path, graph_sha256)?;
            if existing != *graph {
                return Err(PersistenceError::Collision);
            }
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(PersistenceError::Io(error));
        }
    }
    Ok(StoredStateGraph { path, graph_sha256 })
}

pub fn read_state_graph(
    path: &Path,
    expected_graph_sha256: Digest,
) -> Result<StateGraph, PersistenceError> {
    if expected_graph_sha256 == Digest::ZERO {
        return Err(PersistenceError::Invalid(
            "expected state graph identity is missing",
        ));
    }
    let metadata = fs::metadata(path).map_err(PersistenceError::Io)?;
    if metadata.len() == 0
        || metadata.len() > MAXIMUM_STATE_GRAPH_BYTES
        || metadata.len() > usize::MAX as u64
    {
        return Err(PersistenceError::Invalid(
            "state graph file size is invalid",
        ));
    }
    let bytes = fs::read(path).map_err(PersistenceError::Io)?;
    let graph = StateGraph::decode(&bytes)?;
    if graph.content_sha256()? != expected_graph_sha256 {
        return Err(PersistenceError::Invalid(
            "state graph file content identity does not match",
        ));
    }
    Ok(graph)
}

fn lower_hex(digest: Digest) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(64);
    for byte in digest.0 {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0x0f) as usize] as char);
    }
    value
}

#[derive(Debug)]
pub enum PersistenceError {
    Invalid(&'static str),
    Collision,
    Graph(StateGraphError),
    Io(std::io::Error),
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid durable state graph: {message}"),
            Self::Collision => formatter.write_str("state graph content identity collision"),
            Self::Graph(error) => write!(formatter, "state graph persistence failed: {error}"),
            Self::Io(error) => write!(formatter, "state graph persistence I/O failed: {error}"),
        }
    }
}

impl Error for PersistenceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Graph(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Invalid(_) | Self::Collision => None,
        }
    }
}

impl From<StateGraphError> for PersistenceError {
    fn from(value: StateGraphError) -> Self {
        Self::Graph(value)
    }
}
