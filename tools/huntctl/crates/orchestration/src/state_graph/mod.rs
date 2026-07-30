//! Authoritative, content-addressed search state.
//!
//! The graph owns exact native states and completed action expansions.
//! Learner corpora, frontiers, reports, and process-local restore handles are
//! projections or caches of this data, never parallel sources of truth.

mod admission;
mod lifecycle;
mod persistence;
mod restoration;
mod returns;
mod transpositions;
mod types;
mod validation;

pub(crate) use persistence::{StateGraphPersistenceHead, StateGraphPersistencePlan};
pub use types::{
    ActionExpansion, ActionExpansionStatus, CompletedExpansionEvidence, ExactStateId,
    ExpansionAdmission, ExpansionEvidenceAuthority, FUTURE_EQUIVALENCE_PROOF_SCHEMA_V1,
    FutureEquivalenceProof, GRAPH_RESTORATION_PLAN_SCHEMA_V1, GraphRestorationPlan,
    NativeBoundaryLocator, ObservedSegment, RestorationLocator, RestoredStateReceipt, RouteRecord,
    STATE_GRAPH_SCHEMA_V1, StateGraphIdentity, StateGraphNode, TerminalPath,
};

use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_control::option_execution::OptionExecutionError;
use dusklight_learning::fact_snapshot::{FactSnapshot, FactSnapshotError};
use dusklight_learning::option_transition::{OptionTransitionError, OptionTransitionSample};
use dusklight_learning::option_values::{OptionActionDescriptor, OptionValueError};
use im::OrdMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::sync::Arc;

const ROUTE_CHECKPOINT_SCHEMA_V1: &[u8] = b"dusklight-route-checkpoint/v1";
const EXECUTABLE_EXPANSION_SET_SCHEMA_V1: &[u8] = b"dusklight-executable-expansion-set/v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateGraph {
    pub schema: String,
    pub identity: StateGraphIdentity,
    root: ExactStateId,
    root_route_frames: u64,
    nodes: OrdMap<ExactStateId, Arc<StateGraphNode>>,
    expansions: OrdMap<Digest, Arc<ActionExpansion>>,
    segments: OrdMap<Digest, Arc<ObservedSegment>>,
    routes: OrdMap<Digest, Arc<InputTape>>,
    #[serde(default)]
    future_equivalence_proofs: OrdMap<Digest, Arc<FutureEquivalenceProof>>,
    best_terminal: Option<TerminalPath>,
    #[serde(skip)]
    persistence: persistence::StateGraphPersistence,
}

impl StateGraph {
    pub fn new(
        identity: StateGraphIdentity,
        root_state: FactSnapshot,
        root_route: InputTape,
    ) -> Result<Self, StateGraphError> {
        identity.validate().map_err(StateGraphError::Invalid)?;
        root_state.validate()?;
        root_route.validate()?;
        if root_state.tape_frame != root_route.frames.len() as u64 {
            return Err(StateGraphError::Invalid(
                "root facts and route name different boundaries",
            ));
        }
        let root_state_sha256 = root_state.content_sha256()?;
        let root_route_checkpoint =
            route_checkpoint_sha256(identity.root_checkpoint_sha256, &root_route)?;
        let root = ExactStateId {
            route_checkpoint_sha256: root_route_checkpoint,
            state_sha256: root_state_sha256,
        };
        let root_route_record = RouteRecord {
            route_checkpoint_sha256: root_route_checkpoint,
            tape_sha256: tape_sha256(&root_route)?,
            tape_frames: root_route.frames.len() as u64,
        };
        let terminal = root_state.terminal.reached == Some(true);
        let node = StateGraphNode {
            id: root,
            state: Arc::new(root_state),
            terminal,
            root_ticks: 0,
            restoration: RestorationLocator {
                route: root_route_record,
                native_boundary: None,
                executable: true,
            },
            incoming_segments: Default::default(),
            outgoing_segments: Default::default(),
            outgoing_expansions: Default::default(),
        };
        let best_terminal = terminal.then_some(TerminalPath {
            terminal: root,
            route_checkpoint_sha256: root_route_checkpoint,
            root_to_terminal_ticks: 0,
        });
        Ok(Self {
            schema: STATE_GRAPH_SCHEMA_V1.into(),
            identity,
            root,
            root_route_frames: root_route.frames.len() as u64,
            nodes: OrdMap::unit(root, Arc::new(node)),
            expansions: OrdMap::new(),
            segments: OrdMap::new(),
            routes: OrdMap::unit(root_route_checkpoint, Arc::new(root_route)),
            future_equivalence_proofs: OrdMap::new(),
            best_terminal,
            persistence: Default::default(),
        })
    }

    pub fn root(&self) -> ExactStateId {
        self.root
    }

    pub fn node(&self, id: ExactStateId) -> Option<&StateGraphNode> {
        self.nodes.get(&id).map(Arc::as_ref)
    }

    pub fn expansion(&self, identity_sha256: Digest) -> Option<&ActionExpansion> {
        self.expansions.get(&identity_sha256).map(Arc::as_ref)
    }

    pub fn segment(&self, identity_sha256: Digest) -> Option<&ObservedSegment> {
        self.segments.get(&identity_sha256).map(Arc::as_ref)
    }

    pub fn route(&self, route_checkpoint_sha256: Digest) -> Option<&InputTape> {
        self.routes.get(&route_checkpoint_sha256).map(Arc::as_ref)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn nodes(&self) -> impl Iterator<Item = &StateGraphNode> {
        self.nodes.values().map(Arc::as_ref)
    }

    pub fn expansion_count(&self) -> usize {
        self.expansions.len()
    }

    /// Unique graph-owned action expansions backed by executable native
    /// evidence. Registered, leased, retryable, validation-failed, and
    /// learner-only entries are deliberately excluded from throughput.
    pub fn completed_executable_expansion_count(&self) -> usize {
        self.expansions
            .values()
            .filter(|expansion| {
                matches!(
                    expansion.status,
                    ActionExpansionStatus::Completed {
                        authority: ExpansionEvidenceAuthority::Executable,
                        ..
                    }
                )
            })
            .count()
    }

    /// Stable semantic identity of the executable work set. This deliberately
    /// excludes lease and admission order while binding every completed
    /// action, target, route, and transition-evidence identity.
    pub fn completed_executable_expansion_set_sha256(&self) -> Digest {
        let completed = self.expansions.values().filter_map(|expansion| {
            let ActionExpansionStatus::Completed {
                authority: ExpansionEvidenceAuthority::Executable,
                route_checkpoint_sha256,
                evidence,
            } = &expansion.status
            else {
                return None;
            };
            Some((expansion, route_checkpoint_sha256, evidence))
        });
        let mut hasher = Sha256::new();
        hasher.update(EXECUTABLE_EXPANSION_SET_SCHEMA_V1);
        hasher.update((self.completed_executable_expansion_count() as u64).to_le_bytes());
        for (expansion, route_checkpoint_sha256, evidence) in completed {
            hasher.update(expansion.identity_sha256.0);
            let target = expansion.target.unwrap_or(ExactStateId {
                route_checkpoint_sha256: Digest::ZERO,
                state_sha256: Digest::ZERO,
            });
            hasher.update(target.route_checkpoint_sha256.0);
            hasher.update(target.state_sha256.0);
            hasher.update(route_checkpoint_sha256.0);
            hasher.update((evidence.len() as u64).to_le_bytes());
            for evidence_sha256 in evidence.keys() {
                hasher.update(evidence_sha256.0);
            }
        }
        Digest(hasher.finalize().into())
    }

    pub fn expansions(&self) -> impl Iterator<Item = &ActionExpansion> {
        self.expansions.values().map(Arc::as_ref)
    }

    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    pub fn future_equivalence_proof_count(&self) -> usize {
        self.future_equivalence_proofs.len()
    }

    pub fn best_terminal_path(&self) -> Option<&TerminalPath> {
        self.best_terminal.as_ref()
    }

    pub fn encode(&self) -> Result<Vec<u8>, StateGraphError> {
        self.validate()?;
        serde_cbor::to_vec(self).map_err(|error| StateGraphError::Serialization(error.to_string()))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, StateGraphError> {
        let graph: Self = serde_cbor::from_slice(bytes)
            .map_err(|error| StateGraphError::Serialization(error.to_string()))?;
        graph.validate()?;
        Ok(graph)
    }

    pub fn content_sha256(&self) -> Result<Digest, StateGraphError> {
        Ok(Digest(Sha256::digest(self.encode()?).into()))
    }

    /// The learner view is derived from completed graph expansions in stable
    /// content-identity order.
    pub fn completed_transitions(&self) -> impl Iterator<Item = (&OptionTransitionSample, u64)> {
        self.expansions.values().flat_map(|expansion| {
            let evidence = match &expansion.status {
                ActionExpansionStatus::Completed { evidence, .. } => Some(evidence),
                _ => None,
            };
            evidence.into_iter().flat_map(|rows| {
                rows.values()
                    .map(|row| (row.transition.as_ref(), row.episode_group))
            })
        })
    }

    pub fn completed_evidence(
        &self,
    ) -> impl Iterator<Item = (&OptionTransitionSample, &InputTape, u64)> {
        self.expansions.values().flat_map(|expansion| {
            let (route, evidence) = match &expansion.status {
                ActionExpansionStatus::Completed {
                    route_checkpoint_sha256,
                    evidence,
                    ..
                } => (
                    self.routes.get(route_checkpoint_sha256).map(Arc::as_ref),
                    Some(evidence),
                ),
                _ => (None, None),
            };
            route.into_iter().flat_map(move |route| {
                evidence.into_iter().flat_map(move |rows| {
                    rows.values()
                        .map(move |row| (row.transition.as_ref(), route, row.episode_group))
                })
            })
        })
    }

    fn refresh_best_terminal(&mut self) {
        self.best_terminal = self
            .nodes
            .values()
            .filter(|node| node.terminal && node.restoration.executable)
            .min_by_key(|node| (node.root_ticks, node.id))
            .map(|node| TerminalPath {
                terminal: node.id,
                route_checkpoint_sha256: node.id.route_checkpoint_sha256,
                root_to_terminal_ticks: node.root_ticks,
            });
    }
}

pub(crate) fn route_checkpoint_sha256(
    root_checkpoint_sha256: Digest,
    route: &InputTape,
) -> Result<Digest, StateGraphError> {
    if root_checkpoint_sha256 == Digest::ZERO {
        return Err(StateGraphError::Invalid("root checkpoint is missing"));
    }
    let bytes = route.encode()?;
    let mut hasher = Sha256::new();
    hasher.update(ROUTE_CHECKPOINT_SCHEMA_V1);
    hasher.update(root_checkpoint_sha256.0);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

pub(crate) fn route_and_tape_sha256(
    root_checkpoint_sha256: Digest,
    route: &InputTape,
) -> Result<(Digest, Digest), StateGraphError> {
    if root_checkpoint_sha256 == Digest::ZERO {
        return Err(StateGraphError::Invalid("root checkpoint is missing"));
    }
    let bytes = route.encode()?;
    let tape_sha256 = Digest(Sha256::digest(&bytes).into());
    let mut hasher = Sha256::new();
    hasher.update(ROUTE_CHECKPOINT_SCHEMA_V1);
    hasher.update(root_checkpoint_sha256.0);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok((Digest(hasher.finalize().into()), tape_sha256))
}

pub fn action_expansion_identity(
    source: ExactStateId,
    action: &OptionActionDescriptor,
) -> Result<Digest, StateGraphError> {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-action-expansion/v1");
    hasher.update(source.route_checkpoint_sha256.0);
    hasher.update(source.state_sha256.0);
    hasher.update(action.content_sha256()?.0);
    Ok(Digest(hasher.finalize().into()))
}

pub(crate) fn tape_sha256(route: &InputTape) -> Result<Digest, StateGraphError> {
    let bytes = route.encode()?;
    Ok(Digest(Sha256::digest(bytes).into()))
}

pub(crate) fn tape_prefix(
    route: &InputTape,
    frame_count: usize,
) -> Result<InputTape, StateGraphError> {
    if frame_count > route.frames.len() {
        return Err(StateGraphError::Invalid("route prefix exceeds the tape"));
    }
    let mut prefix = route.clone();
    prefix.frames.truncate(frame_count);
    Ok(prefix)
}

pub(crate) fn same_intermediate_boundary_realization(
    left: &[dusklight_learning::option_transition::OptionIntermediateBoundary],
    right: &[dusklight_learning::option_transition::OptionIntermediateBoundary],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.offset_ticks == right.offset_ticks
                && left.state_sha256 == right.state_sha256
                && left.state == right.state
        })
}

pub(crate) fn same_native_realization(
    left: &OptionTransitionSample,
    right: &OptionTransitionSample,
) -> bool {
    left.execution_authority_sha256 == right.execution_authority_sha256
        && left.before_state_sha256 == right.before_state_sha256
        && left.after_state_sha256 == right.after_state_sha256
        && left.source_checkpoint_sha256 == right.source_checkpoint_sha256
        && left.next_checkpoint_sha256 == right.next_checkpoint_sha256
        && left.before == right.before
        && left.after == right.after
        && left.execution == right.execution
        && left.value_sample.terminal == right.value_sample.terminal
        && same_intermediate_boundary_realization(
            &left.intermediate_boundaries,
            &right.intermediate_boundaries,
        )
}

#[derive(Debug)]
pub enum StateGraphError {
    Invalid(&'static str),
    Invariant(&'static str),
    ConflictingNativeEvidence {
        expansion_sha256: Digest,
        canonical_evidence_sha256: Digest,
        conflicting_evidence_sha256: Digest,
        differing_fields: String,
    },
    DigestCollision(&'static str),
    Tape(String),
    Facts(String),
    Execution(String),
    Transition(OptionTransitionError),
    Action(String),
    Serialization(String),
}

impl fmt::Display for StateGraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid state graph: {message}"),
            Self::Invariant(message) => {
                write!(formatter, "state graph invariant failed: {message}")
            }
            Self::ConflictingNativeEvidence {
                expansion_sha256,
                canonical_evidence_sha256,
                conflicting_evidence_sha256,
                differing_fields,
            } => write!(
                formatter,
                "state graph invariant failed: deterministic expansion {expansion_sha256} \
                 has conflicting native evidence {canonical_evidence_sha256} and \
                 {conflicting_evidence_sha256}; differing fields: {differing_fields}"
            ),
            Self::DigestCollision(message) => {
                write!(
                    formatter,
                    "state graph content identity collision: {message}"
                )
            }
            Self::Tape(message) => write!(formatter, "state graph tape failed: {message}"),
            Self::Facts(message) => write!(formatter, "state graph facts failed: {message}"),
            Self::Execution(message) => {
                write!(formatter, "state graph execution failed: {message}")
            }
            Self::Transition(error) => write!(formatter, "state graph transition failed: {error}"),
            Self::Action(message) => write!(formatter, "state graph action failed: {message}"),
            Self::Serialization(message) => {
                write!(formatter, "state graph serialization failed: {message}")
            }
        }
    }
}

impl Error for StateGraphError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Transition(error) => Some(error),
            _ => None,
        }
    }
}

impl From<dusklight_automation_contracts::tape::TapeError> for StateGraphError {
    fn from(value: dusklight_automation_contracts::tape::TapeError) -> Self {
        Self::Tape(value.to_string())
    }
}

impl From<FactSnapshotError> for StateGraphError {
    fn from(value: FactSnapshotError) -> Self {
        Self::Facts(value.to_string())
    }
}

impl From<OptionExecutionError> for StateGraphError {
    fn from(value: OptionExecutionError) -> Self {
        Self::Execution(value.to_string())
    }
}

impl From<OptionTransitionError> for StateGraphError {
    fn from(value: OptionTransitionError) -> Self {
        Self::Transition(value)
    }
}

impl From<OptionValueError> for StateGraphError {
    fn from(value: OptionValueError) -> Self {
        Self::Action(value.to_string())
    }
}

#[cfg(test)]
mod tests;
