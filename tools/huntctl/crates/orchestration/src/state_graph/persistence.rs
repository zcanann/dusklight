use super::{
    ActionExpansion, ExactStateId, FutureEquivalenceProof, ObservedSegment, RestorationLocator,
    StateGraph, StateGraphError, StateGraphIdentity, StateGraphNode, TerminalPath,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_learning::fact_snapshot::FactSnapshot;
use im::OrdMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Arc;

const STATE_GRAPH_PERSISTENCE_BASE_SCHEMA_V1: &str = "dusklight-state-graph-persistence-base/v1";
const STATE_GRAPH_PERSISTENCE_DELTA_SCHEMA_V1: &str = "dusklight-state-graph-persistence-delta/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StateGraphPersistenceHead {
    pub sha256: Digest,
    pub depth: u64,
}

#[derive(Clone, Debug, Default)]
pub(super) struct StateGraphPersistence {
    head: Option<StateGraphPersistenceHead>,
    dirty_nodes: BTreeSet<ExactStateId>,
    dirty_expansions: BTreeSet<Digest>,
    dirty_segments: BTreeSet<Digest>,
    dirty_routes: BTreeSet<Digest>,
    dirty_proofs: BTreeSet<Digest>,
    added_incoming_segments: BTreeSet<(ExactStateId, Digest)>,
    added_outgoing_segments: BTreeSet<(ExactStateId, Digest)>,
    added_outgoing_expansions: BTreeSet<(ExactStateId, Digest)>,
}

// Persistence bookkeeping is an operational cache, never graph truth.
impl PartialEq for StateGraphPersistence {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

#[derive(Clone, Debug)]
pub(crate) enum StateGraphPersistencePlan {
    Reuse(StateGraphPersistenceHead),
    Store {
        parent: Option<StateGraphPersistenceHead>,
        bytes: Vec<u8>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "record", rename_all = "snake_case", deny_unknown_fields)]
enum StateGraphPersistenceRecord {
    Base {
        schema: String,
        graph: StateGraph,
    },
    Delta {
        schema: String,
        parent_sha256: Digest,
        identity: StateGraphIdentity,
        nodes: Vec<PersistedStateGraphNode>,
        expansions: Vec<ActionExpansion>,
        segments: Vec<ObservedSegment>,
        routes: Vec<(Digest, InputTape)>,
        future_equivalence_proofs: Vec<FutureEquivalenceProof>,
        added_incoming_segments: Vec<(ExactStateId, Digest)>,
        added_outgoing_segments: Vec<(ExactStateId, Digest)>,
        added_outgoing_expansions: Vec<(ExactStateId, Digest)>,
        best_terminal: Option<TerminalPath>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedStateGraphNode {
    id: ExactStateId,
    state: FactSnapshot,
    terminal: bool,
    root_ticks: u64,
    restoration: RestorationLocator,
}

impl From<&StateGraphNode> for PersistedStateGraphNode {
    fn from(node: &StateGraphNode) -> Self {
        Self {
            id: node.id,
            state: node.state.as_ref().clone(),
            terminal: node.terminal,
            root_ticks: node.root_ticks,
            restoration: node.restoration.clone(),
        }
    }
}

impl StateGraphPersistenceRecord {
    fn parent_sha256(&self) -> Option<Digest> {
        match self {
            Self::Base { .. } => None,
            Self::Delta { parent_sha256, .. } => Some(*parent_sha256),
        }
    }
}

impl StateGraph {
    pub(crate) fn persistence_plan(&self) -> Result<StateGraphPersistencePlan, StateGraphError> {
        let Some(head) = self.persistence.head else {
            let record = StateGraphPersistenceRecord::Base {
                schema: STATE_GRAPH_PERSISTENCE_BASE_SCHEMA_V1.into(),
                graph: self.clone(),
            };
            return Ok(StateGraphPersistencePlan::Store {
                parent: None,
                bytes: encode_record(&record)?,
            });
        };
        if self.persistence.dirty_nodes.is_empty()
            && self.persistence.dirty_expansions.is_empty()
            && self.persistence.dirty_segments.is_empty()
            && self.persistence.dirty_routes.is_empty()
            && self.persistence.dirty_proofs.is_empty()
            && self.persistence.added_incoming_segments.is_empty()
            && self.persistence.added_outgoing_segments.is_empty()
            && self.persistence.added_outgoing_expansions.is_empty()
        {
            return Ok(StateGraphPersistencePlan::Reuse(head));
        }
        let record = StateGraphPersistenceRecord::Delta {
            schema: STATE_GRAPH_PERSISTENCE_DELTA_SCHEMA_V1.into(),
            parent_sha256: head.sha256,
            identity: self.identity.clone(),
            nodes: self
                .persistence
                .dirty_nodes
                .iter()
                .map(|identity| {
                    self.nodes
                        .get(identity)
                        .map(|node| PersistedStateGraphNode::from(node.as_ref()))
                        .ok_or(StateGraphError::Invariant("dirty graph node is absent"))
                })
                .collect::<Result<Vec<_>, _>>()?,
            expansions: collect_dirty(&self.expansions, &self.persistence.dirty_expansions)?,
            segments: collect_dirty(&self.segments, &self.persistence.dirty_segments)?,
            routes: self
                .persistence
                .dirty_routes
                .iter()
                .map(|identity| {
                    self.routes
                        .get(identity)
                        .map(|route| route.as_ref().clone())
                        .map(|route| (*identity, route))
                        .ok_or(StateGraphError::Invariant("dirty graph route is absent"))
                })
                .collect::<Result<Vec<_>, _>>()?,
            future_equivalence_proofs: collect_dirty(
                &self.future_equivalence_proofs,
                &self.persistence.dirty_proofs,
            )?,
            added_incoming_segments: self
                .persistence
                .added_incoming_segments
                .iter()
                .copied()
                .collect(),
            added_outgoing_segments: self
                .persistence
                .added_outgoing_segments
                .iter()
                .copied()
                .collect(),
            added_outgoing_expansions: self
                .persistence
                .added_outgoing_expansions
                .iter()
                .copied()
                .collect(),
            best_terminal: self.best_terminal.clone(),
        };
        Ok(StateGraphPersistencePlan::Store {
            parent: Some(head),
            bytes: encode_record(&record)?,
        })
    }

    pub(crate) fn persistence_record_parent(
        bytes: &[u8],
    ) -> Result<Option<Digest>, StateGraphError> {
        Ok(decode_record(bytes)?.parent_sha256())
    }

    pub(crate) fn from_persistence_records(
        records: &[(StateGraphPersistenceHead, Vec<u8>)],
    ) -> Result<Self, StateGraphError> {
        let (base_head, base_bytes) = records
            .first()
            .ok_or(StateGraphError::Invalid("graph persistence chain is empty"))?;
        if base_head.depth != 0 {
            return Err(StateGraphError::Invalid(
                "graph persistence base depth is invalid",
            ));
        }
        let StateGraphPersistenceRecord::Base { schema, mut graph } = decode_record(base_bytes)?
        else {
            return Err(StateGraphError::Invalid(
                "graph persistence chain has no base",
            ));
        };
        if schema != STATE_GRAPH_PERSISTENCE_BASE_SCHEMA_V1 {
            return Err(StateGraphError::Invalid(
                "graph persistence base schema is invalid",
            ));
        }
        for (expected_depth, (head, bytes)) in records.iter().enumerate().skip(1) {
            if head.depth != expected_depth as u64 {
                return Err(StateGraphError::Invalid(
                    "graph persistence delta depth is invalid",
                ));
            }
            let StateGraphPersistenceRecord::Delta {
                schema,
                parent_sha256,
                identity,
                nodes,
                expansions,
                segments,
                routes,
                future_equivalence_proofs,
                added_incoming_segments,
                added_outgoing_segments,
                added_outgoing_expansions,
                best_terminal,
            } = decode_record(bytes)?
            else {
                return Err(StateGraphError::Invalid(
                    "graph persistence chain contains a late base",
                ));
            };
            if schema != STATE_GRAPH_PERSISTENCE_DELTA_SCHEMA_V1
                || identity != graph.identity
                || parent_sha256 != records[expected_depth - 1].0.sha256
            {
                return Err(StateGraphError::Invalid(
                    "graph persistence delta is detached",
                ));
            }
            for node in nodes {
                match graph.nodes.get_mut(&node.id) {
                    Some(existing) => {
                        let existing = Arc::make_mut(existing);
                        existing.state = Arc::new(node.state);
                        existing.terminal = node.terminal;
                        existing.root_ticks = node.root_ticks;
                        existing.restoration = node.restoration;
                    }
                    None => {
                        graph.nodes.insert(
                            node.id,
                            Arc::new(StateGraphNode {
                                id: node.id,
                                state: Arc::new(node.state),
                                terminal: node.terminal,
                                root_ticks: node.root_ticks,
                                restoration: node.restoration,
                                incoming_segments: BTreeSet::new(),
                                outgoing_segments: BTreeSet::new(),
                                outgoing_expansions: BTreeSet::new(),
                            }),
                        );
                    }
                }
            }
            upsert(&mut graph.expansions, expansions, |expansion| {
                expansion.identity_sha256
            })?;
            upsert(&mut graph.segments, segments, |segment| {
                segment.identity_sha256
            })?;
            for (identity, route) in routes {
                graph.routes.insert(identity, Arc::new(route));
            }
            upsert(
                &mut graph.future_equivalence_proofs,
                future_equivalence_proofs,
                |proof| proof.proof_sha256,
            )?;
            apply_node_set_additions(&mut graph.nodes, added_incoming_segments, |node| {
                &mut node.incoming_segments
            })?;
            apply_node_set_additions(&mut graph.nodes, added_outgoing_segments, |node| {
                &mut node.outgoing_segments
            })?;
            apply_node_set_additions(&mut graph.nodes, added_outgoing_expansions, |node| {
                &mut node.outgoing_expansions
            })?;
            graph.best_terminal = best_terminal;
        }
        let head = records
            .last()
            .map(|(head, _)| *head)
            .ok_or(StateGraphError::Invalid("graph persistence chain is empty"))?;
        graph.validate()?;
        graph.install_persistence_head(head);
        Ok(graph)
    }

    pub(crate) fn install_persistence_head(&mut self, head: StateGraphPersistenceHead) {
        self.persistence.head = Some(head);
        self.persistence.dirty_nodes.clear();
        self.persistence.dirty_expansions.clear();
        self.persistence.dirty_segments.clear();
        self.persistence.dirty_routes.clear();
        self.persistence.dirty_proofs.clear();
        self.persistence.added_incoming_segments.clear();
        self.persistence.added_outgoing_segments.clear();
        self.persistence.added_outgoing_expansions.clear();
    }

    pub(super) fn mark_node_persistence_dirty(&mut self, identity: ExactStateId) {
        self.persistence.dirty_nodes.insert(identity);
    }

    pub(super) fn mark_expansion_persistence_dirty(&mut self, identity: Digest) {
        self.persistence.dirty_expansions.insert(identity);
    }

    pub(super) fn mark_segment_persistence_dirty(&mut self, identity: Digest) {
        self.persistence.dirty_segments.insert(identity);
    }

    pub(super) fn mark_route_persistence_dirty(&mut self, identity: Digest) {
        self.persistence.dirty_routes.insert(identity);
    }

    pub(super) fn mark_proof_persistence_dirty(&mut self, identity: Digest) {
        self.persistence.dirty_proofs.insert(identity);
    }

    pub(super) fn mark_incoming_segment_persistence_added(
        &mut self,
        node: ExactStateId,
        segment: Digest,
    ) {
        self.persistence
            .added_incoming_segments
            .insert((node, segment));
    }

    pub(super) fn mark_outgoing_segment_persistence_added(
        &mut self,
        node: ExactStateId,
        segment: Digest,
    ) {
        self.persistence
            .added_outgoing_segments
            .insert((node, segment));
    }

    pub(super) fn mark_outgoing_expansion_persistence_added(
        &mut self,
        node: ExactStateId,
        expansion: Digest,
    ) {
        self.persistence
            .added_outgoing_expansions
            .insert((node, expansion));
    }
}

fn encode_record(record: &StateGraphPersistenceRecord) -> Result<Vec<u8>, StateGraphError> {
    serde_cbor::to_vec(record).map_err(|error| StateGraphError::Serialization(error.to_string()))
}

fn decode_record(bytes: &[u8]) -> Result<StateGraphPersistenceRecord, StateGraphError> {
    let mut deserializer = serde_cbor::Deserializer::from_slice(bytes);
    let record = StateGraphPersistenceRecord::deserialize(&mut deserializer)
        .map_err(|error| StateGraphError::Serialization(error.to_string()))?;
    deserializer
        .end()
        .map_err(|error| StateGraphError::Serialization(error.to_string()))?;
    Ok(record)
}

fn collect_dirty<K, V>(
    values: &OrdMap<K, Arc<V>>,
    dirty: &BTreeSet<K>,
) -> Result<Vec<V>, StateGraphError>
where
    K: Clone + Ord,
    V: Clone,
{
    dirty
        .iter()
        .map(|identity| {
            values
                .get(identity)
                .map(|value| value.as_ref().clone())
                .ok_or(StateGraphError::Invariant("dirty graph object is absent"))
        })
        .collect()
}

fn upsert<K, V, F>(
    destination: &mut OrdMap<K, Arc<V>>,
    values: Vec<V>,
    identity: F,
) -> Result<(), StateGraphError>
where
    K: Clone + Ord,
    V: Clone,
    F: Fn(&V) -> K,
{
    for value in values {
        destination.insert(identity(&value), Arc::new(value));
    }
    Ok(())
}

fn apply_node_set_additions<F>(
    nodes: &mut OrdMap<ExactStateId, Arc<StateGraphNode>>,
    additions: Vec<(ExactStateId, Digest)>,
    field: F,
) -> Result<(), StateGraphError>
where
    F: Fn(&mut StateGraphNode) -> &mut BTreeSet<Digest>,
{
    for (node, value) in additions {
        field(Arc::make_mut(nodes.get_mut(&node).ok_or(
            StateGraphError::Invariant("graph journal edge names an absent node"),
        )?))
        .insert(value);
    }
    Ok(())
}
