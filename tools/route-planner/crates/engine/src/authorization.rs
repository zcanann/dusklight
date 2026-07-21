//! Bounded upper-bound authorization graphs over evaluated planner states.

use crate::PlannerContractError;
use crate::artifact::Digest;
use crate::evaluation::EvidencePolicy;
use crate::logic::{FactCatalog, TruthStatus};
use crate::solver::{SearchActionKind, SearchResult, SearchStep, SolverOptions};
use crate::transition::MechanicsCatalog;
use crate::{canonical_json, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const AUTHORIZATION_GRAPH_SCHEMA: &str = "dusklight.route-planner.authorization-graph/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationGraphBounds {
    pub max_depth: usize,
    pub max_states: usize,
    pub max_resolution_combinations: usize,
}

impl From<SolverOptions> for AuthorizationGraphBounds {
    fn from(options: SolverOptions) -> Self {
        Self {
            max_depth: options.max_depth,
            max_states: options.max_states,
            max_resolution_combinations: options.max_resolution_combinations,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationStateNode {
    /// Search-dominance identity used to join graph edges.
    pub state_sha256: Digest,
    /// Full state identity retaining provenance and history for audit.
    pub execution_state_sha256: Digest,
    pub snapshot_sha256: Digest,
    pub minimum_depth: usize,
    /// False identifies a bounded frontier node that was reached but not evaluated.
    pub evaluated: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationEdge {
    pub source_state_sha256: Digest,
    pub result_state_sha256: Digest,
    pub action_kind: SearchActionKind,
    pub action_id: String,
    pub selected_resolver_ids: Vec<String>,
    pub selected_technique_ids: Vec<String>,
    pub weakest_evidence: Option<TruthStatus>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationGraph {
    pub schema: String,
    pub initial_state_sha256: Digest,
    pub initial_execution_state_sha256: Digest,
    pub fact_catalog_sha256: Digest,
    pub mechanics_catalog_sha256: Digest,
    pub refinement_stack_sha256: Option<Digest>,
    pub equivalence_set_sha256: Vec<Digest>,
    pub evidence_policy: EvidencePolicy,
    pub bounds: AuthorizationGraphBounds,
    pub nodes: Vec<AuthorizationStateNode>,
    pub edges: Vec<AuthorizationEdge>,
    pub evaluated_states: usize,
    pub traversal_complete: bool,
    pub unknown_transition_ids: Vec<String>,
    pub unknown_writer_ids: Vec<String>,
    pub execution_error_ids: Vec<String>,
}

impl AuthorizationGraph {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finish(
        recorder: AuthorizationRecorder,
        initial_state_sha256: Digest,
        initial_execution_state_sha256: Digest,
        facts: &FactCatalog,
        mechanics: &MechanicsCatalog,
        equivalence_set_sha256: Vec<Digest>,
        evidence_policy: EvidencePolicy,
        options: SolverOptions,
        search: &SearchResult,
    ) -> Result<Self, PlannerContractError> {
        let mut nodes = recorder.nodes.into_values().collect::<Vec<_>>();
        nodes.sort_by_key(|node| node.state_sha256);
        let mut edges = recorder
            .edges
            .into_iter()
            .map(AuthorizationEdge::from)
            .collect::<Vec<_>>();
        edges.sort_by(|left, right| {
            authorization_edge_key(left).cmp(&authorization_edge_key(right))
        });
        let graph = Self {
            schema: AUTHORIZATION_GRAPH_SCHEMA.into(),
            initial_state_sha256,
            initial_execution_state_sha256,
            fact_catalog_sha256: facts.digest()?,
            mechanics_catalog_sha256: mechanics.digest()?,
            refinement_stack_sha256: None,
            equivalence_set_sha256,
            evidence_policy,
            bounds: options.into(),
            evaluated_states: nodes.iter().filter(|node| node.evaluated).count(),
            traversal_complete: !search.hit_search_limit,
            nodes,
            edges,
            unknown_transition_ids: search.unknown_transition_ids.clone(),
            unknown_writer_ids: search.unknown_writer_ids.clone(),
            execution_error_ids: search.execution_error_ids.clone(),
        };
        graph.validate()?;
        Ok(graph)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != AUTHORIZATION_GRAPH_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        for (field, digest) in [
            ("initial_state_sha256", self.initial_state_sha256),
            (
                "initial_execution_state_sha256",
                self.initial_execution_state_sha256,
            ),
            ("fact_catalog_sha256", self.fact_catalog_sha256),
            ("mechanics_catalog_sha256", self.mechanics_catalog_sha256),
        ] {
            require_digest(field, digest)?;
        }
        if self.refinement_stack_sha256 == Some(Digest::ZERO) {
            return Err(PlannerContractError::new(
                "refinement_stack_sha256",
                "must be absent or nonzero",
            ));
        }
        if self.bounds.max_depth == 0
            || self.bounds.max_states == 0
            || self.bounds.max_resolution_combinations == 0
        {
            return Err(PlannerContractError::new(
                "bounds",
                "must contain only nonzero limits",
            ));
        }
        let mut previous_equivalence = None;
        for digest in &self.equivalence_set_sha256 {
            require_digest("equivalence_set_sha256", *digest)?;
            if previous_equivalence.is_some_and(|previous: Digest| previous >= *digest) {
                return Err(PlannerContractError::new(
                    "equivalence_set_sha256",
                    "must be unique and sorted",
                ));
            }
            previous_equivalence = Some(*digest);
        }
        if self.nodes.is_empty() {
            return Err(PlannerContractError::new("nodes", "must not be empty"));
        }
        let mut node_ids = BTreeSet::new();
        let mut previous_node = None;
        for node in &self.nodes {
            for (field, digest) in [
                ("nodes.state_sha256", node.state_sha256),
                ("nodes.execution_state_sha256", node.execution_state_sha256),
                ("nodes.snapshot_sha256", node.snapshot_sha256),
            ] {
                require_digest(field, digest)?;
            }
            if node.minimum_depth > self.bounds.max_depth
                || previous_node.is_some_and(|previous: Digest| previous >= node.state_sha256)
            {
                return Err(PlannerContractError::new(
                    "nodes",
                    "must be uniquely sorted and within the depth bound",
                ));
            }
            node_ids.insert(node.state_sha256);
            previous_node = Some(node.state_sha256);
        }
        let initial = self
            .nodes
            .iter()
            .find(|node| node.state_sha256 == self.initial_state_sha256)
            .ok_or_else(|| {
                PlannerContractError::new("initial_state_sha256", "is absent from nodes")
            })?;
        if initial.execution_state_sha256 != self.initial_execution_state_sha256
            || initial.minimum_depth != 0
            || !initial.evaluated
        {
            return Err(PlannerContractError::new(
                "initial_state_sha256",
                "does not identify the evaluated depth-zero execution state",
            ));
        }
        if self.evaluated_states != self.nodes.iter().filter(|node| node.evaluated).count()
            || self.evaluated_states > self.bounds.max_states
        {
            return Err(PlannerContractError::new(
                "evaluated_states",
                "does not match the bounded evaluated node count",
            ));
        }
        let mut previous_edge = None;
        for edge in &self.edges {
            validate_stable_id("edges.action_id", &edge.action_id)?;
            let source = self
                .nodes
                .binary_search_by_key(&edge.source_state_sha256, |node| node.state_sha256)
                .ok()
                .map(|index| &self.nodes[index]);
            let result = self
                .nodes
                .binary_search_by_key(&edge.result_state_sha256, |node| node.state_sha256)
                .ok()
                .map(|index| &self.nodes[index]);
            if source.is_none() || result.is_none() {
                return Err(PlannerContractError::new(
                    "edges",
                    "references a state absent from nodes",
                ));
            }
            let (source, result) = (source.unwrap(), result.unwrap());
            if !source.evaluated || result.minimum_depth > source.minimum_depth.saturating_add(1) {
                return Err(PlannerContractError::new(
                    "edges",
                    "must originate at an evaluated node and respect breadth-first depth",
                ));
            }
            validate_sorted_ids("edges.selected_resolver_ids", &edge.selected_resolver_ids)?;
            validate_sorted_ids("edges.selected_technique_ids", &edge.selected_technique_ids)?;
            let key = authorization_edge_key(edge);
            if previous_edge
                .as_ref()
                .is_some_and(|previous| previous >= &key)
            {
                return Err(PlannerContractError::new(
                    "edges",
                    "must be unique and sorted",
                ));
            }
            previous_edge = Some(key);
        }
        validate_sorted_ids("unknown_transition_ids", &self.unknown_transition_ids)?;
        validate_sorted_ids("unknown_writer_ids", &self.unknown_writer_ids)?;
        validate_sorted_ids("execution_error_ids", &self.execution_error_ids)?;
        if self.reachable_state_ids() != node_ids {
            return Err(PlannerContractError::new(
                "nodes",
                "contains a state unreachable from the declared initial node",
            ));
        }
        if self.traversal_complete && self.nodes.iter().any(|node| !node.evaluated) {
            return Err(PlannerContractError::new(
                "traversal_complete",
                "cannot be true while an unevaluated frontier remains",
            ));
        }
        Ok(())
    }

    pub fn reachable_state_ids(&self) -> BTreeSet<Digest> {
        let mut reached = BTreeSet::from([self.initial_state_sha256]);
        let mut queue = VecDeque::from([self.initial_state_sha256]);
        while let Some(source) = queue.pop_front() {
            for edge in self
                .edges
                .iter()
                .filter(|edge| edge.source_state_sha256 == source)
            {
                if reached.insert(edge.result_state_sha256) {
                    queue.push_back(edge.result_state_sha256);
                }
            }
        }
        reached
    }

    pub fn with_refinement_stack_sha256(
        mut self,
        refinement_stack_sha256: Digest,
    ) -> Result<Self, PlannerContractError> {
        require_digest("refinement_stack_sha256", refinement_stack_sha256)?;
        self.refinement_stack_sha256 = Some(refinement_stack_sha256);
        self.validate()?;
        Ok(self)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let graph: Self = serde_json::from_slice(bytes)?;
        graph.validate()?;
        if graph.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "authorization_graph",
                "is not canonical JSON",
            ));
        }
        Ok(graph)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

#[derive(Default)]
pub(crate) struct AuthorizationRecorder {
    nodes: BTreeMap<Digest, AuthorizationStateNode>,
    edges: BTreeSet<AuthorizationEdgeKey>,
}

impl AuthorizationRecorder {
    pub(crate) fn observe_state(
        &mut self,
        state_sha256: Digest,
        execution_state_sha256: Digest,
        snapshot_sha256: Digest,
        depth: usize,
        evaluated: bool,
    ) {
        self.nodes
            .entry(state_sha256)
            .and_modify(|node| {
                if evaluated || depth < node.minimum_depth {
                    node.execution_state_sha256 = execution_state_sha256;
                    node.snapshot_sha256 = snapshot_sha256;
                }
                node.minimum_depth = node.minimum_depth.min(depth);
                node.evaluated |= evaluated;
            })
            .or_insert(AuthorizationStateNode {
                state_sha256,
                execution_state_sha256,
                snapshot_sha256,
                minimum_depth: depth,
                evaluated,
            });
    }

    pub(crate) fn record_edge(&mut self, step: &SearchStep) {
        self.edges.insert(AuthorizationEdgeKey {
            source_state_sha256: step.source_state_sha256,
            result_state_sha256: step.result_state_sha256,
            action_kind: step.action_kind,
            action_id: step.action_id.clone(),
            selected_resolver_ids: step.selected_resolver_ids.clone(),
            selected_technique_ids: step.selected_technique_ids.clone(),
            weakest_evidence: step.weakest_evidence,
        });
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct AuthorizationEdgeKey {
    source_state_sha256: Digest,
    result_state_sha256: Digest,
    action_kind: SearchActionKind,
    action_id: String,
    selected_resolver_ids: Vec<String>,
    selected_technique_ids: Vec<String>,
    weakest_evidence: Option<TruthStatus>,
}

impl From<AuthorizationEdgeKey> for AuthorizationEdge {
    fn from(edge: AuthorizationEdgeKey) -> Self {
        Self {
            source_state_sha256: edge.source_state_sha256,
            result_state_sha256: edge.result_state_sha256,
            action_kind: edge.action_kind,
            action_id: edge.action_id,
            selected_resolver_ids: edge.selected_resolver_ids,
            selected_technique_ids: edge.selected_technique_ids,
            weakest_evidence: edge.weakest_evidence,
        }
    }
}

fn authorization_edge_key(edge: &AuthorizationEdge) -> AuthorizationEdgeKey {
    AuthorizationEdgeKey {
        source_state_sha256: edge.source_state_sha256,
        result_state_sha256: edge.result_state_sha256,
        action_kind: edge.action_kind,
        action_id: edge.action_id.clone(),
        selected_resolver_ids: edge.selected_resolver_ids.clone(),
        selected_technique_ids: edge.selected_technique_ids.clone(),
        weakest_evidence: edge.weakest_evidence,
    }
}

fn validate_sorted_ids(field: &str, ids: &[String]) -> Result<(), PlannerContractError> {
    let mut previous = None;
    for id in ids {
        validate_stable_id(field, id)?;
        if previous.is_some_and(|value: &str| value >= id.as_str()) {
            return Err(PlannerContractError::new(
                field,
                "must be unique and sorted",
            ));
        }
        previous = Some(id.as_str());
    }
    Ok(())
}

fn require_digest(field: &str, digest: Digest) -> Result<(), PlannerContractError> {
    if digest == Digest::ZERO {
        return Err(PlannerContractError::new(field, "must be nonzero"));
    }
    Ok(())
}
