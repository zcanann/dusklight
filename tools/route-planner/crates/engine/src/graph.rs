//! Deterministic planner-native graph projections for browser and tooling clients.

use crate::artifact::Digest;
use crate::evaluation::{
    EvidencePolicy, FeasibilityMode, FeasibilitySelection, PredicateEvaluator,
    TransitionAssessment, TransitionClassification,
};
use crate::execution::PlannerExecutionState;
use crate::identity::EquivalenceSet;
use crate::logic::{
    ComparisonOperator, FactCatalog, PredicateExpression, TruthStatus, ValueReference,
};
use crate::refinement::ComposedPlannerCatalog;
use crate::route_book::{CollapsePolicy, RouteActionRef, RouteBook};
use crate::solver::{
    ContinuationIdentity, SearchActionKind, SearchPlan, SearchResourceLabel, SearchResult,
    SearchStatus, SearchStep,
};
use crate::transition::{MechanicsCatalog, ObligationDetail, ResolutionKind};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const PLANNER_GRAPH_SCHEMA: &str = "dusklight.route-planner.graph/v10";
pub const PLANNER_FEASIBILITY_DIFF_SCHEMA: &str =
    "dusklight.route-planner.feasibility-graph-diff/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerGraph {
    pub schema: String,
    pub fact_catalog_sha256: Digest,
    pub mechanics_catalog_sha256: Digest,
    pub refinement_stack_sha256: Option<Digest>,
    pub route_book_sha256: Option<Digest>,
    pub nodes: Vec<PlannerGraphNode>,
    pub edges: Vec<PlannerGraphEdge>,
    pub regions: Vec<PlannerGraphRegion>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerFeasibilityGraphDiff {
    pub schema: String,
    pub execution_state_sha256: Digest,
    pub snapshot_sha256: Digest,
    pub fact_catalog_sha256: Digest,
    pub mechanics_catalog_sha256: Digest,
    pub transitions: Vec<TransitionFeasibilityDelta>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionFeasibilityDelta {
    pub transition_id: String,
    pub upper_bound: TransitionAssessment,
    pub modeled: TransitionAssessment,
    pub active_obstruction_ids: Vec<String>,
    pub unknown_obstruction_ids: Vec<String>,
    pub discharged_obligation_ids: Vec<String>,
    pub supporting_microtrace_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerGraphNode {
    pub id: String,
    pub label: String,
    pub region_id: Option<String>,
    pub payload: PlannerNodePayload,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlannerNodePayload {
    Alias {
        fact_id: String,
    },
    DerivedFact {
        fact_id: String,
    },
    Goal {
        goal_id: String,
    },
    Transition {
        transition_id: String,
    },
    Obligation {
        obligation_id: String,
    },
    Obstruction {
        obstruction_id: String,
    },
    Resolver {
        resolver_id: String,
        resolution_kind: ResolutionKind,
    },
    Technique {
        technique_id: String,
    },
    Writer {
        writer_id: String,
    },
    Gate {
        gate_id: String,
    },
    Reader {
        reader_id: String,
    },
    Reconstruction {
        reconstruction_rule_id: String,
    },
    Microtrace {
        microtrace_id: String,
    },
    PlanRegion {
        plan_region_id: String,
        collapse_policy: CollapsePolicy,
    },
    PlanMethod {
        method_id: String,
    },
    ReferenceStep {
        step_id: String,
    },
    ExecutionState {
        execution_state_sha256: Digest,
        snapshot_sha256: Digest,
        route_step_id: Option<String>,
    },
    ProofPlan {
        plan_id: String,
        primary: bool,
        result_state_sha256: Digest,
        continuation: ContinuationIdentity,
        preference_score: u64,
        satisfied_preference_ids: Vec<String>,
        route_costs: BTreeMap<String, u64>,
        weakest_evidence: Option<TruthStatus>,
    },
    ProofStep {
        plan_id: String,
        ordinal: u32,
        action_kind: SearchActionKind,
        action_id: String,
        source_state_sha256: Digest,
        result_state_sha256: Digest,
    },
    ProofState {
        plan_id: String,
        ordinal: u32,
        state_sha256: Digest,
    },
    ContinuationMerge {
        state_sha256: Digest,
        dominating: SearchResourceLabel,
        dominated: SearchResourceLabel,
        satisfied_preference_ids: Vec<String>,
    },
    ExternalAction {
        action_id: String,
    },
    ExternalFact {
        fact_id: String,
    },
    Predicate {
        operator: PredicateOperator,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PredicateOperator {
    True,
    False,
    All,
    Any,
    Not,
    Fact {
        fact_id: String,
    },
    Compare {
        left: ValueReference,
        operator: ComparisonOperator,
        right: ValueReference,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerGraphEdge {
    pub id: String,
    pub source_node_id: String,
    pub target_node_id: String,
    pub relation: PlannerGraphRelation,
    pub ordinal: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannerGraphRelation {
    Requires,
    Operand,
    References,
    Blocks,
    Resolves,
    Discharges,
    Introduces,
    Suppresses,
    ConsumedBy,
    Interprets,
    ReconstructsWhen,
    Demonstrates,
    Alternative,
    Contains,
    SelectsAction,
    Selected,
    RoutePrecondition,
    RouteResult,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerExecutionPathState {
    pub label: String,
    pub execution_state_sha256: Digest,
    pub snapshot_sha256: Digest,
    pub route_step_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerGraphRegion {
    pub id: String,
    pub label: String,
    pub parent_region_id: Option<String>,
    pub owner_node_id: Option<String>,
    pub region_kind: PlannerRegionKind,
    pub collapsed_by_default: bool,
    pub collapse_evidence: Option<PlannerCollapseEvidence>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannerRegionKind {
    Facts,
    Mechanics,
    Predicate,
    Plan,
    Proof,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlannerCollapseEvidence {
    ContinuationEquivalent {
        reference_plan_id: String,
        continuation: ContinuationIdentity,
        dominating: SearchResourceLabel,
        dominated: SearchResourceLabel,
    },
    ResidualDifferences {
        reference_plan_id: String,
        differences: Vec<PlannerResidualDifference>,
    },
    ProvenContinuationMerges {
        merge_count: usize,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlannerResidualDifference {
    ResultState {
        primary: Digest,
        alternative: Digest,
    },
    SatisfiedRequiredActions {
        primary: Vec<RouteActionRef>,
        alternative: Vec<RouteActionRef>,
    },
    RequiredSequenceProgress {
        primary: Vec<usize>,
        alternative: Vec<usize>,
    },
    BannedSequenceProgress {
        primary: Vec<usize>,
        alternative: Vec<usize>,
    },
    PreferredSequenceProgress {
        primary: Vec<usize>,
        alternative: Vec<usize>,
    },
    SatisfiedPreferences {
        primary: Vec<String>,
        alternative: Vec<String>,
    },
    RouteConditionUnknown {
        primary: bool,
        alternative: bool,
    },
    ResourceLabel {
        primary: SearchResourceLabel,
        alternative: SearchResourceLabel,
    },
    WeakestEvidence {
        primary: Option<TruthStatus>,
        alternative: Option<TruthStatus>,
    },
}

impl PlannerGraph {
    pub fn project(
        facts: &FactCatalog,
        mechanics: &MechanicsCatalog,
    ) -> Result<Self, PlannerContractError> {
        Self::project_with_context(facts, mechanics, None, None)
    }

    pub fn project_composed(
        catalog: &ComposedPlannerCatalog,
    ) -> Result<Self, PlannerContractError> {
        catalog.validate()?;
        Self::project_with_context(
            &catalog.facts,
            &catalog.mechanics,
            Some(catalog.refinement_stack.digest()?),
            None,
        )
    }

    pub fn project_with_route_book(
        facts: &FactCatalog,
        mechanics: &MechanicsCatalog,
        book: &RouteBook,
    ) -> Result<Self, PlannerContractError> {
        Self::project_with_context(facts, mechanics, None, Some(book))
    }

    pub fn project_composed_with_route_book(
        catalog: &ComposedPlannerCatalog,
        book: &RouteBook,
    ) -> Result<Self, PlannerContractError> {
        catalog.validate()?;
        book.validate_against_composed(catalog)?;
        Self::project_with_context(
            &catalog.facts,
            &catalog.mechanics,
            Some(catalog.refinement_stack.digest()?),
            Some(book),
        )
    }

    fn project_with_context(
        facts: &FactCatalog,
        mechanics: &MechanicsCatalog,
        refinement_stack_sha256: Option<Digest>,
        route_book: Option<&RouteBook>,
    ) -> Result<Self, PlannerContractError> {
        facts.validate()?;
        mechanics.validate()?;
        if let Some(book) = route_book {
            book.validate_against(facts, mechanics)?;
        }
        let mut builder = GraphBuilder::new();
        builder.project_facts(facts)?;
        builder.project_mechanics(mechanics)?;
        if let Some(book) = route_book {
            builder.project_route_book(book)?;
        }
        builder.nodes.sort_by(|left, right| left.id.cmp(&right.id));
        builder.edges.sort_by(|left, right| left.id.cmp(&right.id));
        builder
            .regions
            .sort_by(|left, right| left.id.cmp(&right.id));
        let graph = Self {
            schema: PLANNER_GRAPH_SCHEMA.into(),
            fact_catalog_sha256: facts.digest()?,
            mechanics_catalog_sha256: mechanics.digest()?,
            refinement_stack_sha256,
            route_book_sha256: route_book.map(RouteBook::digest).transpose()?,
            nodes: builder.nodes,
            edges: builder.edges,
            regions: builder.regions,
        };
        graph.validate()?;
        Ok(graph)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != PLANNER_GRAPH_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        if self.fact_catalog_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "fact_catalog_sha256",
                "must be nonzero",
            ));
        }
        if self.mechanics_catalog_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "mechanics_catalog_sha256",
                "must be nonzero",
            ));
        }
        if self.refinement_stack_sha256 == Some(Digest::ZERO) {
            return Err(PlannerContractError::new(
                "refinement_stack_sha256",
                "must be absent or nonzero",
            ));
        }
        if self.route_book_sha256 == Some(Digest::ZERO) {
            return Err(PlannerContractError::new(
                "route_book_sha256",
                "must be absent or nonzero",
            ));
        }
        let region_ids = validate_regions(&self.regions)?;
        let node_ids = validate_nodes(&self.nodes, &region_ids)?;
        validate_edges(&self.edges, &node_ids)?;
        for region in &self.regions {
            if let Some(owner) = &region.owner_node_id
                && !node_ids.contains(owner.as_str())
            {
                return Err(PlannerContractError::new(
                    "regions.owner_node_id",
                    format!("references unknown node {owner}"),
                ));
            }
        }
        Ok(())
    }

    pub fn attach_authored_execution_path(
        &mut self,
        states: &[PlannerExecutionPathState],
    ) -> Result<(), PlannerContractError> {
        if states.is_empty() || states[0].route_step_id.is_some() {
            return Err(PlannerContractError::new(
                "execution_path",
                "must begin with exactly one route-start state",
            ));
        }
        let authored_steps = states
            .iter()
            .skip(1)
            .map(|state| {
                state.route_step_id.as_deref().ok_or_else(|| {
                    PlannerContractError::new(
                        "execution_path.route_step_id",
                        "every non-start state must identify its producing route step",
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let step_nodes = authored_steps
            .iter()
            .map(|step_id| {
                self.nodes
                    .iter()
                    .find(|node| {
                        matches!(
                            &node.payload,
                            PlannerNodePayload::ReferenceStep { step_id: candidate }
                                if candidate == step_id
                        )
                    })
                    .cloned()
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "execution_path.route_step_id",
                            format!("references unprojected route step {step_id}"),
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let region_id = step_nodes
            .first()
            .and_then(|node| node.region_id.clone())
            .or_else(|| {
                self.regions
                    .iter()
                    .find(|region| region.id == "region.plans")
                    .map(|region| region.id.clone())
            })
            .unwrap_or_else(|| "region.mechanics".into());
        let state_node_id = |state: &PlannerExecutionPathState| match &state.route_step_id {
            Some(step_id) => format!("execution-state/after/{step_id}"),
            None => "execution-state/route-start".into(),
        };
        for state in states {
            validate_label("execution_path.label", &state.label)?;
            if state.execution_state_sha256 == Digest::ZERO || state.snapshot_sha256 == Digest::ZERO
            {
                return Err(PlannerContractError::new(
                    "execution_path",
                    "state identities must be nonzero",
                ));
            }
            self.nodes.push(PlannerGraphNode {
                id: state_node_id(state),
                label: state.label.clone(),
                region_id: Some(region_id.clone()),
                payload: PlannerNodePayload::ExecutionState {
                    execution_state_sha256: state.execution_state_sha256,
                    snapshot_sha256: state.snapshot_sha256,
                    route_step_id: state.route_step_id.clone(),
                },
            });
        }
        for (index, (before, after)) in states.iter().zip(states.iter().skip(1)).enumerate() {
            let step = &step_nodes[index];
            let step_id = authored_steps[index];
            self.edges.push(PlannerGraphEdge {
                id: format!("edge.execution-path/{step_id}/precondition"),
                source_node_id: state_node_id(before),
                target_node_id: step.id.clone(),
                relation: PlannerGraphRelation::RoutePrecondition,
                ordinal: Some(index as u32),
            });
            self.edges.push(PlannerGraphEdge {
                id: format!("edge.execution-path/{step_id}/result"),
                source_node_id: step.id.clone(),
                target_node_id: state_node_id(after),
                relation: PlannerGraphRelation::RouteResult,
                ordinal: Some(index as u32),
            });
        }
        self.nodes.sort_by(|left, right| left.id.cmp(&right.id));
        self.edges.sort_by(|left, right| left.id.cmp(&right.id));
        self.validate()
    }

    /// Projects reached plans and exact continuation-merge proofs into nested
    /// proof regions. Alternative regions collapse only when their terminal
    /// continuation identity matches the primary plan and the primary resource
    /// label is no worse on depth or any cost axis. Otherwise the region stays
    /// expanded with explicit residual differences.
    pub fn attach_solver_proof(
        &mut self,
        initial_state_sha256: Digest,
        result: &SearchResult,
    ) -> Result<(), PlannerContractError> {
        if initial_state_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "solver_proof.initial_state_sha256",
                "must be nonzero",
            ));
        }
        if self
            .regions
            .iter()
            .any(|region| region.id == "region.proof")
        {
            return Err(PlannerContractError::new(
                "solver_proof",
                "is already attached to this graph",
            ));
        }
        self.regions.push(PlannerGraphRegion {
            id: "region.proof".into(),
            label: "Solver proof".into(),
            parent_region_id: None,
            owner_node_id: None,
            region_kind: PlannerRegionKind::Proof,
            collapsed_by_default: false,
            collapse_evidence: None,
        });

        let mut plans = Vec::new();
        if result.status == SearchStatus::Reached {
            let continuation = result.result_continuation.clone().ok_or_else(|| {
                PlannerContractError::new(
                    "solver_proof.result_continuation",
                    "a reached result must retain its exact terminal continuation",
                )
            })?;
            let primary = SearchPlan {
                result_state_sha256: result
                    .steps
                    .last()
                    .map_or(initial_state_sha256, |step| step.result_state_sha256),
                continuation,
                steps: result.steps.clone(),
                preference_score: result.preference_score,
                satisfied_preference_ids: result.satisfied_preference_ids.clone(),
                route_costs: result.route_costs.clone(),
            };
            primary.validate()?;
            plans.push(("primary".to_owned(), true, primary));
            for (index, plan) in result.alternative_plans.iter().enumerate() {
                plan.validate()?;
                plans.push((format!("alternative-{index:03}"), false, plan.clone()));
            }
        } else if result.result_continuation.is_some()
            || !result.steps.is_empty()
            || !result.alternative_plans.is_empty()
        {
            return Err(PlannerContractError::new(
                "solver_proof",
                "an unreached result cannot contain reached plan steps",
            ));
        }

        let primary = plans.first().map(|(_, _, plan)| plan.clone());
        for (plan_id, is_primary, plan) in &plans {
            validate_search_step_chain(initial_state_sha256, &plan.steps)?;
            let weakest_evidence = plan
                .steps
                .iter()
                .filter_map(|step| step.weakest_evidence)
                .max();
            let resource_label = SearchResourceLabel {
                depth: plan.steps.len(),
                route_costs: plan.route_costs.clone(),
            };
            resource_label.validate()?;
            let (collapsed_by_default, collapse_evidence) = if *is_primary {
                (false, None)
            } else {
                let reference = primary.as_ref().expect("an alternative has a primary plan");
                proof_plan_collapse(reference, plan, weakest_evidence)
            };
            let region_id = format!("region.proof.plan.{plan_id}");
            let plan_node_id = format!("proof-plan/{plan_id}");
            self.nodes.push(PlannerGraphNode {
                id: plan_node_id.clone(),
                label: if *is_primary {
                    "Primary solver plan".into()
                } else {
                    format!("Alternative solver plan {}", &plan_id[12..])
                },
                region_id: Some(region_id.clone()),
                payload: PlannerNodePayload::ProofPlan {
                    plan_id: plan_id.clone(),
                    primary: *is_primary,
                    result_state_sha256: plan.result_state_sha256,
                    continuation: plan.continuation.clone(),
                    preference_score: plan.preference_score,
                    satisfied_preference_ids: plan.satisfied_preference_ids.clone(),
                    route_costs: plan.route_costs.clone(),
                    weakest_evidence,
                },
            });
            self.regions.push(PlannerGraphRegion {
                id: region_id.clone(),
                label: if *is_primary {
                    "Primary plan".into()
                } else {
                    format!("Alternative {}", &plan_id[12..])
                },
                parent_region_id: Some("region.proof".into()),
                owner_node_id: Some(plan_node_id.clone()),
                region_kind: PlannerRegionKind::Proof,
                collapsed_by_default,
                collapse_evidence,
            });

            let mut state_sha256 = initial_state_sha256;
            for ordinal in 0..=plan.steps.len() {
                let state_node_id = format!("proof-state/{plan_id}/{ordinal:04}");
                self.nodes.push(PlannerGraphNode {
                    id: state_node_id.clone(),
                    label: if ordinal == 0 {
                        "Plan start".into()
                    } else {
                        format!("State after step {ordinal}")
                    },
                    region_id: Some(region_id.clone()),
                    payload: PlannerNodePayload::ProofState {
                        plan_id: plan_id.clone(),
                        ordinal: ordinal as u32,
                        state_sha256,
                    },
                });
                push_graph_edge(
                    &mut self.edges,
                    &plan_node_id,
                    &state_node_id,
                    PlannerGraphRelation::Contains,
                    Some((ordinal * 2) as u32),
                )?;
                let Some(step) = plan.steps.get(ordinal) else {
                    continue;
                };
                let step_node_id = format!("proof-step/{plan_id}/{ordinal:04}");
                self.nodes.push(PlannerGraphNode {
                    id: step_node_id.clone(),
                    label: format!(
                        "{} · {}",
                        action_kind_label(step.action_kind),
                        step.action_id
                    ),
                    region_id: Some(region_id.clone()),
                    payload: PlannerNodePayload::ProofStep {
                        plan_id: plan_id.clone(),
                        ordinal: ordinal as u32,
                        action_kind: step.action_kind,
                        action_id: step.action_id.clone(),
                        source_state_sha256: step.source_state_sha256,
                        result_state_sha256: step.result_state_sha256,
                    },
                });
                let action_node = search_action_node_id(step.action_kind, &step.action_id);
                if !self.nodes.iter().any(|node| node.id == action_node) {
                    return Err(PlannerContractError::new(
                        "solver_proof.steps.action_id",
                        format!("references unprojected action {}", step.action_id),
                    ));
                }
                push_graph_edge(
                    &mut self.edges,
                    &plan_node_id,
                    &step_node_id,
                    PlannerGraphRelation::Contains,
                    Some((ordinal * 2 + 1) as u32),
                )?;
                push_graph_edge(
                    &mut self.edges,
                    &state_node_id,
                    &step_node_id,
                    PlannerGraphRelation::RoutePrecondition,
                    Some(ordinal as u32),
                )?;
                push_graph_edge(
                    &mut self.edges,
                    &step_node_id,
                    &format!("proof-state/{plan_id}/{:04}", ordinal + 1),
                    PlannerGraphRelation::RouteResult,
                    Some(ordinal as u32),
                )?;
                push_graph_edge(
                    &mut self.edges,
                    &step_node_id,
                    &action_node,
                    PlannerGraphRelation::SelectsAction,
                    None,
                )?;
                state_sha256 = step.result_state_sha256;
            }
        }

        if !result.continuation_merge_proofs.is_empty() {
            let region_id = "region.proof.continuation-merges";
            self.regions.push(PlannerGraphRegion {
                id: region_id.into(),
                label: "Proven continuation merges".into(),
                parent_region_id: Some("region.proof".into()),
                owner_node_id: None,
                region_kind: PlannerRegionKind::Proof,
                collapsed_by_default: true,
                collapse_evidence: Some(PlannerCollapseEvidence::ProvenContinuationMerges {
                    merge_count: result.continuation_merge_proofs.len(),
                }),
            });
            for (index, proof) in result.continuation_merge_proofs.iter().enumerate() {
                proof.validate()?;
                self.nodes.push(PlannerGraphNode {
                    id: format!("continuation-merge/{index:04}"),
                    label: format!("Dominated frontier label {}", index + 1),
                    region_id: Some(region_id.into()),
                    payload: PlannerNodePayload::ContinuationMerge {
                        state_sha256: proof.continuation.state_sha256,
                        dominating: proof.dominating.clone(),
                        dominated: proof.dominated.clone(),
                        satisfied_preference_ids: proof
                            .continuation
                            .satisfied_preference_ids
                            .clone(),
                    },
                });
            }
        }

        self.nodes.sort_by(|left, right| left.id.cmp(&right.id));
        self.edges.sort_by(|left, right| left.id.cmp(&right.id));
        self.regions.sort_by(|left, right| left.id.cmp(&right.id));
        self.validate()
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
                "planner_graph",
                "is not canonical JSON",
            ));
        }
        Ok(graph)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

impl PlannerFeasibilityGraphDiff {
    pub fn project(
        state: &PlannerExecutionState,
        facts: &FactCatalog,
        mechanics: &MechanicsCatalog,
        equivalence_sets: &[EquivalenceSet],
        evidence_policy: EvidencePolicy,
    ) -> Result<Self, PlannerContractError> {
        state.validate()?;
        let snapshot = &state.snapshot;
        snapshot.validate()?;
        facts.validate()?;
        mechanics.validate()?;
        let evaluator = PredicateEvaluator::new(
            snapshot,
            facts,
            equivalence_sets,
            &state.gate_states,
            evidence_policy,
        )?;
        let empty = BTreeSet::new();
        let mut transitions = Vec::new();
        for transition in &mechanics.transitions {
            let upper_bound = evaluator.assess_transition(
                transition,
                &empty,
                &empty,
                FeasibilityMode::UpperBound,
            );
            let resolution = evaluator.resolve_feasibility(
                transition,
                &mechanics.obligations,
                &mechanics.obstructions,
                &mechanics.resolvers,
                &mechanics.techniques,
                FeasibilitySelection {
                    resolver_ids: &empty,
                    technique_ids: &empty,
                    already_discharged: &empty,
                    microtraces: &mechanics.microtraces,
                },
            );
            let mut modeled = evaluator.assess_transition(
                transition,
                &resolution.discharged_obligation_ids,
                &resolution.unknown_obligation_ids,
                FeasibilityMode::Modeled,
            );
            if matches!(
                modeled.classification,
                TransitionClassification::Executable | TransitionClassification::Obstructed
            ) {
                if !resolution.unknown_obstruction_ids.is_empty() {
                    modeled.classification = TransitionClassification::FeasibilityUnknown;
                } else if !resolution.active_obstruction_ids.is_empty() {
                    modeled.classification = TransitionClassification::Obstructed;
                }
            }
            if upper_bound != modeled
                || !resolution.active_obstruction_ids.is_empty()
                || !resolution.unknown_obstruction_ids.is_empty()
                || !resolution.supporting_microtrace_ids.is_empty()
            {
                transitions.push(TransitionFeasibilityDelta {
                    transition_id: transition.id.clone(),
                    upper_bound,
                    modeled,
                    active_obstruction_ids: resolution.active_obstruction_ids,
                    unknown_obstruction_ids: resolution.unknown_obstruction_ids,
                    discharged_obligation_ids: resolution
                        .discharged_obligation_ids
                        .into_iter()
                        .collect(),
                    supporting_microtrace_ids: resolution
                        .supporting_microtrace_ids
                        .into_iter()
                        .collect(),
                });
            }
        }
        let diff = Self {
            schema: PLANNER_FEASIBILITY_DIFF_SCHEMA.into(),
            execution_state_sha256: state.semantic_digest()?,
            snapshot_sha256: snapshot.digest()?,
            fact_catalog_sha256: facts.digest()?,
            mechanics_catalog_sha256: mechanics.digest()?,
            transitions,
        };
        diff.validate()?;
        Ok(diff)
    }

    pub fn project_composed(
        state: &PlannerExecutionState,
        catalog: &ComposedPlannerCatalog,
        equivalence_sets: &[EquivalenceSet],
        evidence_policy: EvidencePolicy,
    ) -> Result<Self, PlannerContractError> {
        catalog.validate()?;
        Self::project(
            state,
            &catalog.facts,
            &catalog.mechanics,
            equivalence_sets,
            evidence_policy,
        )
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != PLANNER_FEASIBILITY_DIFF_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        if self.execution_state_sha256 == Digest::ZERO
            || self.snapshot_sha256 == Digest::ZERO
            || self.fact_catalog_sha256 == Digest::ZERO
            || self.mechanics_catalog_sha256 == Digest::ZERO
        {
            return Err(PlannerContractError::new(
                "feasibility_graph_diff",
                "contains a zero source digest",
            ));
        }
        let mut previous = None;
        for transition in &self.transitions {
            validate_stable_id("transitions.transition_id", &transition.transition_id)?;
            if previous.is_some_and(|id: &str| id >= transition.transition_id.as_str()) {
                return Err(PlannerContractError::new(
                    "transitions",
                    "must be unique and sorted by transition ID",
                ));
            }
            if transition.upper_bound.transition_id != transition.transition_id
                || transition.modeled.transition_id != transition.transition_id
            {
                return Err(PlannerContractError::new(
                    "transitions.assessment.transition_id",
                    "must match the enclosing transition ID",
                ));
            }
            validate_transition_assessment(&transition.upper_bound)?;
            validate_transition_assessment(&transition.modeled)?;
            validate_sorted_ids(
                "transitions.active_obstruction_ids",
                &transition.active_obstruction_ids,
            )?;
            validate_sorted_ids(
                "transitions.unknown_obstruction_ids",
                &transition.unknown_obstruction_ids,
            )?;
            validate_sorted_ids(
                "transitions.discharged_obligation_ids",
                &transition.discharged_obligation_ids,
            )?;
            validate_sorted_ids(
                "transitions.supporting_microtrace_ids",
                &transition.supporting_microtrace_ids,
            )?;
            previous = Some(transition.transition_id.as_str());
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let diff: Self = serde_json::from_slice(bytes)?;
        diff.validate()?;
        if diff.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "feasibility_graph_diff",
                "is not canonical JSON",
            ));
        }
        Ok(diff)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

struct GraphBuilder {
    nodes: Vec<PlannerGraphNode>,
    edges: Vec<PlannerGraphEdge>,
    regions: Vec<PlannerGraphRegion>,
    node_ids: BTreeSet<String>,
    edge_ids: BTreeSet<String>,
}

impl GraphBuilder {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            regions: vec![
                PlannerGraphRegion {
                    id: "region.facts".into(),
                    label: "Facts".into(),
                    parent_region_id: None,
                    owner_node_id: None,
                    region_kind: PlannerRegionKind::Facts,
                    collapsed_by_default: true,
                    collapse_evidence: None,
                },
                PlannerGraphRegion {
                    id: "region.mechanics".into(),
                    label: "Mechanics".into(),
                    parent_region_id: None,
                    owner_node_id: None,
                    region_kind: PlannerRegionKind::Mechanics,
                    collapsed_by_default: false,
                    collapse_evidence: None,
                },
            ],
            node_ids: BTreeSet::new(),
            edge_ids: BTreeSet::new(),
        }
    }

    fn project_facts(&mut self, facts: &FactCatalog) -> Result<(), PlannerContractError> {
        for alias in &facts.aliases {
            self.add_node(PlannerGraphNode {
                id: fact_node_id(&alias.id),
                label: alias.label.clone(),
                region_id: Some("region.facts".into()),
                payload: PlannerNodePayload::Alias {
                    fact_id: alias.id.clone(),
                },
            })?;
        }
        for fact in &facts.derived_facts {
            self.add_node(PlannerGraphNode {
                id: fact_node_id(&fact.id),
                label: fact.label.clone(),
                region_id: Some("region.facts".into()),
                payload: PlannerNodePayload::DerivedFact {
                    fact_id: fact.id.clone(),
                },
            })?;
        }
        for fact in &facts.derived_facts {
            let owner = fact_node_id(&fact.id);
            self.project_predicate(
                &owner,
                "derived",
                &fact.rule,
                PlannerGraphRelation::Requires,
            )?;
        }
        Ok(())
    }

    fn project_mechanics(
        &mut self,
        mechanics: &MechanicsCatalog,
    ) -> Result<(), PlannerContractError> {
        let transitions = mechanics
            .transitions
            .iter()
            .map(|record| record.id.as_str())
            .collect::<BTreeSet<_>>();
        for record in &mechanics.obligations {
            let owner = format!("obligation/{}", record.id);
            self.add_record_node(
                &owner,
                &record.label,
                PlannerNodePayload::Obligation {
                    obligation_id: record.id.clone(),
                },
            )?;
            match &record.detail {
                ObligationDetail::Predicate { predicate }
                | ObligationDetail::Temporal {
                    precondition: predicate,
                    ..
                } => {
                    self.project_predicate(
                        &owner,
                        "requirement",
                        predicate,
                        PlannerGraphRelation::Requires,
                    )?;
                }
                ObligationDetail::Interaction { pose_predicate, .. } => {
                    self.project_predicate(
                        &owner,
                        "pose",
                        pose_predicate,
                        PlannerGraphRelation::Requires,
                    )?;
                }
                ObligationDetail::Geometry { .. }
                | ObligationDetail::PlaneSide { .. }
                | ObligationDetail::Facing { .. }
                | ObligationDetail::Unresolved { .. } => {}
            }
        }
        for record in &mechanics.transitions {
            let owner = format!("transition/{}", record.id);
            self.add_record_node(
                &owner,
                &record.label,
                PlannerNodePayload::Transition {
                    transition_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "guard",
                &record.activation.hard_guards,
                PlannerGraphRelation::Requires,
            )?;
            for (index, obligation) in record.activation.physical_obligation_ids.iter().enumerate()
            {
                self.add_edge(
                    &owner,
                    &format!("obligation/{obligation}"),
                    PlannerGraphRelation::Requires,
                    Some(index as u32),
                )?;
            }
        }
        for record in &mechanics.writers {
            let owner = format!("writer/{}", record.id);
            self.add_record_node(
                &owner,
                &record.id,
                PlannerNodePayload::Writer {
                    writer_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "activation",
                &record.activation,
                PlannerGraphRelation::Requires,
            )?;
        }
        for record in &mechanics.gates {
            let owner = format!("gate/{}", record.id);
            self.add_record_node(
                &owner,
                &record.id,
                PlannerNodePayload::Gate {
                    gate_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "active",
                &record.active_when,
                PlannerGraphRelation::Requires,
            )?;
            for (index, writer) in record.blocked_writer_ids.iter().enumerate() {
                self.add_edge(
                    &owner,
                    &format!("writer/{writer}"),
                    PlannerGraphRelation::Suppresses,
                    Some(index as u32),
                )?;
            }
        }
        for record in &mechanics.readers {
            let owner = format!("reader/{}", record.id);
            self.add_record_node(
                &owner,
                &record.id,
                PlannerNodePayload::Reader {
                    reader_id: record.id.clone(),
                },
            )?;
            self.add_edge(
                &owner,
                &format!("transition/{}", record.consuming_transition_id),
                PlannerGraphRelation::ConsumedBy,
                None,
            )?;
            if let Some(fact) = &record.interpretation_fact_id {
                let target = self.ensure_fact_node(fact)?;
                self.add_edge(&owner, &target, PlannerGraphRelation::Interprets, None)?;
            }
        }
        for record in &mechanics.reconstruction_rules {
            let owner = format!("reconstruction/{}", record.id);
            self.add_record_node(
                &owner,
                &record.label,
                PlannerNodePayload::Reconstruction {
                    reconstruction_rule_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "instantiate",
                &record.instantiate_when,
                PlannerGraphRelation::ReconstructsWhen,
            )?;
        }
        for record in &mechanics.obstructions {
            let owner = format!("obstruction/{}", record.id);
            self.add_record_node(
                &owner,
                &record.label,
                PlannerNodePayload::Obstruction {
                    obstruction_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "active",
                &record.active_when,
                PlannerGraphRelation::Requires,
            )?;
            let action = if transitions.contains(record.blocked_action_id.as_str()) {
                format!("transition/{}", record.blocked_action_id)
            } else {
                self.ensure_external_action(&record.blocked_action_id)?
            };
            self.add_edge(&owner, &action, PlannerGraphRelation::Blocks, None)?;
            for (index, obligation) in record.obligation_ids.iter().enumerate() {
                self.add_edge(
                    &owner,
                    &format!("obligation/{obligation}"),
                    PlannerGraphRelation::Requires,
                    Some(index as u32),
                )?;
            }
        }
        for record in &mechanics.resolvers {
            let owner = format!("resolver/{}", record.id);
            self.add_record_node(
                &owner,
                &record.label,
                PlannerNodePayload::Resolver {
                    resolver_id: record.id.clone(),
                    resolution_kind: record.resolution_kind,
                },
            )?;
            self.project_predicate(
                &owner,
                "applicable",
                &record.applicable_when,
                PlannerGraphRelation::Requires,
            )?;
            self.add_edge(
                &owner,
                &format!("obstruction/{}", record.obstruction_id),
                PlannerGraphRelation::Resolves,
                None,
            )?;
        }
        for record in &mechanics.techniques {
            let owner = format!("technique/{}", record.id);
            self.add_record_node(
                &owner,
                &record.label,
                PlannerNodePayload::Technique {
                    technique_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "prerequisite",
                &record.prerequisites,
                PlannerGraphRelation::Requires,
            )?;
            for (index, obligation) in record.discharged_obligation_ids.iter().enumerate() {
                self.add_edge(
                    &owner,
                    &format!("obligation/{obligation}"),
                    PlannerGraphRelation::Discharges,
                    Some(index as u32),
                )?;
            }
            for (index, obligation) in record.introduced_obligation_ids.iter().enumerate() {
                self.add_edge(
                    &owner,
                    &format!("obligation/{obligation}"),
                    PlannerGraphRelation::Introduces,
                    Some(index as u32),
                )?;
            }
        }
        for record in &mechanics.microtraces {
            let owner = format!("microtrace/{}", record.id);
            self.add_record_node(
                &owner,
                &record.id,
                PlannerNodePayload::Microtrace {
                    microtrace_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "precondition",
                &record.precondition,
                PlannerGraphRelation::Requires,
            )?;
            self.project_predicate(
                &owner,
                "postcondition",
                &record.postcondition,
                PlannerGraphRelation::Demonstrates,
            )?;
        }
        for obligation in &mechanics.obligations {
            let requirement = match &obligation.detail {
                ObligationDetail::Interaction {
                    temporal_requirement: Some(requirement),
                    ..
                }
                | ObligationDetail::Temporal { requirement, .. } => Some(requirement),
                _ => None,
            };
            let Some(requirement) = requirement else {
                continue;
            };
            for (index, trace) in mechanics
                .microtraces
                .iter()
                .filter(|trace| {
                    trace.witnesses(requirement)
                        && obligation
                            .scope
                            .selectors
                            .iter()
                            .any(|selector| trace.scope.selectors.contains(selector))
                })
                .enumerate()
            {
                self.add_edge(
                    &format!("microtrace/{}", trace.id),
                    &format!("obligation/{}", obligation.id),
                    PlannerGraphRelation::Demonstrates,
                    Some(index as u32),
                )?;
            }
        }
        for record in &mechanics.goals {
            let owner = format!("goal/{}", record.id);
            self.add_record_node(
                &owner,
                &record.label,
                PlannerNodePayload::Goal {
                    goal_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "predicate",
                &record.predicate,
                PlannerGraphRelation::Requires,
            )?;
        }
        Ok(())
    }

    fn project_route_book(&mut self, book: &RouteBook) -> Result<(), PlannerContractError> {
        self.regions.push(PlannerGraphRegion {
            id: "region.plans".into(),
            label: book.manifest.label.clone(),
            parent_region_id: None,
            owner_node_id: None,
            region_kind: PlannerRegionKind::Plan,
            collapsed_by_default: false,
            collapse_evidence: None,
        });
        for region in &book.regions {
            let node_id = format!("plan-region/{}", region.id);
            let graph_region_id = plan_region_graph_id(&region.id);
            self.add_node(PlannerGraphNode {
                id: node_id.clone(),
                label: region.label.clone(),
                region_id: Some(graph_region_id.clone()),
                payload: PlannerNodePayload::PlanRegion {
                    plan_region_id: region.id.clone(),
                    collapse_policy: region.collapse_policy,
                },
            })?;
            self.regions.push(PlannerGraphRegion {
                id: graph_region_id,
                label: region.label.clone(),
                parent_region_id: Some(
                    region
                        .parent_region_id
                        .as_deref()
                        .map(plan_region_graph_id)
                        .unwrap_or_else(|| "region.plans".into()),
                ),
                owner_node_id: Some(node_id),
                region_kind: PlannerRegionKind::Plan,
                // A route book may request collapse, but only a plan/proof
                // projection can prove continuation equivalence or attach
                // residual differences. The catalog projection stays expanded.
                collapsed_by_default: false,
                collapse_evidence: None,
            });
        }
        for method in &book.methods {
            let owner = format!("plan-method/{}", method.id);
            self.add_node(PlannerGraphNode {
                id: owner.clone(),
                label: method.label.clone(),
                region_id: Some(plan_region_graph_id(&method.region_id)),
                payload: PlannerNodePayload::PlanMethod {
                    method_id: method.id.clone(),
                },
            })?;
            self.add_edge(
                &format!("plan-region/{}", method.region_id),
                &owner,
                PlannerGraphRelation::Alternative,
                book.regions
                    .iter()
                    .find(|region| region.id == method.region_id)
                    .and_then(|region| {
                        region
                            .method_ids
                            .iter()
                            .position(|id| id == &method.id)
                            .map(|index| index as u32)
                    }),
            )?;
        }
        for step in &book.steps {
            let owner = format!("plan-step/{}", step.id);
            self.add_node(PlannerGraphNode {
                id: owner.clone(),
                label: step.label.clone(),
                region_id: Some(
                    step.region_id
                        .as_deref()
                        .map(plan_region_graph_id)
                        .unwrap_or_else(|| "region.plans".into()),
                ),
                payload: PlannerNodePayload::ReferenceStep {
                    step_id: step.id.clone(),
                },
            })?;
            self.add_edge(
                &owner,
                &action_node_id(&step.action),
                PlannerGraphRelation::SelectsAction,
                None,
            )?;
            let parent = step
                .region_id
                .as_deref()
                .map(plan_region_graph_id)
                .unwrap_or_else(|| "region.plans".into());
            if let Some(predicate) = &step.precondition {
                self.project_predicate_in_region(
                    &owner,
                    "precondition",
                    predicate,
                    PlannerGraphRelation::Requires,
                    &parent,
                )?;
            }
            if let Some(predicate) = &step.postcondition {
                self.project_predicate_in_region(
                    &owner,
                    "postcondition",
                    predicate,
                    PlannerGraphRelation::Demonstrates,
                    &parent,
                )?;
            }
        }
        for method in &book.methods {
            for (index, step) in method.step_ids.iter().enumerate() {
                self.add_edge(
                    &format!("plan-method/{}", method.id),
                    &format!("plan-step/{step}"),
                    PlannerGraphRelation::Contains,
                    Some(index as u32),
                )?;
            }
        }
        for region in &book.regions {
            let owner = format!("plan-region/{}", region.id);
            let parent = plan_region_graph_id(&region.id);
            if let Some(predicate) = &region.entry_predicate {
                self.project_predicate_in_region(
                    &owner,
                    "entry",
                    predicate,
                    PlannerGraphRelation::Requires,
                    &parent,
                )?;
            }
            self.project_predicate_in_region(
                &owner,
                "outcome",
                &region.outcome_predicate,
                PlannerGraphRelation::Demonstrates,
                &parent,
            )?;
            if let Some(selected) = &region.selected_method_id {
                self.add_edge(
                    &owner,
                    &format!("plan-method/{selected}"),
                    PlannerGraphRelation::Selected,
                    None,
                )?;
            }
        }
        Ok(())
    }

    fn add_record_node(
        &mut self,
        id: &str,
        label: &str,
        payload: PlannerNodePayload,
    ) -> Result<(), PlannerContractError> {
        self.add_node(PlannerGraphNode {
            id: id.into(),
            label: label.into(),
            region_id: Some("region.mechanics".into()),
            payload,
        })
    }

    fn project_predicate(
        &mut self,
        owner: &str,
        role: &str,
        expression: &PredicateExpression,
        relation: PlannerGraphRelation,
    ) -> Result<(), PlannerContractError> {
        let parent = if owner.starts_with("fact/") {
            "region.facts"
        } else {
            "region.mechanics"
        };
        self.project_predicate_in_region(owner, role, expression, relation, parent)
    }

    fn project_predicate_in_region(
        &mut self,
        owner: &str,
        role: &str,
        expression: &PredicateExpression,
        relation: PlannerGraphRelation,
        parent_region_id: &str,
    ) -> Result<(), PlannerContractError> {
        let region_id = format!("region.predicate.{owner}.{role}").replace('/', ".");
        self.regions.push(PlannerGraphRegion {
            id: region_id.clone(),
            label: format!("{role} requirements"),
            parent_region_id: Some(parent_region_id.into()),
            owner_node_id: Some(owner.into()),
            region_kind: PlannerRegionKind::Predicate,
            collapsed_by_default: true,
            collapse_evidence: None,
        });
        let root = self.add_predicate_node(owner, role, "root", expression, &region_id)?;
        self.add_edge(owner, &root, relation, None)
    }

    fn add_predicate_node(
        &mut self,
        owner: &str,
        role: &str,
        path: &str,
        expression: &PredicateExpression,
        region_id: &str,
    ) -> Result<String, PlannerContractError> {
        let id = format!("predicate/{owner}/{role}/{path}");
        let (label, operator, children): (String, PredicateOperator, &[PredicateExpression]) =
            match expression {
                PredicateExpression::True => ("Always".into(), PredicateOperator::True, &[]),
                PredicateExpression::False => ("Never".into(), PredicateOperator::False, &[]),
                PredicateExpression::Fact { fact_id } => (
                    format!("Fact: {fact_id}"),
                    PredicateOperator::Fact {
                        fact_id: fact_id.clone(),
                    },
                    &[],
                ),
                PredicateExpression::Compare {
                    left,
                    operator,
                    right,
                } => (
                    comparison_label(*operator),
                    PredicateOperator::Compare {
                        left: left.clone(),
                        operator: *operator,
                        right: right.clone(),
                    },
                    &[],
                ),
                PredicateExpression::All { terms } => {
                    ("All requirements".into(), PredicateOperator::All, terms)
                }
                PredicateExpression::Any { terms } => {
                    ("Any requirement".into(), PredicateOperator::Any, terms)
                }
                PredicateExpression::Not { term } => (
                    "Not".into(),
                    PredicateOperator::Not,
                    std::slice::from_ref(term.as_ref()),
                ),
            };
        self.add_node(PlannerGraphNode {
            id: id.clone(),
            label,
            region_id: Some(region_id.into()),
            payload: PlannerNodePayload::Predicate { operator },
        })?;
        if let PredicateExpression::Fact { fact_id } = expression {
            let target = self.ensure_fact_node(fact_id)?;
            self.add_edge(&id, &target, PlannerGraphRelation::References, None)?;
        }
        for (index, child) in children.iter().enumerate() {
            let child_id =
                self.add_predicate_node(owner, role, &format!("{path}.{index}"), child, region_id)?;
            self.add_edge(
                &id,
                &child_id,
                PlannerGraphRelation::Operand,
                Some(index as u32),
            )?;
        }
        Ok(id)
    }

    fn ensure_fact_node(&mut self, fact_id: &str) -> Result<String, PlannerContractError> {
        let known = fact_node_id(fact_id);
        if self.node_ids.contains(&known) {
            return Ok(known);
        }
        let external = format!("external/fact/{fact_id}");
        if !self.node_ids.contains(&external) {
            self.add_node(PlannerGraphNode {
                id: external.clone(),
                label: format!("External fact: {fact_id}"),
                region_id: Some("region.facts".into()),
                payload: PlannerNodePayload::ExternalFact {
                    fact_id: fact_id.into(),
                },
            })?;
        }
        Ok(external)
    }

    fn ensure_external_action(&mut self, action_id: &str) -> Result<String, PlannerContractError> {
        let id = format!("external/action/{action_id}");
        if !self.node_ids.contains(&id) {
            self.add_record_node(
                &id,
                &format!("External action: {action_id}"),
                PlannerNodePayload::ExternalAction {
                    action_id: action_id.into(),
                },
            )?;
        }
        Ok(id)
    }

    fn add_node(&mut self, node: PlannerGraphNode) -> Result<(), PlannerContractError> {
        if !self.node_ids.insert(node.id.clone()) {
            return Err(PlannerContractError::new(
                "nodes.id",
                format!("duplicate projected node {}", node.id),
            ));
        }
        self.nodes.push(node);
        Ok(())
    }

    fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        relation: PlannerGraphRelation,
        ordinal: Option<u32>,
    ) -> Result<(), PlannerContractError> {
        let identity = serde_json::to_vec(&(source, target, relation, ordinal))?;
        let digest = Sha256::digest(identity);
        let id = format!("edge.{}", encode_hex(&digest));
        if !self.edge_ids.insert(id.clone()) {
            return Err(PlannerContractError::new(
                "edges.id",
                format!("duplicate projected edge {id}"),
            ));
        }
        self.edges.push(PlannerGraphEdge {
            id,
            source_node_id: source.into(),
            target_node_id: target.into(),
            relation,
            ordinal,
        });
        Ok(())
    }
}

fn fact_node_id(fact_id: &str) -> String {
    format!("fact/{fact_id}")
}

fn plan_region_graph_id(region_id: &str) -> String {
    format!("region.plan.{region_id}")
}

fn action_node_id(action: &RouteActionRef) -> String {
    match action {
        RouteActionRef::Transition { transition_id } => format!("transition/{transition_id}"),
        RouteActionRef::Technique { technique_id } => format!("technique/{technique_id}"),
        RouteActionRef::Resolver { resolver_id } => format!("resolver/{resolver_id}"),
        RouteActionRef::Writer { writer_id } => format!("writer/{writer_id}"),
        RouteActionRef::Microtrace { microtrace_id } => format!("microtrace/{microtrace_id}"),
    }
}

fn search_action_node_id(kind: SearchActionKind, action_id: &str) -> String {
    match kind {
        SearchActionKind::Transition => format!("transition/{action_id}"),
        SearchActionKind::Technique => format!("technique/{action_id}"),
        SearchActionKind::Writer => format!("writer/{action_id}"),
    }
}

fn action_kind_label(kind: SearchActionKind) -> &'static str {
    match kind {
        SearchActionKind::Transition => "Transition",
        SearchActionKind::Technique => "Technique",
        SearchActionKind::Writer => "Writer",
    }
}

fn validate_search_step_chain(
    initial_state_sha256: Digest,
    steps: &[SearchStep],
) -> Result<(), PlannerContractError> {
    let mut expected = initial_state_sha256;
    for step in steps {
        validate_stable_id("solver_proof.steps.action_id", &step.action_id)?;
        if step.source_state_sha256 != expected || step.result_state_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "solver_proof.steps",
                "must form one contiguous nonzero state-identity chain",
            ));
        }
        expected = step.result_state_sha256;
    }
    Ok(())
}

fn proof_plan_collapse(
    primary: &SearchPlan,
    alternative: &SearchPlan,
    alternative_weakest_evidence: Option<TruthStatus>,
) -> (bool, Option<PlannerCollapseEvidence>) {
    let primary_label = SearchResourceLabel {
        depth: primary.steps.len(),
        route_costs: primary.route_costs.clone(),
    };
    let alternative_label = SearchResourceLabel {
        depth: alternative.steps.len(),
        route_costs: alternative.route_costs.clone(),
    };
    let primary_weakest_evidence = primary
        .steps
        .iter()
        .filter_map(|step| step.weakest_evidence)
        .max();
    let same_continuation = primary.continuation == alternative.continuation;
    if same_continuation && resource_no_worse(&primary_label, &alternative_label) {
        return (
            true,
            Some(PlannerCollapseEvidence::ContinuationEquivalent {
                reference_plan_id: "primary".into(),
                continuation: primary.continuation.clone(),
                dominating: primary_label,
                dominated: alternative_label,
            }),
        );
    }

    let mut differences = Vec::new();
    if primary.continuation.state_sha256 != alternative.continuation.state_sha256 {
        differences.push(PlannerResidualDifference::ResultState {
            primary: primary.continuation.state_sha256,
            alternative: alternative.continuation.state_sha256,
        });
    }
    if primary.continuation.satisfied_required_actions
        != alternative.continuation.satisfied_required_actions
    {
        differences.push(PlannerResidualDifference::SatisfiedRequiredActions {
            primary: primary.continuation.satisfied_required_actions.clone(),
            alternative: alternative.continuation.satisfied_required_actions.clone(),
        });
    }
    if primary.continuation.required_sequence_progress
        != alternative.continuation.required_sequence_progress
    {
        differences.push(PlannerResidualDifference::RequiredSequenceProgress {
            primary: primary.continuation.required_sequence_progress.clone(),
            alternative: alternative.continuation.required_sequence_progress.clone(),
        });
    }
    if primary.continuation.banned_sequence_progress
        != alternative.continuation.banned_sequence_progress
    {
        differences.push(PlannerResidualDifference::BannedSequenceProgress {
            primary: primary.continuation.banned_sequence_progress.clone(),
            alternative: alternative.continuation.banned_sequence_progress.clone(),
        });
    }
    if primary.continuation.preferred_sequence_progress
        != alternative.continuation.preferred_sequence_progress
    {
        differences.push(PlannerResidualDifference::PreferredSequenceProgress {
            primary: primary.continuation.preferred_sequence_progress.clone(),
            alternative: alternative.continuation.preferred_sequence_progress.clone(),
        });
    }
    if primary.continuation.satisfied_preference_ids
        != alternative.continuation.satisfied_preference_ids
    {
        differences.push(PlannerResidualDifference::SatisfiedPreferences {
            primary: primary.continuation.satisfied_preference_ids.clone(),
            alternative: alternative.continuation.satisfied_preference_ids.clone(),
        });
    }
    if primary.continuation.route_condition_unknown
        != alternative.continuation.route_condition_unknown
    {
        differences.push(PlannerResidualDifference::RouteConditionUnknown {
            primary: primary.continuation.route_condition_unknown,
            alternative: alternative.continuation.route_condition_unknown,
        });
    }
    if !resource_no_worse(&primary_label, &alternative_label) {
        differences.push(PlannerResidualDifference::ResourceLabel {
            primary: primary_label.clone(),
            alternative: alternative_label.clone(),
        });
    }
    if primary_weakest_evidence != alternative_weakest_evidence {
        differences.push(PlannerResidualDifference::WeakestEvidence {
            primary: primary_weakest_evidence,
            alternative: alternative_weakest_evidence,
        });
    }
    if differences.is_empty() {
        differences.push(PlannerResidualDifference::ResourceLabel {
            primary: primary_label,
            alternative: alternative_label,
        });
    }
    (
        false,
        Some(PlannerCollapseEvidence::ResidualDifferences {
            reference_plan_id: "primary".into(),
            differences,
        }),
    )
}

fn resource_no_worse(left: &SearchResourceLabel, right: &SearchResourceLabel) -> bool {
    left.depth <= right.depth
        && left
            .route_costs
            .keys()
            .chain(right.route_costs.keys())
            .all(|axis| {
                left.route_costs.get(axis).copied().unwrap_or(0)
                    <= right.route_costs.get(axis).copied().unwrap_or(0)
            })
}

fn push_graph_edge(
    edges: &mut Vec<PlannerGraphEdge>,
    source: &str,
    target: &str,
    relation: PlannerGraphRelation,
    ordinal: Option<u32>,
) -> Result<(), PlannerContractError> {
    let identity = serde_json::to_vec(&(source, target, relation, ordinal))?;
    let id = format!("edge.{}", encode_hex(&Sha256::digest(identity)));
    if edges.iter().any(|edge| edge.id == id) {
        return Err(PlannerContractError::new(
            "edges.id",
            format!("duplicate projected edge {id}"),
        ));
    }
    edges.push(PlannerGraphEdge {
        id,
        source_node_id: source.into(),
        target_node_id: target.into(),
        relation,
        ordinal,
    });
    Ok(())
}

fn comparison_label(operator: ComparisonOperator) -> String {
    format!("{operator:?} comparison")
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[usize::from(byte >> 4)] as char);
        output.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    output
}

fn validate_sorted_ids(field: &str, values: &[String]) -> Result<(), PlannerContractError> {
    let mut previous = None;
    for value in values {
        validate_stable_id(field, value)?;
        if previous.is_some_and(|prior: &str| prior >= value.as_str()) {
            return Err(PlannerContractError::new(
                field,
                "must contain unique sorted IDs",
            ));
        }
        previous = Some(value.as_str());
    }
    Ok(())
}

fn validate_transition_assessment(
    assessment: &TransitionAssessment,
) -> Result<(), PlannerContractError> {
    validate_stable_id(
        "transition_assessment.transition_id",
        &assessment.transition_id,
    )?;
    validate_sorted_ids(
        "transition_assessment.outstanding_obligation_ids",
        &assessment.outstanding_obligation_ids,
    )?;
    validate_sorted_ids(
        "transition_assessment.unknown_obligation_ids",
        &assessment.unknown_obligation_ids,
    )?;
    validate_sorted_ids(
        "transition_assessment.unknown_requirement_ids",
        &assessment.unknown_requirement_ids,
    )
}

fn validate_graph_id(field: &str, value: &str) -> Result<(), PlannerContractError> {
    if value.is_empty() || value.len() > 1024 {
        return Err(PlannerContractError::new(
            field,
            "must contain between 1 and 1024 characters",
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

fn validate_regions(
    regions: &[PlannerGraphRegion],
) -> Result<BTreeSet<&str>, PlannerContractError> {
    let mut ids = BTreeSet::new();
    let mut previous = None;
    for region in regions {
        validate_graph_id("regions.id", &region.id)?;
        validate_label("regions.label", &region.label)?;
        validate_collapse_evidence(region)?;
        if !ids.insert(region.id.as_str())
            || previous.is_some_and(|prior: &str| prior >= region.id.as_str())
        {
            return Err(PlannerContractError::new(
                "regions",
                "must be unique and sorted by ID",
            ));
        }
        previous = Some(region.id.as_str());
    }
    for region in regions {
        if let Some(parent) = &region.parent_region_id
            && (!ids.contains(parent.as_str()) || parent == &region.id)
        {
            return Err(PlannerContractError::new(
                "regions.parent_region_id",
                "must reference a different known region",
            ));
        }
    }
    let parents = regions
        .iter()
        .filter_map(|region| {
            region
                .parent_region_id
                .as_deref()
                .map(|parent| (region.id.as_str(), parent))
        })
        .collect::<BTreeMap<_, _>>();
    for start in ids.iter().copied() {
        let mut seen = BTreeSet::new();
        let mut cursor = start;
        while let Some(parent) = parents.get(cursor) {
            if !seen.insert(cursor) {
                return Err(PlannerContractError::new(
                    "regions.parent_region_id",
                    "contains a cycle",
                ));
            }
            cursor = parent;
        }
    }
    Ok(ids)
}

fn validate_collapse_evidence(region: &PlannerGraphRegion) -> Result<(), PlannerContractError> {
    if region.region_kind != PlannerRegionKind::Proof {
        if region.collapse_evidence.is_some() {
            return Err(PlannerContractError::new(
                "regions.collapse_evidence",
                "is reserved for solver proof regions",
            ));
        }
        return Ok(());
    }
    match &region.collapse_evidence {
        None if region.collapsed_by_default => Err(PlannerContractError::new(
            "regions.collapse_evidence",
            "a collapsed proof region requires explicit safety evidence",
        )),
        None => Ok(()),
        Some(PlannerCollapseEvidence::ContinuationEquivalent {
            reference_plan_id,
            continuation,
            dominating,
            dominated,
        }) => {
            if !region.collapsed_by_default {
                return Err(PlannerContractError::new(
                    "regions.collapse_evidence",
                    "continuation equivalence requires a collapsed region",
                ));
            }
            validate_stable_id(
                "regions.collapse_evidence.reference_plan_id",
                reference_plan_id,
            )?;
            continuation.validate()?;
            if continuation.route_condition_unknown {
                return Err(PlannerContractError::new(
                    "regions.collapse_evidence.continuation",
                    "a reached-plan collapse cannot retain unknown route conditions",
                ));
            }
            dominating.validate()?;
            dominated.validate()?;
            if !resource_no_worse(dominating, dominated) {
                return Err(PlannerContractError::new(
                    "regions.collapse_evidence",
                    "dominating resource label is worse than the collapsed alternative",
                ));
            }
            Ok(())
        }
        Some(PlannerCollapseEvidence::ResidualDifferences {
            reference_plan_id,
            differences,
        }) => {
            if region.collapsed_by_default || differences.is_empty() {
                return Err(PlannerContractError::new(
                    "regions.collapse_evidence",
                    "residual differences require an expanded region and at least one difference",
                ));
            }
            validate_stable_id(
                "regions.collapse_evidence.reference_plan_id",
                reference_plan_id,
            )?;
            validate_residual_differences(differences)?;
            Ok(())
        }
        Some(PlannerCollapseEvidence::ProvenContinuationMerges { merge_count }) => {
            if !region.collapsed_by_default || *merge_count == 0 {
                return Err(PlannerContractError::new(
                    "regions.collapse_evidence",
                    "continuation-merge evidence requires a collapsed region and nonzero proof count",
                ));
            }
            Ok(())
        }
    }
}

fn validate_residual_differences(
    differences: &[PlannerResidualDifference],
) -> Result<(), PlannerContractError> {
    let mut kinds = BTreeSet::new();
    for difference in differences {
        let (kind, differs) = match difference {
            PlannerResidualDifference::ResultState {
                primary,
                alternative,
            } => (
                "result_state",
                *primary != Digest::ZERO && *alternative != Digest::ZERO && primary != alternative,
            ),
            PlannerResidualDifference::SatisfiedRequiredActions {
                primary,
                alternative,
            } => {
                for actions in [primary, alternative] {
                    ContinuationIdentity {
                        state_sha256: Digest([1; 32]),
                        satisfied_required_actions: actions.clone(),
                        required_sequence_progress: Vec::new(),
                        banned_sequence_progress: Vec::new(),
                        preferred_sequence_progress: Vec::new(),
                        satisfied_preference_ids: Vec::new(),
                        route_condition_unknown: false,
                    }
                    .validate()?;
                }
                ("satisfied_required_actions", primary != alternative)
            }
            PlannerResidualDifference::RequiredSequenceProgress {
                primary,
                alternative,
            } => ("required_sequence_progress", primary != alternative),
            PlannerResidualDifference::BannedSequenceProgress {
                primary,
                alternative,
            } => ("banned_sequence_progress", primary != alternative),
            PlannerResidualDifference::PreferredSequenceProgress {
                primary,
                alternative,
            } => ("preferred_sequence_progress", primary != alternative),
            PlannerResidualDifference::SatisfiedPreferences {
                primary,
                alternative,
            } => {
                validate_sorted_ids(
                    "regions.collapse_evidence.satisfied_preferences.primary",
                    primary,
                )?;
                validate_sorted_ids(
                    "regions.collapse_evidence.satisfied_preferences.alternative",
                    alternative,
                )?;
                ("satisfied_preferences", primary != alternative)
            }
            PlannerResidualDifference::RouteConditionUnknown {
                primary,
                alternative,
            } => ("route_condition_unknown", primary != alternative),
            PlannerResidualDifference::ResourceLabel {
                primary,
                alternative,
            } => {
                primary.validate()?;
                alternative.validate()?;
                ("resource_label", !resource_no_worse(primary, alternative))
            }
            PlannerResidualDifference::WeakestEvidence {
                primary,
                alternative,
            } => ("weakest_evidence", primary != alternative),
        };
        if !differs || !kinds.insert(kind) {
            return Err(PlannerContractError::new(
                "regions.collapse_evidence.differences",
                "must contain unique differences whose primary and alternative values differ",
            ));
        }
    }
    Ok(())
}

fn validate_nodes<'a>(
    nodes: &'a [PlannerGraphNode],
    region_ids: &BTreeSet<&str>,
) -> Result<BTreeSet<&'a str>, PlannerContractError> {
    let mut ids = BTreeSet::new();
    let mut previous = None;
    for node in nodes {
        validate_graph_id("nodes.id", &node.id)?;
        validate_label("nodes.label", &node.label)?;
        validate_node_payload(&node.payload)?;
        if let Some(region) = &node.region_id
            && !region_ids.contains(region.as_str())
        {
            return Err(PlannerContractError::new(
                "nodes.region_id",
                format!("references unknown region {region}"),
            ));
        }
        if !ids.insert(node.id.as_str())
            || previous.is_some_and(|prior: &str| prior >= node.id.as_str())
        {
            return Err(PlannerContractError::new(
                "nodes",
                "must be unique and sorted by ID",
            ));
        }
        previous = Some(node.id.as_str());
    }
    Ok(ids)
}

fn validate_edges(
    edges: &[PlannerGraphEdge],
    node_ids: &BTreeSet<&str>,
) -> Result<(), PlannerContractError> {
    let mut ids = BTreeSet::new();
    let mut previous = None;
    for edge in edges {
        validate_graph_id("edges.id", &edge.id)?;
        if !node_ids.contains(edge.source_node_id.as_str())
            || !node_ids.contains(edge.target_node_id.as_str())
        {
            return Err(PlannerContractError::new(
                "edges",
                "must reference known source and target nodes",
            ));
        }
        if !ids.insert(edge.id.as_str())
            || previous.is_some_and(|prior: &str| prior >= edge.id.as_str())
        {
            return Err(PlannerContractError::new(
                "edges",
                "must be unique and sorted by ID",
            ));
        }
        previous = Some(edge.id.as_str());
    }
    Ok(())
}

fn validate_node_payload(payload: &PlannerNodePayload) -> Result<(), PlannerContractError> {
    let stable_id = match payload {
        PlannerNodePayload::Alias { fact_id }
        | PlannerNodePayload::DerivedFact { fact_id }
        | PlannerNodePayload::ExternalFact { fact_id }
        | PlannerNodePayload::Predicate {
            operator: PredicateOperator::Fact { fact_id },
        } => Some(("nodes.payload.fact_id", fact_id)),
        PlannerNodePayload::Goal { goal_id } => Some(("nodes.payload.goal_id", goal_id)),
        PlannerNodePayload::Transition { transition_id } => {
            Some(("nodes.payload.transition_id", transition_id))
        }
        PlannerNodePayload::Obligation { obligation_id } => {
            Some(("nodes.payload.obligation_id", obligation_id))
        }
        PlannerNodePayload::Obstruction { obstruction_id } => {
            Some(("nodes.payload.obstruction_id", obstruction_id))
        }
        PlannerNodePayload::Resolver { resolver_id, .. } => {
            Some(("nodes.payload.resolver_id", resolver_id))
        }
        PlannerNodePayload::Technique { technique_id } => {
            Some(("nodes.payload.technique_id", technique_id))
        }
        PlannerNodePayload::Writer { writer_id } => Some(("nodes.payload.writer_id", writer_id)),
        PlannerNodePayload::Gate { gate_id } => Some(("nodes.payload.gate_id", gate_id)),
        PlannerNodePayload::Reader { reader_id } => Some(("nodes.payload.reader_id", reader_id)),
        PlannerNodePayload::Reconstruction {
            reconstruction_rule_id,
        } => Some((
            "nodes.payload.reconstruction_rule_id",
            reconstruction_rule_id,
        )),
        PlannerNodePayload::Microtrace { microtrace_id } => {
            Some(("nodes.payload.microtrace_id", microtrace_id))
        }
        PlannerNodePayload::PlanRegion { plan_region_id, .. } => {
            Some(("nodes.payload.plan_region_id", plan_region_id))
        }
        PlannerNodePayload::PlanMethod { method_id } => {
            Some(("nodes.payload.method_id", method_id))
        }
        PlannerNodePayload::ReferenceStep { step_id } => Some(("nodes.payload.step_id", step_id)),
        PlannerNodePayload::ExecutionState {
            execution_state_sha256,
            snapshot_sha256,
            route_step_id,
        } => {
            if *execution_state_sha256 == Digest::ZERO || *snapshot_sha256 == Digest::ZERO {
                return Err(PlannerContractError::new(
                    "nodes.payload.execution_state",
                    "contains a zero state identity",
                ));
            }
            if let Some(step_id) = route_step_id {
                validate_stable_id("nodes.payload.route_step_id", step_id)?;
            }
            None
        }
        PlannerNodePayload::ProofPlan {
            plan_id,
            result_state_sha256,
            continuation,
            satisfied_preference_ids,
            route_costs,
            ..
        } => {
            validate_stable_id("nodes.payload.plan_id", plan_id)?;
            if *result_state_sha256 == Digest::ZERO {
                return Err(PlannerContractError::new(
                    "nodes.payload.result_state_sha256",
                    "must be nonzero",
                ));
            }
            continuation.validate()?;
            if continuation.state_sha256 != *result_state_sha256
                || continuation.satisfied_preference_ids != *satisfied_preference_ids
                || continuation.route_condition_unknown
            {
                return Err(PlannerContractError::new(
                    "nodes.payload.continuation",
                    "must match the proof plan result and preference identity",
                ));
            }
            validate_sorted_ids(
                "nodes.payload.satisfied_preference_ids",
                satisfied_preference_ids,
            )?;
            SearchResourceLabel {
                depth: 0,
                route_costs: route_costs.clone(),
            }
            .validate()?;
            None
        }
        PlannerNodePayload::ProofStep {
            plan_id,
            action_id,
            source_state_sha256,
            result_state_sha256,
            ..
        } => {
            validate_stable_id("nodes.payload.plan_id", plan_id)?;
            validate_stable_id("nodes.payload.action_id", action_id)?;
            if *source_state_sha256 == Digest::ZERO || *result_state_sha256 == Digest::ZERO {
                return Err(PlannerContractError::new(
                    "nodes.payload.proof_step",
                    "contains a zero state identity",
                ));
            }
            None
        }
        PlannerNodePayload::ProofState {
            plan_id,
            state_sha256,
            ..
        } => {
            validate_stable_id("nodes.payload.plan_id", plan_id)?;
            if *state_sha256 == Digest::ZERO {
                return Err(PlannerContractError::new(
                    "nodes.payload.proof_state",
                    "contains a zero state identity",
                ));
            }
            None
        }
        PlannerNodePayload::ContinuationMerge {
            state_sha256,
            dominating,
            dominated,
            satisfied_preference_ids,
        } => {
            if *state_sha256 == Digest::ZERO {
                return Err(PlannerContractError::new(
                    "nodes.payload.continuation_merge",
                    "contains a zero state identity",
                ));
            }
            dominating.validate()?;
            dominated.validate()?;
            if !resource_no_worse(dominating, dominated) {
                return Err(PlannerContractError::new(
                    "nodes.payload.continuation_merge",
                    "does not contain a dominating resource label",
                ));
            }
            validate_sorted_ids(
                "nodes.payload.satisfied_preference_ids",
                satisfied_preference_ids,
            )?;
            None
        }
        PlannerNodePayload::ExternalAction { action_id } => {
            Some(("nodes.payload.action_id", action_id))
        }
        PlannerNodePayload::Predicate {
            operator:
                PredicateOperator::True
                | PredicateOperator::False
                | PredicateOperator::All
                | PredicateOperator::Any
                | PredicateOperator::Not,
        } => None,
        PlannerNodePayload::Predicate {
            operator:
                PredicateOperator::Compare {
                    left,
                    operator,
                    right,
                },
        } => {
            return PredicateExpression::Compare {
                left: left.clone(),
                operator: *operator,
                right: right.clone(),
            }
            .validate();
        }
    };
    if let Some((field, value)) = stable_id {
        validate_stable_id(field, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{
        ContextSelector, ExactContext, RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration,
    };
    use crate::logic::{
        ContextScope, DerivedFact, EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA, RuleEvidence,
        TruthStatus,
    };
    use crate::relevance::{BACKWARD_RELEVANCE_SCHEMA, BackwardRelevance};
    use crate::route_book::{
        CollapsePolicy, PlanMethod, PlanRegion, ROUTE_BOOK_SCHEMA, ReferenceStep, RouteActionRef,
        RouteBook, RouteBookManifest,
    };
    use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
    use crate::solver::{ContinuationIdentity, ContinuationMergeProof};
    use crate::state::{
        BackingAttachment, EXECUTION_ENVIRONMENT_SCHEMA, ExecutionEnvironment, PlayerForm,
        PlayerState, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation,
        SpatialConnection, SpatialConnectionStatus,
    };
    use crate::transition::{
        ActivationContract, CandidateTransition, FeasibilityObligation, Goal,
        MECHANICS_CATALOG_SCHEMA, MechanicsCatalog, ObligationDetail, ObligationKind, RouteCost,
        StateOperation, Technique, TemporalRequirement, TemporalWindow, TransitionKind,
        WitnessedMicrotrace,
    };

    fn scope() -> ContextScope {
        ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: ExactContext {
                    content_sha256: Digest([1; 32]),
                    runtime_configuration_sha256: Digest([2; 32]),
                },
            }],
        }
    }

    fn evidence() -> RuleEvidence {
        RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![EvidenceRecord {
                id: "source.graph-test".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(Digest([3; 32])),
                note: "Graph projection test evidence.".into(),
            }],
        }
    }

    fn catalogs() -> (FactCatalog, MechanicsCatalog) {
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: vec![
                DerivedFact {
                    id: "fact.can-return".into(),
                    label: "Can return to Ordon".into(),
                    scope: scope(),
                    rule: PredicateExpression::All {
                        terms: vec![
                            PredicateExpression::Fact {
                                fact_id: "fact.has-route".into(),
                            },
                            PredicateExpression::True,
                        ],
                    },
                    evidence: evidence(),
                },
                DerivedFact {
                    id: "fact.has-route".into(),
                    label: "Has a route".into(),
                    scope: scope(),
                    rule: PredicateExpression::True,
                    evidence: evidence(),
                },
            ],
        };
        let mechanics = MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions: Vec::new(),
            obligations: Vec::new(),
            writers: Vec::new(),
            gates: Vec::new(),
            readers: Vec::new(),
            reconstruction_rules: Vec::new(),
            obstructions: Vec::new(),
            resolvers: Vec::new(),
            techniques: vec![Technique {
                id: "technique.ordon-return".into(),
                label: "Return to Ordon".into(),
                scope: scope(),
                prerequisites: PredicateExpression::True,
                operations: Vec::new(),
                discharged_obligation_ids: Vec::new(),
                introduced_obligation_ids: Vec::new(),
                cost: RouteCost {
                    axes: BTreeMap::new(),
                },
                evidence: evidence(),
            }],
            microtraces: Vec::new(),
            goals: vec![Goal {
                id: "goal.ordon".into(),
                label: "Reach Ordon".into(),
                predicate: PredicateExpression::Any {
                    terms: vec![
                        PredicateExpression::Fact {
                            fact_id: "fact.can-return".into(),
                        },
                        PredicateExpression::False,
                    ],
                },
            }],
        };
        (facts, mechanics)
    }

    fn route_book() -> RouteBook {
        RouteBook {
            schema: ROUTE_BOOK_SCHEMA.into(),
            manifest: RouteBookManifest {
                id: "route-book.ordon".into(),
                version: "1.0.0".into(),
                label: "Ordon routes".into(),
                author: "Route research".into(),
                source: "Graph test".into(),
                scope: scope(),
                refinement_stack_sha256: None,
            },
            goal_ids: vec!["goal.ordon".into()],
            constraints: Vec::new(),
            directives: Vec::new(),
            steps: vec![ReferenceStep {
                id: "step.return".into(),
                label: "Return to Ordon".into(),
                scope: scope(),
                action: RouteActionRef::Technique {
                    technique_id: "technique.ordon-return".into(),
                },
                precondition: None,
                postcondition: Some(PredicateExpression::Fact {
                    fact_id: "fact.can-return".into(),
                }),
                region_id: Some("region.return".into()),
                annotation_ids: Vec::new(),
            }],
            methods: vec![PlanMethod {
                id: "method.return".into(),
                label: "Known return".into(),
                scope: scope(),
                region_id: "region.return".into(),
                step_ids: vec!["step.return".into()],
            }],
            regions: vec![PlanRegion {
                id: "region.return".into(),
                label: "Reach Ordon".into(),
                scope: scope(),
                parent_region_id: None,
                entry_predicate: None,
                outcome_predicate: PredicateExpression::Fact {
                    fact_id: "fact.can-return".into(),
                },
                method_ids: vec!["method.return".into()],
                selected_method_id: Some("method.return".into()),
                collapse_policy: CollapsePolicy::OnlyContinuationEquivalent,
            }],
            annotations: Vec::new(),
        }
    }

    #[test]
    fn projection_preserves_nested_and_or_requirements_in_collapsible_regions() {
        let (facts, mechanics) = catalogs();
        let graph = PlannerGraph::project(&facts, &mechanics).unwrap();
        assert!(graph.nodes.iter().any(|node| matches!(
            node.payload,
            PlannerNodePayload::Predicate {
                operator: PredicateOperator::All
            }
        )));
        assert!(graph.nodes.iter().any(|node| matches!(
            node.payload,
            PlannerNodePayload::Predicate {
                operator: PredicateOperator::Any
            }
        )));
        assert_eq!(
            graph
                .regions
                .iter()
                .filter(|region| region.region_kind == PlannerRegionKind::Predicate)
                .count(),
            4
        );
        assert!(
            graph
                .regions
                .iter()
                .filter(|region| { region.region_kind == PlannerRegionKind::Predicate })
                .all(|region| region.collapsed_by_default)
        );
        assert!(
            !graph
                .nodes
                .iter()
                .any(|node| matches!(node.payload, PlannerNodePayload::ExternalFact { .. }))
        );
        let bytes = graph.canonical_bytes().unwrap();
        assert_eq!(PlannerGraph::decode_canonical(&bytes).unwrap(), graph);
    }

    #[test]
    fn temporal_witness_auto_binds_to_its_exact_obligation_in_the_graph() {
        let (facts, mut mechanics) = catalogs();
        let requirement = TemporalRequirement {
            action_id: "dialogue.auru".into(),
            window: TemporalWindow {
                earliest_frame: 0,
                latest_frame: 1,
                required_input: Some("sidehop".into()),
            },
        };
        mechanics.obligations = vec![FeasibilityObligation {
            id: "obligation.auru-window".into(),
            label: "Interrupt Auru during the item dialogue window".into(),
            scope: scope(),
            obligation_kind: ObligationKind::Timing,
            detail: ObligationDetail::Temporal {
                requirement: requirement.clone(),
                precondition: PredicateExpression::True,
            },
            evidence: evidence(),
        }];
        mechanics.microtraces = vec![WitnessedMicrotrace {
            id: "microtrace.auru-sidehop".into(),
            scope: scope(),
            precondition: PredicateExpression::True,
            operations: vec![StateOperation::Interrupt {
                action_id: requirement.action_id,
                window: TemporalWindow {
                    earliest_frame: 1,
                    latest_frame: 1,
                    required_input: Some("sidehop".into()),
                },
            }],
            postcondition: PredicateExpression::True,
            timing: TemporalWindow {
                earliest_frame: 1,
                latest_frame: 1,
                required_input: Some("sidehop".into()),
            },
            evidence: evidence(),
        }];

        let graph = PlannerGraph::project(&facts, &mechanics).unwrap();
        assert!(graph.edges.iter().any(|edge| {
            edge.source_node_id == "microtrace/microtrace.auru-sidehop"
                && edge.target_node_id == "obligation/obligation.auru-window"
                && edge.relation == PlannerGraphRelation::Demonstrates
        }));

        mechanics.microtraces[0].scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: ExactContext {
                    content_sha256: Digest([9; 32]),
                    runtime_configuration_sha256: Digest([8; 32]),
                },
            }],
        };
        let disjoint = PlannerGraph::project(&facts, &mechanics).unwrap();
        assert!(!disjoint.edges.iter().any(|edge| {
            edge.source_node_id == "microtrace/microtrace.auru-sidehop"
                && edge.target_node_id == "obligation/obligation.auru-window"
                && edge.relation == PlannerGraphRelation::Demonstrates
        }));
    }

    #[test]
    fn feasibility_diff_separates_authorized_obstructed_and_unknown_edges() {
        let mut snapshot = StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.graph-diff".into(),
            sequence: 1,
            environment: ExecutionEnvironment {
                schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
                runtime_configuration: RuntimeConfiguration {
                    schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
                    content_sha256: Digest([4; 32]),
                    language: "en".into(),
                    settings: BTreeMap::new(),
                },
                active_runtime_file: RuntimeFile {
                    id: "file-0".into(),
                    origin: RuntimeFileOrigin::TitleFile0,
                    backing: BackingAttachment::MemoryOnly,
                    allowed_serialization_targets: Vec::new(),
                    lifecycle: RuntimeFileLifecycle::Active,
                },
                inactive_runtime_files: Vec::new(),
                physical_slots: Vec::new(),
                physical_slot_observations: Vec::new(),
                execution_context: crate::state::ExecutionContext::World,
                location: SceneLocation {
                    stage: "STAGE_A".into(),
                    room: 0,
                    layer: 0,
                    spawn: 0,
                },
                player: PlayerState {
                    form: PlayerForm::Human,
                    mount: None,
                    position: [0.0; 3],
                    rotation: [0; 3],
                    has_control: Some(true),
                    action: "idle".into(),
                },
                components: Vec::new(),
                static_world_objects: Vec::new(),
                spatial_volumes: Vec::new(),
                spatial_connections: vec![SpatialConnection {
                    approach_id: "approach.front".into(),
                    source_region_id: "region.before-wall".into(),
                    destination_region_id: "region.exit".into(),
                    status: SpatialConnectionStatus::Blocked,
                    source_sha256: Digest([5; 32]),
                }],
                spatial_planes: Vec::new(),
                persisted_object_controls: Vec::new(),
                live_world_objects: Vec::new(),
            },
            semantic_observations: Vec::new(),
        };
        let exact_scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: snapshot
                    .environment
                    .runtime_configuration
                    .exact_context()
                    .unwrap(),
            }],
        };
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
        };
        let obligation = FeasibilityObligation {
            id: "obligation.wall".into(),
            label: "Reach the exit past the wall".into(),
            scope: exact_scope.clone(),
            obligation_kind: ObligationKind::Geometry,
            detail: ObligationDetail::Geometry {
                approach_id: "approach.front".into(),
                source_region_id: "region.before-wall".into(),
                destination_region_id: "region.exit".into(),
            },
            evidence: evidence(),
        };
        let transition = CandidateTransition {
            id: "transition.exit".into(),
            label: "Use the exit behind the wall".into(),
            scope: exact_scope,
            transition_kind: TransitionKind::EncodedMapExit,
            approach_id: "approach.front".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::True,
                physical_obligation_ids: vec![obligation.id.clone()],
                effects: Vec::new(),
                unknown_requirements: Vec::new(),
            },
            evidence: evidence(),
        };
        let mechanics = MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions: vec![transition],
            obligations: vec![obligation],
            writers: Vec::new(),
            gates: Vec::new(),
            readers: Vec::new(),
            reconstruction_rules: Vec::new(),
            obstructions: Vec::new(),
            resolvers: Vec::new(),
            techniques: Vec::new(),
            microtraces: Vec::new(),
            goals: Vec::new(),
        };

        let blocked_state = PlannerExecutionState::new(snapshot.clone()).unwrap();
        let blocked = PlannerFeasibilityGraphDiff::project(
            &blocked_state,
            &facts,
            &mechanics,
            &[],
            EvidencePolicy::ESTABLISHED_ONLY,
        )
        .unwrap();
        assert_eq!(blocked.transitions.len(), 1);
        assert_eq!(
            blocked.transitions[0].upper_bound.classification,
            TransitionClassification::Executable
        );
        assert_eq!(
            blocked.transitions[0].modeled.classification,
            TransitionClassification::Obstructed
        );
        let canonical = blocked.canonical_bytes().unwrap();
        assert_eq!(
            PlannerFeasibilityGraphDiff::decode_canonical(&canonical).unwrap(),
            blocked
        );
        let mut different_gate_state = blocked_state.clone();
        different_gate_state
            .gate_states
            .insert("gate.unrelated".into(), true);
        let gated = PlannerFeasibilityGraphDiff::project(
            &different_gate_state,
            &facts,
            &mechanics,
            &[],
            EvidencePolicy::ESTABLISHED_ONLY,
        )
        .unwrap();
        assert_ne!(gated.execution_state_sha256, blocked.execution_state_sha256);

        snapshot.environment.spatial_connections.clear();
        let unknown_state = PlannerExecutionState::new(snapshot).unwrap();
        let unknown = PlannerFeasibilityGraphDiff::project(
            &unknown_state,
            &facts,
            &mechanics,
            &[],
            EvidencePolicy::ESTABLISHED_ONLY,
        )
        .unwrap();
        assert_eq!(
            unknown.transitions[0].modeled.classification,
            TransitionClassification::FeasibilityUnknown
        );
        assert_eq!(
            unknown.transitions[0].modeled.unknown_obligation_ids,
            vec!["obligation.wall"]
        );
    }

    #[test]
    fn projection_is_deterministic_and_does_not_use_browser_state() {
        let (facts, mechanics) = catalogs();
        let first = PlannerGraph::project(&facts, &mechanics).unwrap();
        let second = PlannerGraph::project(&facts, &mechanics).unwrap();
        assert_eq!(first.digest().unwrap(), second.digest().unwrap());
        assert!(first.nodes.windows(2).all(|pair| pair[0].id < pair[1].id));
        assert!(first.edges.windows(2).all(|pair| pair[0].id < pair[1].id));
    }

    #[test]
    fn route_book_projects_as_nested_preferences_without_replacing_mechanics() {
        let (facts, mechanics) = catalogs();
        let book = route_book();
        let graph = PlannerGraph::project_with_route_book(&facts, &mechanics, &book).unwrap();
        assert_eq!(graph.route_book_sha256, Some(book.digest().unwrap()));
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| matches!(node.payload, PlannerNodePayload::PlanRegion { .. }))
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| matches!(node.payload, PlannerNodePayload::PlanMethod { .. }))
        );
        assert!(graph.edges.iter().any(|edge| {
            edge.relation == PlannerGraphRelation::SelectsAction
                && edge.target_node_id == "technique/technique.ordon-return"
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.relation == PlannerGraphRelation::Selected
                && edge.target_node_id == "plan-method/method.return"
        }));
        assert!(
            graph
                .regions
                .iter()
                .filter(|region| {
                    region.region_kind == PlannerRegionKind::Plan && region.id != "region.plans"
                })
                .all(|region| !region.collapsed_by_default)
        );
    }

    #[test]
    fn solver_proof_regions_collapse_only_safe_continuations_and_keep_residuals() {
        let (facts, mechanics) = catalogs();
        let mut graph = PlannerGraph::project(&facts, &mechanics).unwrap();
        let initial = Digest([10; 32]);
        let reached = Digest([11; 32]);
        let residual = Digest([12; 32]);
        let step = |result_state_sha256| SearchStep {
            action_kind: SearchActionKind::Technique,
            action_id: "technique.ordon-return".into(),
            selected_resolver_ids: Vec::new(),
            selected_technique_ids: Vec::new(),
            active_obstruction_ids: Vec::new(),
            unknown_obstruction_ids: Vec::new(),
            discharged_obligation_ids: Vec::new(),
            outstanding_obligation_ids: Vec::new(),
            unknown_obligation_ids: Vec::new(),
            supporting_microtrace_ids: Vec::new(),
            introduced_obligation_ids: Vec::new(),
            reader_results: Vec::new(),
            unknown_reader_ids: Vec::new(),
            evidence_dependencies: Vec::new(),
            weakest_evidence: Some(TruthStatus::Established),
            action_derivations: Vec::new(),
            obligation_derivations: Vec::new(),
            source_state_sha256: initial,
            result_state_sha256,
        };
        let continuation = |state_sha256| ContinuationIdentity {
            state_sha256,
            satisfied_required_actions: Vec::new(),
            required_sequence_progress: Vec::new(),
            banned_sequence_progress: Vec::new(),
            preferred_sequence_progress: Vec::new(),
            satisfied_preference_ids: Vec::new(),
            route_condition_unknown: false,
        };
        let alternative = |result_state_sha256| SearchPlan {
            result_state_sha256,
            continuation: continuation(result_state_sha256),
            steps: vec![step(result_state_sha256)],
            preference_score: 0,
            satisfied_preference_ids: Vec::new(),
            route_costs: BTreeMap::new(),
        };
        let mut continuation_distinct = alternative(reached);
        continuation_distinct.continuation.banned_sequence_progress = vec![1];
        let result = SearchResult {
            backward_relevance: BackwardRelevance {
                schema: BACKWARD_RELEVANCE_SCHEMA.into(),
                dependencies: Vec::new(),
                frontier_dependencies: Vec::new(),
                transition_ids: Vec::new(),
                writer_ids: Vec::new(),
                technique_ids: vec!["technique.ordon-return".into()],
                obstruction_ids: Vec::new(),
                resolver_ids: Vec::new(),
                obligation_ids: Vec::new(),
                gate_ids: Vec::new(),
                reader_ids: Vec::new(),
                reconstruction_rule_ids: Vec::new(),
                microtrace_ids: Vec::new(),
            },
            backward_pruning_applied: true,
            status: SearchStatus::Reached,
            steps: vec![step(reached)],
            explored_states: 4,
            hit_search_limit: false,
            preference_score: 0,
            satisfied_preference_ids: Vec::new(),
            route_costs: BTreeMap::new(),
            result_continuation: Some(continuation(reached)),
            alternative_plans: vec![
                alternative(reached),
                continuation_distinct,
                alternative(residual),
            ],
            minimum_evidence: Some(TruthStatus::Established),
            unknown_transition_ids: Vec::new(),
            unknown_writer_ids: Vec::new(),
            execution_error_ids: Vec::new(),
            blocked_transition_witnesses: Vec::new(),
            blocked_writer_witnesses: Vec::new(),
            blocked_technique_witnesses: Vec::new(),
            blocked_resolver_witnesses: Vec::new(),
            blocked_reconstruction_witnesses: Vec::new(),
            continuation_merge_proofs: vec![ContinuationMergeProof {
                continuation: ContinuationIdentity {
                    state_sha256: reached,
                    satisfied_required_actions: Vec::new(),
                    required_sequence_progress: Vec::new(),
                    banned_sequence_progress: Vec::new(),
                    preferred_sequence_progress: Vec::new(),
                    satisfied_preference_ids: Vec::new(),
                    route_condition_unknown: false,
                },
                dominating: SearchResourceLabel {
                    depth: 1,
                    route_costs: BTreeMap::new(),
                },
                dominated: SearchResourceLabel {
                    depth: 2,
                    route_costs: BTreeMap::new(),
                },
            }],
            failed_producer_cuts: Vec::new(),
            failed_producer_cut_sets: Vec::new(),
            failed_producer_cut_sets_complete: true,
        };

        graph.attach_solver_proof(initial, &result).unwrap();
        let equivalent = graph
            .regions
            .iter()
            .find(|region| region.id == "region.proof.plan.alternative-000")
            .unwrap();
        assert!(equivalent.collapsed_by_default);
        assert!(matches!(
            equivalent.collapse_evidence,
            Some(PlannerCollapseEvidence::ContinuationEquivalent { .. })
        ));
        let distinct = graph
            .regions
            .iter()
            .find(|region| region.id == "region.proof.plan.alternative-001")
            .unwrap();
        assert!(!distinct.collapsed_by_default);
        assert!(matches!(
            distinct.collapse_evidence,
            Some(PlannerCollapseEvidence::ResidualDifferences { .. })
        ));
        let Some(PlannerCollapseEvidence::ResidualDifferences { differences, .. }) =
            &distinct.collapse_evidence
        else {
            unreachable!()
        };
        assert!(differences.iter().any(|difference| matches!(
            difference,
            PlannerResidualDifference::BannedSequenceProgress {
                primary,
                alternative
            } if primary.is_empty() && alternative == &[1]
        )));
        assert!(graph.regions.iter().any(|region| {
            region.id == "region.proof.continuation-merges"
                && region.collapsed_by_default
                && matches!(
                    region.collapse_evidence,
                    Some(PlannerCollapseEvidence::ProvenContinuationMerges { merge_count: 1 })
                )
        }));
        assert!(graph.nodes.iter().any(|node| {
            node.id == "proof-state/alternative-002/0001"
                && matches!(
                    node.payload,
                    PlannerNodePayload::ProofState { state_sha256, .. }
                        if state_sha256 == residual
                )
        }));
        assert_eq!(
            PlannerGraph::decode_canonical(&graph.canonical_bytes().unwrap()).unwrap(),
            graph
        );
    }
}
