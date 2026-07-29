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

mod construction;

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
#[path = "graph_tests.rs"]
mod tests;
