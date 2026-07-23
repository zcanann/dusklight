//! Version-scoped causal route-planning contracts.
//!
//! This crate owns the authoritative planner IR, validation, fact-pack identity,
//! solving, proofs, and planner graph projection as those layers are implemented.
//! It also owns the portable observation and world-fact inputs accepted by the
//! planner. It does not depend on TAS timelines, playback, native execution, or
//! browser code.

pub mod artifact;
pub mod authorization;
pub mod binary_evidence;
pub mod cutscene;
pub mod cutscene_corruption;
pub mod cutscene_import;
pub mod cutscene_outer;
pub mod cutscene_runtime;
pub mod demo_actor;
pub mod evaluation;
pub mod execution;
pub mod fact_pack;
pub mod fact_pack_cache;
pub mod graph;
pub mod identity;
pub mod jstudio_import;
pub mod jstudio_semantics;
pub mod logic;
pub mod message_flow;
pub mod message_import;
pub mod native_observation;
pub mod native_snapshot;
pub mod orig_diff;
pub mod orig_discovery;
pub mod orig_extraction;
pub mod refinement;
pub mod relevance;
pub mod return_place;
pub mod route_book;
pub mod snapshot;
pub mod solver;
pub mod state;
pub mod supported_context;
pub mod title_boundary;
pub mod transition;
pub mod world_data;
pub mod world_import;

use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannerContractError {
    field: String,
    detail: String,
}

impl PlannerContractError {
    pub fn new(field: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            detail: detail.into(),
        }
    }

    pub fn field(&self) -> &str {
        &self.field
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for PlannerContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.field, self.detail)
    }
}

impl Error for PlannerContractError {}

impl From<serde_json::Error> for PlannerContractError {
    fn from(value: serde_json::Error) -> Self {
        Self::new("json", value.to_string())
    }
}

pub(crate) fn validate_stable_id(field: &str, value: &str) -> Result<(), PlannerContractError> {
    if value.is_empty() || value.len() > 128 {
        return Err(PlannerContractError::new(
            field,
            "must contain between 1 and 128 characters",
        ));
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
    }) {
        return Err(PlannerContractError::new(
            field,
            "must use lowercase ASCII letters, digits, '.', '_', '-', '/', or ':'",
        ));
    }
    Ok(())
}

pub(crate) fn validate_label(field: &str, value: &str) -> Result<(), PlannerContractError> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(PlannerContractError::new(
            field,
            "must be nonempty printable text of at most 256 characters",
        ));
    }
    Ok(())
}

pub(crate) fn canonical_json<T: serde::Serialize>(
    value: &T,
) -> Result<Vec<u8>, PlannerContractError> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(crate) fn require_canonical_json_bytes(
    field: &str,
    bytes: &[u8],
    canonical: &[u8],
) -> Result<(), PlannerContractError> {
    if bytes == canonical {
        return Ok(());
    }
    let detail = if bytes.contains(&b'\r') {
        "contains carriage returns; canonical JSON must use LF line endings"
    } else {
        "is not canonical JSON"
    };
    Err(PlannerContractError::new(field, detail))
}
