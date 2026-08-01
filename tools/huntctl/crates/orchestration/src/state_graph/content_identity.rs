use super::{
    ActionExpansionStatus, ExactStateId, ExpansionEvidenceAuthority, FutureEquivalenceProof,
    ObservedSegment, RestorationLocator, StateGraph, StateGraphError, StateGraphIdentity,
    TerminalPath,
};
use dusklight_automation_contracts::artifact::Digest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::io::{self, Write};

const STATE_GRAPH_CONTENT_IDENTITY_SCHEMA_V2: &str = "dusklight-state-graph-content/v2";

#[derive(Serialize)]
struct StateGraphContentIdentity<'a> {
    schema: &'static str,
    graph_schema: &'a str,
    identity: &'a StateGraphIdentity,
    root: ExactStateId,
    root_route_frames: u64,
    nodes: Vec<StateGraphNodeIdentity<'a>>,
    expansions: Vec<ActionExpansionIdentity<'a>>,
    segments: Vec<&'a ObservedSegment>,
    route_sha256: Vec<Digest>,
    future_equivalence_proofs: Vec<&'a FutureEquivalenceProof>,
    best_terminal: &'a Option<TerminalPath>,
}

#[derive(Serialize)]
struct StateGraphNodeIdentity<'a> {
    map_identity: ExactStateId,
    state_sha256: Digest,
    terminal: bool,
    root_ticks: u64,
    restoration: &'a RestorationLocator,
    incoming_segments: &'a BTreeSet<Digest>,
    outgoing_segments: &'a BTreeSet<Digest>,
    outgoing_expansions: &'a BTreeSet<Digest>,
}

#[derive(Serialize)]
struct ActionExpansionIdentity<'a> {
    map_identity: Digest,
    identity_sha256: Digest,
    source: ExactStateId,
    target: Option<ExactStateId>,
    observed_segments: &'a [Digest],
    status: ActionExpansionStatusIdentity,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case", tag = "status")]
enum ActionExpansionStatusIdentity {
    Untried,
    Leased {
        lease_sha256: Digest,
        expires_at_generation: u64,
    },
    Completed {
        authority: ExpansionEvidenceAuthority,
        route_checkpoint_sha256: Digest,
        evidence: Vec<CompletedEvidenceIdentity>,
    },
    FailedValidation {
        evidence_sha256: Digest,
    },
    Retryable {
        attempts: u32,
    },
}

#[derive(Serialize)]
struct CompletedEvidenceIdentity {
    transition_sha256: Digest,
    episode_group: u64,
    authority: ExpansionEvidenceAuthority,
}

pub(super) fn content_sha256(graph: &StateGraph) -> Result<Digest, StateGraphError> {
    let nodes = graph
        .nodes
        .iter()
        .map(|(map_identity, node)| StateGraphNodeIdentity {
            map_identity: *map_identity,
            state_sha256: node.id.state_sha256,
            terminal: node.terminal,
            root_ticks: node.root_ticks,
            restoration: &node.restoration,
            incoming_segments: &node.incoming_segments,
            outgoing_segments: &node.outgoing_segments,
            outgoing_expansions: &node.outgoing_expansions,
        })
        .collect();
    let expansions = graph
        .expansions
        .iter()
        .map(|(map_identity, expansion)| ActionExpansionIdentity {
            map_identity: *map_identity,
            identity_sha256: expansion.identity_sha256,
            source: expansion.source,
            target: expansion.target,
            observed_segments: &expansion.observed_segments,
            status: match &expansion.status {
                ActionExpansionStatus::Untried => ActionExpansionStatusIdentity::Untried,
                ActionExpansionStatus::Leased {
                    lease_sha256,
                    expires_at_generation,
                } => ActionExpansionStatusIdentity::Leased {
                    lease_sha256: *lease_sha256,
                    expires_at_generation: *expires_at_generation,
                },
                ActionExpansionStatus::Completed {
                    authority,
                    route_checkpoint_sha256,
                    evidence,
                } => ActionExpansionStatusIdentity::Completed {
                    authority: *authority,
                    route_checkpoint_sha256: *route_checkpoint_sha256,
                    evidence: evidence
                        .iter()
                        .map(|(transition_sha256, row)| CompletedEvidenceIdentity {
                            transition_sha256: *transition_sha256,
                            episode_group: row.episode_group,
                            authority: row.authority,
                        })
                        .collect(),
                },
                ActionExpansionStatus::FailedValidation { evidence_sha256 } => {
                    ActionExpansionStatusIdentity::FailedValidation {
                        evidence_sha256: *evidence_sha256,
                    }
                }
                ActionExpansionStatus::Retryable { attempts } => {
                    ActionExpansionStatusIdentity::Retryable {
                        attempts: *attempts,
                    }
                }
            },
        })
        .collect();
    let identity = StateGraphContentIdentity {
        schema: STATE_GRAPH_CONTENT_IDENTITY_SCHEMA_V2,
        graph_schema: &graph.schema,
        identity: &graph.identity,
        root: graph.root,
        root_route_frames: graph.root_route_frames,
        nodes,
        expansions,
        segments: graph.segments.values().map(AsRef::as_ref).collect(),
        route_sha256: graph.routes.keys().copied().collect(),
        future_equivalence_proofs: graph
            .future_equivalence_proofs
            .values()
            .map(AsRef::as_ref)
            .collect(),
        best_terminal: &graph.best_terminal,
    };
    hash_cbor(&identity)
}

pub(super) fn legacy_content_sha256(graph: &StateGraph) -> Result<Digest, StateGraphError> {
    hash_cbor(graph)
}

fn hash_cbor(value: &impl Serialize) -> Result<Digest, StateGraphError> {
    let mut writer = Sha256Writer::default();
    serde_cbor::to_writer(&mut writer, value)
        .map_err(|error| StateGraphError::Serialization(error.to_string()))?;
    Ok(Digest(writer.0.finalize().into()))
}

#[derive(Default)]
struct Sha256Writer(Sha256);

impl Write for Sha256Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
