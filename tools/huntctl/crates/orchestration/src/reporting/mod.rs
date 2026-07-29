//! Read-only exported projections of authoritative graph truth.

use crate::state_graph::{
    ActionExpansionStatus, StateGraph, StateGraphError, StateGraphIdentity, TerminalPath,
};
use dusklight_automation_contracts::artifact::Digest;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

pub const GRAPH_SEARCH_REPORT_SCHEMA_V1: &str = "dusklight-graph-search-report/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSearchReport {
    pub schema: String,
    pub graph_sha256: Digest,
    pub graph_identity: StateGraphIdentity,
    pub nodes: u64,
    pub observed_segments: u64,
    pub future_equivalence_proofs: u64,
    pub untried_expansions: u64,
    pub leased_expansions: u64,
    pub retryable_expansions: u64,
    pub completed_expansions: u64,
    pub failed_validation_expansions: u64,
    pub best_terminal: Option<TerminalPath>,
}

impl GraphSearchReport {
    pub fn from_graph(graph: &StateGraph) -> Result<Self, ReportingError> {
        graph.validate()?;
        let mut untried_expansions = 0_u64;
        let mut leased_expansions = 0_u64;
        let mut retryable_expansions = 0_u64;
        let mut completed_expansions = 0_u64;
        let mut failed_validation_expansions = 0_u64;
        for expansion in graph.expansions() {
            match &expansion.status {
                ActionExpansionStatus::Untried => untried_expansions += 1,
                ActionExpansionStatus::Leased { .. } => leased_expansions += 1,
                ActionExpansionStatus::Retryable { .. } => retryable_expansions += 1,
                ActionExpansionStatus::Completed { .. } => completed_expansions += 1,
                ActionExpansionStatus::FailedValidation { .. } => {
                    failed_validation_expansions += 1;
                }
            }
        }
        let report = Self {
            schema: GRAPH_SEARCH_REPORT_SCHEMA_V1.into(),
            graph_sha256: graph.content_sha256()?,
            graph_identity: graph.identity.clone(),
            nodes: graph.node_count() as u64,
            observed_segments: graph.segment_count() as u64,
            future_equivalence_proofs: graph.future_equivalence_proof_count() as u64,
            untried_expansions,
            leased_expansions,
            retryable_expansions,
            completed_expansions,
            failed_validation_expansions,
            best_terminal: graph.best_terminal_path().cloned(),
        };
        report.validate_against(graph)?;
        Ok(report)
    }

    pub fn validate_against(&self, graph: &StateGraph) -> Result<(), ReportingError> {
        if self.schema != GRAPH_SEARCH_REPORT_SCHEMA_V1
            || self.graph_sha256 != graph.content_sha256()?
            || self.graph_identity != graph.identity
        {
            return Err(ReportingError::Invalid(
                "graph search report identity is detached",
            ));
        }
        let expected = Self::from_graph_unchecked(graph)?;
        if *self != expected {
            return Err(ReportingError::Invalid(
                "graph search report metrics are detached",
            ));
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, ReportingError> {
        serde_json::to_vec_pretty(self)
            .map_err(|error| ReportingError::Serialization(error.to_string()))
    }

    fn from_graph_unchecked(graph: &StateGraph) -> Result<Self, ReportingError> {
        let mut report = Self {
            schema: GRAPH_SEARCH_REPORT_SCHEMA_V1.into(),
            graph_sha256: graph.content_sha256()?,
            graph_identity: graph.identity.clone(),
            nodes: graph.node_count() as u64,
            observed_segments: graph.segment_count() as u64,
            future_equivalence_proofs: graph.future_equivalence_proof_count() as u64,
            untried_expansions: 0,
            leased_expansions: 0,
            retryable_expansions: 0,
            completed_expansions: 0,
            failed_validation_expansions: 0,
            best_terminal: graph.best_terminal_path().cloned(),
        };
        for expansion in graph.expansions() {
            match &expansion.status {
                ActionExpansionStatus::Untried => report.untried_expansions += 1,
                ActionExpansionStatus::Leased { .. } => report.leased_expansions += 1,
                ActionExpansionStatus::Retryable { .. } => report.retryable_expansions += 1,
                ActionExpansionStatus::Completed { .. } => report.completed_expansions += 1,
                ActionExpansionStatus::FailedValidation { .. } => {
                    report.failed_validation_expansions += 1;
                }
            }
        }
        Ok(report)
    }
}

#[derive(Debug)]
pub enum ReportingError {
    Invalid(&'static str),
    Serialization(String),
    Graph(StateGraphError),
}

impl fmt::Display for ReportingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid graph search report: {message}"),
            Self::Serialization(message) => {
                write!(
                    formatter,
                    "graph search report serialization failed: {message}"
                )
            }
            Self::Graph(error) => write!(formatter, "graph search report failed: {error}"),
        }
    }
}

impl Error for ReportingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Graph(error) => Some(error),
            Self::Invalid(_) | Self::Serialization(_) => None,
        }
    }
}

impl From<StateGraphError> for ReportingError {
    fn from(value: StateGraphError) -> Self {
        Self::Graph(value)
    }
}
