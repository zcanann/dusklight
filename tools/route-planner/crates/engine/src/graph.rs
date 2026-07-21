//! Deterministic planner-native graph projections for browser and tooling clients.

use crate::artifact::Digest;
use crate::evaluation::{
    EvidencePolicy, FeasibilityMode, FeasibilitySelection, PredicateEvaluator,
    TransitionAssessment, TransitionClassification,
};
use crate::execution::PlannerExecutionState;
use crate::identity::EquivalenceSet;
use crate::logic::{ComparisonOperator, FactCatalog, PredicateExpression, ValueReference};
use crate::refinement::ComposedPlannerCatalog;
use crate::route_book::{CollapsePolicy, RouteActionRef, RouteBook};
use crate::transition::{MechanicsCatalog, ObligationDetail, ResolutionKind};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const PLANNER_GRAPH_SCHEMA: &str = "dusklight.route-planner.graph/v6";
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
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannerRegionKind {
    Facts,
    Mechanics,
    Predicate,
    Plan,
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
                },
                PlannerGraphRegion {
                    id: "region.mechanics".into(),
                    label: "Mechanics".into(),
                    parent_region_id: None,
                    owner_node_id: None,
                    region_kind: PlannerRegionKind::Mechanics,
                    collapsed_by_default: false,
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
    use crate::route_book::{
        CollapsePolicy, PlanMethod, PlanRegion, ROUTE_BOOK_SCHEMA, ReferenceStep, RouteActionRef,
        RouteBook, RouteBookManifest,
    };
    use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
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
}
