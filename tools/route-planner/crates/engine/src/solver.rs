//! Bounded forward state search with explicit feasibility choices and proofs.

use crate::authorization::{AuthorizationGraph, AuthorizationRecorder};
use crate::evaluation::{
    EvaluatedTruth, EvidencePolicy, FeasibilityMode, FeasibilityResolution, FeasibilitySelection,
    PredicateEvaluator, RuleClassification, TransitionAssessment, TransitionClassification,
    WriterAssessment, WriterClassification,
};
use crate::execution::PlannerExecutionState;
use crate::identity::EquivalenceSet;
use crate::logic::{FactCatalog, PredicateExpression, RuleEvidence, TruthStatus};
use crate::relevance::{
    BackwardRelevance, StateDependency, dependencies_overlap, operation_outputs,
};
use crate::route_book::{RouteActionRef, RouteBook, RouteDirectiveKind};
use crate::transition::{
    CandidateTransition, FeasibilityObligation, GateRule, MechanicsCatalog, PathConstraint,
    StateOperation, UnknownRequirement,
};
use crate::{PlannerContractError, artifact::Digest, validate_stable_id};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

#[derive(Clone)]
struct RouteActionSequence {
    steps: Vec<RouteSequenceStep>,
}

#[derive(Clone)]
struct RouteSequenceStep {
    action: RouteActionRef,
    precondition: Option<PredicateExpression>,
    postcondition: Option<PredicateExpression>,
}

struct ActionPreference {
    directive_id: String,
    action: RouteActionRef,
    weight: u32,
}

struct MethodPreference {
    directive_id: String,
    sequence: RouteActionSequence,
    weight: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SolverOptions {
    pub max_depth: usize,
    pub max_states: usize,
    pub max_resolution_combinations: usize,
    pub feasibility_mode: FeasibilityMode,
    pub evidence_policy: EvidencePolicy,
}

fn compile_route_policy(
    book: &RouteBook,
    evaluator: &PredicateEvaluator<'_>,
    base_evidence_policy: EvidencePolicy,
) -> Result<RouteSearchPolicy, PlannerContractError> {
    if !evaluator.scope_applies(&book.manifest.scope) {
        return Err(PlannerContractError::new(
            "route_book.manifest.scope",
            "does not apply to the starting execution context",
        ));
    }
    let mut policy = RouteSearchPolicy {
        required_actions: BTreeSet::new(),
        banned_actions: BTreeSet::new(),
        required_predicates: Vec::new(),
        forbidden_predicates: Vec::new(),
        required_sequences: Vec::new(),
        banned_sequences: Vec::new(),
        action_preferences: Vec::new(),
        method_preferences: Vec::new(),
        cost_limits: BTreeMap::new(),
        minimum_evidence: None,
        evidence_policy: EvidencePolicy::ESTABLISHED_ONLY,
    };
    let mut required_methods = BTreeMap::<String, RouteActionSequence>::new();
    let mut banned_methods = BTreeMap::<String, RouteActionSequence>::new();

    for constraint in book
        .constraints
        .iter()
        .filter(|constraint| evaluator.scope_applies(&constraint.scope))
    {
        match &constraint.constraint {
            PathConstraint::RequirePredicate { predicate } => {
                policy.required_predicates.push(predicate.clone());
            }
            PathConstraint::ForbidPredicate { predicate } => {
                policy.forbidden_predicates.push(predicate.clone());
            }
            PathConstraint::RequireTechnique { technique_id } => {
                policy.required_actions.insert(RouteActionRef::Technique {
                    technique_id: technique_id.clone(),
                });
            }
            PathConstraint::ForbidTechnique { technique_id } => {
                policy.banned_actions.insert(RouteActionRef::Technique {
                    technique_id: technique_id.clone(),
                });
            }
            PathConstraint::CostAtMost { axis, maximum } => {
                policy
                    .cost_limits
                    .entry(axis.clone())
                    .and_modify(|current| *current = (*current).min(*maximum))
                    .or_insert(*maximum);
            }
            PathConstraint::EvidenceAtLeast { minimum } => {
                let minimum = parse_evidence_minimum(minimum)?;
                if policy
                    .minimum_evidence
                    .is_none_or(|current| evidence_quality(minimum) > evidence_quality(current))
                {
                    policy.minimum_evidence = Some(minimum);
                }
            }
        }
    }

    for directive in book
        .directives
        .iter()
        .filter(|directive| evaluator.scope_applies(&directive.scope))
    {
        match &directive.directive {
            RouteDirectiveKind::PinAction { action } => {
                require_searchable_action(action, &directive.id)?;
                policy.required_actions.insert(action.clone());
            }
            RouteDirectiveKind::BanAction { action } => {
                require_searchable_action(action, &directive.id)?;
                policy.banned_actions.insert(action.clone());
            }
            RouteDirectiveKind::PinMethod { method_id } => {
                let sequence = compile_method_sequence(book, method_id, evaluator, true)?
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "route_book.directives.method_id",
                            "required method unexpectedly had no active sequence",
                        )
                    })?;
                required_methods.insert(method_id.clone(), sequence);
            }
            RouteDirectiveKind::BanMethod { method_id } => {
                if let Some(sequence) = compile_method_sequence(book, method_id, evaluator, false)?
                {
                    banned_methods.insert(method_id.clone(), sequence);
                }
            }
            RouteDirectiveKind::PreferAction { action, weight } => {
                require_searchable_action(action, &directive.id)?;
                policy.action_preferences.push(ActionPreference {
                    directive_id: directive.id.clone(),
                    action: action.clone(),
                    weight: *weight,
                });
            }
            RouteDirectiveKind::PreferMethod { method_id, weight } => {
                if let Some(sequence) = compile_method_sequence(book, method_id, evaluator, false)?
                {
                    policy.method_preferences.push(MethodPreference {
                        directive_id: directive.id.clone(),
                        sequence,
                        weight: *weight,
                    });
                }
            }
        }
    }

    for region in book
        .regions
        .iter()
        .filter(|region| evaluator.scope_applies(&region.scope))
    {
        if let Some(method_id) = &region.selected_method_id {
            let sequence =
                compile_method_sequence(book, method_id, evaluator, true)?.ok_or_else(|| {
                    PlannerContractError::new(
                        "route_book.regions.selected_method_id",
                        "selected method unexpectedly had no active sequence",
                    )
                })?;
            required_methods.insert(method_id.clone(), sequence);
        }
    }
    if let Some(action) = policy
        .required_actions
        .intersection(&policy.banned_actions)
        .next()
    {
        return Err(PlannerContractError::new(
            "route_book",
            format!("action {action:?} is both required and banned"),
        ));
    }
    if let Some(method_id) = required_methods
        .keys()
        .find(|method_id| banned_methods.contains_key(*method_id))
    {
        return Err(PlannerContractError::new(
            "route_book",
            format!("method {method_id} is both required and banned"),
        ));
    }
    if let Some((method_id, action)) = required_methods.iter().find_map(|(method_id, sequence)| {
        sequence
            .steps
            .iter()
            .find(|step| policy.banned_actions.contains(&step.action))
            .map(|step| (method_id, &step.action))
    }) {
        return Err(PlannerContractError::new(
            "route_book",
            format!("required method {method_id} contains banned action {action:?}"),
        ));
    }
    policy.required_sequences = required_methods.into_values().collect();
    policy.banned_sequences = banned_methods.into_values().collect();
    policy.evidence_policy =
        evidence_policy_for_minimum(base_evidence_policy, policy.minimum_evidence);
    Ok(policy)
}

fn parse_evidence_minimum(value: &str) -> Result<TruthStatus, PlannerContractError> {
    match value {
        "established" => Ok(TruthStatus::Established),
        "contested" => Ok(TruthStatus::Contested),
        "hypothetical" => Ok(TruthStatus::Hypothetical),
        _ => Err(PlannerContractError::new(
            "route_book.constraints.minimum",
            "must be established, contested, or hypothetical",
        )),
    }
}

fn evidence_quality(status: TruthStatus) -> u8 {
    match status {
        TruthStatus::Established => 3,
        TruthStatus::Contested => 2,
        TruthStatus::Hypothetical => 1,
        TruthStatus::Unknown => 0,
    }
}

fn evidence_policy_for_minimum(
    base: EvidencePolicy,
    minimum: Option<TruthStatus>,
) -> EvidencePolicy {
    let required = match minimum {
        Some(TruthStatus::Established) => EvidencePolicy::ESTABLISHED_ONLY,
        Some(TruthStatus::Contested) => EvidencePolicy {
            allow_contested: true,
            allow_hypothetical: false,
        },
        Some(TruthStatus::Hypothetical) | None => EvidencePolicy::RESEARCH,
        Some(TruthStatus::Unknown) => EvidencePolicy::ESTABLISHED_ONLY,
    };
    EvidencePolicy {
        allow_contested: base.allow_contested && required.allow_contested,
        allow_hypothetical: base.allow_hypothetical && required.allow_hypothetical,
    }
}

fn compile_method_sequence(
    book: &RouteBook,
    method_id: &str,
    evaluator: &PredicateEvaluator<'_>,
    required: bool,
) -> Result<Option<RouteActionSequence>, PlannerContractError> {
    let method = book
        .methods
        .iter()
        .find(|method| method.id == method_id)
        .ok_or_else(|| {
            PlannerContractError::new(
                "route_book.directives.method_id",
                format!("references unknown method {method_id}"),
            )
        })?;
    if !evaluator.scope_applies(&method.scope) {
        if required {
            return Err(PlannerContractError::new(
                "route_book.methods.scope",
                format!("required method {method_id} does not apply to the starting context"),
            ));
        }
        return Ok(None);
    }
    let mut steps = Vec::with_capacity(method.step_ids.len());
    for step_id in &method.step_ids {
        let step = book
            .steps
            .iter()
            .find(|step| step.id == *step_id)
            .ok_or_else(|| {
                PlannerContractError::new(
                    "route_book.methods.step_ids",
                    format!("references unknown step {step_id}"),
                )
            })?;
        require_searchable_action(&step.action, method_id)?;
        steps.push(RouteSequenceStep {
            action: step.action.clone(),
            precondition: step.precondition.clone(),
            postcondition: step.postcondition.clone(),
        });
    }
    Ok(Some(RouteActionSequence { steps }))
}

fn require_searchable_action(
    action: &RouteActionRef,
    directive_id: &str,
) -> Result<(), PlannerContractError> {
    if matches!(
        action,
        RouteActionRef::Transition { .. }
            | RouteActionRef::Technique { .. }
            | RouteActionRef::Resolver { .. }
            | RouteActionRef::Writer { .. }
    ) {
        Ok(())
    } else {
        Err(PlannerContractError::new(
            "route_book.directives",
            format!(
                "directive {directive_id} references an action kind the bounded forward solver cannot execute"
            ),
        ))
    }
}

fn evaluate_all(
    evaluator: &PredicateEvaluator<'_>,
    predicates: &[PredicateExpression],
) -> EvaluatedTruth {
    let mut result = EvaluatedTruth::True;
    for predicate in predicates {
        match evaluator.evaluate(predicate) {
            EvaluatedTruth::False => return EvaluatedTruth::False,
            EvaluatedTruth::Unknown => result = EvaluatedTruth::Unknown,
            EvaluatedTruth::True => {}
        }
    }
    result
}

impl Default for SolverOptions {
    fn default() -> Self {
        Self {
            max_depth: 128,
            max_states: 100_000,
            max_resolution_combinations: 256,
            feasibility_mode: FeasibilityMode::Modeled,
            evidence_policy: EvidencePolicy::ESTABLISHED_ONLY,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchStatus {
    Reached,
    UnreachableUnderModel,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchActionKind {
    Transition,
    Technique,
    Writer,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceDependencyKind {
    Fact,
    Transition,
    Obstruction,
    Obligation,
    Resolver,
    Technique,
    Writer,
    Gate,
    Reader,
    Microtrace,
    UnknownRequirement,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceDependency {
    pub dependency_kind: EvidenceDependencyKind,
    pub record_id: String,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchStep {
    pub action_kind: SearchActionKind,
    pub action_id: String,
    pub selected_resolver_ids: Vec<String>,
    pub selected_technique_ids: Vec<String>,
    pub active_obstruction_ids: Vec<String>,
    pub unknown_obstruction_ids: Vec<String>,
    pub discharged_obligation_ids: Vec<String>,
    pub outstanding_obligation_ids: Vec<String>,
    pub unknown_obligation_ids: Vec<String>,
    pub supporting_microtrace_ids: Vec<String>,
    pub introduced_obligation_ids: Vec<String>,
    pub reader_results: Vec<ReaderResult>,
    pub unknown_reader_ids: Vec<String>,
    pub evidence_dependencies: Vec<EvidenceDependency>,
    pub weakest_evidence: Option<TruthStatus>,
    pub action_derivations: Vec<ActionDerivation>,
    pub obligation_derivations: Vec<FeasibilityObligation>,
    pub source_state_sha256: Digest,
    pub result_state_sha256: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionDerivation {
    pub action: RouteActionRef,
    pub precondition: PredicateExpression,
    pub precondition_result: EvaluatedTruth,
    pub operations: Vec<StateOperation>,
    pub source_state_sha256: Digest,
    pub result_state_sha256: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReaderResult {
    pub reader_id: String,
    pub source_value: crate::state::StateValue,
    pub interpretation: Option<EvaluatedTruth>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlockedTransitionWitness {
    pub transition_id: String,
    pub source_state_sha256: Digest,
    pub classification: TransitionClassification,
    pub hard_guard: EvaluatedTruth,
    pub selected_resolver_ids: Vec<String>,
    pub selected_technique_ids: Vec<String>,
    pub active_obstruction_ids: Vec<String>,
    pub unknown_obstruction_ids: Vec<String>,
    pub discharged_obligation_ids: Vec<String>,
    pub outstanding_obligation_ids: Vec<String>,
    pub unknown_obligation_ids: Vec<String>,
    pub supporting_microtrace_ids: Vec<String>,
    pub unknown_requirement_ids: Vec<String>,
    pub reader_results: Vec<ReaderResult>,
    pub unknown_reader_ids: Vec<String>,
    pub evidence_dependencies: Vec<EvidenceDependency>,
    pub weakest_evidence: Option<TruthStatus>,
    pub hard_guard_expression: PredicateExpression,
    pub effect_operations: Vec<StateOperation>,
    pub obligation_derivations: Vec<FeasibilityObligation>,
    pub unknown_requirements: Vec<UnknownRequirement>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlockedWriterWitness {
    pub writer_id: String,
    pub source_state_sha256: Digest,
    pub classification: WriterClassification,
    pub activation: EvaluatedTruth,
    pub active_gate_ids: Vec<String>,
    pub unknown_gate_ids: Vec<String>,
    pub evidence_dependencies: Vec<EvidenceDependency>,
    pub weakest_evidence: Option<TruthStatus>,
    pub activation_expression: PredicateExpression,
    pub operation: StateOperation,
    pub gate_derivations: Vec<GateRule>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FailedProducerBlocker {
    Transition {
        transition_id: String,
        source_state_sha256: Digest,
        classification: TransitionClassification,
    },
    Writer {
        writer_id: String,
        source_state_sha256: Digest,
        classification: WriterClassification,
    },
}

impl FailedProducerBlocker {
    fn action(&self) -> RouteActionRef {
        match self {
            Self::Transition { transition_id, .. } => RouteActionRef::Transition {
                transition_id: transition_id.clone(),
            },
            Self::Writer { writer_id, .. } => RouteActionRef::Writer {
                writer_id: writer_id.clone(),
            },
        }
    }

    fn validate(&self) -> Result<(), PlannerContractError> {
        let (action, source_state_sha256, executable) = match self {
            Self::Transition {
                source_state_sha256,
                classification,
                ..
            } => (
                self.action(),
                *source_state_sha256,
                *classification == TransitionClassification::Executable,
            ),
            Self::Writer {
                source_state_sha256,
                classification,
                ..
            } => (
                self.action(),
                *source_state_sha256,
                *classification == WriterClassification::Executable,
            ),
        };
        validate_route_action_ref(&action)?;
        if source_state_sha256 == Digest::ZERO || executable {
            return Err(PlannerContractError::new(
                "solver.failed_producer_blocker",
                "must retain a nonzero source state and a non-executable classification",
            ));
        }
        Ok(())
    }
}

/// A conservative action-level cut for one exact state dependency. Every
/// catalog producer represented here was observed blocked and none executed in
/// this bounded solve. Dependencies with an executable or unsupported producer
/// deliberately receive no cut.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FailedProducerCut {
    pub dependency: StateDependency,
    pub blocked_producers: Vec<FailedProducerBlocker>,
}

impl FailedProducerCut {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.blocked_producers.is_empty()
            || matches!(
                self.dependency,
                StateDependency::Fact { .. } | StateDependency::AnyState
            )
        {
            return Err(PlannerContractError::new(
                "solver.failed_producer_cut",
                "requires a concrete dependency and at least one blocked producer",
            ));
        }
        let mut prior = None;
        for blocker in &self.blocked_producers {
            blocker.validate()?;
            let action = blocker.action();
            if prior.as_ref().is_some_and(|candidate| candidate >= &action) {
                return Err(PlannerContractError::new(
                    "solver.failed_producer_cut.blocked_producers",
                    "must be unique and sorted by action identity",
                ));
            }
            prior = Some(action);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchResult {
    pub backward_relevance: BackwardRelevance,
    pub backward_pruning_applied: bool,
    pub status: SearchStatus,
    pub steps: Vec<SearchStep>,
    pub explored_states: usize,
    pub hit_search_limit: bool,
    pub preference_score: u64,
    pub satisfied_preference_ids: Vec<String>,
    pub route_costs: BTreeMap<String, u64>,
    /// Additional nondominated goal plans, ordered by the same deterministic
    /// presentation order as the primary plan. The legacy fields above remain
    /// the primary plan so single-plan consumers do not need a second shape.
    pub alternative_plans: Vec<SearchPlan>,
    pub minimum_evidence: Option<TruthStatus>,
    pub unknown_transition_ids: Vec<String>,
    pub unknown_writer_ids: Vec<String>,
    pub execution_error_ids: Vec<String>,
    pub blocked_transition_witnesses: Vec<BlockedTransitionWitness>,
    pub blocked_writer_witnesses: Vec<BlockedWriterWitness>,
    pub continuation_merge_proofs: Vec<ContinuationMergeProof>,
    pub failed_producer_cuts: Vec<FailedProducerCut>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchPlan {
    pub result_state_sha256: Digest,
    pub steps: Vec<SearchStep>,
    pub preference_score: u64,
    pub satisfied_preference_ids: Vec<String>,
    pub route_costs: BTreeMap<String, u64>,
}

impl SearchPlan {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.result_state_sha256 == Digest::ZERO
            || self
                .steps
                .last()
                .is_some_and(|step| step.result_state_sha256 != self.result_state_sha256)
        {
            return Err(PlannerContractError::new(
                "solver.search_plan.result_state_sha256",
                "must identify the final reached state",
            ));
        }
        let mut prior = None;
        for id in &self.satisfied_preference_ids {
            validate_stable_id("solver.search_plan.satisfied_preference_ids", id)?;
            if prior.is_some_and(|candidate: &String| candidate >= id) {
                return Err(PlannerContractError::new(
                    "solver.search_plan.satisfied_preference_ids",
                    "must be unique and sorted",
                ));
            }
            prior = Some(id);
        }
        resource_label(self.steps.len(), &self.route_costs).validate()
    }
}

/// Everything that can change whether an otherwise identical live state may
/// continue through the remainder of one solve. Resource totals and elapsed
/// search depth are deliberately separate so a Pareto-better label can safely
/// dominate a worse route to this exact continuation.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuationIdentity {
    pub state_sha256: Digest,
    pub satisfied_required_actions: Vec<RouteActionRef>,
    pub required_sequence_progress: Vec<usize>,
    pub banned_sequence_progress: Vec<usize>,
    pub preferred_sequence_progress: Vec<usize>,
    pub satisfied_preference_ids: Vec<String>,
    pub route_condition_unknown: bool,
}

impl ContinuationIdentity {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.state_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "solver.continuation_identity.state_sha256",
                "must be nonzero",
            ));
        }
        let mut prior_action = None;
        for action in &self.satisfied_required_actions {
            if prior_action.is_some_and(|prior| prior >= action) {
                return Err(PlannerContractError::new(
                    "solver.continuation_identity.satisfied_required_actions",
                    "must be unique and sorted",
                ));
            }
            validate_route_action_ref(action)?;
            prior_action = Some(action);
        }
        let mut prior_preference = None;
        for preference in &self.satisfied_preference_ids {
            if prior_preference.is_some_and(|prior: &String| prior >= preference) {
                return Err(PlannerContractError::new(
                    "solver.continuation_identity.satisfied_preference_ids",
                    "must be unique and sorted",
                ));
            }
            validate_stable_id(
                "solver.continuation_identity.satisfied_preference_ids",
                preference,
            )?;
            prior_preference = Some(preference);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchResourceLabel {
    pub depth: usize,
    pub route_costs: BTreeMap<String, u64>,
}

impl SearchResourceLabel {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        for (axis, value) in &self.route_costs {
            validate_stable_id("solver.resource_label.route_costs.axis", axis)?;
            if *value == 0 {
                return Err(PlannerContractError::new(
                    "solver.resource_label.route_costs",
                    "must omit zero-valued axes",
                ));
            }
        }
        Ok(())
    }
}

/// Proof that one queued state was merged into an already explored state with
/// the exact same continuation identity and no worse depth or cost on any axis.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuationMergeProof {
    pub continuation: ContinuationIdentity,
    pub dominating: SearchResourceLabel,
    pub dominated: SearchResourceLabel,
}

impl ContinuationMergeProof {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        self.continuation.validate()?;
        self.dominating.validate()?;
        self.dominated.validate()?;
        if !strictly_dominates(&self.dominating, &self.dominated) {
            return Err(PlannerContractError::new(
                "solver.continuation_merge_proof",
                "requires an exact nonzero continuation identity and a strictly Pareto-better resource label",
            ));
        }
        Ok(())
    }
}

fn validate_route_action_ref(action: &RouteActionRef) -> Result<(), PlannerContractError> {
    let (field, id) = match action {
        RouteActionRef::Transition { transition_id } => (
            "solver.continuation_identity.action.transition_id",
            transition_id,
        ),
        RouteActionRef::Technique { technique_id } => (
            "solver.continuation_identity.action.technique_id",
            technique_id,
        ),
        RouteActionRef::Resolver { resolver_id } => (
            "solver.continuation_identity.action.resolver_id",
            resolver_id,
        ),
        RouteActionRef::Writer { writer_id } => {
            ("solver.continuation_identity.action.writer_id", writer_id)
        }
        RouteActionRef::Microtrace { microtrace_id } => (
            "solver.continuation_identity.action.microtrace_id",
            microtrace_id,
        ),
    };
    validate_stable_id(field, id)
}

struct SearchNode {
    state: PlannerExecutionState,
    steps: Vec<SearchStep>,
    depth: usize,
    satisfied_required_actions: BTreeSet<RouteActionRef>,
    required_sequence_progress: Vec<usize>,
    banned_sequence_progress: Vec<usize>,
    preferred_sequence_progress: Vec<usize>,
    satisfied_preference_ids: BTreeSet<String>,
    preference_score: u64,
    route_condition_unknown: bool,
    route_costs: BTreeMap<String, u64>,
}

struct QueueEntry {
    node: SearchNode,
    insertion_order: u64,
}

struct AppliedActionBoundary {
    action: RouteActionRef,
    before: PlannerExecutionState,
    after: PlannerExecutionState,
}

impl PartialEq for QueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.node.depth == other.node.depth
            && self.node.preference_score == other.node.preference_score
            && self.insertion_order == other.insertion_order
    }
}

impl Eq for QueueEntry {}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .node
            .depth
            .cmp(&self.node.depth)
            .then_with(|| self.node.preference_score.cmp(&other.node.preference_score))
            .then_with(|| other.insertion_order.cmp(&self.insertion_order))
    }
}

#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct SearchIdentity {
    continuation: ContinuationIdentity,
    route_costs: BTreeMap<String, u64>,
}

fn continuation_identity(node: &SearchNode, state_sha256: Digest) -> ContinuationIdentity {
    ContinuationIdentity {
        state_sha256,
        satisfied_required_actions: node.satisfied_required_actions.iter().cloned().collect(),
        required_sequence_progress: node.required_sequence_progress.clone(),
        banned_sequence_progress: node.banned_sequence_progress.clone(),
        preferred_sequence_progress: node.preferred_sequence_progress.clone(),
        satisfied_preference_ids: node.satisfied_preference_ids.iter().cloned().collect(),
        route_condition_unknown: node.route_condition_unknown,
    }
}

fn resource_label(depth: usize, route_costs: &BTreeMap<String, u64>) -> SearchResourceLabel {
    SearchResourceLabel {
        depth,
        route_costs: route_costs
            .iter()
            .filter(|(_, value)| **value != 0)
            .map(|(axis, value)| (axis.clone(), *value))
            .collect(),
    }
}

fn strictly_dominates(left: &SearchResourceLabel, right: &SearchResourceLabel) -> bool {
    if left.depth > right.depth {
        return false;
    }
    let mut strict = left.depth < right.depth;
    for axis in left
        .route_costs
        .keys()
        .chain(right.route_costs.keys())
        .collect::<BTreeSet<_>>()
    {
        let left_cost = left.route_costs.get(axis).copied().unwrap_or(0);
        let right_cost = right.route_costs.get(axis).copied().unwrap_or(0);
        if left_cost > right_cost {
            return false;
        }
        strict |= left_cost < right_cost;
    }
    strict
}

fn plan_strictly_dominates(left: &SearchPlan, right: &SearchPlan) -> bool {
    let left_resources = resource_label(left.steps.len(), &left.route_costs);
    let right_resources = resource_label(right.steps.len(), &right.route_costs);
    let resources_no_worse = left_resources.depth <= right_resources.depth
        && left_resources
            .route_costs
            .keys()
            .chain(right_resources.route_costs.keys())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .all(|axis| {
                left_resources.route_costs.get(axis).copied().unwrap_or(0)
                    <= right_resources.route_costs.get(axis).copied().unwrap_or(0)
            });
    resources_no_worse
        && left.preference_score >= right.preference_score
        && (strictly_dominates(&left_resources, &right_resources)
            || left.preference_score > right.preference_score)
}

fn search_plan_signature(plan: &SearchPlan) -> Vec<(SearchActionKind, String)> {
    plan.steps
        .iter()
        .map(|step| (step.action_kind, step.action_id.clone()))
        .collect()
}

fn retain_nondominated_plan(plans: &mut Vec<SearchPlan>, candidate: SearchPlan) {
    if plans
        .iter()
        .any(|plan| plan == &candidate || plan_strictly_dominates(plan, &candidate))
    {
        return;
    }
    plans.retain(|plan| !plan_strictly_dominates(&candidate, plan));
    plans.push(candidate);
}

fn order_search_plans(plans: &mut [SearchPlan]) {
    plans.sort_by(|left, right| {
        left.steps
            .len()
            .cmp(&right.steps.len())
            .then_with(|| right.preference_score.cmp(&left.preference_score))
            .then_with(|| left.route_costs.cmp(&right.route_costs))
            .then_with(|| search_plan_signature(left).cmp(&search_plan_signature(right)))
            .then_with(|| left.result_state_sha256.cmp(&right.result_state_sha256))
    });
}

struct RouteSearchPolicy {
    required_actions: BTreeSet<RouteActionRef>,
    banned_actions: BTreeSet<RouteActionRef>,
    required_predicates: Vec<PredicateExpression>,
    forbidden_predicates: Vec<PredicateExpression>,
    required_sequences: Vec<RouteActionSequence>,
    banned_sequences: Vec<RouteActionSequence>,
    action_preferences: Vec<ActionPreference>,
    method_preferences: Vec<MethodPreference>,
    cost_limits: BTreeMap<String, u64>,
    minimum_evidence: Option<TruthStatus>,
    evidence_policy: EvidencePolicy,
}

pub struct ForwardSolver<'a> {
    facts: &'a FactCatalog,
    mechanics: &'a MechanicsCatalog,
    equivalence_sets: &'a [EquivalenceSet],
    options: SolverOptions,
    route_book: Option<&'a RouteBook>,
}

impl<'a> ForwardSolver<'a> {
    pub fn new(
        facts: &'a FactCatalog,
        mechanics: &'a MechanicsCatalog,
        equivalence_sets: &'a [EquivalenceSet],
        options: SolverOptions,
    ) -> Result<Self, PlannerContractError> {
        facts.validate()?;
        mechanics.validate()?;
        for set in equivalence_sets {
            set.validate()?;
        }
        if options.max_depth == 0
            || options.max_states == 0
            || options.max_resolution_combinations == 0
        {
            return Err(PlannerContractError::new(
                "solver.options",
                "all search bounds must be nonzero",
            ));
        }
        Ok(Self {
            facts,
            mechanics,
            equivalence_sets,
            options,
            route_book: None,
        })
    }

    pub fn new_with_route_book(
        facts: &'a FactCatalog,
        mechanics: &'a MechanicsCatalog,
        equivalence_sets: &'a [EquivalenceSet],
        options: SolverOptions,
        route_book: &'a RouteBook,
    ) -> Result<Self, PlannerContractError> {
        route_book.validate_against(facts, mechanics)?;
        let mut solver = Self::new(facts, mechanics, equivalence_sets, options)?;
        solver.route_book = Some(route_book);
        Ok(solver)
    }

    pub fn solve(
        &self,
        start: PlannerExecutionState,
        goal: &PredicateExpression,
    ) -> Result<SearchResult, PlannerContractError> {
        self.solve_internal(start, goal, &[], None, 1)
    }

    /// Complete the bounded search and return up to `max_plans` deterministic
    /// Pareto plans. Depth and every named route-cost axis are minimized while
    /// route-book preference score is maximized. Incomparable tradeoffs remain
    /// alternatives; strictly dominated goal plans are omitted.
    pub fn solve_alternatives(
        &self,
        start: PlannerExecutionState,
        goal: &PredicateExpression,
        max_plans: usize,
    ) -> Result<SearchResult, PlannerContractError> {
        if max_plans == 0 {
            return Err(PlannerContractError::new(
                "solver.max_plans",
                "must be nonzero",
            ));
        }
        self.solve_internal(start, goal, &[], None, max_plans)
    }

    /// Enumerate the bounded, permissive authorization graph without a goal-
    /// directed slice. Unknown guards, readers, requirements, and disallowed
    /// evidence remain blocked; physical obligations and obstructions are the
    /// only constraints relaxed by upper-bound evaluation.
    pub fn authorization_graph(
        &self,
        start: PlannerExecutionState,
    ) -> Result<AuthorizationGraph, PlannerContractError> {
        if self.options.feasibility_mode != FeasibilityMode::UpperBound {
            return Err(PlannerContractError::new(
                "authorization_graph.feasibility_mode",
                "requires upper-bound evaluation",
            ));
        }
        if self.route_book.is_some() {
            return Err(PlannerContractError::new(
                "authorization_graph.route_book",
                "cannot apply route-specific pruning or preferences",
            ));
        }
        start.validate()?;
        let initial_state_sha256 = start.semantic_digest()?;
        let initial_execution_state_sha256 = start.digest()?;
        let mut action_roots =
            self.mechanics
                .transitions
                .iter()
                .map(|transition| RouteActionRef::Transition {
                    transition_id: transition.id.clone(),
                })
                .chain(
                    self.mechanics
                        .writers
                        .iter()
                        .map(|writer| RouteActionRef::Writer {
                            writer_id: writer.id.clone(),
                        }),
                )
                .chain(self.mechanics.techniques.iter().map(|technique| {
                    RouteActionRef::Technique {
                        technique_id: technique.id.clone(),
                    }
                }))
                .collect::<Vec<_>>();
        action_roots.sort();
        action_roots.dedup();
        let mut recorder = AuthorizationRecorder::default();
        let search = self.solve_internal(
            start,
            &PredicateExpression::False,
            &action_roots,
            Some(&mut recorder),
            1,
        )?;
        let mut equivalence_set_sha256 = self
            .equivalence_sets
            .iter()
            .map(EquivalenceSet::digest)
            .collect::<Result<Vec<_>, _>>()?;
        equivalence_set_sha256.sort();
        equivalence_set_sha256.dedup();
        AuthorizationGraph::finish(
            recorder,
            initial_state_sha256,
            initial_execution_state_sha256,
            self.facts,
            self.mechanics,
            equivalence_set_sha256,
            self.options.evidence_policy,
            self.options,
            &search,
        )
    }

    fn solve_internal(
        &self,
        start: PlannerExecutionState,
        goal: &PredicateExpression,
        additional_action_roots: &[RouteActionRef],
        mut authorization: Option<&mut AuthorizationRecorder>,
        max_plans: usize,
    ) -> Result<SearchResult, PlannerContractError> {
        start.validate()?;
        let start_evaluator = PredicateEvaluator::new(
            &start.snapshot,
            self.facts,
            self.equivalence_sets,
            &start.gate_states,
            self.options.evidence_policy,
        )?;
        let route_policy = self
            .route_book
            .map(|book| compile_route_policy(book, &start_evaluator, self.options.evidence_policy))
            .transpose()?;
        let mut backward_predicate_roots = Vec::new();
        let mut backward_action_roots = BTreeSet::new();
        if let Some(policy) = &route_policy {
            backward_predicate_roots.extend(policy.required_predicates.iter().cloned());
            backward_action_roots.extend(policy.required_actions.iter().cloned());
            for sequence in &policy.required_sequences {
                for step in &sequence.steps {
                    backward_action_roots.insert(step.action.clone());
                    backward_predicate_roots.extend(step.precondition.iter().cloned());
                    backward_predicate_roots.extend(step.postcondition.iter().cloned());
                }
            }
        }
        backward_action_roots.extend(additional_action_roots.iter().cloned());
        let backward_action_roots = backward_action_roots.into_iter().collect::<Vec<_>>();
        let backward_relevance = BackwardRelevance::analyze_with_roots(
            self.facts,
            self.mechanics,
            goal,
            &backward_predicate_roots,
            &backward_action_roots,
        )?;
        let backward_pruning_applied = true;
        let search_evidence_policy = route_policy
            .as_ref()
            .map_or(self.options.evidence_policy, |policy| {
                policy.evidence_policy
            });
        let initial_node = SearchNode {
            state: start,
            steps: Vec::new(),
            depth: 0,
            satisfied_required_actions: BTreeSet::new(),
            required_sequence_progress: vec![
                0;
                route_policy.as_ref().map_or(0, |policy| policy
                    .required_sequences
                    .len())
            ],
            banned_sequence_progress: vec![
                0;
                route_policy
                    .as_ref()
                    .map_or(0, |policy| policy.banned_sequences.len())
            ],
            preferred_sequence_progress: vec![
                0;
                route_policy.as_ref().map_or(0, |policy| policy
                    .method_preferences
                    .len())
            ],
            satisfied_preference_ids: BTreeSet::new(),
            preference_score: 0,
            route_condition_unknown: false,
            route_costs: BTreeMap::new(),
        };
        let mut queue = BinaryHeap::from([QueueEntry {
            node: initial_node,
            insertion_order: 0,
        }]);
        let mut visited = BTreeSet::new();
        let mut resource_frontier =
            BTreeMap::<ContinuationIdentity, Vec<SearchResourceLabel>>::new();
        let mut continuation_merge_proofs = Vec::new();
        let mut unknown_transition_ids = BTreeSet::new();
        let mut unknown_writer_ids = BTreeSet::new();
        let mut execution_error_ids = BTreeSet::new();
        let mut blocked_transition_witnesses = BTreeMap::new();
        let mut blocked_writer_witnesses = BTreeMap::new();
        let mut executed_actions = BTreeSet::new();
        let mut saw_unknown_goal = false;
        let mut hit_search_limit = false;
        let mut generated_id = 0_u64;
        let mut reached_plans = Vec::new();

        while let Some(QueueEntry { node, .. }) = queue.pop() {
            let state_identity = node.state.semantic_digest()?;
            let continuation = continuation_identity(&node, state_identity);
            let search_identity = SearchIdentity {
                continuation: continuation.clone(),
                route_costs: node.route_costs.clone(),
            };
            let candidate_resources = resource_label(node.depth, &node.route_costs);
            if let Some(dominating) = resource_frontier
                .get(&continuation)
                .and_then(|labels| {
                    labels
                        .iter()
                        .find(|label| strictly_dominates(label, &candidate_resources))
                })
                .cloned()
            {
                if continuation_merge_proofs.len() == self.options.max_states {
                    hit_search_limit = true;
                    break;
                }
                let proof = ContinuationMergeProof {
                    continuation,
                    dominating,
                    dominated: candidate_resources,
                };
                proof.validate()?;
                continuation_merge_proofs.push(proof);
                continue;
            }
            if visited.contains(&search_identity) {
                continue;
            }
            if visited.len() == self.options.max_states {
                hit_search_limit = true;
                break;
            }
            visited.insert(search_identity);
            let labels = resource_frontier.entry(continuation).or_default();
            labels.retain(|label| !strictly_dominates(&candidate_resources, label));
            labels.push(candidate_resources);
            if let Some(recorder) = authorization.as_deref_mut() {
                recorder.observe_state(
                    state_identity,
                    node.state.digest()?,
                    node.state.snapshot.digest()?,
                    node.depth,
                    true,
                );
            }

            let evaluator = PredicateEvaluator::new(
                &node.state.snapshot,
                self.facts,
                self.equivalence_sets,
                &node.state.gate_states,
                search_evidence_policy,
            )?;
            if let Some(policy) = &route_policy {
                let mut forbidden = false;
                for predicate in &policy.forbidden_predicates {
                    match evaluator.evaluate(predicate) {
                        EvaluatedTruth::True => {
                            forbidden = true;
                            break;
                        }
                        EvaluatedTruth::Unknown => {
                            saw_unknown_goal = true;
                            forbidden = true;
                            break;
                        }
                        EvaluatedTruth::False => {}
                    }
                }
                if forbidden {
                    continue;
                }
            }
            let required_predicates = route_policy
                .as_ref()
                .map_or(EvaluatedTruth::True, |policy| {
                    evaluate_all(&evaluator, &policy.required_predicates)
                });
            let required_actions_satisfied = route_policy.as_ref().is_none_or(|policy| {
                policy
                    .required_actions
                    .is_subset(&node.satisfied_required_actions)
            });
            let required_sequences_satisfied = route_policy.as_ref().is_none_or(|policy| {
                policy
                    .required_sequences
                    .iter()
                    .zip(&node.required_sequence_progress)
                    .all(|(sequence, progress)| *progress == sequence.steps.len())
            });
            match evaluator.evaluate(goal) {
                EvaluatedTruth::True
                    if required_predicates == EvaluatedTruth::True
                        && required_actions_satisfied
                        && required_sequences_satisfied
                        && !node.route_condition_unknown =>
                {
                    if max_plans > 1 {
                        let plan = SearchPlan {
                            result_state_sha256: state_identity,
                            steps: node.steps,
                            preference_score: node.preference_score,
                            satisfied_preference_ids: node
                                .satisfied_preference_ids
                                .into_iter()
                                .collect(),
                            route_costs: node.route_costs,
                        };
                        plan.validate()?;
                        retain_nondominated_plan(&mut reached_plans, plan);
                        continue;
                    }
                    return Ok(SearchResult {
                        backward_relevance,
                        backward_pruning_applied,
                        status: SearchStatus::Reached,
                        steps: node.steps,
                        explored_states: visited.len(),
                        hit_search_limit: false,
                        preference_score: node.preference_score,
                        satisfied_preference_ids: node
                            .satisfied_preference_ids
                            .into_iter()
                            .collect(),
                        route_costs: node.route_costs,
                        alternative_plans: Vec::new(),
                        minimum_evidence: route_policy
                            .as_ref()
                            .and_then(|policy| policy.minimum_evidence),
                        unknown_transition_ids: unknown_transition_ids.into_iter().collect(),
                        unknown_writer_ids: unknown_writer_ids.into_iter().collect(),
                        execution_error_ids: execution_error_ids.into_iter().collect(),
                        blocked_transition_witnesses: Vec::new(),
                        blocked_writer_witnesses: Vec::new(),
                        continuation_merge_proofs,
                        failed_producer_cuts: Vec::new(),
                    });
                }
                EvaluatedTruth::True => {
                    if required_predicates == EvaluatedTruth::Unknown
                        || node.route_condition_unknown
                    {
                        saw_unknown_goal = true;
                    }
                }
                EvaluatedTruth::Unknown => saw_unknown_goal = true,
                EvaluatedTruth::False => {}
            }
            if node.depth >= self.options.max_depth {
                if (!backward_pruning_applied
                    && (!self.mechanics.transitions.is_empty()
                        || !self.mechanics.techniques.is_empty()
                        || !self.mechanics.writers.is_empty()))
                    || (backward_pruning_applied
                        && (!backward_relevance.transition_ids.is_empty()
                            || !backward_relevance.technique_ids.is_empty()
                            || !backward_relevance.writer_ids.is_empty()))
                {
                    hit_search_limit = true;
                }
                continue;
            }

            // Writer records are standalone engine actions. Their activation
            // and every gate that names them are reevaluated against each
            // concrete state before the operation is applied.
            for writer in &self.mechanics.writers {
                if backward_pruning_applied && !backward_relevance.contains_writer(&writer.id) {
                    continue;
                }
                let action = RouteActionRef::Writer {
                    writer_id: writer.id.clone(),
                };
                if route_policy
                    .as_ref()
                    .is_some_and(|policy| policy.banned_actions.contains(&action))
                {
                    continue;
                }
                let assessment = evaluator.assess_writer(writer, &self.mechanics.gates);
                if assessment.classification != WriterClassification::Executable {
                    if matches!(
                        assessment.classification,
                        WriterClassification::ActivationUnknown | WriterClassification::GateUnknown
                    ) {
                        unknown_writer_ids.insert(writer.id.clone());
                    }
                    if matches!(
                        assessment.classification,
                        WriterClassification::ActivationUnknown
                            | WriterClassification::GateBlocked
                            | WriterClassification::GateUnknown
                    ) {
                        let evidence_dependencies = writer_evidence_dependencies(
                            self.facts,
                            self.mechanics,
                            writer,
                            &assessment,
                        );
                        blocked_writer_witnesses.insert(
                            writer.id.clone(),
                            BlockedWriterWitness {
                                writer_id: writer.id.clone(),
                                source_state_sha256: state_identity,
                                classification: assessment.classification,
                                activation: assessment.activation,
                                active_gate_ids: assessment.active_gate_ids,
                                unknown_gate_ids: assessment.unknown_gate_ids,
                                weakest_evidence: weakest_evidence(&evidence_dependencies),
                                evidence_dependencies,
                                activation_expression: writer.activation.clone(),
                                operation: writer.operation.clone(),
                                gate_derivations: self
                                    .mechanics
                                    .gates
                                    .iter()
                                    .filter(|gate| gate.blocked_writer_ids.contains(&writer.id))
                                    .cloned()
                                    .collect(),
                            },
                        );
                    }
                    continue;
                }
                let mut next = node.state.clone();
                generated_id = generated_id.saturating_add(1);
                if next
                    .apply_operations(
                        &writer.id,
                        &format!("search-state-{generated_id}"),
                        std::slice::from_ref(&writer.operation),
                    )
                    .is_err()
                {
                    execution_error_ids.insert(writer.id.clone());
                    continue;
                }
                let boundary = AppliedActionBoundary {
                    action: action.clone(),
                    before: node.state.clone(),
                    after: next.clone(),
                };
                executed_actions.insert(action);
                let evidence_dependencies =
                    writer_evidence_dependencies(self.facts, self.mechanics, writer, &assessment);
                let weakest_evidence = weakest_evidence(&evidence_dependencies);
                saw_unknown_goal |= self.enqueue_if_new(
                    &mut queue,
                    &visited,
                    &node,
                    next,
                    std::slice::from_ref(&boundary),
                    route_policy.as_ref(),
                    generated_id,
                    authorization.as_deref_mut(),
                    SearchStep {
                        action_kind: SearchActionKind::Writer,
                        action_id: writer.id.clone(),
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
                        evidence_dependencies,
                        weakest_evidence,
                        action_derivations: Vec::new(),
                        obligation_derivations: Vec::new(),
                        source_state_sha256: state_identity,
                        result_state_sha256: Digest::ZERO,
                    },
                )?;
            }

            // Techniques with concrete state operations are also standalone
            // actions. Their obligation annotations are action-local and are
            // considered separately when combining a technique with a target
            // transition below.
            for technique in &self.mechanics.techniques {
                if backward_pruning_applied && !backward_relevance.contains_technique(&technique.id)
                {
                    continue;
                }
                let action = RouteActionRef::Technique {
                    technique_id: technique.id.clone(),
                };
                if route_policy
                    .as_ref()
                    .is_some_and(|policy| policy.banned_actions.contains(&action))
                {
                    continue;
                }
                if evaluator.assess_technique(technique).classification
                    != RuleClassification::Active
                    || technique.operations.is_empty()
                {
                    continue;
                }
                let mut next = node.state.clone();
                generated_id = generated_id.saturating_add(1);
                if next
                    .apply_operations(
                        &technique.id,
                        &format!("search-state-{generated_id}"),
                        &technique.operations,
                    )
                    .is_err()
                {
                    execution_error_ids.insert(technique.id.clone());
                    continue;
                }
                let boundary = AppliedActionBoundary {
                    action: action.clone(),
                    before: node.state.clone(),
                    after: next.clone(),
                };
                executed_actions.insert(action);
                let evidence_dependencies =
                    technique_evidence_dependencies(self.facts, self.mechanics, technique);
                let weakest_evidence = weakest_evidence(&evidence_dependencies);
                saw_unknown_goal |= self.enqueue_if_new(
                    &mut queue,
                    &visited,
                    &node,
                    next,
                    std::slice::from_ref(&boundary),
                    route_policy.as_ref(),
                    generated_id,
                    authorization.as_deref_mut(),
                    SearchStep {
                        action_kind: SearchActionKind::Technique,
                        action_id: technique.id.clone(),
                        selected_resolver_ids: Vec::new(),
                        selected_technique_ids: vec![technique.id.clone()],
                        active_obstruction_ids: Vec::new(),
                        unknown_obstruction_ids: Vec::new(),
                        discharged_obligation_ids: technique.discharged_obligation_ids.clone(),
                        outstanding_obligation_ids: Vec::new(),
                        unknown_obligation_ids: Vec::new(),
                        supporting_microtrace_ids: Vec::new(),
                        introduced_obligation_ids: technique.introduced_obligation_ids.clone(),
                        reader_results: Vec::new(),
                        unknown_reader_ids: Vec::new(),
                        evidence_dependencies,
                        weakest_evidence,
                        action_derivations: Vec::new(),
                        obligation_derivations: Vec::new(),
                        source_state_sha256: state_identity,
                        result_state_sha256: Digest::ZERO,
                    },
                )?;
            }

            for transition in &self.mechanics.transitions {
                if backward_pruning_applied
                    && !backward_relevance.contains_transition(&transition.id)
                {
                    continue;
                }
                let transition_action = RouteActionRef::Transition {
                    transition_id: transition.id.clone(),
                };
                if route_policy
                    .as_ref()
                    .is_some_and(|policy| policy.banned_actions.contains(&transition_action))
                {
                    continue;
                }
                let applicable_resolver_ids = self
                    .mechanics
                    .resolvers
                    .iter()
                    .filter(|resolver| {
                        evaluator.assess_resolver(resolver).classification
                            == RuleClassification::Active
                    })
                    .filter(|resolver| {
                        !route_policy.as_ref().is_some_and(|policy| {
                            policy.banned_actions.contains(&RouteActionRef::Resolver {
                                resolver_id: resolver.id.clone(),
                            })
                        })
                    })
                    .filter(|resolver| {
                        self.mechanics.obstructions.iter().any(|obstruction| {
                            obstruction.id == resolver.obstruction_id
                                && obstruction.blocked_action_id == transition.id
                                && obstruction.approach_id == transition.approach_id
                        })
                    })
                    .map(|resolver| resolver.id.clone())
                    .collect::<Vec<_>>();
                let applicable_technique_ids =
                    self.mechanics
                        .techniques
                        .iter()
                        .filter(|technique| {
                            evaluator.assess_technique(technique).classification
                                == RuleClassification::Active
                        })
                        .filter(|technique| {
                            !route_policy.as_ref().is_some_and(|policy| {
                                policy.banned_actions.contains(&RouteActionRef::Technique {
                                    technique_id: technique.id.clone(),
                                })
                            })
                        })
                        .filter(|technique| {
                            technique.discharged_obligation_ids.iter().any(|id| {
                                transition.activation.physical_obligation_ids.contains(id)
                            })
                        })
                        .map(|technique| technique.id.clone())
                        .collect::<Vec<_>>();
                let resolver_selections = bounded_subsets(
                    &applicable_resolver_ids,
                    self.options.max_resolution_combinations,
                );
                let technique_selections = bounded_subsets(
                    &applicable_technique_ids,
                    self.options.max_resolution_combinations,
                );
                let mut combinations = 0_usize;
                for selected_resolvers in &resolver_selections {
                    for selected_techniques in &technique_selections {
                        combinations += 1;
                        if combinations > self.options.max_resolution_combinations {
                            hit_search_limit = true;
                            break;
                        }
                        let mut resolution = evaluator.resolve_feasibility(
                            transition,
                            &self.mechanics.obligations,
                            &self.mechanics.obstructions,
                            &self.mechanics.resolvers,
                            &self.mechanics.techniques,
                            FeasibilitySelection {
                                resolver_ids: selected_resolvers,
                                technique_ids: selected_techniques,
                                already_discharged: &BTreeSet::new(),
                                microtraces: &self.mechanics.microtraces,
                            },
                        );
                        let unresolved_active_obstruction = resolution
                            .active_obstruction_ids
                            .iter()
                            .any(|obstruction_id| {
                                !self.mechanics.resolvers.iter().any(|resolver| {
                                    resolver.obstruction_id == *obstruction_id
                                        && resolution.applied_resolver_ids.contains(&resolver.id)
                                })
                            });
                        if self.options.feasibility_mode == FeasibilityMode::Modeled
                            && (!resolution.unknown_obstruction_ids.is_empty()
                                || unresolved_active_obstruction)
                        {
                            if !resolution.unknown_obstruction_ids.is_empty() {
                                unknown_transition_ids.insert(transition.id.clone());
                            }
                            let preliminary = evaluator.assess_transition(
                                transition,
                                &resolution.discharged_obligation_ids,
                                &resolution.unknown_obligation_ids,
                                self.options.feasibility_mode,
                            );
                            let (reader_results, unknown_reader_ids) = assess_transition_readers(
                                &evaluator,
                                self.mechanics,
                                &transition.id,
                            );
                            let evidence_dependencies = transition_evidence_dependencies(
                                self.facts,
                                self.mechanics,
                                transition,
                                &resolution,
                                &preliminary,
                                &reader_results,
                                &unknown_reader_ids,
                            );
                            let weakest_evidence = weakest_evidence(&evidence_dependencies);
                            record_blocked_transition_witness(
                                &mut blocked_transition_witnesses,
                                BlockedTransitionWitness {
                                    transition_id: transition.id.clone(),
                                    source_state_sha256: state_identity,
                                    classification: if !resolution
                                        .unknown_obstruction_ids
                                        .is_empty()
                                    {
                                        TransitionClassification::FeasibilityUnknown
                                    } else {
                                        TransitionClassification::Obstructed
                                    },
                                    hard_guard: preliminary.hard_guard,
                                    selected_resolver_ids: resolution.applied_resolver_ids.clone(),
                                    selected_technique_ids: resolution
                                        .applicable_technique_ids
                                        .clone(),
                                    active_obstruction_ids: resolution
                                        .active_obstruction_ids
                                        .clone(),
                                    unknown_obstruction_ids: resolution
                                        .unknown_obstruction_ids
                                        .clone(),
                                    discharged_obligation_ids: resolution
                                        .discharged_obligation_ids
                                        .iter()
                                        .cloned()
                                        .collect(),
                                    outstanding_obligation_ids: preliminary
                                        .outstanding_obligation_ids,
                                    unknown_obligation_ids: preliminary.unknown_obligation_ids,
                                    supporting_microtrace_ids: resolution
                                        .supporting_microtrace_ids
                                        .iter()
                                        .cloned()
                                        .collect(),
                                    unknown_requirement_ids: preliminary.unknown_requirement_ids,
                                    reader_results,
                                    unknown_reader_ids,
                                    evidence_dependencies,
                                    weakest_evidence,
                                    hard_guard_expression: transition
                                        .activation
                                        .hard_guards
                                        .clone(),
                                    effect_operations: transition.activation.effects.clone(),
                                    obligation_derivations: self
                                        .mechanics
                                        .obligations
                                        .iter()
                                        .filter(|obligation| {
                                            transition
                                                .activation
                                                .physical_obligation_ids
                                                .contains(&obligation.id)
                                        })
                                        .cloned()
                                        .collect(),
                                    unknown_requirements: transition
                                        .activation
                                        .unknown_requirements
                                        .clone(),
                                },
                            );
                            continue;
                        }

                        let mut next = node.state.clone();
                        let mut action_boundaries = Vec::new();
                        let mut setup_failed = false;
                        for resolver_id in &resolution.applied_resolver_ids {
                            let resolver = self
                                .mechanics
                                .resolvers
                                .iter()
                                .find(|resolver| resolver.id == *resolver_id)
                                .ok_or_else(|| {
                                    PlannerContractError::new(
                                        "solver.resolver",
                                        "feasibility selected an unknown resolver",
                                    )
                                })?;
                            let before = next.clone();
                            generated_id = generated_id.saturating_add(1);
                            if !resolver.operations.is_empty()
                                && next
                                    .apply_operations(
                                        &resolver.id,
                                        &format!("search-setup-{generated_id}"),
                                        &resolver.operations,
                                    )
                                    .is_err()
                            {
                                execution_error_ids.insert(resolver.id.clone());
                                setup_failed = true;
                                break;
                            }
                            action_boundaries.push(AppliedActionBoundary {
                                action: RouteActionRef::Resolver {
                                    resolver_id: resolver.id.clone(),
                                },
                                before,
                                after: next.clone(),
                            });
                        }
                        if setup_failed {
                            continue;
                        }
                        for technique_id in &resolution.applicable_technique_ids {
                            let technique = self
                                .mechanics
                                .techniques
                                .iter()
                                .find(|technique| technique.id == *technique_id)
                                .ok_or_else(|| {
                                    PlannerContractError::new(
                                        "solver.technique",
                                        "feasibility selected an unknown technique",
                                    )
                                })?;
                            let before = next.clone();
                            generated_id = generated_id.saturating_add(1);
                            if !technique.operations.is_empty()
                                && next
                                    .apply_operations(
                                        &technique.id,
                                        &format!("search-setup-{generated_id}"),
                                        &technique.operations,
                                    )
                                    .is_err()
                            {
                                execution_error_ids.insert(technique.id.clone());
                                setup_failed = true;
                                break;
                            }
                            action_boundaries.push(AppliedActionBoundary {
                                action: RouteActionRef::Technique {
                                    technique_id: technique.id.clone(),
                                },
                                before,
                                after: next.clone(),
                            });
                        }
                        if setup_failed {
                            continue;
                        }
                        let post_setup_evaluator = PredicateEvaluator::new(
                            &next.snapshot,
                            self.facts,
                            self.equivalence_sets,
                            &next.gate_states,
                            search_evidence_policy,
                        )?;
                        post_setup_evaluator.refresh_obligation_assessments(
                            transition,
                            &self.mechanics.obligations,
                            &self.mechanics.microtraces,
                            &mut resolution,
                        );
                        let assessment = post_setup_evaluator.assess_transition(
                            transition,
                            &resolution.discharged_obligation_ids,
                            &resolution.unknown_obligation_ids,
                            self.options.feasibility_mode,
                        );
                        let (reader_results, unknown_reader_ids) = assess_transition_readers(
                            &post_setup_evaluator,
                            self.mechanics,
                            &transition.id,
                        );
                        if !unknown_reader_ids.is_empty() {
                            unknown_transition_ids.insert(transition.id.clone());
                            let mut witness = blocked_witness(
                                self.facts,
                                self.mechanics,
                                transition,
                                &next,
                                &resolution,
                                &assessment,
                                (&reader_results, &unknown_reader_ids),
                            )?;
                            witness.classification = TransitionClassification::FeasibilityUnknown;
                            record_blocked_transition_witness(
                                &mut blocked_transition_witnesses,
                                witness,
                            );
                            continue;
                        }
                        match assessment.classification {
                            TransitionClassification::Executable => {}
                            TransitionClassification::FeasibilityUnknown => {
                                unknown_transition_ids.insert(transition.id.clone());
                                record_blocked_transition_witness(
                                    &mut blocked_transition_witnesses,
                                    blocked_witness(
                                        self.facts,
                                        self.mechanics,
                                        transition,
                                        &next,
                                        &resolution,
                                        &assessment,
                                        (&reader_results, &unknown_reader_ids),
                                    )?,
                                );
                                continue;
                            }
                            TransitionClassification::Inapplicable
                            | TransitionClassification::GuardBlocked
                            | TransitionClassification::Obstructed => {
                                record_blocked_transition_witness(
                                    &mut blocked_transition_witnesses,
                                    blocked_witness(
                                        self.facts,
                                        self.mechanics,
                                        transition,
                                        &next,
                                        &resolution,
                                        &assessment,
                                        (&reader_results, &unknown_reader_ids),
                                    )?,
                                );
                                continue;
                            }
                        }

                        generated_id = generated_id.saturating_add(1);
                        let transition_before = next.clone();
                        if next
                            .apply_operations(
                                &transition.id,
                                &format!("search-state-{generated_id}"),
                                &transition.activation.effects,
                            )
                            .is_err()
                        {
                            execution_error_ids.insert(transition.id.clone());
                            continue;
                        }
                        action_boundaries.push(AppliedActionBoundary {
                            action: transition_action.clone(),
                            before: transition_before,
                            after: next.clone(),
                        });
                        executed_actions.insert(transition_action.clone());
                        let evidence_dependencies = transition_evidence_dependencies(
                            self.facts,
                            self.mechanics,
                            transition,
                            &resolution,
                            &assessment,
                            &reader_results,
                            &unknown_reader_ids,
                        );
                        let weakest_evidence = weakest_evidence(&evidence_dependencies);
                        saw_unknown_goal |= self.enqueue_if_new(
                            &mut queue,
                            &visited,
                            &node,
                            next,
                            &action_boundaries,
                            route_policy.as_ref(),
                            generated_id,
                            authorization.as_deref_mut(),
                            SearchStep {
                                action_kind: SearchActionKind::Transition,
                                action_id: transition.id.clone(),
                                selected_resolver_ids: resolution.applied_resolver_ids.clone(),
                                selected_technique_ids: resolution.applicable_technique_ids.clone(),
                                active_obstruction_ids: resolution.active_obstruction_ids.clone(),
                                unknown_obstruction_ids: resolution.unknown_obstruction_ids.clone(),
                                discharged_obligation_ids: resolution
                                    .discharged_obligation_ids
                                    .iter()
                                    .cloned()
                                    .collect(),
                                outstanding_obligation_ids: assessment
                                    .outstanding_obligation_ids
                                    .clone(),
                                unknown_obligation_ids: assessment.unknown_obligation_ids.clone(),
                                supporting_microtrace_ids: resolution
                                    .supporting_microtrace_ids
                                    .iter()
                                    .cloned()
                                    .collect(),
                                introduced_obligation_ids: resolution
                                    .applicable_technique_ids
                                    .iter()
                                    .filter_map(|technique_id| {
                                        self.mechanics
                                            .techniques
                                            .iter()
                                            .find(|technique| technique.id == *technique_id)
                                    })
                                    .flat_map(|technique| {
                                        technique.introduced_obligation_ids.iter().cloned()
                                    })
                                    .collect::<BTreeSet<_>>()
                                    .into_iter()
                                    .collect(),
                                reader_results,
                                unknown_reader_ids,
                                evidence_dependencies,
                                weakest_evidence,
                                action_derivations: Vec::new(),
                                obligation_derivations: Vec::new(),
                                source_state_sha256: state_identity,
                                result_state_sha256: Digest::ZERO,
                            },
                        )?;
                    }
                    if combinations > self.options.max_resolution_combinations {
                        break;
                    }
                }
            }
        }

        if !reached_plans.is_empty() {
            order_search_plans(&mut reached_plans);
            reached_plans.truncate(max_plans);
            let primary = reached_plans.remove(0);
            return Ok(SearchResult {
                backward_relevance,
                backward_pruning_applied,
                status: SearchStatus::Reached,
                steps: primary.steps,
                explored_states: visited.len(),
                hit_search_limit,
                preference_score: primary.preference_score,
                satisfied_preference_ids: primary.satisfied_preference_ids,
                route_costs: primary.route_costs,
                alternative_plans: reached_plans,
                minimum_evidence: route_policy
                    .as_ref()
                    .and_then(|policy| policy.minimum_evidence),
                unknown_transition_ids: unknown_transition_ids.into_iter().collect(),
                unknown_writer_ids: unknown_writer_ids.into_iter().collect(),
                execution_error_ids: execution_error_ids.into_iter().collect(),
                blocked_transition_witnesses: Vec::new(),
                blocked_writer_witnesses: Vec::new(),
                continuation_merge_proofs,
                failed_producer_cuts: Vec::new(),
            });
        }

        let unknown = hit_search_limit
            || saw_unknown_goal
            || !unknown_transition_ids.is_empty()
            || !unknown_writer_ids.is_empty()
            || !execution_error_ids.is_empty();
        let failed_producer_cuts = if hit_search_limit {
            Vec::new()
        } else {
            failed_producer_cuts(
                &backward_relevance,
                self.mechanics,
                &executed_actions,
                &blocked_transition_witnesses,
                &blocked_writer_witnesses,
            )?
        };
        Ok(SearchResult {
            backward_relevance,
            backward_pruning_applied,
            status: if unknown {
                SearchStatus::Unknown
            } else {
                SearchStatus::UnreachableUnderModel
            },
            steps: Vec::new(),
            explored_states: visited.len(),
            hit_search_limit,
            preference_score: 0,
            satisfied_preference_ids: Vec::new(),
            route_costs: BTreeMap::new(),
            alternative_plans: Vec::new(),
            minimum_evidence: route_policy
                .as_ref()
                .and_then(|policy| policy.minimum_evidence),
            unknown_transition_ids: unknown_transition_ids.into_iter().collect(),
            unknown_writer_ids: unknown_writer_ids.into_iter().collect(),
            execution_error_ids: execution_error_ids.into_iter().collect(),
            blocked_transition_witnesses: blocked_transition_witnesses.into_values().collect(),
            blocked_writer_witnesses: blocked_writer_witnesses.into_values().collect(),
            continuation_merge_proofs,
            failed_producer_cuts,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn enqueue_if_new(
        &self,
        queue: &mut BinaryHeap<QueueEntry>,
        visited: &BTreeSet<SearchIdentity>,
        node: &SearchNode,
        next: PlannerExecutionState,
        boundaries: &[AppliedActionBoundary],
        route_policy: Option<&RouteSearchPolicy>,
        insertion_order: u64,
        authorization: Option<&mut AuthorizationRecorder>,
        mut step: SearchStep,
    ) -> Result<bool, PlannerContractError> {
        let result = next.semantic_digest()?;
        let mut satisfied_required_actions = node.satisfied_required_actions.clone();
        let mut required_sequence_progress = node.required_sequence_progress.clone();
        let mut banned_sequence_progress = node.banned_sequence_progress.clone();
        let mut preferred_sequence_progress = node.preferred_sequence_progress.clone();
        let mut satisfied_preference_ids = node.satisfied_preference_ids.clone();
        let mut preference_score = node.preference_score;
        let mut route_condition_unknown = node.route_condition_unknown;
        let mut saw_unknown_condition = false;
        let mut route_costs = node.route_costs.clone();
        for boundary in boundaries {
            let RouteActionRef::Technique { technique_id } = &boundary.action else {
                continue;
            };
            let technique = self
                .mechanics
                .techniques
                .iter()
                .find(|technique| technique.id == *technique_id)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "solver.cost",
                        format!("references unknown technique {technique_id}"),
                    )
                })?;
            for (axis, increment) in &technique.cost.axes {
                if *increment == 0 {
                    continue;
                }
                let total = route_costs.entry(axis.clone()).or_default();
                *total = total.checked_add(*increment).ok_or_else(|| {
                    PlannerContractError::new("solver.cost", format!("axis {axis} overflowed u64"))
                })?;
            }
        }
        if let Some(policy) = route_policy {
            if policy
                .cost_limits
                .iter()
                .any(|(axis, maximum)| route_costs.get(axis).copied().unwrap_or(0) > *maximum)
            {
                return Ok(false);
            }
            for boundary in boundaries {
                if policy.required_actions.contains(&boundary.action) {
                    satisfied_required_actions.insert(boundary.action.clone());
                }
                for preference in &policy.action_preferences {
                    if preference.action == boundary.action
                        && satisfied_preference_ids.insert(preference.directive_id.clone())
                    {
                        preference_score =
                            preference_score.saturating_add(u64::from(preference.weight));
                    }
                }
                for (preference, progress) in policy
                    .method_preferences
                    .iter()
                    .zip(preferred_sequence_progress.iter_mut())
                {
                    if let Some(expected) = preference.sequence.steps.get(*progress)
                        && expected.action == boundary.action
                        && self.evaluate_step_boundary(
                            expected,
                            boundary,
                            policy.evidence_policy,
                        )? == EvaluatedTruth::True
                    {
                        *progress += 1;
                        if *progress == preference.sequence.steps.len()
                            && satisfied_preference_ids.insert(preference.directive_id.clone())
                        {
                            preference_score =
                                preference_score.saturating_add(u64::from(preference.weight));
                        }
                    }
                }
            }
            saw_unknown_condition |= self.advance_sequence_progress(
                &policy.required_sequences,
                &mut required_sequence_progress,
                boundaries,
                policy.evidence_policy,
            )?;
            let banned_unknown = self.advance_sequence_progress(
                &policy.banned_sequences,
                &mut banned_sequence_progress,
                boundaries,
                policy.evidence_policy,
            )?;
            saw_unknown_condition |= banned_unknown;
            route_condition_unknown |= banned_unknown;
            if policy
                .banned_sequences
                .iter()
                .zip(&banned_sequence_progress)
                .any(|(sequence, progress)| *progress == sequence.steps.len())
            {
                return Ok(saw_unknown_condition);
            }
        }
        let continuation = ContinuationIdentity {
            state_sha256: result,
            satisfied_required_actions: satisfied_required_actions.iter().cloned().collect(),
            required_sequence_progress: required_sequence_progress.clone(),
            banned_sequence_progress: banned_sequence_progress.clone(),
            preferred_sequence_progress: preferred_sequence_progress.clone(),
            satisfied_preference_ids: satisfied_preference_ids.iter().cloned().collect(),
            route_condition_unknown,
        };
        let search_identity = SearchIdentity {
            continuation,
            route_costs: route_costs.clone(),
        };
        let derivation_evidence_policy = route_policy
            .map_or(self.options.evidence_policy, |policy| {
                policy.evidence_policy
            });
        step.action_derivations = boundaries
            .iter()
            .map(|boundary| self.action_derivation(boundary, derivation_evidence_policy))
            .collect::<Result<Vec<_>, _>>()?;
        let obligation_ids = step
            .discharged_obligation_ids
            .iter()
            .chain(&step.outstanding_obligation_ids)
            .chain(&step.unknown_obligation_ids)
            .chain(&step.introduced_obligation_ids)
            .collect::<BTreeSet<_>>();
        step.obligation_derivations = self
            .mechanics
            .obligations
            .iter()
            .filter(|obligation| obligation_ids.contains(&obligation.id))
            .cloned()
            .collect();
        step.result_state_sha256 = result;
        if let Some(recorder) = authorization {
            recorder.observe_state(
                result,
                next.digest()?,
                next.snapshot.digest()?,
                node.depth + 1,
                false,
            );
            recorder.record_edge(&step);
        }
        if visited.contains(&search_identity) {
            return Ok(saw_unknown_condition);
        }
        let mut steps = node.steps.clone();
        steps.push(step);
        queue.push(QueueEntry {
            node: SearchNode {
                state: next,
                steps,
                depth: node.depth + 1,
                satisfied_required_actions,
                required_sequence_progress,
                banned_sequence_progress,
                preferred_sequence_progress,
                satisfied_preference_ids,
                preference_score,
                route_condition_unknown,
                route_costs,
            },
            insertion_order,
        });
        Ok(saw_unknown_condition)
    }

    fn action_derivation(
        &self,
        boundary: &AppliedActionBoundary,
        evidence_policy: EvidencePolicy,
    ) -> Result<ActionDerivation, PlannerContractError> {
        let (precondition, operations) = match &boundary.action {
            RouteActionRef::Transition { transition_id } => {
                let transition = self
                    .mechanics
                    .transitions
                    .iter()
                    .find(|transition| transition.id == *transition_id)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "solver.action_derivation",
                            format!("references unknown transition {transition_id}"),
                        )
                    })?;
                (
                    transition.activation.hard_guards.clone(),
                    transition.activation.effects.clone(),
                )
            }
            RouteActionRef::Technique { technique_id } => {
                let technique = self
                    .mechanics
                    .techniques
                    .iter()
                    .find(|technique| technique.id == *technique_id)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "solver.action_derivation",
                            format!("references unknown technique {technique_id}"),
                        )
                    })?;
                (
                    technique.prerequisites.clone(),
                    technique.operations.clone(),
                )
            }
            RouteActionRef::Resolver { resolver_id } => {
                let resolver = self
                    .mechanics
                    .resolvers
                    .iter()
                    .find(|resolver| resolver.id == *resolver_id)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "solver.action_derivation",
                            format!("references unknown resolver {resolver_id}"),
                        )
                    })?;
                (
                    resolver.applicable_when.clone(),
                    resolver.operations.clone(),
                )
            }
            RouteActionRef::Writer { writer_id } => {
                let writer = self
                    .mechanics
                    .writers
                    .iter()
                    .find(|writer| writer.id == *writer_id)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "solver.action_derivation",
                            format!("references unknown writer {writer_id}"),
                        )
                    })?;
                (writer.activation.clone(), vec![writer.operation.clone()])
            }
            RouteActionRef::Microtrace { microtrace_id } => {
                let microtrace = self
                    .mechanics
                    .microtraces
                    .iter()
                    .find(|microtrace| microtrace.id == *microtrace_id)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "solver.action_derivation",
                            format!("references unknown microtrace {microtrace_id}"),
                        )
                    })?;
                (
                    microtrace.precondition.clone(),
                    microtrace.operations.clone(),
                )
            }
        };
        let evaluator = PredicateEvaluator::new(
            &boundary.before.snapshot,
            self.facts,
            self.equivalence_sets,
            &boundary.before.gate_states,
            evidence_policy,
        )?;
        Ok(ActionDerivation {
            action: boundary.action.clone(),
            precondition_result: evaluator.evaluate(&precondition),
            precondition,
            operations,
            source_state_sha256: boundary.before.semantic_digest()?,
            result_state_sha256: boundary.after.semantic_digest()?,
        })
    }

    fn advance_sequence_progress(
        &self,
        sequences: &[RouteActionSequence],
        progress: &mut [usize],
        boundaries: &[AppliedActionBoundary],
        evidence_policy: EvidencePolicy,
    ) -> Result<bool, PlannerContractError> {
        let mut saw_unknown = false;
        for boundary in boundaries {
            for (sequence, progress) in sequences.iter().zip(progress.iter_mut()) {
                let Some(expected) = sequence.steps.get(*progress) else {
                    continue;
                };
                if expected.action != boundary.action {
                    continue;
                }
                match self.evaluate_step_boundary(expected, boundary, evidence_policy)? {
                    EvaluatedTruth::True => *progress += 1,
                    EvaluatedTruth::Unknown => saw_unknown = true,
                    EvaluatedTruth::False => {}
                }
            }
        }
        Ok(saw_unknown)
    }

    fn evaluate_step_boundary(
        &self,
        step: &RouteSequenceStep,
        boundary: &AppliedActionBoundary,
        evidence_policy: EvidencePolicy,
    ) -> Result<EvaluatedTruth, PlannerContractError> {
        let before = PredicateEvaluator::new(
            &boundary.before.snapshot,
            self.facts,
            self.equivalence_sets,
            &boundary.before.gate_states,
            evidence_policy,
        )?;
        let after = PredicateEvaluator::new(
            &boundary.after.snapshot,
            self.facts,
            self.equivalence_sets,
            &boundary.after.gate_states,
            evidence_policy,
        )?;
        Ok(and_truth(
            step.precondition
                .as_ref()
                .map_or(EvaluatedTruth::True, |predicate| before.evaluate(predicate)),
            step.postcondition
                .as_ref()
                .map_or(EvaluatedTruth::True, |predicate| after.evaluate(predicate)),
        ))
    }
}

fn blocked_witness(
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
    transition: &CandidateTransition,
    source: &PlannerExecutionState,
    resolution: &FeasibilityResolution,
    assessment: &TransitionAssessment,
    readers: (&[ReaderResult], &[String]),
) -> Result<BlockedTransitionWitness, PlannerContractError> {
    let (reader_results, unknown_reader_ids) = readers;
    let evidence_dependencies = transition_evidence_dependencies(
        facts,
        mechanics,
        transition,
        resolution,
        assessment,
        reader_results,
        unknown_reader_ids,
    );
    let weakest_evidence = weakest_evidence(&evidence_dependencies);
    Ok(BlockedTransitionWitness {
        transition_id: transition.id.clone(),
        source_state_sha256: source.semantic_digest()?,
        classification: assessment.classification,
        hard_guard: assessment.hard_guard,
        selected_resolver_ids: resolution.applied_resolver_ids.clone(),
        selected_technique_ids: resolution.applicable_technique_ids.clone(),
        active_obstruction_ids: resolution.active_obstruction_ids.clone(),
        unknown_obstruction_ids: resolution.unknown_obstruction_ids.clone(),
        discharged_obligation_ids: resolution
            .discharged_obligation_ids
            .iter()
            .cloned()
            .collect(),
        outstanding_obligation_ids: assessment.outstanding_obligation_ids.clone(),
        unknown_obligation_ids: assessment.unknown_obligation_ids.clone(),
        supporting_microtrace_ids: resolution
            .supporting_microtrace_ids
            .iter()
            .cloned()
            .collect(),
        unknown_requirement_ids: assessment.unknown_requirement_ids.clone(),
        reader_results: reader_results.to_vec(),
        unknown_reader_ids: unknown_reader_ids.to_vec(),
        evidence_dependencies,
        weakest_evidence,
        hard_guard_expression: transition.activation.hard_guards.clone(),
        effect_operations: transition.activation.effects.clone(),
        obligation_derivations: mechanics
            .obligations
            .iter()
            .filter(|obligation| {
                transition
                    .activation
                    .physical_obligation_ids
                    .contains(&obligation.id)
            })
            .cloned()
            .collect(),
        unknown_requirements: transition.activation.unknown_requirements.clone(),
    })
}

fn assess_transition_readers(
    evaluator: &PredicateEvaluator<'_>,
    mechanics: &MechanicsCatalog,
    transition_id: &str,
) -> (Vec<ReaderResult>, Vec<String>) {
    let mut results = Vec::new();
    let mut unknown = Vec::new();
    for reader in mechanics
        .readers
        .iter()
        .filter(|reader| reader.consuming_transition_id == transition_id)
    {
        let assessment = evaluator.assess_reader(reader);
        if !assessment.scope_applies {
            continue;
        }
        if !assessment.evidence_permitted {
            unknown.push(reader.id.clone());
            continue;
        }
        let Some(source_value) = assessment.source_value else {
            unknown.push(reader.id.clone());
            continue;
        };
        results.push(ReaderResult {
            reader_id: reader.id.clone(),
            source_value,
            interpretation: assessment.interpretation,
        });
    }
    (results, unknown)
}

fn technique_evidence_dependencies(
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
    technique: &crate::transition::Technique,
) -> Vec<EvidenceDependency> {
    let mut dependencies = BTreeMap::new();
    insert_evidence(
        &mut dependencies,
        EvidenceDependencyKind::Technique,
        &technique.id,
        &technique.evidence,
    );
    collect_predicate_evidence(
        &technique.prerequisites,
        facts,
        &mut dependencies,
        &mut BTreeSet::new(),
    );
    for obligation_id in technique
        .discharged_obligation_ids
        .iter()
        .chain(&technique.introduced_obligation_ids)
    {
        if let Some(obligation) = mechanics
            .obligations
            .iter()
            .find(|obligation| obligation.id == *obligation_id)
        {
            collect_obligation_evidence(obligation, facts, &mut dependencies);
        }
    }
    dependencies
        .into_iter()
        .map(
            |((dependency_kind, record_id), evidence)| EvidenceDependency {
                dependency_kind,
                record_id,
                evidence,
            },
        )
        .collect()
}

fn writer_evidence_dependencies(
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
    writer: &crate::transition::WriterRule,
    assessment: &WriterAssessment,
) -> Vec<EvidenceDependency> {
    let mut dependencies = BTreeMap::new();
    insert_evidence(
        &mut dependencies,
        EvidenceDependencyKind::Writer,
        &writer.id,
        &writer.evidence,
    );
    collect_predicate_evidence(
        &writer.activation,
        facts,
        &mut dependencies,
        &mut BTreeSet::new(),
    );
    for gate in mechanics.gates.iter().filter(|gate| {
        gate.blocked_writer_ids
            .iter()
            .any(|writer_id| writer_id == &writer.id)
    }) {
        insert_evidence(
            &mut dependencies,
            EvidenceDependencyKind::Gate,
            &gate.id,
            &gate.evidence,
        );
        collect_predicate_evidence(
            &gate.active_when,
            facts,
            &mut dependencies,
            &mut BTreeSet::new(),
        );
    }
    for gate_id in assessment
        .active_gate_ids
        .iter()
        .chain(&assessment.unknown_gate_ids)
    {
        debug_assert!(mechanics.gates.iter().any(|gate| gate.id == *gate_id));
    }
    dependencies
        .into_iter()
        .map(
            |((dependency_kind, record_id), evidence)| EvidenceDependency {
                dependency_kind,
                record_id,
                evidence,
            },
        )
        .collect()
}

fn transition_evidence_dependencies(
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
    transition: &CandidateTransition,
    resolution: &FeasibilityResolution,
    assessment: &TransitionAssessment,
    reader_results: &[ReaderResult],
    unknown_reader_ids: &[String],
) -> Vec<EvidenceDependency> {
    let mut dependencies = BTreeMap::new();
    insert_evidence(
        &mut dependencies,
        EvidenceDependencyKind::Transition,
        &transition.id,
        &transition.evidence,
    );
    collect_predicate_evidence(
        &transition.activation.hard_guards,
        facts,
        &mut dependencies,
        &mut BTreeSet::new(),
    );
    for requirement_id in &assessment.unknown_requirement_ids {
        if let Some(requirement) = transition
            .activation
            .unknown_requirements
            .iter()
            .find(|requirement| requirement.id == *requirement_id)
        {
            insert_evidence(
                &mut dependencies,
                EvidenceDependencyKind::UnknownRequirement,
                &requirement.id,
                &requirement.evidence,
            );
        }
    }
    for obstruction_id in resolution
        .active_obstruction_ids
        .iter()
        .chain(&resolution.unknown_obstruction_ids)
    {
        if let Some(obstruction) = mechanics
            .obstructions
            .iter()
            .find(|obstruction| obstruction.id == *obstruction_id)
        {
            insert_evidence(
                &mut dependencies,
                EvidenceDependencyKind::Obstruction,
                &obstruction.id,
                &obstruction.evidence,
            );
            collect_predicate_evidence(
                &obstruction.active_when,
                facts,
                &mut dependencies,
                &mut BTreeSet::new(),
            );
        }
    }
    for resolver_id in &resolution.applied_resolver_ids {
        if let Some(resolver) = mechanics
            .resolvers
            .iter()
            .find(|resolver| resolver.id == *resolver_id)
        {
            insert_evidence(
                &mut dependencies,
                EvidenceDependencyKind::Resolver,
                &resolver.id,
                &resolver.evidence,
            );
            collect_predicate_evidence(
                &resolver.applicable_when,
                facts,
                &mut dependencies,
                &mut BTreeSet::new(),
            );
        }
    }
    for technique_id in &resolution.applicable_technique_ids {
        if let Some(technique) = mechanics
            .techniques
            .iter()
            .find(|technique| technique.id == *technique_id)
        {
            for dependency in technique_evidence_dependencies(facts, mechanics, technique) {
                dependencies.insert(
                    (dependency.dependency_kind, dependency.record_id),
                    dependency.evidence,
                );
            }
        }
    }
    let obligation_ids = resolution
        .discharged_obligation_ids
        .iter()
        .chain(&assessment.outstanding_obligation_ids)
        .chain(&assessment.unknown_obligation_ids)
        .collect::<BTreeSet<_>>();
    for obligation_id in obligation_ids {
        if let Some(obligation) = mechanics
            .obligations
            .iter()
            .find(|obligation| obligation.id == *obligation_id)
        {
            collect_obligation_evidence(obligation, facts, &mut dependencies);
        }
    }
    for microtrace_id in &resolution.supporting_microtrace_ids {
        if let Some(microtrace) = mechanics
            .microtraces
            .iter()
            .find(|microtrace| microtrace.id == *microtrace_id)
        {
            insert_evidence(
                &mut dependencies,
                EvidenceDependencyKind::Microtrace,
                &microtrace.id,
                &microtrace.evidence,
            );
            collect_predicate_evidence(
                &microtrace.precondition,
                facts,
                &mut dependencies,
                &mut BTreeSet::new(),
            );
            collect_predicate_evidence(
                &microtrace.postcondition,
                facts,
                &mut dependencies,
                &mut BTreeSet::new(),
            );
        }
    }
    let reader_ids = reader_results
        .iter()
        .map(|result| result.reader_id.as_str())
        .chain(unknown_reader_ids.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    for reader_id in reader_ids {
        if let Some(reader) = mechanics
            .readers
            .iter()
            .find(|reader| reader.id == reader_id)
        {
            insert_evidence(
                &mut dependencies,
                EvidenceDependencyKind::Reader,
                &reader.id,
                &reader.evidence,
            );
            if let Some(fact_id) = &reader.interpretation_fact_id {
                collect_predicate_evidence(
                    &PredicateExpression::Fact {
                        fact_id: fact_id.clone(),
                    },
                    facts,
                    &mut dependencies,
                    &mut BTreeSet::new(),
                );
            }
        }
    }
    dependencies
        .into_iter()
        .map(
            |((dependency_kind, record_id), evidence)| EvidenceDependency {
                dependency_kind,
                record_id,
                evidence,
            },
        )
        .collect()
}

fn collect_obligation_evidence(
    obligation: &crate::transition::FeasibilityObligation,
    facts: &FactCatalog,
    dependencies: &mut BTreeMap<(EvidenceDependencyKind, String), RuleEvidence>,
) {
    insert_evidence(
        dependencies,
        EvidenceDependencyKind::Obligation,
        &obligation.id,
        &obligation.evidence,
    );
    let predicate = match &obligation.detail {
        crate::transition::ObligationDetail::Predicate { predicate } => Some(predicate),
        crate::transition::ObligationDetail::Interaction { pose_predicate, .. } => {
            Some(pose_predicate)
        }
        crate::transition::ObligationDetail::Temporal { precondition, .. } => Some(precondition),
        crate::transition::ObligationDetail::Geometry { .. }
        | crate::transition::ObligationDetail::PlaneSide { .. }
        | crate::transition::ObligationDetail::Unresolved { .. } => None,
    };
    if let Some(predicate) = predicate {
        collect_predicate_evidence(predicate, facts, dependencies, &mut BTreeSet::new());
    }
}

fn collect_predicate_evidence(
    predicate: &PredicateExpression,
    facts: &FactCatalog,
    dependencies: &mut BTreeMap<(EvidenceDependencyKind, String), RuleEvidence>,
    visiting: &mut BTreeSet<String>,
) {
    match predicate {
        PredicateExpression::Fact { fact_id } => {
            if !visiting.insert(fact_id.clone()) {
                return;
            }
            if let Some(alias) = facts.aliases.iter().find(|alias| alias.id == *fact_id) {
                insert_evidence(
                    dependencies,
                    EvidenceDependencyKind::Fact,
                    &alias.id,
                    &alias.evidence,
                );
            } else if let Some(fact) = facts.derived_facts.iter().find(|fact| fact.id == *fact_id) {
                insert_evidence(
                    dependencies,
                    EvidenceDependencyKind::Fact,
                    &fact.id,
                    &fact.evidence,
                );
                collect_predicate_evidence(&fact.rule, facts, dependencies, visiting);
            }
            visiting.remove(fact_id);
        }
        PredicateExpression::All { terms } | PredicateExpression::Any { terms } => {
            for term in terms {
                collect_predicate_evidence(term, facts, dependencies, visiting);
            }
        }
        PredicateExpression::Not { term } => {
            collect_predicate_evidence(term, facts, dependencies, visiting)
        }
        PredicateExpression::True
        | PredicateExpression::False
        | PredicateExpression::Compare { .. } => {}
    }
}

fn insert_evidence(
    dependencies: &mut BTreeMap<(EvidenceDependencyKind, String), RuleEvidence>,
    dependency_kind: EvidenceDependencyKind,
    record_id: &str,
    evidence: &RuleEvidence,
) {
    dependencies.insert((dependency_kind, record_id.into()), evidence.clone());
}

fn weakest_evidence(dependencies: &[EvidenceDependency]) -> Option<TruthStatus> {
    dependencies
        .iter()
        .map(|dependency| dependency.evidence.truth)
        .max()
}

fn record_blocked_transition_witness(
    witnesses: &mut BTreeMap<String, BlockedTransitionWitness>,
    candidate: BlockedTransitionWitness,
) {
    let replace = witnesses
        .get(&candidate.transition_id)
        .is_none_or(|current| blocker_rank(&candidate) < blocker_rank(current));
    if replace {
        witnesses.insert(candidate.transition_id.clone(), candidate);
    }
}

fn failed_producer_cuts(
    relevance: &BackwardRelevance,
    mechanics: &MechanicsCatalog,
    executed_actions: &BTreeSet<RouteActionRef>,
    transition_witnesses: &BTreeMap<String, BlockedTransitionWitness>,
    writer_witnesses: &BTreeMap<String, BlockedWriterWitness>,
) -> Result<Vec<FailedProducerCut>, PlannerContractError> {
    let mut cuts = Vec::new();
    for dependency in &relevance.dependencies {
        if matches!(
            dependency,
            StateDependency::Fact { .. } | StateDependency::AnyState
        ) {
            continue;
        }
        let produces = |operations: &[crate::transition::StateOperation]| {
            operations.iter().any(|operation| {
                operation_outputs(operation)
                    .iter()
                    .any(|output| dependencies_overlap(dependency, output))
            })
        };
        let mut blockers = Vec::new();
        let mut saw_producer = false;
        let mut complete = true;
        for transition in &mechanics.transitions {
            if !produces(&transition.activation.effects) {
                continue;
            }
            saw_producer = true;
            let action = RouteActionRef::Transition {
                transition_id: transition.id.clone(),
            };
            if executed_actions.contains(&action) {
                complete = false;
                continue;
            }
            if let Some(witness) = transition_witnesses.get(&transition.id) {
                blockers.push(FailedProducerBlocker::Transition {
                    transition_id: transition.id.clone(),
                    source_state_sha256: witness.source_state_sha256,
                    classification: witness.classification,
                });
            } else {
                complete = false;
            }
        }
        for writer in &mechanics.writers {
            if !produces(std::slice::from_ref(&writer.operation)) {
                continue;
            }
            saw_producer = true;
            let action = RouteActionRef::Writer {
                writer_id: writer.id.clone(),
            };
            if executed_actions.contains(&action) {
                complete = false;
                continue;
            }
            if let Some(witness) = writer_witnesses.get(&writer.id) {
                blockers.push(FailedProducerBlocker::Writer {
                    writer_id: writer.id.clone(),
                    source_state_sha256: witness.source_state_sha256,
                    classification: witness.classification,
                });
            } else {
                complete = false;
            }
        }
        let unsupported_producer = mechanics
            .techniques
            .iter()
            .any(|technique| produces(&technique.operations))
            || mechanics
                .resolvers
                .iter()
                .any(|resolver| produces(&resolver.operations))
            || mechanics
                .reconstruction_rules
                .iter()
                .any(|rule| produces(&rule.initialization_operations));
        if !saw_producer || !complete || unsupported_producer {
            continue;
        }
        blockers.sort_by_key(FailedProducerBlocker::action);
        let cut = FailedProducerCut {
            dependency: dependency.clone(),
            blocked_producers: blockers,
        };
        cut.validate()?;
        cuts.push(cut);
    }
    Ok(cuts)
}

fn blocker_rank(witness: &BlockedTransitionWitness) -> (usize, u8, Digest) {
    let unresolved = witness
        .active_obstruction_ids
        .len()
        .saturating_add(witness.unknown_obstruction_ids.len())
        .saturating_add(witness.outstanding_obligation_ids.len())
        .saturating_add(witness.unknown_obligation_ids.len())
        .saturating_add(witness.unknown_requirement_ids.len())
        .saturating_add(usize::from(witness.hard_guard != EvaluatedTruth::True));
    let classification = match witness.classification {
        TransitionClassification::Executable => 0,
        TransitionClassification::Obstructed => 1,
        TransitionClassification::GuardBlocked => 2,
        TransitionClassification::FeasibilityUnknown => 3,
        TransitionClassification::Inapplicable => 4,
    };
    (unresolved, classification, witness.source_state_sha256)
}

fn and_truth(left: EvaluatedTruth, right: EvaluatedTruth) -> EvaluatedTruth {
    match (left, right) {
        (EvaluatedTruth::False, _) | (_, EvaluatedTruth::False) => EvaluatedTruth::False,
        (EvaluatedTruth::Unknown, _) | (_, EvaluatedTruth::Unknown) => EvaluatedTruth::Unknown,
        (EvaluatedTruth::True, EvaluatedTruth::True) => EvaluatedTruth::True,
    }
}

fn bounded_subsets(ids: &[String], maximum: usize) -> Vec<BTreeSet<String>> {
    let mut subsets = vec![BTreeSet::new()];
    for id in ids {
        let additions = subsets
            .iter()
            .take(maximum.saturating_sub(subsets.len()))
            .cloned()
            .map(|mut subset| {
                subset.insert(id.clone());
                subset
            })
            .collect::<Vec<_>>();
        subsets.extend(additions);
        if subsets.len() >= maximum {
            subsets.truncate(maximum);
            break;
        }
    }
    subsets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Digest;
    use crate::identity::{ContextSelector, RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration};
    use crate::logic::{
        ComparisonOperator, ContextScope, DerivedFact, EvidenceKind, EvidenceRecord,
        FACT_CATALOG_SCHEMA, FriendlyAlias, RawFactBinding, RuleEvidence, TruthStatus,
        ValueReference,
    };
    use crate::refinement::{
        ComposedPlannerCatalog, REFINEMENT_PACK_SCHEMA, RefinementLayers, RefinementOperation,
        RefinementPack, RefinementPackManifest, RefinementRule,
    };
    use crate::route_book::{
        CollapsePolicy, PlanMethod, PlanRegion, ROUTE_BOOK_SCHEMA, ReferenceStep,
        RouteBookManifest, RouteConstraint, RouteDirective, RouteDirectiveKind,
    };
    use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateDiff, StateSnapshot};
    use crate::state::{
        ActorLifecycle, BackingAttachment, BoundaryKind, ComponentBinding,
        ComponentBindingReference, ComponentKind, ComponentPayload, ComponentProvenance,
        ComponentSelector, EXECUTION_ENVIRONMENT_SCHEMA, ExecutionEnvironment, LiveWorldObject,
        PlayerForm, PlayerState, ProvenanceSourceKind, RuntimeFile, RuntimeFileLifecycle,
        RuntimeFileOrigin, SceneLocation, SemanticLifetime, SerializationOwner, SpatialVolume,
        SpatialVolumeShape, StateComponent, StateValue,
    };
    use crate::transition::{
        ActivationContract, ActorReconstructionRule, CandidateTransition, FeasibilityObligation,
        GateRule, Goal, MECHANICS_CATALOG_SCHEMA, MechanicsCatalog, ObligationDetail,
        ObligationKind, Obstruction, ObstructionResolver, ReaderRule, ResolutionKind, RouteCost,
        StateOperation, Technique, TemporalRequirement, TemporalWindow, TransitionKind,
        UnknownRequirement, WitnessedMicrotrace, WriterRule,
    };
    use std::collections::BTreeMap;

    fn snapshot() -> StateSnapshot {
        StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.start".into(),
            sequence: 1,
            environment: ExecutionEnvironment {
                schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
                runtime_configuration: RuntimeConfiguration {
                    schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
                    content_sha256: Digest([1; 32]),
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
                spatial_connections: Vec::new(),
                spatial_planes: Vec::new(),
                persisted_object_controls: Vec::new(),
                live_world_objects: Vec::new(),
            },
            semantic_observations: Vec::new(),
        }
    }

    fn scope(snapshot: &StateSnapshot) -> ContextScope {
        ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: snapshot
                    .environment
                    .runtime_configuration
                    .exact_context()
                    .unwrap(),
            }],
        }
    }

    fn evidence(truth: TruthStatus) -> RuleEvidence {
        RuleEvidence {
            truth,
            records: if matches!(truth, TruthStatus::Established | TruthStatus::Contested) {
                vec![EvidenceRecord {
                    id: "source.solver-test".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(Digest([2; 32])),
                    note: "Solver test evidence.".into(),
                }]
            } else {
                Vec::new()
            },
        }
    }

    fn stage_is(stage: &str) -> PredicateExpression {
        PredicateExpression::Compare {
            left: ValueReference::LocationStage,
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Text(stage.into()),
            },
        }
    }

    fn gate_is(gate_id: &str, value: bool) -> PredicateExpression {
        PredicateExpression::Compare {
            left: ValueReference::GateState {
                gate_id: gate_id.into(),
            },
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Boolean(value),
            },
        }
    }

    fn transition(
        snapshot: &StateSnapshot,
        id: &str,
        source: &str,
        destination: &str,
        obligations: Vec<String>,
    ) -> CandidateTransition {
        CandidateTransition {
            id: id.into(),
            label: format!("Travel from {source} to {destination}"),
            scope: scope(snapshot),
            transition_kind: TransitionKind::EncodedMapExit,
            approach_id: "approach.front".into(),
            activation: ActivationContract {
                hard_guards: stage_is(source),
                physical_obligation_ids: obligations,
                effects: vec![StateOperation::SetLocation {
                    location: SceneLocation {
                        stage: destination.into(),
                        room: 0,
                        layer: 0,
                        spawn: 0,
                    },
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: evidence(TruthStatus::Established),
        }
    }

    fn catalog(transitions: Vec<CandidateTransition>) -> MechanicsCatalog {
        MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions,
            obligations: Vec::new(),
            writers: Vec::new(),
            gates: Vec::new(),
            readers: Vec::new(),
            reconstruction_rules: Vec::new(),
            obstructions: Vec::new(),
            resolvers: Vec::new(),
            techniques: Vec::new(),
            microtraces: Vec::new(),
            goals: Vec::new(),
        }
    }

    fn location_technique(
        snapshot: &StateSnapshot,
        id: &str,
        destination: &str,
        costs: &[(&str, u64)],
    ) -> Technique {
        Technique {
            id: id.into(),
            label: format!("Reach {destination}"),
            scope: scope(snapshot),
            prerequisites: PredicateExpression::True,
            operations: vec![StateOperation::SetLocation {
                location: SceneLocation {
                    stage: destination.into(),
                    room: 0,
                    layer: 0,
                    spawn: 0,
                },
            }],
            discharged_obligation_ids: Vec::new(),
            introduced_obligation_ids: Vec::new(),
            cost: RouteCost {
                axes: costs
                    .iter()
                    .map(|(axis, value)| ((*axis).into(), *value))
                    .collect(),
            },
            evidence: evidence(TruthStatus::Established),
        }
    }

    fn obstructed_transition_catalog(snapshot: &StateSnapshot) -> MechanicsCatalog {
        let mut mechanics = catalog(vec![transition(
            snapshot,
            "transition.a-to-b",
            "STAGE_A",
            "STAGE_B",
            vec!["obligation.blocker".into()],
        )]);
        mechanics.transitions[0].activation.hard_guards = PredicateExpression::All {
            terms: vec![stage_is("STAGE_A"), gate_is("gate.entrance-open", true)],
        };
        mechanics.obligations = vec![FeasibilityObligation {
            id: "obligation.blocker".into(),
            label: "Reach the loading zone past the blocker".into(),
            scope: scope(snapshot),
            obligation_kind: ObligationKind::Geometry,
            detail: ObligationDetail::Geometry {
                approach_id: "approach.front".into(),
                source_region_id: "region.a".into(),
                destination_region_id: "region.exit".into(),
            },
            evidence: evidence(TruthStatus::Established),
        }];
        mechanics.obstructions = vec![Obstruction {
            id: "obstruction.npc".into(),
            label: "NPC blocks the transition".into(),
            scope: scope(snapshot),
            blocked_action_id: "transition.a-to-b".into(),
            approach_id: "approach.front".into(),
            active_when: PredicateExpression::True,
            obligation_ids: vec!["obligation.blocker".into()],
            evidence: evidence(TruthStatus::Established),
        }];
        mechanics.resolvers = vec![ObstructionResolver {
            id: "resolver.text-state".into(),
            label: "Use displaced text state".into(),
            scope: scope(snapshot),
            obstruction_id: "obstruction.npc".into(),
            resolution_kind: ResolutionKind::Bypass,
            applicable_when: PredicateExpression::True,
            operations: vec![StateOperation::SetGate {
                gate_id: "gate.entrance-open".into(),
            }],
            evidence: evidence(TruthStatus::Established),
        }];
        mechanics
    }

    fn facts() -> FactCatalog {
        FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
        }
    }

    fn route_book(snapshot: &StateSnapshot, directives: Vec<RouteDirective>) -> RouteBook {
        RouteBook {
            schema: ROUTE_BOOK_SCHEMA.into(),
            manifest: RouteBookManifest {
                id: "route.solver-test".into(),
                version: "1.0.0".into(),
                label: "Solver test route".into(),
                author: "route planner tests".into(),
                source: "in-memory fixture".into(),
                scope: scope(snapshot),
                refinement_stack_sha256: None,
            },
            goal_ids: vec!["goal.b".into()],
            constraints: Vec::new(),
            directives,
            steps: Vec::new(),
            methods: Vec::new(),
            regions: Vec::new(),
            annotations: Vec::new(),
        }
    }

    fn goal(id: &str, stage: &str) -> Goal {
        Goal {
            id: id.into(),
            label: format!("Reach {stage}"),
            predicate: stage_is(stage),
        }
    }

    #[test]
    fn forward_search_reaches_a_goal_and_retains_the_transition_proof() {
        let snapshot = snapshot();
        let mut mechanics = catalog(vec![
            transition(
                &snapshot,
                "transition.a-to-b",
                "STAGE_A",
                "STAGE_B",
                Vec::new(),
            ),
            transition(
                &snapshot,
                "transition.b-to-c",
                "STAGE_B",
                "STAGE_C",
                Vec::new(),
            ),
        ]);
        mechanics.writers = vec![WriterRule {
            id: "writer.irrelevant-missing-component".into(),
            scope: scope(&snapshot),
            activation: PredicateExpression::True,
            operation: StateOperation::Write {
                target: crate::transition::ComponentFieldTarget {
                    component_id: "state.irrelevant".into(),
                    field: "noise".into(),
                },
                value: StateValue::Unsigned(1),
            },
            evidence: evidence(TruthStatus::Established),
        }];
        let facts = facts();
        let solver = ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default()).unwrap();
        let result = solver
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_C"),
            )
            .unwrap();
        assert_eq!(result.status, SearchStatus::Reached);
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<Vec<_>>(),
            vec!["transition.a-to-b", "transition.b-to-c"]
        );
        assert_ne!(
            result.steps[0].source_state_sha256,
            result.steps[0].result_state_sha256
        );
        assert_eq!(result.steps[0].action_derivations.len(), 1);
        assert_eq!(
            result.steps[0].action_derivations[0].action,
            RouteActionRef::Transition {
                transition_id: "transition.a-to-b".into()
            }
        );
        assert_eq!(
            result.steps[0].action_derivations[0].precondition_result,
            EvaluatedTruth::True
        );
        assert_eq!(
            result.steps[0].action_derivations[0].source_state_sha256,
            result.steps[0].source_state_sha256
        );
        assert_eq!(
            result.steps[0].action_derivations[0].result_state_sha256,
            result.steps[0].result_state_sha256
        );
        assert!(result.backward_pruning_applied);
        assert!(result.backward_relevance.writer_ids.is_empty());
        assert!(result.execution_error_ids.is_empty());
    }

    #[test]
    fn upper_bound_authorization_graph_materializes_and_traverses_evaluated_states() {
        let snapshot = snapshot();
        let mut mechanics = catalog(vec![
            transition(
                &snapshot,
                "transition.a-to-b",
                "STAGE_A",
                "STAGE_B",
                vec!["obligation.unmodeled".into()],
            ),
            transition(
                &snapshot,
                "transition.b-to-c",
                "STAGE_B",
                "STAGE_C",
                vec!["obligation.state-dependent".into()],
            ),
            transition(
                &snapshot,
                "transition.c-to-d-unknown",
                "STAGE_C",
                "STAGE_D",
                Vec::new(),
            ),
        ]);
        mechanics.obligations = vec![
            FeasibilityObligation {
                id: "obligation.state-dependent".into(),
                label: "State-dependent physical activation".into(),
                scope: scope(&snapshot),
                obligation_kind: ObligationKind::Geometry,
                detail: ObligationDetail::Predicate {
                    predicate: PredicateExpression::Compare {
                        left: ValueReference::RawBits {
                            component_id: "component.missing-physics".into(),
                            byte_offset: 0,
                            byte_width: 1,
                            mask: 1,
                        },
                        operator: ComparisonOperator::Equal,
                        right: ValueReference::Literal {
                            value: StateValue::Unsigned(1),
                        },
                    },
                },
                evidence: evidence(TruthStatus::Established),
            },
            FeasibilityObligation {
                id: "obligation.unmodeled".into(),
                label: "Unmodeled physical activation".into(),
                scope: scope(&snapshot),
                obligation_kind: ObligationKind::Geometry,
                detail: ObligationDetail::Unresolved {
                    research_question: "geometry has not been imported".into(),
                },
                evidence: evidence(TruthStatus::Established),
            },
        ];
        mechanics.transitions[2].activation.unknown_requirements = vec![UnknownRequirement {
            id: "requirement.activation-physics".into(),
            description: "activation physics are unknown".into(),
            evidence: evidence(TruthStatus::Established),
        }];
        let facts = facts();
        let options = SolverOptions {
            max_depth: 8,
            max_states: 100,
            max_resolution_combinations: 16,
            feasibility_mode: FeasibilityMode::UpperBound,
            evidence_policy: EvidencePolicy::ESTABLISHED_ONLY,
        };
        let solver = ForwardSolver::new(&facts, &mechanics, &[], options).unwrap();
        let graph = solver
            .authorization_graph(PlannerExecutionState::new(snapshot.clone()).unwrap())
            .unwrap();

        assert!(graph.traversal_complete);
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.evaluated_states, 3);
        assert_eq!(graph.edges.len(), 2);
        assert_eq!(
            graph
                .edges
                .iter()
                .find(|edge| edge.action_id == "transition.b-to-c")
                .unwrap()
                .unknown_obligation_ids,
            vec!["obligation.state-dependent"],
            "{:#?}",
            graph.edges
        );
        assert_eq!(
            graph.unknown_activation_candidates.len(),
            3,
            "{:#?}",
            graph.unknown_activation_candidates
        );
        let unresolved = graph
            .unknown_activation_candidates
            .iter()
            .find(|candidate| candidate.transition_id == "transition.a-to-b")
            .unwrap();
        assert!(unresolved.unknown_requirement_ids.is_empty());
        assert_eq!(
            unresolved.unresolved_obligation_ids,
            vec!["obligation.unmodeled"]
        );
        assert_eq!(
            unresolved.evaluated_unknown_obligation_ids,
            vec!["obligation.unmodeled"]
        );
        let state_dependent = graph
            .unknown_activation_candidates
            .iter()
            .find(|candidate| candidate.transition_id == "transition.b-to-c")
            .unwrap();
        assert!(state_dependent.unknown_requirement_ids.is_empty());
        assert!(state_dependent.unresolved_obligation_ids.is_empty());
        assert_eq!(
            state_dependent.evaluated_unknown_obligation_ids,
            vec!["obligation.state-dependent"]
        );
        let explicit_unknown = graph
            .unknown_activation_candidates
            .iter()
            .find(|candidate| candidate.transition_id == "transition.c-to-d-unknown")
            .unwrap();
        assert_eq!(
            explicit_unknown.unknown_requirement_ids,
            vec!["requirement.activation-physics"]
        );
        assert!(explicit_unknown.unresolved_obligation_ids.is_empty());
        assert!(explicit_unknown.evaluated_unknown_obligation_ids.is_empty());
        let permissive_edge = graph
            .edges
            .iter()
            .find(|edge| edge.action_id == "transition.a-to-b")
            .unwrap();
        assert_eq!(
            permissive_edge.outstanding_obligation_ids,
            vec!["obligation.unmodeled"]
        );
        assert_eq!(
            permissive_edge.unknown_obligation_ids,
            vec!["obligation.unmodeled"]
        );
        assert_eq!(
            graph.unknown_transition_ids,
            vec!["transition.c-to-d-unknown"]
        );
        assert_eq!(
            graph
                .edges
                .iter()
                .map(|edge| edge.action_id.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["transition.a-to-b", "transition.b-to-c"])
        );
        assert_eq!(graph.reachable_state_ids().len(), 3);
        let bytes = graph.canonical_bytes().unwrap();
        assert_eq!(
            crate::authorization::AuthorizationGraph::decode_canonical(&bytes).unwrap(),
            graph
        );

        let bounded = ForwardSolver::new(
            &facts,
            &mechanics,
            &[],
            SolverOptions {
                max_states: 2,
                ..options
            },
        )
        .unwrap()
        .authorization_graph(PlannerExecutionState::new(snapshot).unwrap())
        .unwrap();
        assert!(!bounded.traversal_complete);
        assert_eq!(bounded.evaluated_states, 2);
        assert_eq!(bounded.nodes.len(), 3);
        assert_eq!(
            bounded.nodes.iter().filter(|node| !node.evaluated).count(),
            1
        );
    }

    #[test]
    fn authorization_graph_requires_explicit_upper_bound_mode() {
        let snapshot = snapshot();
        let mechanics = catalog(vec![transition(
            &snapshot,
            "transition.a-to-b",
            "STAGE_A",
            "STAGE_B",
            Vec::new(),
        )]);
        let facts = facts();
        let solver = ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default()).unwrap();
        assert!(
            solver
                .authorization_graph(PlannerExecutionState::new(snapshot).unwrap())
                .unwrap_err()
                .to_string()
                .contains("requires upper-bound evaluation")
        );
    }

    #[test]
    fn state_operations_can_satisfy_predicate_obligations_without_named_discharge_claims() {
        let snapshot = snapshot();
        let mut mechanics = catalog(vec![transition(
            &snapshot,
            "transition.a-to-b",
            "STAGE_A",
            "STAGE_B",
            vec!["obligation.gate-open".into()],
        )]);
        mechanics.obligations = vec![FeasibilityObligation {
            id: "obligation.gate-open".into(),
            label: "Gate state is open".into(),
            scope: scope(&snapshot),
            obligation_kind: ObligationKind::ActorState,
            detail: ObligationDetail::Predicate {
                predicate: gate_is("gate.path-open", true),
            },
            evidence: evidence(TruthStatus::Established),
        }];
        mechanics.techniques = vec![Technique {
            id: "technique.open-gate".into(),
            label: "Open the gate".into(),
            scope: scope(&snapshot),
            prerequisites: PredicateExpression::True,
            operations: vec![StateOperation::SetGate {
                gate_id: "gate.path-open".into(),
            }],
            discharged_obligation_ids: Vec::new(),
            introduced_obligation_ids: Vec::new(),
            cost: RouteCost {
                axes: BTreeMap::new(),
            },
            evidence: evidence(TruthStatus::Established),
        }];
        let facts = facts();
        let solver = ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default()).unwrap();
        let result = solver
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_B"),
            )
            .unwrap();
        assert_eq!(result.status, SearchStatus::Reached);
        assert_eq!(result.steps.len(), 2);
        assert_eq!(result.steps[0].action_id, "technique.open-gate");
        assert_eq!(result.steps[1].action_id, "transition.a-to-b");
        assert_eq!(
            result.steps[1].discharged_obligation_ids,
            vec!["obligation.gate-open"]
        );
        assert!(result.steps[1].selected_technique_ids.is_empty());
    }

    #[test]
    fn writer_actions_execute_only_when_their_gates_are_clear() {
        let mut start = snapshot();
        start.environment.components = vec![
            StateComponent {
                id: "restart.return-place".into(),
                component_kind: ComponentKind::Restart,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([(
                        "stage".into(),
                        StateValue::Text("ORDON_SPRING".into()),
                    )]),
                },
                binding: ComponentBinding::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                lifetime: SemanticLifetime::RuntimeFile,
                serialization_owner: SerializationOwner::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::TraceObservation,
                    source_id: "trace.held-return-place".into(),
                    source_sha256: Some(Digest([0x41; 32])),
                    transition_id: None,
                }],
            },
            StateComponent {
                id: "temporary.event-flags".into(),
                component_kind: ComponentKind::TemporaryFlags,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([("no_telop".into(), StateValue::Boolean(true))]),
                },
                binding: ComponentBinding::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                lifetime: SemanticLifetime::RuntimeFile,
                serialization_owner: SerializationOwner::None,
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::TraceObservation,
                    source_id: "trace.fanadi-lock-active".into(),
                    source_sha256: Some(Digest([0x42; 32])),
                    transition_id: None,
                }],
            },
        ];
        let exact_scope = scope(&start);
        let mut mechanics = catalog(Vec::new());
        mechanics.writers = vec![WriterRule {
            id: "writer.savmem-castle-town".into(),
            scope: exact_scope.clone(),
            activation: PredicateExpression::True,
            operation: StateOperation::Write {
                target: crate::transition::ComponentFieldTarget {
                    component_id: "restart.return-place".into(),
                    field: "stage".into(),
                },
                value: StateValue::Text("CASTLE_TOWN".into()),
            },
            evidence: evidence(TruthStatus::Established),
        }];
        mechanics.gates = vec![GateRule {
            id: "gate.no-telop".into(),
            scope: exact_scope.clone(),
            active_when: PredicateExpression::Compare {
                left: ValueReference::ComponentField {
                    component_id: "temporary.event-flags".into(),
                    field: "no_telop".into(),
                },
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Boolean(true),
                },
            },
            blocked_writer_ids: vec!["writer.savmem-castle-town".into()],
            lifetime: SemanticLifetime::RuntimeFile,
            evidence: evidence(TruthStatus::Established),
        }];
        let goal = PredicateExpression::Compare {
            left: ValueReference::ComponentField {
                component_id: "restart.return-place".into(),
                field: "stage".into(),
            },
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Text("CASTLE_TOWN".into()),
            },
        };

        let blocked = ForwardSolver::new(&facts(), &mechanics, &[], SolverOptions::default())
            .unwrap()
            .solve(PlannerExecutionState::new(start.clone()).unwrap(), &goal)
            .unwrap();
        assert_eq!(blocked.status, SearchStatus::UnreachableUnderModel);
        assert_eq!(blocked.blocked_writer_witnesses.len(), 1);
        assert_eq!(
            blocked.blocked_writer_witnesses[0].classification,
            WriterClassification::GateBlocked
        );
        assert_eq!(
            blocked.blocked_writer_witnesses[0].active_gate_ids,
            vec!["gate.no-telop"]
        );
        assert!(
            blocked.blocked_writer_witnesses[0]
                .evidence_dependencies
                .iter()
                .any(|dependency| dependency.dependency_kind == EvidenceDependencyKind::Writer)
        );
        assert!(
            blocked.blocked_writer_witnesses[0]
                .evidence_dependencies
                .iter()
                .any(|dependency| dependency.dependency_kind == EvidenceDependencyKind::Gate)
        );

        mechanics.techniques = vec![Technique {
            id: "technique.end-fanadi-lock".into(),
            label: "Clear NO_TELOP at the evidenced end of Fanadi's flow".into(),
            scope: exact_scope,
            prerequisites: PredicateExpression::True,
            operations: vec![StateOperation::Write {
                target: crate::transition::ComponentFieldTarget {
                    component_id: "temporary.event-flags".into(),
                    field: "no_telop".into(),
                },
                value: StateValue::Boolean(false),
            }],
            discharged_obligation_ids: Vec::new(),
            introduced_obligation_ids: Vec::new(),
            cost: RouteCost {
                axes: BTreeMap::new(),
            },
            evidence: evidence(TruthStatus::Established),
        }];
        let reached = ForwardSolver::new(&facts(), &mechanics, &[], SolverOptions::default())
            .unwrap()
            .solve(PlannerExecutionState::new(start).unwrap(), &goal)
            .unwrap();
        assert_eq!(reached.status, SearchStatus::Reached);
        assert_eq!(
            reached
                .steps
                .iter()
                .map(|step| (step.action_kind, step.action_id.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (SearchActionKind::Technique, "technique.end-fanadi-lock"),
                (SearchActionKind::Writer, "writer.savmem-castle-town")
            ]
        );
        assert!(
            reached.steps[1]
                .evidence_dependencies
                .iter()
                .any(|dependency| dependency.dependency_kind == EvidenceDependencyKind::Gate)
        );
    }

    #[test]
    fn fanadi_lock_preserves_only_the_last_successful_prelock_savmem_write() {
        let mut start = snapshot();
        start.environment.location.stage = "ROUTE_START".into();
        start.environment.components = vec![
            StateComponent {
                id: "inventory.quest".into(),
                component_kind: ComponentKind::Inventory,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([("has_ooccoo".into(), StateValue::Boolean(true))]),
                },
                binding: ComponentBinding::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                lifetime: SemanticLifetime::RuntimeFile,
                serialization_owner: SerializationOwner::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::TraceObservation,
                    source_id: "trace.ooccoo-owned".into(),
                    source_sha256: Some(Digest([0x51; 32])),
                    transition_id: None,
                }],
            },
            StateComponent {
                id: "restart.return-place".into(),
                component_kind: ComponentKind::Restart,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([(
                        "stage".into(),
                        StateValue::Text("ORDON_SPRING".into()),
                    )]),
                },
                binding: ComponentBinding::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                lifetime: SemanticLifetime::RuntimeFile,
                serialization_owner: SerializationOwner::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::TraceObservation,
                    source_id: "trace.ordon-return-place".into(),
                    source_sha256: Some(Digest([0x52; 32])),
                    transition_id: None,
                }],
            },
            StateComponent {
                id: "route.fanadi".into(),
                component_kind: ComponentKind::Session,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([("lock_seen".into(), StateValue::Boolean(false))]),
                },
                binding: ComponentBinding::Session {
                    session_id: "session-1".into(),
                },
                lifetime: SemanticLifetime::Session,
                serialization_owner: SerializationOwner::None,
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::Initialized,
                    source_id: "fixture.fanadi-route".into(),
                    source_sha256: Some(Digest([0x53; 32])),
                    transition_id: None,
                }],
            },
            StateComponent {
                id: "savmem.castle-town-placement".into(),
                component_kind: ComponentKind::ActorInstance,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([
                        ("event_guard".into(), StateValue::Boolean(true)),
                        ("inside".into(), StateValue::Boolean(false)),
                        ("switch_guard".into(), StateValue::Boolean(false)),
                    ]),
                },
                binding: ComponentBinding::Actor {
                    instance_id: "savmem.castle-town-placement".into(),
                },
                lifetime: SemanticLifetime::StageLoad,
                serialization_owner: SerializationOwner::None,
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::ExtractedFact,
                    source_id: "placement.castle-town-savmem".into(),
                    source_sha256: Some(Digest([0x54; 32])),
                    transition_id: None,
                }],
            },
            StateComponent {
                id: "temporary.event-flags".into(),
                component_kind: ComponentKind::TemporaryFlags,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([("no_telop".into(), StateValue::Boolean(false))]),
                },
                binding: ComponentBinding::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                lifetime: SemanticLifetime::RuntimeFile,
                serialization_owner: SerializationOwner::None,
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::TraceObservation,
                    source_id: "trace.no-telop-clear".into(),
                    source_sha256: Some(Digest([0x55; 32])),
                    transition_id: None,
                }],
            },
        ];
        start
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
        let exact_scope = scope(&start);
        let field_is =
            |component_id: &str, field: &str, value: StateValue| PredicateExpression::Compare {
                left: ValueReference::ComponentField {
                    component_id: component_id.into(),
                    field: field.into(),
                },
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal { value },
            };
        let return_is = |stage: &str| {
            field_is(
                "restart.return-place",
                "stage",
                StateValue::Text(stage.into()),
            )
        };
        let write = |component_id: &str, field: &str, value: StateValue| StateOperation::Write {
            target: crate::transition::ComponentFieldTarget {
                component_id: component_id.into(),
                field: field.into(),
            },
            value,
        };
        let location = |stage: &str| StateOperation::SetLocation {
            location: SceneLocation {
                stage: stage.into(),
                room: 0,
                layer: 0,
                spawn: 0,
            },
        };
        let candidate =
            |id: &str,
             label: &str,
             kind: TransitionKind,
             guard: PredicateExpression,
             effects: Vec<StateOperation>| CandidateTransition {
                id: id.into(),
                label: label.into(),
                scope: exact_scope.clone(),
                transition_kind: kind,
                approach_id: format!("approach.{id}"),
                activation: ActivationContract {
                    hard_guards: guard,
                    physical_obligation_ids: Vec::new(),
                    effects,
                    unknown_requirements: Vec::new(),
                },
                evidence: evidence(TruthStatus::Established),
            };

        let mut mechanics = catalog(vec![
            candidate(
                "transition.enter-fanadi-after-savmem",
                "Continue from the Castle Town SavMem placement to Fanadi",
                TransitionKind::EncodedMapExit,
                PredicateExpression::All {
                    terms: vec![stage_is("CASTLE_SAVMEM"), return_is("CASTLE_TOWN")],
                },
                vec![
                    write(
                        "savmem.castle-town-placement",
                        "inside",
                        StateValue::Boolean(false),
                    ),
                    location("FANADI"),
                ],
            ),
            candidate(
                "transition.fanadi-clear-no-telop",
                "Fanadi's normal cleanup clears NO_TELOP",
                TransitionKind::MessageAction,
                PredicateExpression::All {
                    terms: vec![
                        stage_is("FANADI"),
                        field_is(
                            "temporary.event-flags",
                            "no_telop",
                            StateValue::Boolean(true),
                        ),
                    ],
                },
                vec![write(
                    "temporary.event-flags",
                    "no_telop",
                    StateValue::Boolean(false),
                )],
            ),
            candidate(
                "transition.fanadi-set-no-telop",
                "Fanadi sets NO_TELOP with Ooccoo available",
                TransitionKind::MessageAction,
                PredicateExpression::All {
                    terms: vec![
                        stage_is("FANADI"),
                        field_is("inventory.quest", "has_ooccoo", StateValue::Boolean(true)),
                        field_is(
                            "temporary.event-flags",
                            "no_telop",
                            StateValue::Boolean(false),
                        ),
                    ],
                },
                vec![
                    write(
                        "temporary.event-flags",
                        "no_telop",
                        StateValue::Boolean(true),
                    ),
                    write("route.fanadi", "lock_seen", StateValue::Boolean(true)),
                ],
            ),
            candidate(
                "transition.pass-castle-savmem",
                "Pass through the Castle Town SavMem placement",
                TransitionKind::ActorReload,
                stage_is("ROUTE_START"),
                vec![
                    write(
                        "savmem.castle-town-placement",
                        "inside",
                        StateValue::Boolean(true),
                    ),
                    location("CASTLE_SAVMEM"),
                ],
            ),
            candidate(
                "transition.savewarp-castle-town",
                "Savewarp using a held Castle Town return place",
                TransitionKind::SaveWarp,
                PredicateExpression::All {
                    terms: vec![
                        return_is("CASTLE_TOWN"),
                        field_is("route.fanadi", "lock_seen", StateValue::Boolean(true)),
                    ],
                },
                vec![location("SAVEWARP_CASTLE_TOWN")],
            ),
            candidate(
                "transition.savewarp-ordon-spring",
                "Savewarp using a held Ordon Spring return place",
                TransitionKind::SaveWarp,
                PredicateExpression::All {
                    terms: vec![
                        return_is("ORDON_SPRING"),
                        field_is("route.fanadi", "lock_seen", StateValue::Boolean(true)),
                    ],
                },
                vec![location("SAVEWARP_ORDON_SPRING")],
            ),
        ]);
        mechanics
            .transitions
            .sort_by(|left, right| left.id.cmp(&right.id));
        mechanics.writers = vec![WriterRule {
            id: "writer.savmem-castle-town".into(),
            scope: exact_scope.clone(),
            activation: PredicateExpression::All {
                terms: vec![
                    stage_is("CASTLE_SAVMEM"),
                    field_is(
                        "savmem.castle-town-placement",
                        "inside",
                        StateValue::Boolean(true),
                    ),
                    field_is(
                        "savmem.castle-town-placement",
                        "event_guard",
                        StateValue::Boolean(true),
                    ),
                    field_is(
                        "savmem.castle-town-placement",
                        "switch_guard",
                        StateValue::Boolean(false),
                    ),
                ],
            },
            operation: write(
                "restart.return-place",
                "stage",
                StateValue::Text("CASTLE_TOWN".into()),
            ),
            evidence: evidence(TruthStatus::Established),
        }];
        mechanics.gates = vec![GateRule {
            id: "gate.no-telop".into(),
            scope: exact_scope.clone(),
            active_when: field_is(
                "temporary.event-flags",
                "no_telop",
                StateValue::Boolean(true),
            ),
            blocked_writer_ids: vec!["writer.savmem-castle-town".into()],
            lifetime: SemanticLifetime::RuntimeFile,
            evidence: evidence(TruthStatus::Established),
        }];
        mechanics.readers = vec![
            ReaderRule {
                id: "reader.savewarp-castle-town-return".into(),
                scope: exact_scope.clone(),
                source: ValueReference::ComponentField {
                    component_id: "restart.return-place".into(),
                    field: "stage".into(),
                },
                consuming_transition_id: "transition.savewarp-castle-town".into(),
                interpretation_fact_id: None,
                evidence: evidence(TruthStatus::Established),
            },
            ReaderRule {
                id: "reader.savewarp-ordon-spring-return".into(),
                scope: exact_scope.clone(),
                source: ValueReference::ComponentField {
                    component_id: "restart.return-place".into(),
                    field: "stage".into(),
                },
                consuming_transition_id: "transition.savewarp-ordon-spring".into(),
                interpretation_fact_id: None,
                evidence: evidence(TruthStatus::Established),
            },
        ];

        let castle = ForwardSolver::new(&facts(), &mechanics, &[], SolverOptions::default())
            .unwrap()
            .solve(
                PlannerExecutionState::new(start.clone()).unwrap(),
                &stage_is("SAVEWARP_CASTLE_TOWN"),
            )
            .unwrap();
        assert_eq!(castle.status, SearchStatus::Reached);
        assert_eq!(
            castle
                .steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "transition.pass-castle-savmem",
                "writer.savmem-castle-town",
                "transition.enter-fanadi-after-savmem",
                "transition.fanadi-set-no-telop",
                "transition.savewarp-castle-town",
            ]
        );
        assert_eq!(
            castle.steps[4].reader_results[0].source_value,
            StateValue::Text("CASTLE_TOWN".into())
        );

        let ordon_without_bypass = ForwardSolver::new(
            &facts(),
            &mechanics,
            &[],
            SolverOptions {
                evidence_policy: EvidencePolicy::RESEARCH,
                ..SolverOptions::default()
            },
        )
        .unwrap()
        .solve(
            PlannerExecutionState::new(start.clone()).unwrap(),
            &stage_is("SAVEWARP_ORDON_SPRING"),
        )
        .unwrap();
        assert_ne!(ordon_without_bypass.status, SearchStatus::Reached);

        let bypass = Technique {
            id: "technique.hypothetical-direct-fanadi-access".into(),
            label: "Reach Fanadi without crossing the Castle Town SavMem placement".into(),
            scope: exact_scope.clone(),
            prerequisites: stage_is("ROUTE_START"),
            operations: vec![location("FANADI")],
            discharged_obligation_ids: Vec::new(),
            introduced_obligation_ids: Vec::new(),
            cost: RouteCost {
                axes: BTreeMap::from([("theorycraft".into(), 1)]),
            },
            evidence: evidence(TruthStatus::Hypothetical),
        };
        let pack = RefinementPack {
            schema: REFINEMENT_PACK_SCHEMA.into(),
            manifest: RefinementPackManifest {
                id: "pack.hypothetical-direct-fanadi-access".into(),
                version: "1.0.0".into(),
                author: "Route planner acceptance fixture".into(),
                source: "Hypothetical Fanadi access avoiding intervening SavMem".into(),
                scope: exact_scope,
                precedence: 100,
                dependencies: Vec::new(),
                conflicts: Vec::new(),
            },
            rules: vec![RefinementRule {
                id: "rule.add-direct-fanadi-access".into(),
                label: "Add hypothetical direct Fanadi access".into(),
                operation: RefinementOperation::AddTechnique { technique: bypass },
                evidence: evidence(TruthStatus::Hypothetical),
            }],
        };
        let composed = ComposedPlannerCatalog::compose_layered(
            &facts(),
            &mechanics,
            &RefinementLayers {
                enabled_packs: Vec::new(),
                route_local_overlays: Vec::new(),
                ephemeral_what_if_overlays: vec![pack],
            },
        )
        .unwrap();
        let ordon_with_bypass = ForwardSolver::new(
            &composed.facts,
            &composed.mechanics,
            &[],
            SolverOptions {
                evidence_policy: EvidencePolicy::RESEARCH,
                ..SolverOptions::default()
            },
        )
        .unwrap()
        .solve(
            PlannerExecutionState::new(start).unwrap(),
            &stage_is("SAVEWARP_ORDON_SPRING"),
        )
        .unwrap();
        assert_eq!(ordon_with_bypass.status, SearchStatus::Reached);
        assert_eq!(
            ordon_with_bypass
                .steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "technique.hypothetical-direct-fanadi-access",
                "transition.fanadi-set-no-telop",
                "transition.savewarp-ordon-spring",
            ]
        );
        assert_eq!(
            ordon_with_bypass.steps[2].reader_results[0].source_value,
            StateValue::Text("ORDON_SPRING".into())
        );
        assert_eq!(
            ordon_with_bypass.steps[0].weakest_evidence,
            Some(TruthStatus::Hypothetical)
        );
    }

    #[test]
    fn hypothetical_local_bank_rebind_changes_semantics_without_changing_payload() {
        let mut snapshot = snapshot();
        snapshot.environment.components = vec![StateComponent {
            id: "stage.local-bank".into(),
            component_kind: ComponentKind::StageMemory,
            payload: ComponentPayload::Raw {
                bytes: vec![0x01],
                known_mask: vec![0xff],
            },
            binding: ComponentBinding::Stage {
                stage: "D_MN05".into(),
            },
            lifetime: SemanticLifetime::StageLoad,
            serialization_owner: SerializationOwner::StageBank {
                runtime_file_id: "file-0".into(),
                stage: "D_MN05".into(),
            },
            provenance: vec![ComponentProvenance {
                source_kind: ProvenanceSourceKind::TraceObservation,
                source_id: "trace.forest-local-bank".into(),
                source_sha256: Some(Digest([6; 32])),
                transition_id: None,
            }],
        }];
        let exact_scope = scope(&snapshot);
        let raw_binding = |stage: &str| RawFactBinding {
            component_kind: ComponentKind::StageMemory,
            binding: ComponentBindingReference::Exact {
                binding: ComponentBinding::Stage {
                    stage: stage.into(),
                },
            },
            byte_offset: 0,
            mask: vec![0x01],
            expected: vec![0x01],
        };
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: vec![
                FriendlyAlias {
                    id: "local.forest-switch".into(),
                    label: "Forest local switch".into(),
                    scope: exact_scope.clone(),
                    raw: raw_binding("D_MN05"),
                    evidence: evidence(TruthStatus::Established),
                },
                FriendlyAlias {
                    id: "local.tot-switch".into(),
                    label: "Temple of Time local switch".into(),
                    scope: exact_scope.clone(),
                    raw: raw_binding("D_MN06"),
                    evidence: evidence(TruthStatus::Established),
                },
            ],
            derived_facts: vec![DerivedFact {
                id: "path.tot-open".into(),
                label: "Temple of Time path is open".into(),
                scope: exact_scope.clone(),
                rule: PredicateExpression::Fact {
                    fact_id: "local.tot-switch".into(),
                },
                evidence: evidence(TruthStatus::Established),
            }],
        };
        let mut mechanics = catalog(vec![transition(
            &snapshot,
            "transition.a-to-b",
            "STAGE_A",
            "STAGE_B",
            vec!["obligation.tot-path".into()],
        )]);
        mechanics.obligations = vec![FeasibilityObligation {
            id: "obligation.tot-path".into(),
            label: "Temple of Time local path is open".into(),
            scope: exact_scope.clone(),
            obligation_kind: ObligationKind::ActorState,
            detail: ObligationDetail::Predicate {
                predicate: PredicateExpression::Fact {
                    fact_id: "path.tot-open".into(),
                },
            },
            evidence: evidence(TruthStatus::Established),
        }];
        let technique = Technique {
            id: "technique.hypothetical-local-bank-rebind".into(),
            label: "Preserve Forest local memory and interpret it as Temple of Time".into(),
            scope: exact_scope.clone(),
            prerequisites: PredicateExpression::True,
            operations: vec![
                StateOperation::Preserve {
                    selector: ComponentSelector::Id {
                        component_id: "stage.local-bank".into(),
                    },
                },
                StateOperation::Rebind {
                    selector: ComponentSelector::Id {
                        component_id: "stage.local-bank".into(),
                    },
                    binding: ComponentBinding::Stage {
                        stage: "D_MN06".into(),
                    },
                },
            ],
            discharged_obligation_ids: Vec::new(),
            introduced_obligation_ids: Vec::new(),
            cost: RouteCost {
                axes: BTreeMap::from([("theorycraft".into(), 1)]),
            },
            evidence: evidence(TruthStatus::Hypothetical),
        };
        let pack = RefinementPack {
            schema: REFINEMENT_PACK_SCHEMA.into(),
            manifest: RefinementPackManifest {
                id: "pack.hypothetical-local-bank-rebind".into(),
                version: "1.0.0".into(),
                author: "Route planner acceptance fixture".into(),
                source: "Hypothetical local-bank transfer".into(),
                scope: exact_scope,
                precedence: 100,
                dependencies: Vec::new(),
                conflicts: Vec::new(),
            },
            rules: vec![RefinementRule {
                id: "rule.add-local-bank-rebind".into(),
                label: "Add the hypothetical local-bank rebind".into(),
                operation: RefinementOperation::AddTechnique {
                    technique: technique.clone(),
                },
                evidence: evidence(TruthStatus::Hypothetical),
            }],
        };
        let composed = ComposedPlannerCatalog::compose_layered(
            &facts,
            &mechanics,
            &RefinementLayers {
                enabled_packs: Vec::new(),
                route_local_overlays: Vec::new(),
                ephemeral_what_if_overlays: vec![pack],
            },
        )
        .unwrap();
        let options = SolverOptions {
            evidence_policy: EvidencePolicy::RESEARCH,
            ..SolverOptions::default()
        };

        let without_overlay = ForwardSolver::new(&facts, &mechanics, &[], options)
            .unwrap()
            .solve(
                PlannerExecutionState::new(snapshot.clone()).unwrap(),
                &stage_is("STAGE_B"),
            )
            .unwrap();
        assert_ne!(without_overlay.status, SearchStatus::Reached);

        let reached = ForwardSolver::new(&composed.facts, &composed.mechanics, &[], options)
            .unwrap()
            .solve(
                PlannerExecutionState::new(snapshot.clone()).unwrap(),
                &stage_is("STAGE_B"),
            )
            .unwrap();
        assert_eq!(reached.status, SearchStatus::Reached);
        assert_eq!(
            reached
                .steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "technique.hypothetical-local-bank-rebind",
                "transition.a-to-b"
            ]
        );
        assert!(reached.steps[1].selected_technique_ids.is_empty());
        assert_eq!(
            reached.steps[0].weakest_evidence,
            Some(TruthStatus::Hypothetical)
        );
        assert_eq!(
            reached.steps[1].weakest_evidence,
            Some(TruthStatus::Established)
        );
        assert!(
            reached.steps[1]
                .evidence_dependencies
                .iter()
                .any(|dependency| {
                    dependency.dependency_kind == EvidenceDependencyKind::Fact
                        && dependency.record_id == "path.tot-open"
                })
        );
        assert!(
            reached.steps[1]
                .evidence_dependencies
                .iter()
                .any(|dependency| {
                    dependency.dependency_kind == EvidenceDependencyKind::Fact
                        && dependency.record_id == "local.tot-switch"
                })
        );

        let before = PlannerExecutionState::new(snapshot).unwrap();
        let mut rebound = before.clone();
        rebound
            .apply_operations(
                &technique.id,
                "snapshot.rebound-local-bank",
                &technique.operations,
            )
            .unwrap();
        let diff = StateDiff::between(
            &before.snapshot,
            &rebound.snapshot,
            BoundaryKind::WrongStateRespawn,
        )
        .unwrap();
        assert_eq!(diff.component_deltas.len(), 1);
        assert_eq!(
            diff.component_deltas[0].payload_sha256_before,
            diff.component_deltas[0].payload_sha256_after
        );
        assert!(diff.component_deltas[0].raw_byte_deltas.is_empty());
        assert_ne!(
            diff.component_deltas[0].binding_before,
            diff.component_deltas[0].binding_after
        );
        let evaluator = PredicateEvaluator::new(
            &rebound.snapshot,
            &facts,
            &[],
            &rebound.gate_states,
            EvidencePolicy::RESEARCH,
        )
        .unwrap();
        assert_eq!(
            evaluator.evaluate(&PredicateExpression::Fact {
                fact_id: "path.tot-open".into(),
            }),
            EvaluatedTruth::True
        );
    }

    #[test]
    fn witnessed_temporal_obligation_reaches_and_retains_microtrace_proof() {
        let snapshot = snapshot();
        let mut mechanics = catalog(vec![transition(
            &snapshot,
            "transition.a-to-b",
            "STAGE_A",
            "STAGE_B",
            vec!["obligation.interrupt-window".into()],
        )]);
        mechanics.obligations = vec![FeasibilityObligation {
            id: "obligation.interrupt-window".into(),
            label: "Interrupt the dialogue within its input window".into(),
            scope: scope(&snapshot),
            obligation_kind: ObligationKind::Timing,
            detail: ObligationDetail::Temporal {
                requirement: TemporalRequirement {
                    action_id: "dialogue.test".into(),
                    window: TemporalWindow {
                        earliest_frame: 10,
                        latest_frame: 12,
                        required_input: Some("sidehop".into()),
                    },
                },
                precondition: PredicateExpression::True,
            },
            evidence: evidence(TruthStatus::Established),
        }];
        mechanics.microtraces = vec![WitnessedMicrotrace {
            id: "microtrace.dialogue-sidehop".into(),
            scope: scope(&snapshot),
            precondition: PredicateExpression::True,
            operations: vec![StateOperation::Interrupt {
                action_id: "dialogue.test".into(),
                window: TemporalWindow {
                    earliest_frame: 11,
                    latest_frame: 11,
                    required_input: Some("sidehop".into()),
                },
            }],
            postcondition: PredicateExpression::True,
            timing: TemporalWindow {
                earliest_frame: 11,
                latest_frame: 11,
                required_input: Some("sidehop".into()),
            },
            evidence: evidence(TruthStatus::Established),
        }];
        let facts = facts();
        let solver = ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default()).unwrap();
        let reached = solver
            .solve(
                PlannerExecutionState::new(snapshot.clone()).unwrap(),
                &stage_is("STAGE_B"),
            )
            .unwrap();
        assert_eq!(reached.status, SearchStatus::Reached);
        assert_eq!(
            reached.steps[0].supporting_microtrace_ids,
            vec!["microtrace.dialogue-sidehop"]
        );

        mechanics.microtraces.clear();
        let solver = ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default()).unwrap();
        let unknown = solver
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_B"),
            )
            .unwrap();
        assert_eq!(unknown.status, SearchStatus::Unknown);
        assert_eq!(
            unknown.blocked_transition_witnesses[0].unknown_obligation_ids,
            vec!["obligation.interrupt-window"]
        );
    }

    #[test]
    fn route_book_pin_and_ban_actions_change_the_reached_path() {
        let snapshot = snapshot();
        let mut mechanics = catalog(vec![
            transition(
                &snapshot,
                "transition.a-to-b",
                "STAGE_A",
                "STAGE_B",
                Vec::new(),
            ),
            transition(
                &snapshot,
                "transition.a-to-d",
                "STAGE_A",
                "STAGE_D",
                Vec::new(),
            ),
            transition(
                &snapshot,
                "transition.d-to-b",
                "STAGE_D",
                "STAGE_B",
                Vec::new(),
            ),
        ]);
        mechanics.goals = vec![goal("goal.b", "STAGE_B")];
        let facts = facts();
        let book = route_book(
            &snapshot,
            vec![
                RouteDirective {
                    id: "directive.ban-direct".into(),
                    scope: scope(&snapshot),
                    directive: RouteDirectiveKind::BanAction {
                        action: RouteActionRef::Transition {
                            transition_id: "transition.a-to-b".into(),
                        },
                    },
                },
                RouteDirective {
                    id: "directive.pin-detour".into(),
                    scope: scope(&snapshot),
                    directive: RouteDirectiveKind::PinAction {
                        action: RouteActionRef::Transition {
                            transition_id: "transition.a-to-d".into(),
                        },
                    },
                },
            ],
        );
        let solver = ForwardSolver::new_with_route_book(
            &facts,
            &mechanics,
            &[],
            SolverOptions::default(),
            &book,
        )
        .unwrap();
        let result = solver
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_B"),
            )
            .unwrap();
        assert_eq!(result.status, SearchStatus::Reached);
        assert!(result.backward_pruning_applied);
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<Vec<_>>(),
            vec!["transition.a-to-d", "transition.d-to-b"]
        );
    }

    #[test]
    fn route_book_required_actions_are_backward_roots_without_retaining_noise() {
        let snapshot = snapshot();
        let mut mechanics = catalog(vec![transition(
            &snapshot,
            "transition.a-to-b",
            "STAGE_A",
            "STAGE_B",
            Vec::new(),
        )]);
        mechanics.goals = vec![goal("goal.b", "STAGE_B")];
        mechanics.techniques = vec![
            Technique {
                id: "technique.noise".into(),
                label: "Irrelevant noise".into(),
                scope: scope(&snapshot),
                prerequisites: PredicateExpression::True,
                operations: vec![StateOperation::SetGate {
                    gate_id: "gate.noise".into(),
                }],
                discharged_obligation_ids: Vec::new(),
                introduced_obligation_ids: Vec::new(),
                cost: RouteCost {
                    axes: BTreeMap::new(),
                },
                evidence: evidence(TruthStatus::Established),
            },
            Technique {
                id: "technique.required".into(),
                label: "Required route action".into(),
                scope: scope(&snapshot),
                prerequisites: PredicateExpression::True,
                operations: vec![StateOperation::SetGate {
                    gate_id: "gate.required".into(),
                }],
                discharged_obligation_ids: Vec::new(),
                introduced_obligation_ids: Vec::new(),
                cost: RouteCost {
                    axes: BTreeMap::new(),
                },
                evidence: evidence(TruthStatus::Established),
            },
        ];
        let facts = facts();
        let book = route_book(
            &snapshot,
            vec![RouteDirective {
                id: "directive.pin-required-technique".into(),
                scope: scope(&snapshot),
                directive: RouteDirectiveKind::PinAction {
                    action: RouteActionRef::Technique {
                        technique_id: "technique.required".into(),
                    },
                },
            }],
        );
        let result = ForwardSolver::new_with_route_book(
            &facts,
            &mechanics,
            &[],
            SolverOptions::default(),
            &book,
        )
        .unwrap()
        .solve(
            PlannerExecutionState::new(snapshot).unwrap(),
            &stage_is("STAGE_B"),
        )
        .unwrap();

        assert_eq!(result.status, SearchStatus::Reached);
        assert!(result.backward_pruning_applied);
        assert_eq!(
            result.backward_relevance.technique_ids,
            vec!["technique.required"]
        );
        assert_eq!(result.steps.len(), 2);
        assert!(
            result
                .steps
                .iter()
                .any(|step| step.action_id == "technique.required")
        );
    }

    #[test]
    fn selected_method_requires_its_actions_in_authored_order() {
        let snapshot = snapshot();
        let mut mechanics = catalog(vec![
            transition(
                &snapshot,
                "transition.a-to-b",
                "STAGE_A",
                "STAGE_B",
                Vec::new(),
            ),
            transition(
                &snapshot,
                "transition.a-to-d",
                "STAGE_A",
                "STAGE_D",
                Vec::new(),
            ),
            transition(
                &snapshot,
                "transition.d-to-b",
                "STAGE_D",
                "STAGE_B",
                Vec::new(),
            ),
        ]);
        mechanics.goals = vec![goal("goal.b", "STAGE_B")];
        let facts = facts();
        let mut book = route_book(&snapshot, Vec::new());
        book.steps = vec![
            ReferenceStep {
                id: "step.detour-enter".into(),
                label: "Enter detour".into(),
                scope: scope(&snapshot),
                action: RouteActionRef::Transition {
                    transition_id: "transition.a-to-d".into(),
                },
                precondition: None,
                postcondition: None,
                region_id: Some("region.detour".into()),
                annotation_ids: Vec::new(),
            },
            ReferenceStep {
                id: "step.detour-exit".into(),
                label: "Exit detour".into(),
                scope: scope(&snapshot),
                action: RouteActionRef::Transition {
                    transition_id: "transition.d-to-b".into(),
                },
                precondition: None,
                postcondition: None,
                region_id: Some("region.detour".into()),
                annotation_ids: Vec::new(),
            },
        ];
        book.methods = vec![PlanMethod {
            id: "method.detour".into(),
            label: "Take detour".into(),
            scope: scope(&snapshot),
            region_id: "region.detour".into(),
            step_ids: vec!["step.detour-enter".into(), "step.detour-exit".into()],
        }];
        book.regions = vec![PlanRegion {
            id: "region.detour".into(),
            label: "Detour".into(),
            scope: scope(&snapshot),
            parent_region_id: None,
            entry_predicate: Some(stage_is("STAGE_A")),
            outcome_predicate: stage_is("STAGE_B"),
            method_ids: vec!["method.detour".into()],
            selected_method_id: Some("method.detour".into()),
            collapse_policy: CollapsePolicy::Never,
        }];
        let solver = ForwardSolver::new_with_route_book(
            &facts,
            &mechanics,
            &[],
            SolverOptions::default(),
            &book,
        )
        .unwrap();
        let result = solver
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_B"),
            )
            .unwrap();
        assert_eq!(result.status, SearchStatus::Reached);
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<Vec<_>>(),
            vec!["transition.a-to-d", "transition.d-to-b"]
        );
    }

    #[test]
    fn preferences_choose_the_highest_scoring_equal_depth_route_once() {
        let snapshot = snapshot();
        let mut mechanics = catalog(vec![
            transition(
                &snapshot,
                "transition.a-to-b",
                "STAGE_A",
                "STAGE_B",
                Vec::new(),
            ),
            transition(
                &snapshot,
                "transition.a-to-c",
                "STAGE_A",
                "STAGE_C",
                Vec::new(),
            ),
            transition(
                &snapshot,
                "transition.b-to-g",
                "STAGE_B",
                "STAGE_G",
                Vec::new(),
            ),
            transition(
                &snapshot,
                "transition.c-to-g",
                "STAGE_C",
                "STAGE_G",
                Vec::new(),
            ),
        ]);
        mechanics.goals = vec![goal("goal.g", "STAGE_G")];
        let facts = facts();
        let mut book = route_book(
            &snapshot,
            vec![
                RouteDirective {
                    id: "directive.prefer-action".into(),
                    scope: scope(&snapshot),
                    directive: RouteDirectiveKind::PreferAction {
                        action: RouteActionRef::Transition {
                            transition_id: "transition.a-to-c".into(),
                        },
                        weight: 5,
                    },
                },
                RouteDirective {
                    id: "directive.prefer-method".into(),
                    scope: scope(&snapshot),
                    directive: RouteDirectiveKind::PreferMethod {
                        method_id: "method.c-route".into(),
                        weight: 10,
                    },
                },
            ],
        );
        book.goal_ids = vec!["goal.g".into()];
        book.steps = vec![
            ReferenceStep {
                id: "step.c-enter".into(),
                label: "Enter C".into(),
                scope: scope(&snapshot),
                action: RouteActionRef::Transition {
                    transition_id: "transition.a-to-c".into(),
                },
                precondition: None,
                postcondition: None,
                region_id: Some("region.c-route".into()),
                annotation_ids: Vec::new(),
            },
            ReferenceStep {
                id: "step.c-exit".into(),
                label: "Exit C".into(),
                scope: scope(&snapshot),
                action: RouteActionRef::Transition {
                    transition_id: "transition.c-to-g".into(),
                },
                precondition: None,
                postcondition: None,
                region_id: Some("region.c-route".into()),
                annotation_ids: Vec::new(),
            },
        ];
        book.methods = vec![PlanMethod {
            id: "method.c-route".into(),
            label: "C route".into(),
            scope: scope(&snapshot),
            region_id: "region.c-route".into(),
            step_ids: vec!["step.c-enter".into(), "step.c-exit".into()],
        }];
        book.regions = vec![PlanRegion {
            id: "region.c-route".into(),
            label: "C route".into(),
            scope: scope(&snapshot),
            parent_region_id: None,
            entry_predicate: Some(stage_is("STAGE_A")),
            outcome_predicate: stage_is("STAGE_G"),
            method_ids: vec!["method.c-route".into()],
            selected_method_id: None,
            collapse_policy: CollapsePolicy::Never,
        }];
        let solver = ForwardSolver::new_with_route_book(
            &facts,
            &mechanics,
            &[],
            SolverOptions::default(),
            &book,
        )
        .unwrap();
        let result = solver
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_G"),
            )
            .unwrap();
        assert_eq!(result.status, SearchStatus::Reached);
        assert_eq!(result.preference_score, 15);
        assert_eq!(
            result.satisfied_preference_ids,
            vec!["directive.prefer-action", "directive.prefer-method"]
        );
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<Vec<_>>(),
            vec!["transition.a-to-c", "transition.c-to-g"]
        );
    }

    #[test]
    fn cost_limits_prune_over_budget_techniques_and_report_route_totals() {
        let snapshot = snapshot();
        let mut mechanics = catalog(Vec::new());
        mechanics.techniques = vec![Technique {
            id: "technique.to-c".into(),
            label: "Reach C".into(),
            scope: scope(&snapshot),
            prerequisites: PredicateExpression::True,
            operations: vec![StateOperation::SetLocation {
                location: SceneLocation {
                    stage: "STAGE_C".into(),
                    room: 0,
                    layer: 0,
                    spawn: 0,
                },
            }],
            discharged_obligation_ids: Vec::new(),
            introduced_obligation_ids: Vec::new(),
            cost: RouteCost {
                axes: BTreeMap::from([("difficulty".into(), 2)]),
            },
            evidence: evidence(TruthStatus::Established),
        }];
        mechanics.goals = vec![goal("goal.c", "STAGE_C")];
        let facts = facts();
        let mut book = route_book(&snapshot, Vec::new());
        book.goal_ids = vec!["goal.c".into()];
        book.constraints = vec![RouteConstraint {
            id: "constraint.difficulty".into(),
            scope: scope(&snapshot),
            constraint: PathConstraint::CostAtMost {
                axis: "difficulty".into(),
                maximum: 1,
            },
        }];
        let constrained = ForwardSolver::new_with_route_book(
            &facts,
            &mechanics,
            &[],
            SolverOptions::default(),
            &book,
        )
        .unwrap()
        .solve(
            PlannerExecutionState::new(snapshot.clone()).unwrap(),
            &stage_is("STAGE_C"),
        )
        .unwrap();
        assert_eq!(constrained.status, SearchStatus::UnreachableUnderModel);

        let PathConstraint::CostAtMost { maximum, .. } = &mut book.constraints[0].constraint else {
            unreachable!();
        };
        *maximum = 2;
        let admitted = ForwardSolver::new_with_route_book(
            &facts,
            &mechanics,
            &[],
            SolverOptions::default(),
            &book,
        )
        .unwrap()
        .solve(
            PlannerExecutionState::new(snapshot).unwrap(),
            &stage_is("STAGE_C"),
        )
        .unwrap();
        assert_eq!(admitted.status, SearchStatus::Reached);
        assert_eq!(
            admitted.route_costs,
            BTreeMap::from([("difficulty".into(), 2)])
        );
    }

    #[test]
    fn resource_dominance_merges_only_an_exact_continuation_with_a_proof() {
        let snapshot = snapshot();
        let mut mechanics = catalog(vec![transition(
            &snapshot,
            "transition.b-to-g",
            "STAGE_B",
            "STAGE_G",
            Vec::new(),
        )]);
        mechanics.techniques = vec![
            location_technique(
                &snapshot,
                "technique.cheap-to-b",
                "STAGE_B",
                &[("difficulty", 1)],
            ),
            location_technique(
                &snapshot,
                "technique.expensive-to-b",
                "STAGE_B",
                &[("difficulty", 3)],
            ),
        ];
        mechanics.goals = vec![goal("goal.g", "STAGE_G")];
        let result = ForwardSolver::new(&facts(), &mechanics, &[], SolverOptions::default())
            .unwrap()
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_G"),
            )
            .unwrap();

        assert_eq!(result.status, SearchStatus::Reached);
        assert_eq!(result.explored_states, 3);
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<Vec<_>>(),
            vec!["technique.cheap-to-b", "transition.b-to-g"]
        );
        for proof in &result.continuation_merge_proofs {
            proof.validate().unwrap();
        }
        let proof = result
            .continuation_merge_proofs
            .iter()
            .find(|proof| proof.dominating.depth == 1 && proof.dominated.depth == 1)
            .unwrap();
        assert_eq!(proof.dominating.depth, 1);
        assert_eq!(proof.dominated.depth, 1);
        assert_eq!(
            proof.dominating.route_costs,
            BTreeMap::from([("difficulty".into(), 1)])
        );
        assert_eq!(
            proof.dominated.route_costs,
            BTreeMap::from([("difficulty".into(), 3)])
        );
        let mut detached = proof.clone();
        detached
            .dominating
            .route_costs
            .insert("difficulty".into(), 4);
        assert!(detached.validate().is_err());
        ContinuationMergeProof {
            continuation: proof.continuation.clone(),
            dominating: SearchResourceLabel {
                depth: 1,
                route_costs: BTreeMap::new(),
            },
            dominated: SearchResourceLabel {
                depth: 2,
                route_costs: BTreeMap::new(),
            },
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn incomparable_resource_labels_remain_separate_continuations() {
        let snapshot = snapshot();
        let mut mechanics = catalog(vec![transition(
            &snapshot,
            "transition.b-to-g",
            "STAGE_B",
            "STAGE_G",
            Vec::new(),
        )]);
        mechanics.techniques = vec![
            location_technique(
                &snapshot,
                "technique.low-difficulty-to-b",
                "STAGE_B",
                &[("difficulty", 1), ("time", 3)],
            ),
            location_technique(
                &snapshot,
                "technique.low-time-to-b",
                "STAGE_B",
                &[("difficulty", 3), ("time", 1)],
            ),
        ];
        mechanics.goals = vec![goal("goal.g", "STAGE_G")];
        let result = ForwardSolver::new(&facts(), &mechanics, &[], SolverOptions::default())
            .unwrap()
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_G"),
            )
            .unwrap();

        assert_eq!(result.status, SearchStatus::Reached);
        assert_eq!(result.explored_states, 4);
        assert!(
            result
                .continuation_merge_proofs
                .iter()
                .all(|proof| proof.dominated.depth > 1)
        );
        for proof in &result.continuation_merge_proofs {
            proof.validate().unwrap();
        }
        assert!(!strictly_dominates(
            &SearchResourceLabel {
                depth: 1,
                route_costs: BTreeMap::from([("difficulty".into(), 1), ("time".into(), 3),]),
            },
            &SearchResourceLabel {
                depth: 1,
                route_costs: BTreeMap::from([("difficulty".into(), 3), ("time".into(), 1),]),
            },
        ));
    }

    #[test]
    fn bounded_alternative_search_returns_only_nondominated_goal_plans() {
        let snapshot = snapshot();
        let mut mechanics = catalog(Vec::new());
        mechanics.techniques = vec![
            location_technique(
                &snapshot,
                "technique.dominated",
                "STAGE_G",
                &[("difficulty", 4), ("time", 4)],
            ),
            location_technique(
                &snapshot,
                "technique.low-difficulty",
                "STAGE_G",
                &[("difficulty", 1), ("time", 3)],
            ),
            location_technique(
                &snapshot,
                "technique.low-time",
                "STAGE_G",
                &[("difficulty", 3), ("time", 1)],
            ),
        ];
        mechanics.goals = vec![goal("goal.g", "STAGE_G")];
        let facts = facts();
        let solver = ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default()).unwrap();
        let result = solver
            .solve_alternatives(
                PlannerExecutionState::new(snapshot.clone()).unwrap(),
                &stage_is("STAGE_G"),
                3,
            )
            .unwrap();

        assert_eq!(result.status, SearchStatus::Reached);
        assert!(!result.hit_search_limit);
        assert_eq!(result.steps[0].action_id, "technique.low-difficulty");
        assert_eq!(
            result.route_costs,
            BTreeMap::from([("difficulty".into(), 1), ("time".into(), 3)])
        );
        assert_eq!(result.alternative_plans.len(), 1);
        assert_eq!(
            result.alternative_plans[0].steps[0].action_id,
            "technique.low-time"
        );
        assert_eq!(
            result.alternative_plans[0].route_costs,
            BTreeMap::from([("difficulty".into(), 3), ("time".into(), 1)])
        );
        result.alternative_plans[0].validate().unwrap();
        assert!(
            solver
                .solve_alternatives(
                    PlannerExecutionState::new(snapshot).unwrap(),
                    &stage_is("STAGE_G"),
                    0,
                )
                .is_err()
        );
    }

    #[test]
    fn evidence_threshold_restricts_research_mode_without_relaxing_it() {
        let snapshot = snapshot();
        let mut mechanics = catalog(Vec::new());
        mechanics.techniques = vec![Technique {
            id: "technique.contested-to-c".into(),
            label: "Contested route to C".into(),
            scope: scope(&snapshot),
            prerequisites: PredicateExpression::True,
            operations: vec![StateOperation::SetLocation {
                location: SceneLocation {
                    stage: "STAGE_C".into(),
                    room: 0,
                    layer: 0,
                    spawn: 0,
                },
            }],
            discharged_obligation_ids: Vec::new(),
            introduced_obligation_ids: Vec::new(),
            cost: RouteCost {
                axes: BTreeMap::new(),
            },
            evidence: evidence(TruthStatus::Contested),
        }];
        mechanics.goals = vec![goal("goal.c", "STAGE_C")];
        let facts = facts();
        let options = SolverOptions {
            evidence_policy: EvidencePolicy::RESEARCH,
            ..SolverOptions::default()
        };
        let mut book = route_book(&snapshot, Vec::new());
        book.goal_ids = vec!["goal.c".into()];
        book.constraints = vec![RouteConstraint {
            id: "constraint.evidence".into(),
            scope: scope(&snapshot),
            constraint: PathConstraint::EvidenceAtLeast {
                minimum: "established".into(),
            },
        }];
        let established_only =
            ForwardSolver::new_with_route_book(&facts, &mechanics, &[], options, &book)
                .unwrap()
                .solve(
                    PlannerExecutionState::new(snapshot.clone()).unwrap(),
                    &stage_is("STAGE_C"),
                )
                .unwrap();
        assert_eq!(established_only.status, SearchStatus::UnreachableUnderModel);
        assert_eq!(
            established_only.minimum_evidence,
            Some(TruthStatus::Established)
        );

        let PathConstraint::EvidenceAtLeast { minimum } = &mut book.constraints[0].constraint
        else {
            unreachable!();
        };
        *minimum = "contested".into();
        let contested = ForwardSolver::new_with_route_book(&facts, &mechanics, &[], options, &book)
            .unwrap()
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_C"),
            )
            .unwrap();
        assert_eq!(contested.status, SearchStatus::Reached);
        assert_eq!(contested.minimum_evidence, Some(TruthStatus::Contested));
    }

    #[test]
    fn missing_guard_state_returns_unknown_instead_of_unreachable() {
        let snapshot = snapshot();
        let mut candidate = transition(
            &snapshot,
            "transition.unknown",
            "STAGE_A",
            "STAGE_B",
            Vec::new(),
        );
        candidate.activation.hard_guards = PredicateExpression::Compare {
            left: ValueReference::ComponentField {
                component_id: "missing.component".into(),
                field: "flag".into(),
            },
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Boolean(true),
            },
        };
        let mechanics = catalog(vec![candidate]);
        let facts = facts();
        let solver = ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default()).unwrap();
        let result = solver
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_B"),
            )
            .unwrap();
        assert_eq!(result.status, SearchStatus::Unknown);
        assert_eq!(result.unknown_transition_ids, vec!["transition.unknown"]);
        assert_eq!(result.blocked_transition_witnesses.len(), 1);
        assert_eq!(
            result.blocked_transition_witnesses[0].classification,
            TransitionClassification::FeasibilityUnknown
        );
        assert_eq!(
            result.blocked_transition_witnesses[0].hard_guard,
            EvaluatedTruth::Unknown
        );
    }

    #[test]
    fn exhausted_known_graph_returns_unreachable_under_the_model() {
        let snapshot = snapshot();
        let mechanics = catalog(Vec::new());
        let facts = facts();
        let solver = ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default()).unwrap();
        let result = solver
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_B"),
            )
            .unwrap();
        assert_eq!(result.status, SearchStatus::UnreachableUnderModel);
        assert!(!result.hit_search_limit);
    }

    #[test]
    fn unreachable_or_producers_form_one_validated_multi_action_cut() {
        let snapshot = snapshot();
        let mechanics = catalog(vec![
            transition(
                &snapshot,
                "transition.b-to-g",
                "STAGE_B",
                "STAGE_G",
                Vec::new(),
            ),
            transition(
                &snapshot,
                "transition.c-to-g",
                "STAGE_C",
                "STAGE_G",
                Vec::new(),
            ),
        ]);
        let result = ForwardSolver::new(&facts(), &mechanics, &[], SolverOptions::default())
            .unwrap()
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_G"),
            )
            .unwrap();

        assert_eq!(result.status, SearchStatus::UnreachableUnderModel);
        let cut = result
            .failed_producer_cuts
            .iter()
            .find(|cut| cut.dependency == StateDependency::LocationStage)
            .unwrap();
        cut.validate().unwrap();
        assert_eq!(
            cut.blocked_producers
                .iter()
                .map(FailedProducerBlocker::action)
                .collect::<Vec<_>>(),
            vec![
                RouteActionRef::Transition {
                    transition_id: "transition.b-to-g".into(),
                },
                RouteActionRef::Transition {
                    transition_id: "transition.c-to-g".into(),
                },
            ]
        );
        let mut detached = cut.clone();
        detached.blocked_producers.reverse();
        assert!(detached.validate().is_err());
    }

    #[test]
    fn an_executed_or_unsupported_producer_suppresses_a_false_cut() {
        let snapshot = snapshot();
        let mut mechanics = catalog(vec![
            transition(
                &snapshot,
                "transition.a-to-b",
                "STAGE_A",
                "STAGE_B",
                Vec::new(),
            ),
            transition(
                &snapshot,
                "transition.c-to-g",
                "STAGE_C",
                "STAGE_G",
                Vec::new(),
            ),
        ]);
        let executed = ForwardSolver::new(&facts(), &mechanics, &[], SolverOptions::default())
            .unwrap()
            .solve(
                PlannerExecutionState::new(snapshot.clone()).unwrap(),
                &stage_is("STAGE_G"),
            )
            .unwrap();
        assert_eq!(executed.status, SearchStatus::UnreachableUnderModel);
        assert!(
            executed
                .failed_producer_cuts
                .iter()
                .all(|cut| cut.dependency != StateDependency::LocationStage)
        );

        mechanics.transitions.remove(0);
        mechanics.techniques = vec![location_technique(
            &snapshot,
            "technique.to-b",
            "STAGE_B",
            &[],
        )];
        let unsupported = ForwardSolver::new(&facts(), &mechanics, &[], SolverOptions::default())
            .unwrap()
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_G"),
            )
            .unwrap();
        assert_eq!(unsupported.status, SearchStatus::UnreachableUnderModel);
        assert!(
            unsupported
                .failed_producer_cuts
                .iter()
                .all(|cut| cut.dependency != StateDependency::LocationStage)
        );
    }

    #[test]
    fn resolver_choice_is_applied_to_the_specific_obstructed_edge() {
        let snapshot = snapshot();
        let mechanics = obstructed_transition_catalog(&snapshot);
        let facts = facts();
        let solver = ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default()).unwrap();
        let result = solver
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_B"),
            )
            .unwrap();
        assert_eq!(result.status, SearchStatus::Reached);
        assert_eq!(
            result.steps[0].selected_resolver_ids,
            vec!["resolver.text-state"]
        );
        assert_eq!(
            result.steps[0].active_obstruction_ids,
            vec!["obstruction.npc"]
        );
        assert_eq!(
            result.steps[0].discharged_obligation_ids,
            vec!["obligation.blocker"]
        );
        assert!(result.steps[0].outstanding_obligation_ids.is_empty());
        assert_eq!(result.steps[0].action_derivations.len(), 2);
        assert_eq!(
            result.steps[0]
                .action_derivations
                .iter()
                .map(|derivation| derivation.action.clone())
                .collect::<Vec<_>>(),
            vec![
                RouteActionRef::Resolver {
                    resolver_id: "resolver.text-state".into()
                },
                RouteActionRef::Transition {
                    transition_id: "transition.a-to-b".into()
                }
            ]
        );
        assert!(
            result.steps[0]
                .action_derivations
                .iter()
                .all(|derivation| derivation.precondition_result == EvaluatedTruth::True)
        );
        assert_eq!(result.steps[0].obligation_derivations.len(), 1);
        assert_eq!(
            result.steps[0].obligation_derivations[0].id,
            "obligation.blocker"
        );
        assert!(result.blocked_transition_witnesses.is_empty());
    }

    #[test]
    fn auru_activation_is_separate_from_the_shared_recent_item_grant() {
        const FISHING_ROD: u64 = 0x4a;
        const AURUS_MEMO: u64 = 0x90;

        let setup = |content_byte: u8, recent_item_id: u64| {
            let mut state = snapshot();
            state.environment.runtime_configuration.content_sha256 = Digest([content_byte; 32]);
            state.environment.player.action = "talk".into();
            state.environment.components = vec![
                StateComponent {
                    id: "event.recent-item".into(),
                    component_kind: ComponentKind::Session,
                    payload: ComponentPayload::Structured {
                        fields: BTreeMap::from([(
                            "get_item_no".into(),
                            StateValue::Unsigned(recent_item_id),
                        )]),
                    },
                    binding: ComponentBinding::Session {
                        session_id: "session-1".into(),
                    },
                    lifetime: SemanticLifetime::Session,
                    serialization_owner: SerializationOwner::None,
                    provenance: vec![ComponentProvenance {
                        source_kind: ProvenanceSourceKind::Transition,
                        source_id: format!("writer.item-{recent_item_id:02x}"),
                        source_sha256: Some(Digest([7; 32])),
                        transition_id: Some(format!("writer.item-{recent_item_id:02x}")),
                    }],
                },
                StateComponent {
                    id: "inventory.active".into(),
                    component_kind: ComponentKind::Inventory,
                    payload: ComponentPayload::Structured {
                        fields: BTreeMap::from([(
                            "owned_item_ids".into(),
                            StateValue::Bytes(vec![0; 32]),
                        )]),
                    },
                    binding: ComponentBinding::RuntimeFile {
                        runtime_file_id: "file-0".into(),
                    },
                    lifetime: SemanticLifetime::RuntimeFile,
                    serialization_owner: SerializationOwner::RuntimeFile {
                        runtime_file_id: "file-0".into(),
                    },
                    provenance: vec![ComponentProvenance {
                        source_kind: ProvenanceSourceKind::Initialized,
                        source_id: "fixture.empty-inventory".into(),
                        source_sha256: Some(Digest([8; 32])),
                        transition_id: None,
                    }],
                },
            ];
            state
        };
        let item_goal = |item_id: u64| {
            let mut mask = vec![0; 32];
            mask[item_id as usize / 8] |= 1 << (item_id % 8);
            PredicateExpression::Compare {
                left: ValueReference::ComponentField {
                    component_id: "inventory.active".into(),
                    field: "owned_item_ids".into(),
                },
                operator: ComparisonOperator::ContainsBits,
                right: ValueReference::Literal {
                    value: StateValue::Bytes(mask),
                },
            }
        };
        let mechanics_for = |state: &StateSnapshot| {
            let exact_scope = scope(state);
            let mut mechanics = catalog(vec![CandidateTransition {
                id: "transition.auru-generic-get-item".into(),
                label: "Auru DEFAULT_GETITEM handoff".into(),
                scope: exact_scope.clone(),
                transition_kind: TransitionKind::ItemAcquisition,
                approach_id: "approach.auru-talk-outside-trigger".into(),
                activation: ActivationContract {
                    hard_guards: PredicateExpression::Compare {
                        left: ValueReference::ComponentField {
                            component_id: "event.recent-item".into(),
                            field: "get_item_no".into(),
                        },
                        operator: ComparisonOperator::NotEqual,
                        right: ValueReference::Literal {
                            value: StateValue::Unsigned(0xff),
                        },
                    },
                    physical_obligation_ids: vec!["obligation.auru-activation".into()],
                    effects: vec![StateOperation::SetBitFromValue {
                        source: crate::transition::ComponentFieldTarget {
                            component_id: "event.recent-item".into(),
                            field: "get_item_no".into(),
                        },
                        target: crate::transition::ComponentFieldTarget {
                            component_id: "inventory.active".into(),
                            field: "owned_item_ids".into(),
                        },
                    }],
                    unknown_requirements: Vec::new(),
                },
                evidence: evidence(TruthStatus::Established),
            }]);
            mechanics.obligations = vec![FeasibilityObligation {
                id: "obligation.auru-activation".into(),
                label: "Reach Auru's talk volume outside his cutscene trigger with control".into(),
                scope: exact_scope.clone(),
                obligation_kind: ObligationKind::Interaction,
                detail: ObligationDetail::Interaction {
                    actor_instance_id: "actor.auru".into(),
                    interaction_mode: "talk".into(),
                    required_volumes: vec![crate::transition::VolumeReference {
                        object_id: "actor.auru".into(),
                        volume_id: "talk".into(),
                    }],
                    excluded_volumes: vec![crate::transition::VolumeReference {
                        object_id: "actor.auru".into(),
                        volume_id: "cutscene-trigger".into(),
                    }],
                    pose_predicate: PredicateExpression::All {
                        terms: vec![
                            PredicateExpression::Compare {
                                left: ValueReference::PlayerControl,
                                operator: ComparisonOperator::Equal,
                                right: ValueReference::Literal {
                                    value: StateValue::Boolean(true),
                                },
                            },
                            PredicateExpression::Compare {
                                left: ValueReference::PlayerAction,
                                operator: ComparisonOperator::Equal,
                                right: ValueReference::Literal {
                                    value: StateValue::Text("talk".into()),
                                },
                            },
                        ],
                    },
                    temporal_requirement: None,
                },
                evidence: evidence(TruthStatus::Established),
            }];
            mechanics.readers = vec![ReaderRule {
                id: "reader.default-get-item-recent-item".into(),
                scope: exact_scope.clone(),
                source: ValueReference::ComponentField {
                    component_id: "event.recent-item".into(),
                    field: "get_item_no".into(),
                },
                consuming_transition_id: "transition.auru-generic-get-item".into(),
                interpretation_fact_id: None,
                evidence: evidence(TruthStatus::Established),
            }];
            mechanics.obstructions = vec![Obstruction {
                id: "obstruction.auru-trigger-overlap".into(),
                label: "Known Auru activation geometry has no enabled setup".into(),
                scope: exact_scope,
                blocked_action_id: "transition.auru-generic-get-item".into(),
                approach_id: "approach.auru-talk-outside-trigger".into(),
                active_when: PredicateExpression::True,
                obligation_ids: vec!["obligation.auru-activation".into()],
                evidence: evidence(TruthStatus::Established),
            }];
            mechanics
        };
        let resolver = |state: &StateSnapshot, id: &str, truth: TruthStatus| ObstructionResolver {
            id: id.into(),
            label: "Resolve Auru interaction geometry".into(),
            scope: scope(state),
            obstruction_id: "obstruction.auru-trigger-overlap".into(),
            resolution_kind: ResolutionKind::Satisfy,
            applicable_when: PredicateExpression::True,
            operations: Vec::new(),
            evidence: if truth == TruthStatus::Established {
                RuleEvidence {
                    truth,
                    records: vec![EvidenceRecord {
                        id: "external.hd-auru-targeting".into(),
                        kind: EvidenceKind::CommunityReported,
                        source_sha256: None,
                        note:
                            "External TPHD targeting-range evidence; no HD executable is imported."
                                .into(),
                    }],
                }
            } else {
                evidence(truth)
            },
        };

        let hd = setup(0x44, FISHING_ROD);
        let mut hd_mechanics = mechanics_for(&hd);
        hd_mechanics.resolvers = vec![resolver(
            &hd,
            "resolver.hd-external-auru-targeting",
            TruthStatus::Established,
        )];
        let hd_result = ForwardSolver::new(&facts(), &hd_mechanics, &[], SolverOptions::default())
            .unwrap()
            .solve(
                PlannerExecutionState::new(hd).unwrap(),
                &item_goal(FISHING_ROD),
            )
            .unwrap();
        assert_eq!(hd_result.status, SearchStatus::Reached);
        assert_eq!(
            hd_result.steps[0].selected_resolver_ids,
            vec!["resolver.hd-external-auru-targeting"]
        );
        assert_eq!(
            hd_result.steps[0].weakest_evidence,
            Some(TruthStatus::Established)
        );
        assert_eq!(
            hd_result.steps[0].reader_results,
            vec![ReaderResult {
                reader_id: "reader.default-get-item-recent-item".into(),
                source_value: StateValue::Unsigned(FISHING_ROD),
                interpretation: None,
            }]
        );
        assert!(
            hd_result.steps[0]
                .evidence_dependencies
                .iter()
                .any(|dependency| dependency.dependency_kind == EvidenceDependencyKind::Reader)
        );

        let mut missing_recent_item = setup(0x45, FISHING_ROD);
        missing_recent_item
            .environment
            .components
            .retain(|component| component.id != "event.recent-item");
        let mut missing_mechanics = mechanics_for(&missing_recent_item);
        missing_mechanics.resolvers = vec![resolver(
            &missing_recent_item,
            "resolver.hd-external-auru-targeting",
            TruthStatus::Established,
        )];
        let missing =
            ForwardSolver::new(&facts(), &missing_mechanics, &[], SolverOptions::default())
                .unwrap()
                .solve(
                    PlannerExecutionState::new(missing_recent_item).unwrap(),
                    &item_goal(FISHING_ROD),
                )
                .unwrap();
        assert_eq!(missing.status, SearchStatus::Unknown);
        assert_eq!(
            missing.blocked_transition_witnesses[0].unknown_reader_ids,
            vec!["reader.default-get-item-recent-item"]
        );

        for item_id in [FISHING_ROD, AURUS_MEMO] {
            let sd = setup(0x53, item_id);
            let base_mechanics = mechanics_for(&sd);
            let base = ForwardSolver::new(
                &facts(),
                &base_mechanics,
                &[],
                SolverOptions {
                    evidence_policy: EvidencePolicy::RESEARCH,
                    ..SolverOptions::default()
                },
            )
            .unwrap()
            .solve(
                PlannerExecutionState::new(sd.clone()).unwrap(),
                &item_goal(item_id),
            )
            .unwrap();
            assert_ne!(base.status, SearchStatus::Reached);
            assert_eq!(
                base.blocked_transition_witnesses[0].transition_id,
                "transition.auru-generic-get-item"
            );

            let hypothetical = resolver(
                &sd,
                "resolver.sd-hypothetical-auru-geometry",
                TruthStatus::Hypothetical,
            );
            let pack = RefinementPack {
                schema: REFINEMENT_PACK_SCHEMA.into(),
                manifest: RefinementPackManifest {
                    id: "pack.sd-hypothetical-auru-geometry".into(),
                    version: "1.0.0".into(),
                    author: "Route planner acceptance fixture".into(),
                    source: "Hypothetical SD interaction resolver".into(),
                    scope: scope(&sd),
                    precedence: 100,
                    dependencies: Vec::new(),
                    conflicts: Vec::new(),
                },
                rules: vec![RefinementRule {
                    id: "rule.add-sd-auru-geometry".into(),
                    label: "Add hypothetical SD Auru interaction resolver".into(),
                    operation: RefinementOperation::AddResolver {
                        resolver: hypothetical,
                    },
                    evidence: evidence(TruthStatus::Hypothetical),
                }],
            };
            let composed = ComposedPlannerCatalog::compose_layered(
                &facts(),
                &base_mechanics,
                &RefinementLayers {
                    enabled_packs: Vec::new(),
                    route_local_overlays: Vec::new(),
                    ephemeral_what_if_overlays: vec![pack],
                },
            )
            .unwrap();
            let reached = ForwardSolver::new(
                &composed.facts,
                &composed.mechanics,
                &[],
                SolverOptions {
                    evidence_policy: EvidencePolicy::RESEARCH,
                    ..SolverOptions::default()
                },
            )
            .unwrap()
            .solve(PlannerExecutionState::new(sd).unwrap(), &item_goal(item_id))
            .unwrap();
            assert_eq!(reached.status, SearchStatus::Reached);
            assert_eq!(
                reached.steps[0].selected_resolver_ids,
                vec!["resolver.sd-hypothetical-auru-geometry"]
            );
            assert_eq!(
                reached.steps[0].action_id,
                "transition.auru-generic-get-item"
            );
            assert_eq!(
                reached.steps[0].reader_results[0].source_value,
                StateValue::Unsigned(item_id)
            );
            assert_eq!(
                reached.steps[0].weakest_evidence,
                Some(TruthStatus::Hypothetical)
            );
            assert!(
                reached.steps[0]
                    .evidence_dependencies
                    .iter()
                    .any(|dependency| {
                        dependency.dependency_kind == EvidenceDependencyKind::Resolver
                            && dependency.record_id == "resolver.sd-hypothetical-auru-geometry"
                            && dependency.evidence.truth == TruthStatus::Hypothetical
                    })
            );
            assert!(
                reached.steps[0]
                    .evidence_dependencies
                    .iter()
                    .any(|dependency| {
                        dependency.dependency_kind == EvidenceDependencyKind::Transition
                            && dependency.record_id == "transition.auru-generic-get-item"
                            && dependency.evidence.truth == TruthStatus::Established
                    })
            );
        }
    }

    #[test]
    fn auru_memo_overwrite_and_interrupted_grant_are_distinct_temporal_paths() {
        const FISHING_ROD: u64 = 0x4a;
        const AURUS_MEMO: u64 = 0x90;

        let mut snapshot = snapshot();
        snapshot.environment.player.action = "sidehop".into();
        snapshot.environment.components = vec![
            StateComponent {
                id: "event.item-handoff".into(),
                component_kind: ComponentKind::PendingOperation,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([(
                        "pre_item_no".into(),
                        StateValue::Unsigned(AURUS_MEMO),
                    )]),
                },
                binding: ComponentBinding::Session {
                    session_id: "session-1".into(),
                },
                lifetime: SemanticLifetime::Action,
                serialization_owner: SerializationOwner::None,
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::Transition,
                    source_id: "auru.pending-memo-item".into(),
                    source_sha256: Some(Digest([6; 32])),
                    transition_id: Some("auru.pending-memo-item".into()),
                }],
            },
            StateComponent {
                id: "event.recent-item".into(),
                component_kind: ComponentKind::Session,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([(
                        "get_item_no".into(),
                        StateValue::Unsigned(FISHING_ROD),
                    )]),
                },
                binding: ComponentBinding::Session {
                    session_id: "session-1".into(),
                },
                lifetime: SemanticLifetime::Session,
                serialization_owner: SerializationOwner::None,
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::Transition,
                    source_id: "writer.prior-fishing-rod".into(),
                    source_sha256: Some(Digest([7; 32])),
                    transition_id: Some("writer.prior-fishing-rod".into()),
                }],
            },
            StateComponent {
                id: "inventory.active".into(),
                component_kind: ComponentKind::Inventory,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([(
                        "owned_item_ids".into(),
                        StateValue::Bytes(vec![0; 32]),
                    )]),
                },
                binding: ComponentBinding::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                lifetime: SemanticLifetime::RuntimeFile,
                serialization_owner: SerializationOwner::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::Initialized,
                    source_id: "fixture.empty-inventory".into(),
                    source_sha256: Some(Digest([8; 32])),
                    transition_id: None,
                }],
            },
        ];
        let scope = scope(&snapshot);
        let grant_recent_item = StateOperation::SetBitFromValue {
            source: crate::transition::ComponentFieldTarget {
                component_id: "event.recent-item".into(),
                field: "get_item_no".into(),
            },
            target: crate::transition::ComponentFieldTarget {
                component_id: "inventory.active".into(),
                field: "owned_item_ids".into(),
            },
        };
        let consume_pending = StateOperation::Consume {
            pending_operation_id: "event.item-handoff".into(),
        };
        let interrupted = CandidateTransition {
            id: "transition.auru-interrupted-default-getitem".into(),
            label: "Interrupt Auru before memo overwrite and continue DEFAULT_GETITEM".into(),
            scope: scope.clone(),
            transition_kind: TransitionKind::MessageAction,
            approach_id: "approach.auru-dialogue-interrupt".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::True,
                physical_obligation_ids: vec!["obligation.auru-one-frame-interrupt".into()],
                effects: vec![grant_recent_item.clone(), consume_pending.clone()],
                unknown_requirements: Vec::new(),
            },
            evidence: evidence(TruthStatus::Established),
        };
        let normal = CandidateTransition {
            id: "transition.auru-normal-memo-getitem".into(),
            label: "Auru overwrites recent item with the memo before DEFAULT_GETITEM".into(),
            scope: scope.clone(),
            transition_kind: TransitionKind::MessageAction,
            approach_id: "approach.auru-normal-dialogue".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::True,
                physical_obligation_ids: Vec::new(),
                effects: vec![
                    StateOperation::Write {
                        target: crate::transition::ComponentFieldTarget {
                            component_id: "event.recent-item".into(),
                            field: "get_item_no".into(),
                        },
                        value: StateValue::Unsigned(AURUS_MEMO),
                    },
                    grant_recent_item,
                    consume_pending,
                ],
                unknown_requirements: Vec::new(),
            },
            evidence: evidence(TruthStatus::Established),
        };
        let timing = crate::transition::TemporalWindow {
            earliest_frame: 17,
            latest_frame: 17,
            required_input: Some("sidehop".into()),
        };
        let obligation = FeasibilityObligation {
            id: "obligation.auru-one-frame-interrupt".into(),
            label: "Interrupt Auru's memo overwrite on the witnessed frame".into(),
            scope: scope.clone(),
            obligation_kind: ObligationKind::Timing,
            detail: ObligationDetail::Temporal {
                requirement: crate::transition::TemporalRequirement {
                    action_id: "auru.memo-overwrite-dialogue".into(),
                    window: timing.clone(),
                },
                precondition: PredicateExpression::Compare {
                    left: ValueReference::ComponentField {
                        component_id: "event.recent-item".into(),
                        field: "get_item_no".into(),
                    },
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Unsigned(FISHING_ROD),
                    },
                },
            },
            evidence: evidence(TruthStatus::Established),
        };
        let microtrace = WitnessedMicrotrace {
            id: "microtrace.auru-sidehop-frame-17".into(),
            scope,
            precondition: PredicateExpression::Compare {
                left: ValueReference::PlayerAction,
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Text("sidehop".into()),
                },
            },
            operations: vec![StateOperation::Interrupt {
                action_id: "auru.memo-overwrite-dialogue".into(),
                window: timing.clone(),
            }],
            postcondition: PredicateExpression::True,
            timing,
            evidence: evidence(TruthStatus::Hypothetical),
        };
        let mut mechanics = catalog(vec![interrupted.clone(), normal.clone()]);
        mechanics.obligations = vec![obligation];
        mechanics.microtraces = vec![microtrace];
        let facts = facts();
        let item_goal = |item_id: u64| {
            let mut mask = vec![0; 32];
            mask[item_id as usize / 8] |= 1 << (item_id % 8);
            PredicateExpression::Compare {
                left: ValueReference::ComponentField {
                    component_id: "inventory.active".into(),
                    field: "owned_item_ids".into(),
                },
                operator: ComparisonOperator::ContainsBits,
                right: ValueReference::Literal {
                    value: StateValue::Bytes(mask),
                },
            }
        };
        let research = SolverOptions {
            evidence_policy: EvidencePolicy::RESEARCH,
            ..SolverOptions::default()
        };

        let rod = ForwardSolver::new(&facts, &mechanics, &[], research)
            .unwrap()
            .solve(
                PlannerExecutionState::new(snapshot.clone()).unwrap(),
                &item_goal(FISHING_ROD),
            )
            .unwrap();
        assert_eq!(rod.status, SearchStatus::Reached);
        assert_eq!(
            rod.steps[0].action_id,
            "transition.auru-interrupted-default-getitem"
        );
        assert_eq!(
            rod.steps[0].supporting_microtrace_ids,
            vec!["microtrace.auru-sidehop-frame-17"]
        );
        assert_eq!(
            rod.steps[0].weakest_evidence,
            Some(TruthStatus::Hypothetical)
        );

        let memo = ForwardSolver::new(&facts, &mechanics, &[], research)
            .unwrap()
            .solve(
                PlannerExecutionState::new(snapshot.clone()).unwrap(),
                &item_goal(AURUS_MEMO),
            )
            .unwrap();
        assert_eq!(memo.status, SearchStatus::Reached);
        assert_eq!(
            memo.steps[0].action_id,
            "transition.auru-normal-memo-getitem"
        );
        assert_eq!(
            memo.steps[0].weakest_evidence,
            Some(TruthStatus::Established)
        );

        let mut without_witness = mechanics.clone();
        without_witness.microtraces.clear();
        let missing = ForwardSolver::new(&facts, &without_witness, &[], research)
            .unwrap()
            .solve(
                PlannerExecutionState::new(snapshot.clone()).unwrap(),
                &item_goal(FISHING_ROD),
            )
            .unwrap();
        assert_ne!(missing.status, SearchStatus::Reached);
        assert!(missing.blocked_transition_witnesses.iter().any(|witness| {
            witness.transition_id == "transition.auru-interrupted-default-getitem"
                && witness
                    .unknown_obligation_ids
                    .iter()
                    .chain(&witness.outstanding_obligation_ids)
                    .any(|id| id == "obligation.auru-one-frame-interrupt")
        }));

        let established_only =
            ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default())
                .unwrap()
                .solve(
                    PlannerExecutionState::new(snapshot.clone()).unwrap(),
                    &item_goal(FISHING_ROD),
                )
                .unwrap();
        assert_ne!(established_only.status, SearchStatus::Reached);

        let mut normal_state = PlannerExecutionState::new(snapshot.clone()).unwrap();
        normal_state
            .apply_operations(
                &normal.id,
                "snapshot.auru-normal",
                &normal.activation.effects,
            )
            .unwrap();
        let mut interrupted_state = PlannerExecutionState::new(snapshot).unwrap();
        interrupted_state
            .apply_operations(
                &interrupted.id,
                "snapshot.auru-interrupted",
                &interrupted.activation.effects,
            )
            .unwrap();
        let recent_item = |state: &PlannerExecutionState| {
            let component = state
                .snapshot
                .environment
                .components
                .iter()
                .find(|component| component.id == "event.recent-item")
                .unwrap();
            let ComponentPayload::Structured { fields } = &component.payload else {
                unreachable!()
            };
            fields["get_item_no"].clone()
        };
        assert_eq!(recent_item(&normal_state), StateValue::Unsigned(AURUS_MEMO));
        assert_eq!(
            recent_item(&interrupted_state),
            StateValue::Unsigned(FISHING_ROD)
        );
        assert!(
            normal_state
                .snapshot
                .environment
                .components
                .iter()
                .all(|component| component.id != "event.item-handoff")
        );
        assert!(
            interrupted_state
                .snapshot
                .environment
                .components
                .iter()
                .all(|component| component.id != "event.item-handoff")
        );
    }

    #[test]
    fn gz2e01_lanayru_placement_switch_and_flow21_remain_separate_steps() {
        const VESSEL_ITEM: u64 = 0xa3;

        let mut start = snapshot();
        start.id = "snapshot.gz2e01-lanayru-before-layer-selection".into();
        start.environment.location = SceneLocation {
            stage: "F_SP115".into(),
            room: 1,
            layer: 14,
            spawn: 20,
        };
        start.environment.player.form = PlayerForm::Wolf;
        start.environment.components = vec![
            StateComponent {
                id: "actor.lanayru-spirit".into(),
                component_kind: ComponentKind::ActorInstance,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([
                        ("flow_node".into(), StateValue::Unsigned(21)),
                        ("loaded".into(), StateValue::Boolean(false)),
                        ("parameters".into(), StateValue::Unsigned(0x0000_c102)),
                        ("post_present_latch".into(), StateValue::Boolean(false)),
                    ]),
                },
                binding: ComponentBinding::Actor {
                    instance_id: "actor.lanayru-spirit".into(),
                },
                lifetime: SemanticLifetime::RoomLoad,
                serialization_owner: SerializationOwner::None,
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::ExtractedFact,
                    source_id: "gz2e01:f_sp115/r01/actd/0".into(),
                    source_sha256: Some(
                        "c904e517476e46884cd719930d45129a480cefd6405f05e48fa0cb43737db4c8"
                            .parse()
                            .unwrap(),
                    ),
                    transition_id: None,
                }],
            },
            StateComponent {
                id: "flow.lanayru-21".into(),
                component_kind: ComponentKind::MessageFlow,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([
                        ("active".into(), StateValue::Boolean(false)),
                        ("cursor".into(), StateValue::Unsigned(0xffff)),
                        ("event_id".into(), StateValue::Unsigned(0)),
                        ("item_id".into(), StateValue::Unsigned(0)),
                    ]),
                },
                binding: ComponentBinding::Session {
                    session_id: "session-1".into(),
                },
                lifetime: SemanticLifetime::Action,
                serialization_owner: SerializationOwner::None,
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::ExtractedFact,
                    source_id: "gz2e01:msgus/bmgres8/flow/21".into(),
                    source_sha256: Some(
                        "2562ae9662648e71b8f30a5682dbc440dae3a7de55782bbd5992e4192e38e2cb"
                            .parse()
                            .unwrap(),
                    ),
                    transition_id: None,
                }],
            },
            StateComponent {
                id: "actor.f_sp115-sw-area-0c".into(),
                component_kind: ComponentKind::ActorInstance,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([
                        ("inside".into(), StateValue::Boolean(false)),
                        ("loaded".into(), StateValue::Boolean(false)),
                    ]),
                },
                binding: ComponentBinding::Actor {
                    instance_id: "actor.f_sp115-sw-area-0c".into(),
                },
                lifetime: SemanticLifetime::RoomLoad,
                serialization_owner: SerializationOwner::None,
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::ExtractedFact,
                    source_id: "gz2e01:f_sp115/r01/scod/0".into(),
                    source_sha256: Some(
                        "c904e517476e46884cd719930d45129a480cefd6405f05e48fa0cb43737db4c8"
                            .parse()
                            .unwrap(),
                    ),
                    transition_id: None,
                }],
            },
            StateComponent {
                id: "stage.f_sp115-memory".into(),
                component_kind: ComponentKind::StageMemory,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([
                        ("switch_0c".into(), StateValue::Boolean(false)),
                        ("switch_105".into(), StateValue::Boolean(false)),
                    ]),
                },
                binding: ComponentBinding::Stage {
                    stage: "F_SP115".into(),
                },
                lifetime: SemanticLifetime::StageLoad,
                serialization_owner: SerializationOwner::StageBank {
                    runtime_file_id: "file-0".into(),
                    stage: "F_SP115".into(),
                },
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::Initialized,
                    source_id: "fixture.f-sp115-stage-memory".into(),
                    source_sha256: Some(Digest([0x34; 32])),
                    transition_id: None,
                }],
            },
            StateComponent {
                id: "save.event-flags".into(),
                component_kind: ComponentKind::PersistentSave,
                payload: ComponentPayload::Raw {
                    bytes: {
                        let mut bytes = vec![0; 0x4c];
                        bytes[0x08] = 0x80; // M_032: normal layer-13 producer.
                        bytes
                    },
                    known_mask: vec![0xff; 0x4c],
                },
                binding: ComponentBinding::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                lifetime: SemanticLifetime::RuntimeFile,
                serialization_owner: SerializationOwner::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::Initialized,
                    source_id: "fixture.m032-set-f0615-clear".into(),
                    source_sha256: Some(Digest([0x32; 32])),
                    transition_id: None,
                }],
            },
            StateComponent {
                id: "save.player-light-drop".into(),
                component_kind: ComponentKind::PersistentSave,
                payload: ComponentPayload::Raw {
                    bytes: vec![0; 5],
                    known_mask: vec![0xff; 5],
                },
                binding: ComponentBinding::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                lifetime: SemanticLifetime::RuntimeFile,
                serialization_owner: SerializationOwner::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::Initialized,
                    source_id: "fixture.lanayru-vessel-clear".into(),
                    source_sha256: Some(Digest([0x33; 32])),
                    transition_id: None,
                }],
            },
        ];
        start
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
        start.validate().unwrap();

        let component_is =
            |component_id: &str, field: &str, value: StateValue| PredicateExpression::Compare {
                left: ValueReference::ComponentField {
                    component_id: component_id.into(),
                    field: field.into(),
                },
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal { value },
            };
        let raw_is = |component_id: &str, byte_offset: u32, mask: u64, value: u64| {
            PredicateExpression::Compare {
                left: ValueReference::RawBits {
                    component_id: component_id.into(),
                    byte_offset,
                    byte_width: 1,
                    mask,
                },
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Unsigned(value),
                },
            }
        };
        let exact_scene = |layer: i8| PredicateExpression::All {
            terms: vec![
                stage_is("F_SP115"),
                PredicateExpression::Compare {
                    left: ValueReference::LocationRoom,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Signed(1),
                    },
                },
                PredicateExpression::Compare {
                    left: ValueReference::LocationLayer,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Signed(i64::from(layer)),
                    },
                },
            ],
        };
        let m032 = raw_is("save.event-flags", 0x08, 0x80, 0x80);
        let vessel = raw_is("save.player-light-drop", 4, 0x04, 0x04);
        let no_vessel = raw_is("save.player-light-drop", 4, 0x04, 0);
        let f0615 = raw_is("save.event-flags", 0x4b, 0x04, 0x04);
        let no_f0615 = raw_is("save.event-flags", 0x4b, 0x04, 0);
        let switch_0c_off = component_is(
            "stage.f_sp115-memory",
            "switch_0c",
            StateValue::Boolean(false),
        );
        let field = |component_id: &str, name: &str| crate::transition::ComponentFieldTarget {
            component_id: component_id.into(),
            field: name.into(),
        };
        let transition =
            |id: &str,
             kind: TransitionKind,
             guard: PredicateExpression,
             effects: Vec<StateOperation>| CandidateTransition {
                id: id.into(),
                label: id.into(),
                scope: scope(&start),
                transition_kind: kind,
                approach_id: "approach.gz2e01-lanayru-flow21".into(),
                activation: ActivationContract {
                    hard_guards: guard,
                    physical_obligation_ids: Vec::new(),
                    effects,
                    unknown_requirements: Vec::new(),
                },
                evidence: evidence(TruthStatus::Established),
            };

        let mut mechanics = catalog(vec![
            transition(
                "transition.lanayru-01-select-layer13",
                TransitionKind::Other,
                PredicateExpression::All {
                    terms: vec![exact_scene(14), m032.clone()],
                },
                vec![StateOperation::SetLocation {
                    location: SceneLocation {
                        stage: "F_SP115".into(),
                        room: 1,
                        layer: 13,
                        spawn: 20,
                    },
                }],
            ),
            transition(
                "transition.lanayru-02-load-actd0",
                TransitionKind::ActorDriven,
                PredicateExpression::All {
                    terms: vec![
                        exact_scene(13),
                        component_is(
                            "actor.lanayru-spirit",
                            "parameters",
                            StateValue::Unsigned(0x0000_c102),
                        ),
                        component_is("actor.lanayru-spirit", "loaded", StateValue::Boolean(false)),
                    ],
                },
                vec![StateOperation::Write {
                    target: field("actor.lanayru-spirit", "loaded"),
                    value: StateValue::Boolean(true),
                }],
            ),
            transition(
                "transition.lanayru-02b-load-scod0",
                TransitionKind::ActorDriven,
                PredicateExpression::All {
                    terms: vec![
                        exact_scene(13),
                        component_is(
                            "actor.f_sp115-sw-area-0c",
                            "loaded",
                            StateValue::Boolean(false),
                        ),
                    ],
                },
                vec![
                    StateOperation::Write {
                        target: field("actor.f_sp115-sw-area-0c", "loaded"),
                        value: StateValue::Boolean(true),
                    },
                    StateOperation::Write {
                        target: field("stage.f_sp115-memory", "switch_0c"),
                        value: StateValue::Boolean(false),
                    },
                ],
            ),
            transition(
                "transition.lanayru-03-enter-scod0-switch-area",
                TransitionKind::ActorDriven,
                PredicateExpression::All {
                    terms: vec![
                        exact_scene(13),
                        component_is(
                            "actor.f_sp115-sw-area-0c",
                            "loaded",
                            StateValue::Boolean(true),
                        ),
                    ],
                },
                vec![
                    StateOperation::Write {
                        target: field("actor.f_sp115-sw-area-0c", "inside"),
                        value: StateValue::Boolean(true),
                    },
                    StateOperation::Write {
                        target: field("stage.f_sp115-memory", "switch_0c"),
                        value: StateValue::Boolean(true),
                    },
                ],
            ),
            transition(
                "transition.lanayru-03b-sw-area-outside-tick",
                TransitionKind::ActorDriven,
                PredicateExpression::All {
                    terms: vec![
                        exact_scene(13),
                        component_is(
                            "actor.f_sp115-sw-area-0c",
                            "loaded",
                            StateValue::Boolean(true),
                        ),
                        component_is(
                            "actor.f_sp115-sw-area-0c",
                            "inside",
                            StateValue::Boolean(false),
                        ),
                        component_is(
                            "stage.f_sp115-memory",
                            "switch_0c",
                            StateValue::Boolean(true),
                        ),
                    ],
                },
                vec![StateOperation::Write {
                    target: field("stage.f_sp115-memory", "switch_0c"),
                    value: StateValue::Boolean(false),
                }],
            ),
            transition(
                "transition.lanayru-04-start-talk-flow21",
                TransitionKind::MessageAction,
                PredicateExpression::All {
                    terms: vec![
                        exact_scene(13),
                        component_is("actor.lanayru-spirit", "loaded", StateValue::Boolean(true)),
                        component_is(
                            "actor.f_sp115-sw-area-0c",
                            "loaded",
                            StateValue::Boolean(true),
                        ),
                        component_is(
                            "stage.f_sp115-memory",
                            "switch_0c",
                            StateValue::Boolean(true),
                        ),
                        PredicateExpression::Compare {
                            left: ValueReference::PlayerControl,
                            operator: ComparisonOperator::Equal,
                            right: ValueReference::Literal {
                                value: StateValue::Boolean(true),
                            },
                        },
                    ],
                },
                vec![
                    StateOperation::Write {
                        target: field("flow.lanayru-21", "active"),
                        value: StateValue::Boolean(true),
                    },
                    StateOperation::Write {
                        target: field("flow.lanayru-21", "cursor"),
                        value: StateValue::Unsigned(321),
                    },
                ],
            ),
            transition(
                "transition.lanayru-05-flow21-request-vessel",
                TransitionKind::MessageAction,
                PredicateExpression::All {
                    terms: vec![
                        component_is("flow.lanayru-21", "cursor", StateValue::Unsigned(321)),
                        no_f0615.clone(),
                        no_vessel,
                    ],
                },
                vec![
                    StateOperation::Write {
                        target: field("flow.lanayru-21", "cursor"),
                        value: StateValue::Unsigned(315),
                    },
                    StateOperation::Write {
                        target: field("flow.lanayru-21", "event_id"),
                        value: StateValue::Unsigned(1),
                    },
                    StateOperation::Write {
                        target: field("flow.lanayru-21", "item_id"),
                        value: StateValue::Unsigned(VESSEL_ITEM),
                    },
                ],
            ),
            transition(
                "transition.lanayru-06-create-presentation",
                TransitionKind::Cutscene,
                PredicateExpression::All {
                    terms: vec![
                        exact_scene(13),
                        component_is("actor.lanayru-spirit", "loaded", StateValue::Boolean(true)),
                        component_is("flow.lanayru-21", "event_id", StateValue::Unsigned(1)),
                        component_is(
                            "flow.lanayru-21",
                            "item_id",
                            StateValue::Unsigned(VESSEL_ITEM),
                        ),
                    ],
                },
                vec![
                    StateOperation::Initialize {
                        component: StateComponent {
                            id: "pending.lanayru-vessel-presentation".into(),
                            component_kind: ComponentKind::PendingOperation,
                            payload: ComponentPayload::Structured {
                                fields: BTreeMap::from([(
                                    "item_id".into(),
                                    StateValue::Unsigned(VESSEL_ITEM),
                                )]),
                            },
                            binding: ComponentBinding::Session {
                                session_id: "session-1".into(),
                            },
                            lifetime: SemanticLifetime::Action,
                            serialization_owner: SerializationOwner::None,
                            provenance: vec![ComponentProvenance {
                                source_kind: ProvenanceSourceKind::Transition,
                                source_id: "transition.lanayru-06-create-presentation".into(),
                                source_sha256: None,
                                transition_id: Some(
                                    "transition.lanayru-06-create-presentation".into(),
                                ),
                            }],
                        },
                    },
                    StateOperation::Write {
                        target: field("actor.lanayru-spirit", "post_present_latch"),
                        value: StateValue::Boolean(true),
                    },
                ],
            ),
            transition(
                "transition.lanayru-07-generic-vessel-grant",
                TransitionKind::ItemAcquisition,
                component_is(
                    "pending.lanayru-vessel-presentation",
                    "item_id",
                    StateValue::Unsigned(VESSEL_ITEM),
                ),
                vec![
                    StateOperation::WriteRaw {
                        component_id: "save.player-light-drop".into(),
                        byte_offset: 4,
                        mask: vec![0x04],
                        value: vec![0x04],
                    },
                    StateOperation::Consume {
                        pending_operation_id: "pending.lanayru-vessel-presentation".into(),
                    },
                ],
            ),
            transition(
                "transition.lanayru-08-post-grant-autospeak",
                TransitionKind::MessageAction,
                PredicateExpression::All {
                    terms: vec![
                        exact_scene(13),
                        component_is("actor.lanayru-spirit", "loaded", StateValue::Boolean(true)),
                        component_is(
                            "actor.f_sp115-sw-area-0c",
                            "loaded",
                            StateValue::Boolean(true),
                        ),
                        vessel.clone(),
                        no_f0615.clone(),
                        component_is(
                            "actor.lanayru-spirit",
                            "post_present_latch",
                            StateValue::Boolean(true),
                        ),
                        component_is(
                            "stage.f_sp115-memory",
                            "switch_0c",
                            StateValue::Boolean(true),
                        ),
                    ],
                },
                vec![
                    StateOperation::Write {
                        target: field("flow.lanayru-21", "active"),
                        value: StateValue::Boolean(true),
                    },
                    StateOperation::Write {
                        target: field("flow.lanayru-21", "cursor"),
                        value: StateValue::Unsigned(321),
                    },
                ],
            ),
            transition(
                "transition.lanayru-09-flow21-owned-branch",
                TransitionKind::MessageAction,
                PredicateExpression::All {
                    terms: vec![
                        component_is("flow.lanayru-21", "cursor", StateValue::Unsigned(321)),
                        vessel.clone(),
                        no_f0615.clone(),
                    ],
                },
                vec![StateOperation::Write {
                    target: field("flow.lanayru-21", "cursor"),
                    value: StateValue::Unsigned(314),
                }],
            ),
            transition(
                "transition.lanayru-09b-flow21-complete-dialogue-branch",
                TransitionKind::MessageAction,
                PredicateExpression::All {
                    terms: vec![
                        component_is("flow.lanayru-21", "cursor", StateValue::Unsigned(321)),
                        f0615.clone(),
                    ],
                },
                vec![
                    StateOperation::Write {
                        target: field("flow.lanayru-21", "active"),
                        value: StateValue::Boolean(false),
                    },
                    StateOperation::Write {
                        target: field("flow.lanayru-21", "cursor"),
                        value: StateValue::Unsigned(0xffff),
                    },
                ],
            ),
            transition(
                "transition.lanayru-10-flow21-set-f0615",
                TransitionKind::MessageAction,
                PredicateExpression::All {
                    terms: vec![
                        component_is("flow.lanayru-21", "cursor", StateValue::Unsigned(314)),
                        vessel.clone(),
                    ],
                },
                vec![
                    StateOperation::WriteRaw {
                        component_id: "save.event-flags".into(),
                        byte_offset: 0x4b,
                        mask: vec![0x04],
                        value: vec![0x04],
                    },
                    StateOperation::Write {
                        target: field("flow.lanayru-21", "cursor"),
                        value: StateValue::Unsigned(323),
                    },
                ],
            ),
            transition(
                "transition.lanayru-11-flow21-reassert-vessel",
                TransitionKind::MessageAction,
                component_is("flow.lanayru-21", "cursor", StateValue::Unsigned(323)),
                vec![
                    StateOperation::WriteRaw {
                        component_id: "save.player-light-drop".into(),
                        byte_offset: 4,
                        mask: vec![0x04],
                        value: vec![0x04],
                    },
                    StateOperation::Write {
                        target: field("flow.lanayru-21", "cursor"),
                        value: StateValue::Unsigned(326),
                    },
                ],
            ),
            transition(
                "transition.lanayru-12-flow21-set-save-switch105",
                TransitionKind::MessageAction,
                component_is("flow.lanayru-21", "cursor", StateValue::Unsigned(326)),
                vec![
                    StateOperation::Write {
                        target: field("stage.f_sp115-memory", "switch_105"),
                        value: StateValue::Boolean(true),
                    },
                    StateOperation::Write {
                        target: field("flow.lanayru-21", "cursor"),
                        value: StateValue::Unsigned(0xffff),
                    },
                ],
            ),
        ]);
        mechanics
            .transitions
            .sort_by(|left, right| left.id.cmp(&right.id));
        mechanics.techniques = vec![Technique {
            id: "technique.hypothetical-lanayru-layer13-respawn".into(),
            label: "Hypothetical wrong-state respawn forcing Lanayru layer 13".into(),
            scope: scope(&start),
            prerequisites: exact_scene(14),
            operations: vec![StateOperation::SetLocation {
                location: SceneLocation {
                    stage: "F_SP115".into(),
                    room: 1,
                    layer: 13,
                    spawn: 20,
                },
            }],
            discharged_obligation_ids: Vec::new(),
            introduced_obligation_ids: Vec::new(),
            cost: RouteCost {
                axes: BTreeMap::from([("hypotheses".into(), 1)]),
            },
            evidence: evidence(TruthStatus::Hypothetical),
        }];
        mechanics.goals = vec![
            Goal {
                id: "goal.lanayru-spirit-visible".into(),
                label: "Lanayru spirit particle is visible and speak-eligible".into(),
                predicate: PredicateExpression::All {
                    terms: vec![
                        exact_scene(13),
                        component_is("actor.lanayru-spirit", "loaded", StateValue::Boolean(true)),
                        component_is(
                            "actor.f_sp115-sw-area-0c",
                            "loaded",
                            StateValue::Boolean(true),
                        ),
                        component_is(
                            "stage.f_sp115-memory",
                            "switch_0c",
                            StateValue::Boolean(true),
                        ),
                    ],
                },
            },
            Goal {
                id: "goal.lanayru-vessel-owned".into(),
                label: "Lanayru Vessel backing bit is set".into(),
                predicate: vessel.clone(),
            },
            Goal {
                id: "goal.lanayru-flow-complete".into(),
                label: "Lanayru story bit and post-flow save switch are set".into(),
                predicate: PredicateExpression::All {
                    terms: vec![
                        f0615.clone(),
                        component_is(
                            "stage.f_sp115-memory",
                            "switch_105",
                            StateValue::Boolean(true),
                        ),
                    ],
                },
            },
        ];
        mechanics
            .goals
            .sort_by(|left, right| left.id.cmp(&right.id));
        mechanics.validate().unwrap();

        let facts = facts();
        let solver = ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default()).unwrap();
        let initial = PlannerExecutionState::new(start.clone()).unwrap();
        let visible = solver
            .solve(initial.clone(), &mechanics.goals[1].predicate)
            .unwrap();
        assert_eq!(visible.status, SearchStatus::Reached);
        assert_eq!(visible.steps.len(), 4);
        assert_eq!(
            visible.steps.last().unwrap().action_id,
            "transition.lanayru-03-enter-scod0-switch-area"
        );

        let owned = solver.solve(initial.clone(), &vessel).unwrap();
        assert_eq!(owned.status, SearchStatus::Reached);
        assert_eq!(
            owned.steps.last().unwrap().action_id,
            "transition.lanayru-07-generic-vessel-grant"
        );
        assert!(
            !owned
                .steps
                .iter()
                .any(|step| { step.action_id == "transition.lanayru-10-flow21-set-f0615" })
        );

        let completed = solver
            .solve(initial, &mechanics.goals[0].predicate)
            .unwrap();
        assert_eq!(completed.status, SearchStatus::Reached);
        assert_eq!(completed.steps.len(), 13);
        let completed_ids = completed
            .steps
            .iter()
            .map(|step| step.action_id.as_str())
            .collect::<Vec<_>>();
        let grant_index = completed_ids
            .iter()
            .position(|id| *id == "transition.lanayru-07-generic-vessel-grant")
            .unwrap();
        let story_index = completed_ids
            .iter()
            .position(|id| *id == "transition.lanayru-10-flow21-set-f0615")
            .unwrap();
        assert!(grant_index < story_index, "{completed_ids:?}");
        assert_eq!(
            completed.steps.last().unwrap().action_id,
            "transition.lanayru-12-flow21-set-save-switch105"
        );

        let mut vessel_without_story = start.clone();
        let light_drop = vessel_without_story
            .environment
            .components
            .iter_mut()
            .find(|component| component.id == "save.player-light-drop")
            .unwrap();
        let ComponentPayload::Raw { bytes, .. } = &mut light_drop.payload else {
            unreachable!();
        };
        bytes[4] = 0x04;
        let transferred_item_route = solver
            .solve(
                PlannerExecutionState::new(vessel_without_story).unwrap(),
                &mechanics.goals[0].predicate,
            )
            .unwrap();
        assert_eq!(transferred_item_route.status, SearchStatus::Reached);
        assert!(
            transferred_item_route
                .steps
                .iter()
                .any(|step| { step.action_id == "transition.lanayru-09-flow21-owned-branch" })
        );
        assert!(!transferred_item_route.steps.iter().any(|step| {
            step.action_id == "transition.lanayru-06-create-presentation"
                || step.action_id == "transition.lanayru-07-generic-vessel-grant"
        }));

        let mut transferred_switch_outside = start.clone();
        transferred_switch_outside.environment.location.layer = 13;
        for component in &mut transferred_switch_outside.environment.components {
            let ComponentPayload::Structured { fields } = &mut component.payload else {
                continue;
            };
            if component.id == "actor.f_sp115-sw-area-0c" {
                fields.insert("loaded".into(), StateValue::Boolean(true));
            } else if component.id == "stage.f_sp115-memory" {
                fields.insert("switch_0c".into(), StateValue::Boolean(true));
            }
        }
        let cleared_transfer = solver
            .solve(
                PlannerExecutionState::new(transferred_switch_outside).unwrap(),
                &switch_0c_off,
            )
            .unwrap();
        assert_eq!(cleared_transfer.status, SearchStatus::Reached);
        assert_eq!(cleared_transfer.steps.len(), 1);
        assert_eq!(
            cleared_transfer.steps[0].action_id,
            "transition.lanayru-03b-sw-area-outside-tick"
        );

        let mut f0615_without_vessel = start.clone();
        let event_flags = f0615_without_vessel
            .environment
            .components
            .iter_mut()
            .find(|component| component.id == "save.event-flags")
            .unwrap();
        let ComponentPayload::Raw { bytes, .. } = &mut event_flags.payload else {
            unreachable!();
        };
        bytes[0x4b] = 0x04;
        let blocked_vessel = solver
            .solve(
                PlannerExecutionState::new(f0615_without_vessel).unwrap(),
                &vessel,
            )
            .unwrap();
        assert_eq!(blocked_vessel.status, SearchStatus::Unknown);
        assert!(
            !blocked_vessel
                .steps
                .iter()
                .any(|step| step.action_id == "transition.lanayru-07-generic-vessel-grant")
        );

        let mut wrong_layer = start;
        let event_flags = wrong_layer
            .environment
            .components
            .iter_mut()
            .find(|component| component.id == "save.event-flags")
            .unwrap();
        let ComponentPayload::Raw { bytes, .. } = &mut event_flags.payload else {
            unreachable!();
        };
        bytes[0x08] = 0;
        let stage_memory = wrong_layer
            .environment
            .components
            .iter_mut()
            .find(|component| component.id == "stage.f_sp115-memory")
            .unwrap();
        let ComponentPayload::Structured { fields } = &mut stage_memory.payload else {
            unreachable!();
        };
        fields.insert("switch_0c".into(), StateValue::Boolean(true));
        let established_wrong_layer = solver
            .solve(
                PlannerExecutionState::new(wrong_layer.clone()).unwrap(),
                &mechanics.goals[1].predicate,
            )
            .unwrap();
        assert_ne!(established_wrong_layer.status, SearchStatus::Reached);
        let research_wrong_state = ForwardSolver::new(
            &facts,
            &mechanics,
            &[],
            SolverOptions {
                evidence_policy: EvidencePolicy::RESEARCH,
                ..SolverOptions::default()
            },
        )
        .unwrap()
        .solve(
            PlannerExecutionState::new(wrong_layer).unwrap(),
            &mechanics.goals[1].predicate,
        )
        .unwrap();
        assert_eq!(research_wrong_state.status, SearchStatus::Reached);
        assert!(research_wrong_state.steps.iter().any(|step| {
            step.action_id == "technique.hypothetical-lanayru-layer13-respawn"
                && step.weakest_evidence == Some(TruthStatus::Hypothetical)
        }));
    }

    #[test]
    fn keyed_door_uses_bound_fungible_keys_and_oob_does_not_mutate_it() {
        let dungeon_field = |dungeon: &str, field: &str| ValueReference::BoundComponentField {
            component_kind: ComponentKind::DungeonMemory,
            binding: ComponentBindingReference::Exact {
                binding: ComponentBinding::Dungeon {
                    dungeon: dungeon.into(),
                },
            },
            field: field.into(),
        };
        let compare = |left: ValueReference, operator: ComparisonOperator, value: StateValue| {
            PredicateExpression::Compare {
                left,
                operator,
                right: ValueReference::Literal { value },
            }
        };
        let setup = |stage: &str, bound_dungeon: &str, keys: u64| {
            let mut state = snapshot();
            state.environment.location.stage = stage.into();
            state.environment.components = vec![
                StateComponent {
                    id: "actor.small-key-door".into(),
                    component_kind: ComponentKind::ActorInstance,
                    payload: ComponentPayload::Structured {
                        fields: BTreeMap::from([("open".into(), StateValue::Boolean(false))]),
                    },
                    binding: ComponentBinding::Actor {
                        instance_id: "actor.small-key-door".into(),
                    },
                    lifetime: SemanticLifetime::RoomLoad,
                    serialization_owner: SerializationOwner::None,
                    provenance: vec![ComponentProvenance {
                        source_kind: ProvenanceSourceKind::Initialized,
                        source_id: "fixture.closed-door-actor".into(),
                        source_sha256: Some(Digest([0x31; 32])),
                        transition_id: None,
                    }],
                },
                StateComponent {
                    id: "dungeon.active".into(),
                    component_kind: ComponentKind::DungeonMemory,
                    payload: ComponentPayload::Structured {
                        fields: BTreeMap::from([
                            ("door_01_unlocked".into(), StateValue::Boolean(false)),
                            ("small_keys".into(), StateValue::Unsigned(keys)),
                        ]),
                    },
                    binding: ComponentBinding::Dungeon {
                        dungeon: bound_dungeon.into(),
                    },
                    lifetime: SemanticLifetime::StageLoad,
                    serialization_owner: SerializationOwner::StageBank {
                        runtime_file_id: "file-0".into(),
                        stage: if bound_dungeon == "forest-temple" {
                            "D_MN05".into()
                        } else {
                            "D_MN04".into()
                        },
                    },
                    provenance: vec![ComponentProvenance {
                        source_kind: ProvenanceSourceKind::TraceObservation,
                        source_id: format!("trace.{bound_dungeon}-memory"),
                        source_sha256: Some(Digest([0x32; 32])),
                        transition_id: None,
                    }],
                },
            ];
            state
        };
        let unlock_transition =
            |state: &StateSnapshot, dungeon: &str, source: &str, destination: &str| {
                CandidateTransition {
                    id: format!("transition.{dungeon}-door-01-unlock"),
                    label: format!("Unlock {dungeon} small-key door 01"),
                    scope: scope(state),
                    transition_kind: TransitionKind::Door,
                    approach_id: format!("approach.{dungeon}-door-01-front"),
                    activation: ActivationContract {
                        hard_guards: PredicateExpression::All {
                            terms: vec![
                                stage_is(source),
                                compare(
                                    dungeon_field(dungeon, "small_keys"),
                                    ComparisonOperator::GreaterThan,
                                    StateValue::Unsigned(0),
                                ),
                                compare(
                                    dungeon_field(dungeon, "door_01_unlocked"),
                                    ComparisonOperator::Equal,
                                    StateValue::Boolean(false),
                                ),
                            ],
                        },
                        physical_obligation_ids: Vec::new(),
                        effects: vec![
                            StateOperation::Adjust {
                                target: crate::transition::ComponentFieldTarget {
                                    component_id: "dungeon.active".into(),
                                    field: "small_keys".into(),
                                },
                                delta: -1,
                            },
                            StateOperation::Write {
                                target: crate::transition::ComponentFieldTarget {
                                    component_id: "dungeon.active".into(),
                                    field: "door_01_unlocked".into(),
                                },
                                value: StateValue::Boolean(true),
                            },
                            StateOperation::Write {
                                target: crate::transition::ComponentFieldTarget {
                                    component_id: "actor.small-key-door".into(),
                                    field: "open".into(),
                                },
                                value: StateValue::Boolean(true),
                            },
                            StateOperation::SetLocation {
                                location: SceneLocation {
                                    stage: destination.into(),
                                    room: 1,
                                    layer: 0,
                                    spawn: 0,
                                },
                            },
                        ],
                        unknown_requirements: Vec::new(),
                    },
                    evidence: evidence(TruthStatus::Established),
                }
            };
        let key_pickup = |state: &StateSnapshot, id: &str, stage: &str| CandidateTransition {
            id: id.into(),
            label: format!("Obtain a fungible small key from {id}"),
            scope: scope(state),
            transition_kind: TransitionKind::ItemAcquisition,
            approach_id: format!("approach.{id}"),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        stage_is(stage),
                        compare(
                            dungeon_field("forest-temple", "small_keys"),
                            ComparisonOperator::Equal,
                            StateValue::Unsigned(0),
                        ),
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![StateOperation::Adjust {
                    target: crate::transition::ComponentFieldTarget {
                        component_id: "dungeon.active".into(),
                        field: "small_keys".into(),
                    },
                    delta: 1,
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: evidence(TruthStatus::Established),
        };

        let zero_key_state = setup("FOREST_DOOR", "forest-temple", 0);
        let door = unlock_transition(
            &zero_key_state,
            "forest-temple",
            "FOREST_DOOR",
            "FOREST_BEYOND_DOOR",
        );
        let mut door_only = catalog(vec![door.clone()]);
        door_only.reconstruction_rules = vec![ActorReconstructionRule {
            id: "reconstruct.forest-door-01".into(),
            label: "Reconstruct Forest Temple door 01 from its persisted unlock".into(),
            scope: scope(&zero_key_state),
            actor_type: "small-key-door".into(),
            instantiate_when: compare(
                dungeon_field("forest-temple", "door_01_unlocked"),
                ComparisonOperator::Equal,
                StateValue::Boolean(true),
            ),
            initialization_operations: vec![StateOperation::Write {
                target: crate::transition::ComponentFieldTarget {
                    component_id: "actor.small-key-door".into(),
                    field: "open".into(),
                },
                value: StateValue::Boolean(true),
            }],
            evidence: evidence(TruthStatus::Established),
        }];
        let blocked = ForwardSolver::new(&facts(), &door_only, &[], SolverOptions::default())
            .unwrap()
            .solve(
                PlannerExecutionState::new(zero_key_state.clone()).unwrap(),
                &stage_is("FOREST_BEYOND_DOOR"),
            )
            .unwrap();
        assert_ne!(blocked.status, SearchStatus::Reached);
        assert_eq!(
            blocked.blocked_transition_witnesses[0].classification,
            TransitionClassification::GuardBlocked
        );

        for pickup_id in ["pickup.forest-chest-key", "pickup.forest-actor-key"] {
            let mut with_pickup = door_only.clone();
            with_pickup
                .transitions
                .insert(0, key_pickup(&zero_key_state, pickup_id, "FOREST_DOOR"));
            with_pickup
                .transitions
                .sort_by(|left, right| left.id.cmp(&right.id));
            let result = ForwardSolver::new(&facts(), &with_pickup, &[], SolverOptions::default())
                .unwrap()
                .solve(
                    PlannerExecutionState::new(zero_key_state.clone()).unwrap(),
                    &stage_is("FOREST_BEYOND_DOOR"),
                )
                .unwrap();
            assert_eq!(result.status, SearchStatus::Reached);
            assert_eq!(result.steps[0].action_id, pickup_id);
            assert_eq!(result.steps[1].action_id, door.id);

            let mut executed = PlannerExecutionState::new(zero_key_state.clone()).unwrap();
            executed
                .apply_operations(
                    pickup_id,
                    "snapshot.after-key",
                    &with_pickup
                        .transitions
                        .iter()
                        .find(|transition| transition.id == pickup_id)
                        .unwrap()
                        .activation
                        .effects,
                )
                .unwrap();
            executed
                .apply_operations(&door.id, "snapshot.after-door", &door.activation.effects)
                .unwrap();
            let ComponentPayload::Structured { fields } = &executed
                .snapshot
                .environment
                .components
                .iter()
                .find(|component| component.id == "dungeon.active")
                .unwrap()
                .payload
            else {
                unreachable!()
            };
            assert_eq!(fields["small_keys"], StateValue::Unsigned(0));
            assert_eq!(fields["door_01_unlocked"], StateValue::Boolean(true));
            assert_eq!(
                executed
                    .last_field_writer("dungeon.active", "small_keys")
                    .unwrap()
                    .application_id,
                door.id
            );
        }

        let oob = CandidateTransition {
            id: "transition.forest-door-01-oob-avoid".into(),
            label: "Go out of bounds around Forest Temple door 01".into(),
            scope: scope(&zero_key_state),
            transition_kind: TransitionKind::Technique,
            approach_id: "approach.forest-door-01-oob".into(),
            activation: ActivationContract {
                hard_guards: stage_is("FOREST_DOOR"),
                physical_obligation_ids: Vec::new(),
                effects: vec![StateOperation::SetLocation {
                    location: SceneLocation {
                        stage: "FOREST_BEYOND_DOOR".into(),
                        room: 1,
                        layer: 0,
                        spawn: 0,
                    },
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: evidence(TruthStatus::Established),
        };
        let mut avoided = PlannerExecutionState::new(zero_key_state.clone()).unwrap();
        let before_dungeon = avoided
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "dungeon.active")
            .unwrap()
            .clone();
        avoided
            .apply_operations(&oob.id, "snapshot.after-oob", &oob.activation.effects)
            .unwrap();
        assert_eq!(
            avoided
                .snapshot
                .environment
                .components
                .iter()
                .find(|component| component.id == "dungeon.active")
                .unwrap(),
            &before_dungeon
        );

        let wrong_bank = setup("GORON_DOOR", "forest-temple", 1);
        let goron_door = unlock_transition(
            &wrong_bank,
            "goron-mines",
            "GORON_DOOR",
            "GORON_BEYOND_DOOR",
        );
        let goron_mechanics = catalog(vec![goron_door.clone()]);
        let without_rebind = ForwardSolver::new(
            &facts(),
            &goron_mechanics,
            &[],
            SolverOptions {
                evidence_policy: EvidencePolicy::RESEARCH,
                ..SolverOptions::default()
            },
        )
        .unwrap()
        .solve(
            PlannerExecutionState::new(wrong_bank.clone()).unwrap(),
            &stage_is("GORON_BEYOND_DOOR"),
        )
        .unwrap();
        assert_ne!(without_rebind.status, SearchStatus::Reached);

        let rebind = Technique {
            id: "technique.hypothetical-key-bank-rebind".into(),
            label: "Interpret the Forest key store as Goron Mines memory".into(),
            scope: scope(&wrong_bank),
            prerequisites: PredicateExpression::True,
            operations: vec![StateOperation::Rebind {
                selector: ComponentSelector::Id {
                    component_id: "dungeon.active".into(),
                },
                binding: ComponentBinding::Dungeon {
                    dungeon: "goron-mines".into(),
                },
            }],
            discharged_obligation_ids: Vec::new(),
            introduced_obligation_ids: Vec::new(),
            cost: RouteCost {
                axes: BTreeMap::from([("theorycraft".into(), 1)]),
            },
            evidence: evidence(TruthStatus::Hypothetical),
        };
        let pack = RefinementPack {
            schema: REFINEMENT_PACK_SCHEMA.into(),
            manifest: RefinementPackManifest {
                id: "pack.hypothetical-key-bank-rebind".into(),
                version: "1.0.0".into(),
                author: "Route planner acceptance fixture".into(),
                source: "Hypothetical dungeon-memory transfer".into(),
                scope: scope(&wrong_bank),
                precedence: 100,
                dependencies: Vec::new(),
                conflicts: Vec::new(),
            },
            rules: vec![RefinementRule {
                id: "rule.add-key-bank-rebind".into(),
                label: "Add hypothetical key-bank rebind".into(),
                operation: RefinementOperation::AddTechnique { technique: rebind },
                evidence: evidence(TruthStatus::Hypothetical),
            }],
        };
        let composed = ComposedPlannerCatalog::compose_layered(
            &facts(),
            &goron_mechanics,
            &RefinementLayers {
                enabled_packs: Vec::new(),
                route_local_overlays: Vec::new(),
                ephemeral_what_if_overlays: vec![pack],
            },
        )
        .unwrap();
        let with_rebind = ForwardSolver::new(
            &composed.facts,
            &composed.mechanics,
            &[],
            SolverOptions {
                evidence_policy: EvidencePolicy::RESEARCH,
                ..SolverOptions::default()
            },
        )
        .unwrap()
        .solve(
            PlannerExecutionState::new(wrong_bank).unwrap(),
            &stage_is("GORON_BEYOND_DOOR"),
        )
        .unwrap();
        assert_eq!(with_rebind.status, SearchStatus::Reached);
        assert_eq!(
            with_rebind
                .steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "technique.hypothetical-key-bank-rebind",
                "transition.goron-mines-door-01-unlock"
            ]
        );
    }

    #[test]
    fn gz2e01_forest_door1_requires_live_unlock_and_open_before_room2() {
        let actor_id = "actor.gz2e01-d-mn05-door-1";
        let dungeon_id = "dungeon.d-mn05-memory";
        let key_delta_id = "session.pending-key-delta";
        let actor_field = |name: &str| ValueReference::ComponentField {
            component_id: actor_id.into(),
            field: name.into(),
        };
        let dungeon_field = |name: &str| ValueReference::BoundComponentField {
            component_kind: ComponentKind::DungeonMemory,
            binding: ComponentBindingReference::Exact {
                binding: ComponentBinding::Dungeon {
                    dungeon: "forest-temple".into(),
                },
            },
            field: name.into(),
        };
        let key_delta_field = || ValueReference::ComponentField {
            component_id: key_delta_id.into(),
            field: "pending_delta".into(),
        };
        let compare = |left: ValueReference, operator: ComparisonOperator, value: StateValue| {
            PredicateExpression::Compare {
                left,
                operator,
                right: ValueReference::Literal { value },
            }
        };
        let component_target =
            |component_id: &str, field: &str| crate::transition::ComponentFieldTarget {
                component_id: component_id.into(),
                field: field.into(),
            };
        let room_is = |room: i64| {
            compare(
                ValueReference::LocationRoom,
                ComparisonOperator::Equal,
                StateValue::Signed(room),
            )
        };

        let mut start = snapshot();
        start.id = "snapshot.gz2e01-d-mn05-r01-door1-closed".into();
        start.environment.location = SceneLocation {
            stage: "D_MN05".into(),
            room: 1,
            layer: 0,
            spawn: 0,
        };
        start.environment.components = vec![
            StateComponent {
                id: actor_id.into(),
                component_kind: ComponentKind::ActorInstance,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([
                        ("adjoining_room_loaded".into(), StateValue::Boolean(true)),
                        ("door_animation".into(), StateValue::Text("closed".into())),
                        ("approach_permitted".into(), StateValue::Boolean(true)),
                        ("back_option".into(), StateValue::Unsigned(0)),
                        ("back_room".into(), StateValue::Unsigned(2)),
                        ("collision_registered".into(), StateValue::Boolean(true)),
                        ("event_offered".into(), StateValue::Boolean(false)),
                        ("front_option".into(), StateValue::Unsigned(2)),
                        ("front_room".into(), StateValue::Unsigned(1)),
                        ("keyhole_present".into(), StateValue::Boolean(true)),
                        (
                            "keyhole_animation".into(),
                            StateValue::Text("closed".into()),
                        ),
                        ("kind".into(), StateValue::Unsigned(1)),
                        ("locked".into(), StateValue::Boolean(true)),
                        ("next_room".into(), StateValue::Unsigned(1)),
                        ("parameters".into(), StateValue::Unsigned(0x6c10_2201)),
                        ("unlock_switch".into(), StateValue::Unsigned(0x0b)),
                    ]),
                },
                binding: ComponentBinding::Actor {
                    instance_id: actor_id.into(),
                },
                lifetime: SemanticLifetime::RoomLoad,
                serialization_owner: SerializationOwner::None,
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::ExtractedFact,
                    source_id: "gz2e01:d_mn05/stage.dzs/door/1".into(),
                    source_sha256: Some(
                        "9d08ac55fce27a6a741a6a502a4a2502146c3ff91abeb7d8c44824a6df8325a4"
                            .parse()
                            .unwrap(),
                    ),
                    transition_id: None,
                }],
            },
            StateComponent {
                id: dungeon_id.into(),
                component_kind: ComponentKind::DungeonMemory,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([
                        ("small_keys".into(), StateValue::Unsigned(1)),
                        ("switch_0b".into(), StateValue::Boolean(false)),
                    ]),
                },
                binding: ComponentBinding::Dungeon {
                    dungeon: "forest-temple".into(),
                },
                lifetime: SemanticLifetime::StageLoad,
                serialization_owner: SerializationOwner::StageBank {
                    runtime_file_id: "file-0".into(),
                    stage: "D_MN05".into(),
                },
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::TraceObservation,
                    source_id: "gz2e01:dsv-memory/d-mn05".into(),
                    source_sha256: Some(Digest([0x45; 32])),
                    transition_id: None,
                }],
            },
            StateComponent {
                id: key_delta_id.into(),
                component_kind: ComponentKind::Session,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([("pending_delta".into(), StateValue::Signed(0))]),
                },
                binding: ComponentBinding::Session {
                    session_id: "session-1".into(),
                },
                lifetime: SemanticLifetime::Session,
                serialization_owner: SerializationOwner::None,
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::Initialized,
                    source_id: "fixture.dcomifgp-item-key-delta".into(),
                    source_sha256: Some(Digest([0x46; 32])),
                    transition_id: None,
                }],
            },
        ];

        let candidate =
            |id: &str,
             label: &str,
             hard_guards: PredicateExpression,
             effects: Vec<StateOperation>| CandidateTransition {
                id: id.into(),
                label: label.into(),
                scope: scope(&start),
                transition_kind: TransitionKind::Door,
                approach_id: "approach.gz2e01-d-mn05-door1-front".into(),
                activation: ActivationContract {
                    hard_guards,
                    physical_obligation_ids: Vec::new(),
                    effects,
                    unknown_requirements: Vec::new(),
                },
                evidence: evidence(TruthStatus::Established),
            };
        let at_front = || PredicateExpression::All {
            terms: vec![stage_is("D_MN05"), room_is(1)],
        };
        let actor_is = |field: &str, value: StateValue| {
            compare(actor_field(field), ComparisonOperator::Equal, value)
        };
        let dungeon_is = |field: &str, value: StateValue| {
            compare(dungeon_field(field), ComparisonOperator::Equal, value)
        };

        let offer_event = candidate(
            "transition.gz2e01-door1-01-offer-event",
            "Offer the front-side shutter event",
            PredicateExpression::All {
                terms: vec![
                    at_front(),
                    actor_is("adjoining_room_loaded", StateValue::Boolean(true)),
                    actor_is("approach_permitted", StateValue::Boolean(true)),
                    actor_is("event_offered", StateValue::Boolean(false)),
                    PredicateExpression::Any {
                        terms: vec![
                            dungeon_is("switch_0b", StateValue::Boolean(true)),
                            compare(
                                dungeon_field("small_keys"),
                                ComparisonOperator::GreaterThan,
                                StateValue::Unsigned(0),
                            ),
                        ],
                    },
                ],
            },
            vec![StateOperation::Write {
                target: component_target(actor_id, "event_offered"),
                value: StateValue::Boolean(true),
            }],
        );
        let unlock_action = candidate(
            "transition.gz2e01-door1-02-demo-action8",
            "Run keyed shutter demo action 8",
            PredicateExpression::All {
                terms: vec![
                    at_front(),
                    actor_is("event_offered", StateValue::Boolean(true)),
                    actor_is("front_option", StateValue::Unsigned(2)),
                    actor_is("locked", StateValue::Boolean(true)),
                    dungeon_is("switch_0b", StateValue::Boolean(false)),
                    compare(
                        dungeon_field("small_keys"),
                        ComparisonOperator::GreaterThan,
                        StateValue::Unsigned(0),
                    ),
                    compare(
                        key_delta_field(),
                        ComparisonOperator::Equal,
                        StateValue::Signed(0),
                    ),
                ],
            },
            vec![
                StateOperation::Write {
                    target: component_target(dungeon_id, "switch_0b"),
                    value: StateValue::Boolean(true),
                },
                StateOperation::Write {
                    target: component_target(key_delta_id, "pending_delta"),
                    value: StateValue::Signed(-1),
                },
                StateOperation::Write {
                    target: component_target(actor_id, "keyhole_animation"),
                    value: StateValue::Text("keyhole-opening".into()),
                },
            ],
        );
        let finish_keyhole = candidate(
            "transition.gz2e01-door1-03-finish-keyhole",
            "Finish the keyhole child animation",
            PredicateExpression::All {
                terms: vec![
                    at_front(),
                    dungeon_is("switch_0b", StateValue::Boolean(true)),
                    actor_is("locked", StateValue::Boolean(true)),
                    actor_is(
                        "keyhole_animation",
                        StateValue::Text("keyhole-opening".into()),
                    ),
                ],
            },
            vec![
                StateOperation::Write {
                    target: component_target(actor_id, "locked"),
                    value: StateValue::Boolean(false),
                },
                StateOperation::Write {
                    target: component_target(actor_id, "keyhole_animation"),
                    value: StateValue::Text("open".into()),
                },
            ],
        );
        let flush_key_delta = candidate(
            "transition.gz2e01-door1-04-flush-key-delta",
            "Apply dComIfGp pending key delta to active stage memory",
            compare(
                key_delta_field(),
                ComparisonOperator::Equal,
                StateValue::Signed(-1),
            ),
            vec![
                StateOperation::Adjust {
                    target: component_target(dungeon_id, "small_keys"),
                    delta: -1,
                },
                StateOperation::Write {
                    target: component_target(key_delta_id, "pending_delta"),
                    value: StateValue::Signed(0),
                },
            ],
        );
        let open_init = candidate(
            "transition.gz2e01-door1-05-open-init",
            "Release shutter collision and select room 2",
            PredicateExpression::All {
                terms: vec![
                    at_front(),
                    actor_is("event_offered", StateValue::Boolean(true)),
                    actor_is("locked", StateValue::Boolean(false)),
                    actor_is("collision_registered", StateValue::Boolean(true)),
                    actor_is("door_animation", StateValue::Text("closed".into())),
                    dungeon_is("switch_0b", StateValue::Boolean(true)),
                ],
            },
            vec![
                StateOperation::Write {
                    target: component_target(actor_id, "collision_registered"),
                    value: StateValue::Boolean(false),
                },
                StateOperation::Write {
                    target: component_target(actor_id, "door_animation"),
                    value: StateValue::Text("opening".into()),
                },
                StateOperation::Write {
                    target: component_target(actor_id, "next_room"),
                    value: StateValue::Unsigned(2),
                },
            ],
        );
        let open_proc = candidate(
            "transition.gz2e01-door1-06-open-proc",
            "Finish the wooden shutter opening animation",
            PredicateExpression::All {
                terms: vec![
                    at_front(),
                    actor_is("collision_registered", StateValue::Boolean(false)),
                    actor_is("door_animation", StateValue::Text("opening".into())),
                ],
            },
            vec![StateOperation::Write {
                target: component_target(actor_id, "door_animation"),
                value: StateValue::Text("open".into()),
            }],
        );
        let cross = candidate(
            "transition.gz2e01-door1-07-cross-room-adjacency",
            "Cross the encoded room-1 to room-2 adjacency",
            PredicateExpression::All {
                terms: vec![
                    at_front(),
                    actor_is("next_room", StateValue::Unsigned(2)),
                    actor_is("collision_registered", StateValue::Boolean(false)),
                    actor_is("door_animation", StateValue::Text("open".into())),
                ],
            },
            vec![StateOperation::SetLocation {
                location: SceneLocation {
                    stage: "D_MN05".into(),
                    room: 2,
                    layer: 0,
                    spawn: 0,
                },
            }],
        );
        let close_init = candidate(
            "transition.gz2e01-door1-08-close-init",
            "Re-register shutter collision after crossing",
            PredicateExpression::All {
                terms: vec![
                    stage_is("D_MN05"),
                    room_is(2),
                    actor_is("collision_registered", StateValue::Boolean(false)),
                    actor_is("door_animation", StateValue::Text("open".into())),
                ],
            },
            vec![
                StateOperation::Write {
                    target: component_target(actor_id, "collision_registered"),
                    value: StateValue::Boolean(true),
                },
                StateOperation::Write {
                    target: component_target(actor_id, "door_animation"),
                    value: StateValue::Text("closing".into()),
                },
            ],
        );
        let close_end = candidate(
            "transition.gz2e01-door1-09-close-end",
            "Finish closing the unlocked shutter",
            PredicateExpression::All {
                terms: vec![
                    stage_is("D_MN05"),
                    room_is(2),
                    actor_is("collision_registered", StateValue::Boolean(true)),
                    actor_is("door_animation", StateValue::Text("closing".into())),
                ],
            },
            vec![
                StateOperation::Write {
                    target: component_target(actor_id, "door_animation"),
                    value: StateValue::Text("closed".into()),
                },
                StateOperation::Write {
                    target: component_target(actor_id, "keyhole_present"),
                    value: StateValue::Boolean(false),
                },
                StateOperation::Write {
                    target: component_target(actor_id, "keyhole_animation"),
                    value: StateValue::Text("deleted".into()),
                },
                StateOperation::Write {
                    target: component_target(actor_id, "event_offered"),
                    value: StateValue::Boolean(false),
                },
            ],
        );

        let mut mechanics = catalog(vec![
            offer_event,
            unlock_action,
            finish_keyhole,
            flush_key_delta,
            open_init,
            open_proc,
            cross,
            close_init,
            close_end,
        ]);
        let reconstruct =
            |id: &str, unlocked: bool, locked: bool, keyhole: bool| ActorReconstructionRule {
                id: id.into(),
                label: format!(
                    "Reconstruct GZ2E01 Forest door 1 as {}",
                    if unlocked { "unlocked" } else { "locked" }
                ),
                scope: scope(&start),
                actor_type: "door20".into(),
                instantiate_when: dungeon_is("switch_0b", StateValue::Boolean(unlocked)),
                initialization_operations: vec![
                    StateOperation::Write {
                        target: component_target(actor_id, "locked"),
                        value: StateValue::Boolean(locked),
                    },
                    StateOperation::Write {
                        target: component_target(actor_id, "keyhole_present"),
                        value: StateValue::Boolean(keyhole),
                    },
                    StateOperation::Write {
                        target: component_target(actor_id, "collision_registered"),
                        value: StateValue::Boolean(true),
                    },
                    StateOperation::Write {
                        target: component_target(actor_id, "door_animation"),
                        value: StateValue::Text("closed".into()),
                    },
                    StateOperation::Write {
                        target: component_target(actor_id, "keyhole_animation"),
                        value: StateValue::Text(if keyhole { "closed" } else { "deleted" }.into()),
                    },
                ],
                evidence: evidence(TruthStatus::Established),
            };
        mechanics.reconstruction_rules = vec![
            reconstruct("reconstruct.gz2e01-door1-locked", false, true, true),
            reconstruct("reconstruct.gz2e01-door1-unlocked", true, false, false),
        ];
        mechanics.validate().unwrap();

        let completed_goal = PredicateExpression::All {
            terms: vec![
                stage_is("D_MN05"),
                room_is(2),
                dungeon_is("switch_0b", StateValue::Boolean(true)),
                dungeon_is("small_keys", StateValue::Unsigned(0)),
                actor_is("collision_registered", StateValue::Boolean(true)),
                actor_is("door_animation", StateValue::Text("closed".into())),
                actor_is("keyhole_present", StateValue::Boolean(false)),
            ],
        };
        let solved = ForwardSolver::new(&facts(), &mechanics, &[], SolverOptions::default())
            .unwrap()
            .solve(
                PlannerExecutionState::new(start.clone()).unwrap(),
                &completed_goal,
            )
            .unwrap();
        assert_eq!(solved.status, SearchStatus::Reached);
        assert_eq!(solved.steps.len(), 9);
        assert_eq!(
            solved
                .steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "transition.gz2e01-door1-01-offer-event",
                "transition.gz2e01-door1-02-demo-action8",
                "transition.gz2e01-door1-03-finish-keyhole",
                "transition.gz2e01-door1-04-flush-key-delta",
                "transition.gz2e01-door1-05-open-init",
                "transition.gz2e01-door1-06-open-proc",
                "transition.gz2e01-door1-07-cross-room-adjacency",
                "transition.gz2e01-door1-08-close-init",
                "transition.gz2e01-door1-09-close-end",
            ])
        );

        let mut no_key = start.clone();
        let ComponentPayload::Structured { fields } = &mut no_key
            .environment
            .components
            .iter_mut()
            .find(|component| component.id == dungeon_id)
            .unwrap()
            .payload
        else {
            unreachable!()
        };
        fields.insert("small_keys".into(), StateValue::Unsigned(0));
        let blocked = ForwardSolver::new(&facts(), &mechanics, &[], SolverOptions::default())
            .unwrap()
            .solve(PlannerExecutionState::new(no_key).unwrap(), &room_is(2))
            .unwrap();
        assert_ne!(blocked.status, SearchStatus::Reached);
        assert!(blocked.steps.is_empty());

        let unlocked_rule = mechanics
            .reconstruction_rules
            .iter()
            .find(|rule| rule.id == "reconstruct.gz2e01-door1-unlocked")
            .unwrap();
        let mut reconstructed = start;
        let ComponentPayload::Structured { fields } = &mut reconstructed
            .environment
            .components
            .iter_mut()
            .find(|component| component.id == dungeon_id)
            .unwrap()
            .payload
        else {
            unreachable!()
        };
        fields.insert("switch_0b".into(), StateValue::Boolean(true));
        fields.insert("small_keys".into(), StateValue::Unsigned(0));
        let mut reconstructed = PlannerExecutionState::new(reconstructed).unwrap();
        reconstructed
            .apply_operations(
                &unlocked_rule.id,
                "snapshot.gz2e01-door1-reloaded",
                &unlocked_rule.initialization_operations,
            )
            .unwrap();
        let ComponentPayload::Structured { fields } = &reconstructed
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == actor_id)
            .unwrap()
            .payload
        else {
            unreachable!()
        };
        assert_eq!(fields["locked"], StateValue::Boolean(false));
        assert_eq!(fields["keyhole_present"], StateValue::Boolean(false));
        assert_eq!(fields["collision_registered"], StateValue::Boolean(true));
        assert_eq!(fields["door_animation"], StateValue::Text("closed".into()));
        assert_eq!(
            fields["keyhole_animation"],
            StateValue::Text("deleted".into())
        );

        let reopened = ForwardSolver::new(&facts(), &mechanics, &[], SolverOptions::default())
            .unwrap()
            .solve(reconstructed, &room_is(2))
            .unwrap();
        assert_eq!(reopened.status, SearchStatus::Reached);
        assert!(
            !reopened
                .steps
                .iter()
                .any(|step| step.action_id == "transition.gz2e01-door1-02-demo-action8")
        );
        assert!(
            !reopened
                .steps
                .iter()
                .any(|step| step.action_id == "transition.gz2e01-door1-04-flush-key-delta")
        );
    }

    #[test]
    fn unresolved_obstruction_returns_a_minimal_failure_witness() {
        let snapshot = snapshot();
        let mut mechanics = obstructed_transition_catalog(&snapshot);
        mechanics.resolvers.clear();
        let facts = facts();
        let solver = ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default()).unwrap();
        let result = solver
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_B"),
            )
            .unwrap();
        assert_eq!(result.status, SearchStatus::UnreachableUnderModel);
        assert_eq!(result.blocked_transition_witnesses.len(), 1);
        let witness = &result.blocked_transition_witnesses[0];
        assert_eq!(witness.transition_id, "transition.a-to-b");
        assert_eq!(witness.classification, TransitionClassification::Obstructed);
        assert_eq!(witness.active_obstruction_ids, vec!["obstruction.npc"]);
        assert_eq!(
            witness.outstanding_obligation_ids,
            vec!["obligation.blocker"]
        );
        assert!(witness.selected_resolver_ids.is_empty());
        assert_eq!(witness.weakest_evidence, Some(TruthStatus::Established));
        assert!(witness.evidence_dependencies.iter().any(|dependency| {
            dependency.dependency_kind == EvidenceDependencyKind::Obstruction
                && dependency.record_id == "obstruction.npc"
        }));
        assert!(witness.evidence_dependencies.iter().any(|dependency| {
            dependency.dependency_kind == EvidenceDependencyKind::Obligation
                && dependency.record_id == "obligation.blocker"
        }));
        assert_eq!(
            witness.hard_guard_expression,
            mechanics.transitions[0].activation.hard_guards
        );
        assert_eq!(
            witness.effect_operations,
            mechanics.transitions[0].activation.effects
        );
        assert_eq!(witness.obligation_derivations, mechanics.obligations);
        assert!(witness.unknown_requirements.is_empty());
    }

    #[test]
    fn conditioned_method_steps_observe_each_setup_action_boundary() {
        let snapshot = snapshot();
        let mut mechanics = obstructed_transition_catalog(&snapshot);
        mechanics.goals = vec![goal("goal.b", "STAGE_B")];
        let facts = facts();
        let mut book = route_book(&snapshot, Vec::new());
        book.steps = vec![
            ReferenceStep {
                id: "step.resolve".into(),
                label: "Resolve blocker".into(),
                scope: scope(&snapshot),
                action: RouteActionRef::Resolver {
                    resolver_id: "resolver.text-state".into(),
                },
                precondition: Some(stage_is("STAGE_A")),
                postcondition: Some(gate_is("gate.entrance-open", true)),
                region_id: Some("region.conditioned".into()),
                annotation_ids: Vec::new(),
            },
            ReferenceStep {
                id: "step.travel".into(),
                label: "Use opened entrance".into(),
                scope: scope(&snapshot),
                action: RouteActionRef::Transition {
                    transition_id: "transition.a-to-b".into(),
                },
                precondition: Some(gate_is("gate.entrance-open", true)),
                postcondition: Some(stage_is("STAGE_B")),
                region_id: Some("region.conditioned".into()),
                annotation_ids: Vec::new(),
            },
        ];
        book.methods = vec![PlanMethod {
            id: "method.conditioned".into(),
            label: "Conditioned entrance".into(),
            scope: scope(&snapshot),
            region_id: "region.conditioned".into(),
            step_ids: vec!["step.resolve".into(), "step.travel".into()],
        }];
        book.regions = vec![PlanRegion {
            id: "region.conditioned".into(),
            label: "Conditioned entrance".into(),
            scope: scope(&snapshot),
            parent_region_id: None,
            entry_predicate: Some(stage_is("STAGE_A")),
            outcome_predicate: stage_is("STAGE_B"),
            method_ids: vec!["method.conditioned".into()],
            selected_method_id: Some("method.conditioned".into()),
            collapse_policy: CollapsePolicy::Never,
        }];
        let solver = ForwardSolver::new_with_route_book(
            &facts,
            &mechanics,
            &[],
            SolverOptions::default(),
            &book,
        )
        .unwrap();
        let result = solver
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_B"),
            )
            .unwrap();
        assert_eq!(result.status, SearchStatus::Reached);
        assert_eq!(
            result.steps[0].selected_resolver_ids,
            vec!["resolver.text-state"]
        );
    }

    #[test]
    fn faron_twilight_return_audit_keeps_warps_blockers_and_file_lifetimes_distinct() {
        let mut start = snapshot();
        start.id = "snapshot.faron-twilight".into();
        start.environment.location = SceneLocation {
            stage: "F_SP108".into(),
            room: 0,
            layer: 14,
            spawn: 0,
        };
        start.environment.player.form = PlayerForm::Wolf;
        start.environment.components.push(StateComponent {
            id: "restart.route".into(),
            component_kind: ComponentKind::Restart,
            lifetime: SemanticLifetime::RuntimeFile,
            binding: ComponentBinding::RuntimeFile {
                runtime_file_id: "file-0".into(),
            },
            serialization_owner: SerializationOwner::RuntimeFile {
                runtime_file_id: "file-0".into(),
            },
            payload: ComponentPayload::Structured {
                fields: BTreeMap::from([
                    ("respawn_stage".into(), StateValue::Text("F_SP108".into())),
                    ("return_stage".into(), StateValue::Text("F_SP108".into())),
                ]),
            },
            provenance: vec![ComponentProvenance {
                source_kind: ProvenanceSourceKind::TraceObservation,
                source_id: "source.faron-restart".into(),
                source_sha256: Some(Digest([7; 32])),
                transition_id: None,
            }],
        });
        start
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
        start.validate().unwrap();

        let exact_location = |stage: &str, room: i8| PredicateExpression::All {
            terms: vec![
                stage_is(stage),
                PredicateExpression::Compare {
                    left: ValueReference::LocationRoom,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Signed(i64::from(room)),
                    },
                },
            ],
        };
        let form_is = |form: PlayerForm| PredicateExpression::Compare {
            left: ValueReference::PlayerForm,
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Text(match form {
                    PlayerForm::Human => "human".into(),
                    PlayerForm::Wolf => "wolf".into(),
                    PlayerForm::Other { id } => id,
                    PlayerForm::Unknown => "unknown".into(),
                }),
            },
        };
        let component_is = |field: &str, value: &str| PredicateExpression::Compare {
            left: ValueReference::ComponentField {
                component_id: "restart.route".into(),
                field: field.into(),
            },
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Text(value.into()),
            },
        };
        let location_effect =
            |stage: &str, room: i8, layer: i8, spawn: i16| StateOperation::SetLocation {
                location: SceneLocation {
                    stage: stage.into(),
                    room,
                    layer,
                    spawn,
                },
            };
        let candidate = |id: &str,
                         label: &str,
                         kind: TransitionKind,
                         approach: &str,
                         guard: PredicateExpression,
                         target: SceneLocation,
                         truth: TruthStatus| CandidateTransition {
            id: id.into(),
            label: label.into(),
            scope: scope(&start),
            transition_kind: kind,
            approach_id: approach.into(),
            activation: ActivationContract {
                hard_guards: guard,
                physical_obligation_ids: Vec::new(),
                effects: vec![StateOperation::SetLocation { location: target }],
                unknown_requirements: Vec::new(),
            },
            evidence: evidence(truth),
        };
        let target = |stage: &str, room: i8, layer: i8, spawn: i16| SceneLocation {
            stage: stage.into(),
            room,
            layer,
            spawn,
        };

        let mut transitions = vec![
            candidate(
                "transition.actor-scenechange-to-ordon-spring",
                "Actor-provided scene change to Ordon Spring",
                TransitionKind::ActorDriven,
                "approach.actor-scenechange",
                exact_location("F_SP108", 0),
                target("F_SP104", 1, 4, 3),
                TruthStatus::Unknown,
            ),
            candidate(
                "transition.bit-file0-save-load-faron-spring",
                "BiT file-0 save/load return to Faron Spring",
                TransitionKind::SaveLoad,
                "approach.bit-save-load",
                exact_location("F_SP108", 0),
                target("F_SP108", 0, 14, 0),
                TruthStatus::Established,
            ),
            candidate(
                "transition.bite-load-ordon-file",
                "BiTE load of a compatible Ordon runtime file",
                TransitionKind::WrongStateRespawn,
                "approach.bite-file-swap",
                PredicateExpression::All {
                    terms: vec![
                        exact_location("F_SP108", 0),
                        gate_is("bite.compatible-ordon-slot", true),
                    ],
                },
                target("F_SP103", 0, 1, 0),
                TruthStatus::Established,
            ),
            candidate(
                "transition.cutscene-scls-to-ordon-spring",
                "Cutscene SCLS record to Ordon Spring",
                TransitionKind::CutsceneSceneChange,
                "approach.cutscene-scls",
                exact_location("F_SP108", 0),
                target("F_SP104", 1, 4, 3),
                TruthStatus::Unknown,
            ),
            candidate(
                "transition.death-reload-ordon-spring",
                "Death reload using an Ordon Spring restart",
                TransitionKind::DeathReload,
                "approach.death-reload",
                PredicateExpression::All {
                    terms: vec![
                        exact_location("F_SP108", 0),
                        component_is("respawn_stage", "F_SP104"),
                    ],
                },
                target("F_SP104", 1, 4, 3),
                TruthStatus::Established,
            ),
            candidate(
                "transition.epona-oob-to-ordon-spring",
                "Epona OOB toward Ordon Spring",
                TransitionKind::Technique,
                "approach.epona-oob",
                PredicateExpression::All {
                    terms: vec![
                        exact_location("F_SP108", 0),
                        PredicateExpression::Compare {
                            left: ValueReference::PlayerMount,
                            operator: ComparisonOperator::Equal,
                            right: ValueReference::Literal {
                                value: StateValue::Text("epona".into()),
                            },
                        },
                        PredicateExpression::Not {
                            term: Box::new(gate_is("story.faron-twilight", true)),
                        },
                    ],
                },
                target("F_SP104", 1, 4, 3),
                TruthStatus::Established,
            ),
            candidate(
                "transition.midna-portal-to-ordon-spring",
                "Midna portal warp to Ordon Spring",
                TransitionKind::PortalWarp,
                "approach.portal-table-entry-0",
                PredicateExpression::All {
                    terms: vec![
                        exact_location("F_SP108", 0),
                        gate_is("portal.first-warp-complete", true),
                        gate_is("portal.ordon-stage0-switch34", true),
                        PredicateExpression::Compare {
                            left: ValueReference::PlayerControl,
                            operator: ComparisonOperator::Equal,
                            right: ValueReference::Literal {
                                value: StateValue::Boolean(true),
                            },
                        },
                    ],
                },
                target("F_SP104", 1, 4, 0),
                TruthStatus::Established,
            ),
            candidate(
                "transition.savewarp-ordon-spring",
                "Savewarp using a held Ordon Spring return",
                TransitionKind::SaveWarp,
                "approach.savewarp",
                PredicateExpression::All {
                    terms: vec![
                        gate_is("story.faron-twilight", true),
                        component_is("return_stage", "F_SP104"),
                    ],
                },
                target("F_SP104", 1, 4, 3),
                TruthStatus::Established,
            ),
            candidate(
                "transition.spawn-injection-ordon-spring",
                "Injected Ordon Spring spawn",
                TransitionKind::Spawn,
                "approach.spawn-injection",
                gate_is("hypothesis.spawn-injection", true),
                target("F_SP104", 1, 4, 3),
                TruthStatus::Hypothetical,
            ),
            candidate(
                "transition.title-load-ordon-village",
                "Title/load an Ordon slot retaining Faron twilight",
                TransitionKind::TitleReturn,
                "approach.title-slot-load",
                gate_is("slot.ordon-with-faron-twilight", true),
                target("F_SP103", 0, 1, 0),
                TruthStatus::Established,
            ),
            candidate(
                "transition.void-reload-ordon-spring",
                "Void reload using an Ordon Spring restart",
                TransitionKind::VoidReload,
                "approach.void-reload",
                PredicateExpression::All {
                    terms: vec![
                        exact_location("F_SP108", 0),
                        component_is("respawn_stage", "F_SP104"),
                    ],
                },
                target("F_SP104", 1, 4, 3),
                TruthStatus::Established,
            ),
            CandidateTransition {
                id: "transition.walk-back-through-faron-barrier".into(),
                label: "Walk back through the Faron twilight barrier".into(),
                scope: scope(&start),
                transition_kind: TransitionKind::EncodedMapExit,
                approach_id: "approach.faron-barrier".into(),
                activation: ActivationContract {
                    hard_guards: exact_location("F_SP108", 0),
                    physical_obligation_ids: vec!["obligation.cross-faron-barrier".into()],
                    effects: vec![location_effect("F_SP104", 1, 4, 3)],
                    unknown_requirements: Vec::new(),
                },
                evidence: evidence(TruthStatus::Established),
            },
            candidate(
                "transition.wolf-oob-to-ordon-spring",
                "Unverified wolf OOB toward Ordon Spring",
                TransitionKind::Technique,
                "approach.wolf-oob",
                exact_location("F_SP108", 0),
                target("F_SP104", 1, 4, 3),
                TruthStatus::Established,
            ),
            candidate(
                "transition.ordon-spring-crawlspace-to-house-yard",
                "Ordon Spring crawlspace to outside Link's house",
                TransitionKind::EncodedMapExit,
                "approach.ordon-crawlspace",
                PredicateExpression::All {
                    terms: vec![exact_location("F_SP104", 1), form_is(PlayerForm::Wolf)],
                },
                target("F_SP103", 1, 1, 2),
                TruthStatus::Established,
            ),
            candidate(
                "transition.house-yard-to-ordon-village",
                "Outside Link's house to main Ordon Village",
                TransitionKind::EncodedMapExit,
                "approach.ordon-room-boundary",
                exact_location("F_SP103", 1),
                target("F_SP103", 0, 1, 0),
                TruthStatus::Established,
            ),
            candidate(
                "transition.ordon-village-to-ranch",
                "Main Ordon Village to the goats map",
                TransitionKind::EncodedMapExit,
                "approach.ordon-ranch-gate",
                exact_location("F_SP103", 0),
                target("F_SP00", 0, 1, 1),
                TruthStatus::Established,
            ),
            candidate(
                "transition.house-yard-knob-door-to-links-house",
                "Open Link's house knob door",
                TransitionKind::Door,
                "approach.links-house-kdoor",
                PredicateExpression::All {
                    terms: vec![exact_location("F_SP103", 1), form_is(PlayerForm::Human)],
                },
                target("R_SP01", 4, 1, 0),
                TruthStatus::Established,
            ),
        ];
        transitions
            .iter_mut()
            .find(|transition| transition.id == "transition.wolf-oob-to-ordon-spring")
            .unwrap()
            .activation
            .unknown_requirements = vec![crate::transition::UnknownRequirement {
            id: "unknown.wolf-oob-route".into(),
            description: "No witnessed wolf route reaches this encoded activation.".into(),
            evidence: evidence(TruthStatus::Unknown),
        }];
        transitions.sort_by(|left, right| left.id.cmp(&right.id));

        let obligations = vec![FeasibilityObligation {
            id: "obligation.cross-faron-barrier".into(),
            label: "Cross the active Faron twilight barrier".into(),
            scope: scope(&start),
            obligation_kind: ObligationKind::Twilight,
            detail: ObligationDetail::Predicate {
                predicate: PredicateExpression::Not {
                    term: Box::new(gate_is("story.faron-twilight", true)),
                },
            },
            evidence: evidence(TruthStatus::Established),
        }];
        let obstructions = vec![Obstruction {
            id: "obstruction.faron-twilight-barrier".into(),
            label: "Faron twilight barrier rejects the return approach".into(),
            scope: scope(&start),
            blocked_action_id: "transition.walk-back-through-faron-barrier".into(),
            approach_id: "approach.faron-barrier".into(),
            active_when: gate_is("story.faron-twilight", true),
            obligation_ids: vec!["obligation.cross-faron-barrier".into()],
            evidence: evidence(TruthStatus::Established),
        }];
        let readers = vec![
            ReaderRule {
                id: "reader.death-restart-stage".into(),
                scope: scope(&start),
                source: ValueReference::ComponentField {
                    component_id: "restart.route".into(),
                    field: "respawn_stage".into(),
                },
                consuming_transition_id: "transition.death-reload-ordon-spring".into(),
                interpretation_fact_id: None,
                evidence: evidence(TruthStatus::Established),
            },
            ReaderRule {
                id: "reader.savewarp-return-stage".into(),
                scope: scope(&start),
                source: ValueReference::ComponentField {
                    component_id: "restart.route".into(),
                    field: "return_stage".into(),
                },
                consuming_transition_id: "transition.savewarp-ordon-spring".into(),
                interpretation_fact_id: None,
                evidence: evidence(TruthStatus::Established),
            },
            ReaderRule {
                id: "reader.void-restart-stage".into(),
                scope: scope(&start),
                source: ValueReference::ComponentField {
                    component_id: "restart.route".into(),
                    field: "respawn_stage".into(),
                },
                consuming_transition_id: "transition.void-reload-ordon-spring".into(),
                interpretation_fact_id: None,
                evidence: evidence(TruthStatus::Established),
            },
        ];
        let mut techniques = vec![
            Technique {
                id: "technique.ems-human-in-faron-twilight".into(),
                label: "Early Master Sword enables human form in Faron twilight".into(),
                scope: scope(&start),
                prerequisites: gate_is("story.faron-twilight", true),
                operations: vec![StateOperation::SetPlayerForm {
                    form: PlayerForm::Human,
                }],
                discharged_obligation_ids: Vec::new(),
                introduced_obligation_ids: Vec::new(),
                cost: RouteCost {
                    axes: BTreeMap::from([("difficulty".into(), 8)]),
                },
                evidence: evidence(TruthStatus::Established),
            },
            Technique {
                id: "technique.hypothetical-return-place-transfer".into(),
                label: "Hypothetically transfer an Ordon Spring return place".into(),
                scope: scope(&start),
                prerequisites: gate_is("story.faron-twilight", true),
                operations: vec![StateOperation::Write {
                    target: crate::transition::ComponentFieldTarget {
                        component_id: "restart.route".into(),
                        field: "return_stage".into(),
                    },
                    value: StateValue::Text("F_SP104".into()),
                }],
                discharged_obligation_ids: Vec::new(),
                introduced_obligation_ids: Vec::new(),
                cost: RouteCost {
                    axes: BTreeMap::from([("hypotheses".into(), 1)]),
                },
                evidence: evidence(TruthStatus::Hypothetical),
            },
        ];
        techniques.sort_by(|left, right| left.id.cmp(&right.id));

        let goal_predicate = |stage: &str, room: i8| PredicateExpression::All {
            terms: vec![
                exact_location(stage, room),
                gate_is("story.faron-twilight", true),
            ],
        };
        let mut goals = vec![
            Goal {
                id: "goal.faron-twilight.goats-map".into(),
                label: "Reach the goats map while Faron remains in twilight".into(),
                predicate: goal_predicate("F_SP00", 0),
            },
            Goal {
                id: "goal.faron-twilight.links-house".into(),
                label: "Reach Link's house while Faron remains in twilight".into(),
                predicate: goal_predicate("R_SP01", 4),
            },
            Goal {
                id: "goal.faron-twilight.ordon-spring".into(),
                label: "Reach Ordon Spring while Faron remains in twilight".into(),
                predicate: goal_predicate("F_SP104", 1),
            },
            Goal {
                id: "goal.faron-twilight.ordon-village".into(),
                label: "Reach Ordon Village while Faron remains in twilight".into(),
                predicate: goal_predicate("F_SP103", 0),
            },
            Goal {
                id: "goal.faron-twilight.outside-links-house".into(),
                label: "Reach outside Link's house while Faron remains in twilight".into(),
                predicate: goal_predicate("F_SP103", 1),
            },
        ];
        goals.sort_by(|left, right| left.id.cmp(&right.id));
        let mechanics = MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions,
            obligations,
            writers: Vec::new(),
            gates: Vec::new(),
            readers,
            reconstruction_rules: Vec::new(),
            obstructions,
            resolvers: Vec::new(),
            techniques,
            microtraces: Vec::new(),
            goals: goals.clone(),
        };
        mechanics.validate().unwrap();

        let mut state = PlannerExecutionState::new(start.clone()).unwrap();
        state.gate_states.extend([
            ("portal.first-warp-complete".into(), true),
            ("portal.ordon-stage0-switch34".into(), true),
            ("story.faron-twilight".into(), true),
        ]);
        state.validate().unwrap();
        let fact_catalog = facts();
        let solver =
            ForwardSolver::new(&fact_catalog, &mechanics, &[], SolverOptions::default()).unwrap();
        let results = goals
            .iter()
            .map(|goal| {
                (
                    goal.id.as_str(),
                    solver.solve(state.clone(), &goal.predicate).unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert!(
            results
                .values()
                .all(|result| result.status == SearchStatus::Reached)
        );
        assert_eq!(results["goal.faron-twilight.ordon-spring"].steps.len(), 1);
        assert_eq!(
            results["goal.faron-twilight.ordon-spring"].steps[0].action_id,
            "transition.midna-portal-to-ordon-spring"
        );
        assert!(
            results["goal.faron-twilight.links-house"]
                .steps
                .iter()
                .any(|step| step.action_id == "technique.ems-human-in-faron-twilight")
        );
        assert!(
            results["goal.faron-twilight.links-house"]
                .steps
                .iter()
                .any(|step| step.action_id == "transition.house-yard-knob-door-to-links-house")
        );
        assert!(
            results["goal.faron-twilight.ordon-spring"]
                .backward_relevance
                .contains_transition("transition.bit-file0-save-load-faron-spring")
        );
        assert!(
            !results["goal.faron-twilight.ordon-spring"]
                .steps
                .iter()
                .any(|step| step.action_id == "transition.bit-file0-save-load-faron-spring")
        );

        let mut blocked_state = PlannerExecutionState::new(start).unwrap();
        blocked_state.gate_states.extend([
            ("portal.first-warp-complete".into(), true),
            ("portal.ordon-stage0-switch34".into(), false),
            ("story.faron-twilight".into(), true),
        ]);
        let blocked = solver
            .solve(blocked_state.clone(), &goal_predicate("F_SP104", 1))
            .unwrap();
        assert_eq!(blocked.status, SearchStatus::Unknown);
        let barrier = blocked
            .blocked_transition_witnesses
            .iter()
            .find(|witness| witness.transition_id == "transition.walk-back-through-faron-barrier")
            .unwrap();
        assert_eq!(barrier.classification, TransitionClassification::Obstructed);
        assert_eq!(
            barrier.outstanding_obligation_ids,
            vec!["obligation.cross-faron-barrier"]
        );
        assert_eq!(
            barrier.active_obstruction_ids,
            vec!["obstruction.faron-twilight-barrier"]
        );
        let wolf_oob = blocked
            .blocked_transition_witnesses
            .iter()
            .find(|witness| witness.transition_id == "transition.wolf-oob-to-ordon-spring")
            .unwrap();
        assert_eq!(
            wolf_oob.classification,
            TransitionClassification::FeasibilityUnknown
        );
        assert_eq!(
            wolf_oob.unknown_requirement_ids,
            vec!["unknown.wolf-oob-route"]
        );

        let research = ForwardSolver::new(
            &facts(),
            &mechanics,
            &[],
            SolverOptions {
                evidence_policy: EvidencePolicy::RESEARCH,
                ..SolverOptions::default()
            },
        )
        .unwrap()
        .solve(blocked_state, &goal_predicate("F_SP104", 1))
        .unwrap();
        assert_eq!(research.status, SearchStatus::Reached);
        assert!(research.steps.iter().any(|step| {
            step.action_id == "technique.hypothetical-return-place-transfer"
                && step.weakest_evidence == Some(TruthStatus::Hypothetical)
        }));
        let savewarp = research
            .steps
            .iter()
            .find(|step| step.action_id == "transition.savewarp-ordon-spring")
            .unwrap();
        assert_eq!(
            savewarp.reader_results[0].source_value,
            StateValue::Text("F_SP104".into())
        );
    }

    #[test]
    fn text_displacement_producers_are_distinct_proofs_over_the_same_raw_bits() {
        const FLOW_A: u8 = 0x04;
        const FLOW_B: u8 = 0x02;
        let mut base = snapshot();
        base.environment.components = vec![
            StateComponent {
                id: "message.active-flow".into(),
                component_kind: ComponentKind::MessageFlow,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([
                        ("flow_id".into(), StateValue::Text("producer".into())),
                        ("node_id".into(), StateValue::Text("producer-ready".into())),
                    ]),
                },
                binding: ComponentBinding::Session {
                    session_id: "process".into(),
                },
                lifetime: SemanticLifetime::Session,
                serialization_owner: SerializationOwner::None,
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::ExtractedFact,
                    source_id: "gz2e01.message-flow-producer".into(),
                    source_sha256: Some(Digest([0x71; 32])),
                    transition_id: None,
                }],
            },
            StateComponent {
                id: "route.text-displacement".into(),
                component_kind: ComponentKind::Custom {
                    id: "text-displacement-route".into(),
                },
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([
                        ("auru_talk_count".into(), StateValue::Unsigned(0)),
                        ("coro_bottle_text_reached".into(), StateValue::Boolean(true)),
                        ("ooccoo_intro_unused".into(), StateValue::Boolean(true)),
                        ("ooccoo_zombie_pull_done".into(), StateValue::Boolean(false)),
                        ("yeta_first_talk_unused".into(), StateValue::Boolean(true)),
                    ]),
                },
                binding: ComponentBinding::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                lifetime: SemanticLifetime::RuntimeFile,
                serialization_owner: SerializationOwner::None,
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::TraceObservation,
                    source_id: "route.td-producer-start".into(),
                    source_sha256: Some(Digest([0x72; 32])),
                    transition_id: None,
                }],
            },
            StateComponent {
                id: "temporary.event-flags".into(),
                component_kind: ComponentKind::TemporaryFlags,
                payload: ComponentPayload::Raw {
                    bytes: vec![0; 6],
                    known_mask: vec![0xff; 6],
                },
                binding: ComponentBinding::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                lifetime: SemanticLifetime::RuntimeFile,
                serialization_owner: SerializationOwner::None,
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::ExtractedFact,
                    source_id: "gz2e01.temp-event-backing".into(),
                    source_sha256: Some(Digest([0x73; 32])),
                    transition_id: None,
                }],
            },
        ];
        base.environment.components.sort_by(|a, b| a.id.cmp(&b.id));
        base.environment.live_world_objects = vec![LiveWorldObject {
            instance_id: "actor.auru".into(),
            static_object_id: Some("placement.auru".into()),
            actor_type: "npc_rafrel".into(),
            lifecycle: ActorLifecycle::Loaded,
            fields: BTreeMap::new(),
        }];
        base.environment.spatial_volumes = vec![
            SpatialVolume {
                object_id: "actor.auru".into(),
                volume_id: "cutscene-trigger".into(),
                shape: SpatialVolumeShape::Sphere {
                    center: [0.0, 0.0, 0.0],
                    radius: 5.0,
                },
                source_sha256: Digest([0x74; 32]),
            },
            SpatialVolume {
                object_id: "actor.auru".into(),
                volume_id: "talk".into(),
                shape: SpatialVolumeShape::Sphere {
                    center: [0.0, 0.0, 0.0],
                    radius: 10.0,
                },
                source_sha256: Digest([0x75; 32]),
            },
        ];
        base.environment.spatial_volumes.sort_by(|a, b| {
            (a.object_id.as_str(), a.volume_id.as_str())
                .cmp(&(b.object_id.as_str(), b.volume_id.as_str()))
        });
        let exact_scope = scope(&base);
        let field_is = |field: &str, value: StateValue| PredicateExpression::Compare {
            left: ValueReference::ComponentField {
                component_id: "route.text-displacement".into(),
                field: field.into(),
            },
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal { value },
        };
        let flow_node_is = |node: &str| PredicateExpression::Compare {
            left: ValueReference::FlowNode {
                flow_component_id: "message.active-flow".into(),
            },
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Text(node.into()),
            },
        };
        let raw_bit_is = |mask: u8, set: bool| PredicateExpression::Compare {
            left: ValueReference::RawBits {
                component_id: "temporary.event-flags".into(),
                byte_offset: 0,
                byte_width: 1,
                mask: u64::from(mask),
            },
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Unsigned(if set { u64::from(mask) } else { 0 }),
            },
        };
        let write_raw_bit = |mask: u8| StateOperation::WriteRaw {
            component_id: "temporary.event-flags".into(),
            byte_offset: 0,
            mask: vec![mask],
            value: vec![mask],
        };
        let write_route = |field: &str, value: StateValue| StateOperation::Write {
            target: crate::transition::ComponentFieldTarget {
                component_id: "route.text-displacement".into(),
                field: field.into(),
            },
            value,
        };
        let candidate =
            |id: &str,
             stage: &str,
             guards: Vec<PredicateExpression>,
             obligations: Vec<&str>,
             effects: Vec<StateOperation>| CandidateTransition {
                id: id.into(),
                label: id.replace('.', " "),
                scope: exact_scope.clone(),
                transition_kind: TransitionKind::MessageAction,
                approach_id: format!("approach.{id}"),
                activation: ActivationContract {
                    hard_guards: PredicateExpression::All {
                        terms: std::iter::once(stage_is(stage)).chain(guards).collect(),
                    },
                    physical_obligation_ids: obligations.into_iter().map(str::to_owned).collect(),
                    effects,
                    unknown_requirements: Vec::new(),
                },
                evidence: RuleEvidence {
                    truth: TruthStatus::Established,
                    records: vec![EvidenceRecord {
                        id: format!("community.{id}"),
                        kind: EvidenceKind::CommunityReported,
                        source_sha256: None,
                        note: "Version-scoped route witness; backing-bit effect is source audited."
                            .into(),
                    }],
                },
            };
        let interrupted_effects = |action_id: &str, input: &str, mask: u8| {
            vec![
                StateOperation::ScheduleCleanup {
                    cleanup_id: "cleanup.general-message-flow-bits".into(),
                },
                write_raw_bit(mask),
                StateOperation::Interrupt {
                    action_id: action_id.into(),
                    window: TemporalWindow {
                        earliest_frame: 0,
                        latest_frame: 0,
                        required_input: Some(input.into()),
                    },
                },
                StateOperation::CancelCleanup {
                    cleanup_id: "cleanup.general-message-flow-bits".into(),
                },
            ]
        };

        let mut mechanics = catalog(vec![
            candidate(
                "transition.td-auru-first-edge-talk",
                "AURU",
                vec![field_is("auru_talk_count", StateValue::Unsigned(0))],
                vec!["obligation.td-auru-talk-outside-trigger"],
                vec![
                    write_raw_bit(FLOW_A),
                    write_route("auru_talk_count", StateValue::Unsigned(1)),
                ],
            ),
            candidate(
                "transition.td-auru-second-edge-talk",
                "AURU",
                vec![
                    field_is("auru_talk_count", StateValue::Unsigned(1)),
                    raw_bit_is(FLOW_A, true),
                ],
                vec!["obligation.td-auru-talk-outside-trigger"],
                vec![
                    write_raw_bit(FLOW_B),
                    write_route("auru_talk_count", StateValue::Unsigned(2)),
                ],
            ),
            candidate(
                "transition.td-coro-interrupt-after-bottle",
                "CORO",
                vec![
                    field_is("coro_bottle_text_reached", StateValue::Boolean(true)),
                    flow_node_is("coro.after-bottle-before-next-line"),
                ],
                vec!["obligation.td-coro-one-frame-ooccoo-pull"],
                interrupted_effects("dialogue.coro-after-bottle", "ooccoo", FLOW_B),
            ),
            candidate(
                "transition.td-ooccoo-zombie-death-pull",
                "OOCCOO",
                vec![
                    field_is("ooccoo_intro_unused", StateValue::Boolean(true)),
                    flow_node_is("ooccoo.first-warp-introduction"),
                ],
                vec!["obligation.td-ooccoo-death-pull"],
                {
                    let mut effects = interrupted_effects(
                        "dialogue.ooccoo-first-warp-on-death",
                        "ooccoo",
                        FLOW_A,
                    );
                    effects.push(write_route(
                        "ooccoo_intro_unused",
                        StateValue::Boolean(false),
                    ));
                    effects.push(write_route(
                        "ooccoo_zombie_pull_done",
                        StateValue::Boolean(true),
                    ));
                    effects
                },
            ),
            candidate(
                "transition.td-ooccoo-advance-second-bit",
                "OOCCOO",
                vec![
                    field_is("ooccoo_zombie_pull_done", StateValue::Boolean(true)),
                    raw_bit_is(FLOW_A, true),
                    raw_bit_is(FLOW_B, false),
                ],
                Vec::new(),
                vec![write_raw_bit(FLOW_B)],
            ),
            candidate(
                "transition.td-yeta-map-talk-interrupt",
                "YETA",
                vec![
                    field_is("yeta_first_talk_unused", StateValue::Boolean(true)),
                    flow_node_is("yeta.first-snowpeak-talk"),
                ],
                vec!["obligation.td-yeta-map-same-frame"],
                {
                    let mut effects =
                        interrupted_effects("dialogue.yeta-first-talk", "talk-map", FLOW_B);
                    effects.push(write_route(
                        "yeta_first_talk_unused",
                        StateValue::Boolean(false),
                    ));
                    effects
                },
            ),
        ]);
        mechanics.transitions.sort_by(|a, b| a.id.cmp(&b.id));
        mechanics.obligations = vec![
            FeasibilityObligation {
                id: "obligation.td-auru-talk-outside-trigger".into(),
                label: "Stand in Auru's talk volume but outside his cutscene trigger".into(),
                scope: exact_scope.clone(),
                obligation_kind: ObligationKind::Interaction,
                detail: ObligationDetail::Interaction {
                    actor_instance_id: "actor.auru".into(),
                    interaction_mode: "talk".into(),
                    required_volumes: vec![crate::transition::VolumeReference {
                        object_id: "actor.auru".into(),
                        volume_id: "talk".into(),
                    }],
                    excluded_volumes: vec![crate::transition::VolumeReference {
                        object_id: "actor.auru".into(),
                        volume_id: "cutscene-trigger".into(),
                    }],
                    pose_predicate: PredicateExpression::All {
                        terms: vec![
                            PredicateExpression::Compare {
                                left: ValueReference::PlayerControl,
                                operator: ComparisonOperator::Equal,
                                right: ValueReference::Literal {
                                    value: StateValue::Boolean(true),
                                },
                            },
                            PredicateExpression::Compare {
                                left: ValueReference::PlayerAction,
                                operator: ComparisonOperator::Equal,
                                right: ValueReference::Literal {
                                    value: StateValue::Text("talk".into()),
                                },
                            },
                        ],
                    },
                    temporal_requirement: None,
                },
                evidence: evidence(TruthStatus::Established),
            },
            FeasibilityObligation {
                id: "obligation.td-coro-one-frame-ooccoo-pull".into(),
                label: "Pull Ooccoo on the one frame after Coro's bottle text".into(),
                scope: exact_scope.clone(),
                obligation_kind: ObligationKind::Timing,
                detail: ObligationDetail::Temporal {
                    requirement: TemporalRequirement {
                        action_id: "dialogue.coro-after-bottle".into(),
                        window: TemporalWindow {
                            earliest_frame: 0,
                            latest_frame: 0,
                            required_input: Some("ooccoo".into()),
                        },
                    },
                    precondition: flow_node_is("coro.after-bottle-before-next-line"),
                },
                evidence: evidence(TruthStatus::Established),
            },
            FeasibilityObligation {
                id: "obligation.td-ooccoo-death-pull".into(),
                label: "Pull Ooccoo on the lethal frame of the first introduction".into(),
                scope: exact_scope.clone(),
                obligation_kind: ObligationKind::Timing,
                detail: ObligationDetail::Temporal {
                    requirement: TemporalRequirement {
                        action_id: "dialogue.ooccoo-first-warp-on-death".into(),
                        window: TemporalWindow {
                            earliest_frame: 0,
                            latest_frame: 0,
                            required_input: Some("ooccoo".into()),
                        },
                    },
                    precondition: flow_node_is("ooccoo.first-warp-introduction"),
                },
                evidence: evidence(TruthStatus::Established),
            },
            FeasibilityObligation {
                id: "obligation.td-yeta-map-same-frame".into(),
                label: "Open the map on the first Yeta talk frame and roll away".into(),
                scope: exact_scope.clone(),
                obligation_kind: ObligationKind::Timing,
                detail: ObligationDetail::Temporal {
                    requirement: TemporalRequirement {
                        action_id: "dialogue.yeta-first-talk".into(),
                        window: TemporalWindow {
                            earliest_frame: 0,
                            latest_frame: 0,
                            required_input: Some("talk-map".into()),
                        },
                    },
                    precondition: flow_node_is("yeta.first-snowpeak-talk"),
                },
                evidence: evidence(TruthStatus::Established),
            },
        ];
        let trace = |id: &str, action_id: &str, input: &str, node: &str| WitnessedMicrotrace {
            id: id.into(),
            scope: exact_scope.clone(),
            precondition: flow_node_is(node),
            operations: vec![StateOperation::Interrupt {
                action_id: action_id.into(),
                window: TemporalWindow {
                    earliest_frame: 0,
                    latest_frame: 0,
                    required_input: Some(input.into()),
                },
            }],
            postcondition: PredicateExpression::True,
            timing: TemporalWindow {
                earliest_frame: 0,
                latest_frame: 0,
                required_input: Some(input.into()),
            },
            evidence: RuleEvidence {
                truth: TruthStatus::Established,
                records: vec![EvidenceRecord {
                    id: format!("community.{id}"),
                    kind: EvidenceKind::RouteWitnessed,
                    source_sha256: None,
                    note: "Observed producer-specific interruption window.".into(),
                }],
            },
        };
        mechanics.microtraces = vec![
            trace(
                "microtrace.td-coro-ooccoo-frame",
                "dialogue.coro-after-bottle",
                "ooccoo",
                "coro.after-bottle-before-next-line",
            ),
            trace(
                "microtrace.td-ooccoo-death-frame",
                "dialogue.ooccoo-first-warp-on-death",
                "ooccoo",
                "ooccoo.first-warp-introduction",
            ),
            trace(
                "microtrace.td-yeta-map-frame",
                "dialogue.yeta-first-talk",
                "talk-map",
                "yeta.first-snowpeak-talk",
            ),
        ];
        let goal = raw_bit_is(FLOW_B, true);
        let run = |stage: &str, node: &str, position: [f32; 3], action: &str| {
            let mut start = base.clone();
            start.environment.location.stage = stage.into();
            start.environment.player.position = position;
            start.environment.player.action = action.into();
            let flow = start
                .environment
                .components
                .iter_mut()
                .find(|component| component.id == "message.active-flow")
                .unwrap();
            let ComponentPayload::Structured { fields } = &mut flow.payload else {
                unreachable!()
            };
            fields.insert("node_id".into(), StateValue::Text(node.into()));
            ForwardSolver::new(&facts(), &mechanics, &[], SolverOptions::default())
                .unwrap()
                .solve(PlannerExecutionState::new(start).unwrap(), &goal)
                .unwrap()
        };

        let coro = run(
            "CORO",
            "coro.after-bottle-before-next-line",
            [0.0; 3],
            "ooccoo",
        );
        let auru = run("AURU", "auru.ready", [8.0, 0.0, 0.0], "talk");
        let yeta = run("YETA", "yeta.first-snowpeak-talk", [0.0; 3], "talk-map");
        let ooccoo = run(
            "OOCCOO",
            "ooccoo.first-warp-introduction",
            [0.0; 3],
            "ooccoo",
        );
        for result in [&coro, &auru, &yeta, &ooccoo] {
            assert_eq!(result.status, SearchStatus::Reached);
        }
        assert_eq!(
            coro.steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<Vec<_>>(),
            vec!["transition.td-coro-interrupt-after-bottle"]
        );
        assert_eq!(
            coro.steps[0].supporting_microtrace_ids,
            vec!["microtrace.td-coro-ooccoo-frame"]
        );
        assert_eq!(
            auru.steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "transition.td-auru-first-edge-talk",
                "transition.td-auru-second-edge-talk"
            ]
        );
        assert_eq!(
            yeta.steps[0].supporting_microtrace_ids,
            vec!["microtrace.td-yeta-map-frame"]
        );
        assert_eq!(
            ooccoo
                .steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "transition.td-ooccoo-zombie-death-pull",
                "transition.td-ooccoo-advance-second-bit"
            ]
        );

        let auru_trigger_overlap = run("AURU", "auru.ready", [0.0, 0.0, 0.0], "talk");
        assert_ne!(auru_trigger_overlap.status, SearchStatus::Reached);
        assert!(
            auru_trigger_overlap
                .blocked_transition_witnesses
                .iter()
                .any(|witness| {
                    witness.transition_id == "transition.td-auru-first-edge-talk"
                        && witness.outstanding_obligation_ids
                            == vec!["obligation.td-auru-talk-outside-trigger"]
                })
        );

        let mut no_coro_witness = mechanics.clone();
        no_coro_witness
            .microtraces
            .retain(|trace| trace.id != "microtrace.td-coro-ooccoo-frame");
        let mut coro_start = base;
        coro_start.environment.location.stage = "CORO".into();
        let flow = coro_start
            .environment
            .components
            .iter_mut()
            .find(|component| component.id == "message.active-flow")
            .unwrap();
        let ComponentPayload::Structured { fields } = &mut flow.payload else {
            unreachable!()
        };
        fields.insert(
            "node_id".into(),
            StateValue::Text("coro.after-bottle-before-next-line".into()),
        );
        let blocked = ForwardSolver::new(&facts(), &no_coro_witness, &[], SolverOptions::default())
            .unwrap()
            .solve(PlannerExecutionState::new(coro_start).unwrap(), &goal)
            .unwrap();
        assert_ne!(blocked.status, SearchStatus::Reached);
        assert!(blocked.blocked_transition_witnesses.iter().any(|witness| {
            witness.transition_id == "transition.td-coro-interrupt-after-bottle"
                && witness.unknown_obligation_ids
                    == vec!["obligation.td-coro-one-frame-ooccoo-pull"]
        }));
    }

    #[test]
    fn goron_text_displacement_composes_raw_consumer_and_independent_entrance_blockers() {
        const FLOW_A: u8 = 0x04;
        const FLOW_B: u8 = 0x02;
        const FLOW_C: u8 = 0x01;
        const M029: u8 = 0x04;
        const M031: u8 = 0x01;
        const SWITCH_6F: u8 = 0x80;
        const EXIT_ID: &str = "transition.r-sp110-scls0-goron-mines";
        const B_TO_C_ID: &str = "transition.gor-coron-flow6-b-to-c";
        const PRIME_A_ID: &str = "transition.gor-coron-flow9-prime-a";
        const CONSUMER_ID: &str = "transition.gor-coron-flow9-write-m029";
        const PRODUCER_IDS: [&str; 4] = [
            "transition.td-producer-auru",
            "transition.td-producer-coro",
            "transition.td-producer-ooccoo",
            "transition.td-producer-yeta",
        ];

        let mut start = snapshot();
        start.environment.location.stage = "TEXT_SETUP".into();
        start.environment.player.action = "roll".into();
        let component = |id: &str,
                         kind: ComponentKind,
                         payload: ComponentPayload,
                         binding: ComponentBinding,
                         lifetime: SemanticLifetime,
                         owner: SerializationOwner| StateComponent {
            id: id.into(),
            component_kind: kind,
            payload,
            binding,
            lifetime,
            serialization_owner: owner,
            provenance: vec![ComponentProvenance {
                source_kind: ProvenanceSourceKind::ExtractedFact,
                source_id: format!("gz2e01.{id}"),
                source_sha256: Some(Digest([0x81; 32])),
                transition_id: None,
            }],
        };
        let actor = |id: &str, fields: BTreeMap<String, StateValue>| {
            component(
                id,
                ComponentKind::ActorInstance,
                ComponentPayload::Structured { fields },
                ComponentBinding::Actor {
                    instance_id: id.into(),
                },
                SemanticLifetime::RoomLoad,
                SerializationOwner::None,
            )
        };
        start.environment.components = vec![
            actor(
                "actor.dm-elevator",
                BTreeMap::from([
                    ("approach_complete".into(), StateValue::Boolean(true)),
                    ("heavy_plate_active".into(), StateValue::Boolean(false)),
                ]),
            ),
            actor(
                "actor.elevator-guide-goron",
                BTreeMap::from([("gate_walk_complete".into(), StateValue::Boolean(false))]),
            ),
            actor(
                "actor.goron-blocker",
                BTreeMap::from([
                    ("collision_active".into(), StateValue::Boolean(true)),
                    ("room_generation".into(), StateValue::Unsigned(0)),
                ]),
            ),
            actor(
                "actor.gra-wall",
                BTreeMap::from([("collision_active".into(), StateValue::Boolean(true))]),
            ),
            component(
                "persistent.event-flags",
                ComponentKind::PersistentSave,
                ComponentPayload::Raw {
                    bytes: vec![0; 32],
                    known_mask: vec![0xff; 32],
                },
                ComponentBinding::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
                SemanticLifetime::RuntimeFile,
                SerializationOwner::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
            ),
            component(
                "stage.r-sp110-switches",
                ComponentKind::StageMemory,
                ComponentPayload::Raw {
                    bytes: vec![0; 24],
                    known_mask: vec![0xff; 24],
                },
                ComponentBinding::Stage {
                    stage: "R_SP110".into(),
                },
                SemanticLifetime::StageLoad,
                SerializationOwner::StageBank {
                    runtime_file_id: "file-0".into(),
                    stage: "R_SP110".into(),
                },
            ),
            component(
                "temporary.event-flags",
                ComponentKind::TemporaryFlags,
                ComponentPayload::Raw {
                    bytes: vec![0; 8],
                    known_mask: vec![0xff; 8],
                },
                ComponentBinding::Session {
                    session_id: "process".into(),
                },
                SemanticLifetime::Session,
                SerializationOwner::None,
            ),
        ];
        start
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
        let exact_scope = scope(&start);
        let alias = |id: &str,
                     label: &str,
                     component_kind: ComponentKind,
                     binding: ComponentBinding,
                     byte_offset: u32,
                     mask: u8| FriendlyAlias {
            id: id.into(),
            label: label.into(),
            scope: exact_scope.clone(),
            raw: RawFactBinding {
                component_kind,
                binding: ComponentBindingReference::Exact { binding },
                byte_offset,
                mask: vec![mask],
                expected: vec![mask],
            },
            evidence: evidence(TruthStatus::Established),
        };
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: vec![
                alias(
                    "event.gor-coron-won",
                    "M029: won Gor Coron match",
                    ComponentKind::PersistentSave,
                    ComponentBinding::RuntimeFile {
                        runtime_file_id: "file-0".into(),
                    },
                    7,
                    M029,
                ),
                alias(
                    "event.goron-mines-clear",
                    "M031: Goron Mines clear",
                    ComponentKind::PersistentSave,
                    ComponentBinding::RuntimeFile {
                        runtime_file_id: "file-0".into(),
                    },
                    7,
                    M031,
                ),
                alias(
                    "message.flow-a",
                    "Shared message flow-control A",
                    ComponentKind::TemporaryFlags,
                    ComponentBinding::Session {
                        session_id: "process".into(),
                    },
                    0,
                    FLOW_A,
                ),
                alias(
                    "message.flow-b",
                    "Shared message flow-control B",
                    ComponentKind::TemporaryFlags,
                    ComponentBinding::Session {
                        session_id: "process".into(),
                    },
                    0,
                    FLOW_B,
                ),
                alias(
                    "message.flow-c",
                    "Shared message flow-control C",
                    ComponentKind::TemporaryFlags,
                    ComponentBinding::Session {
                        session_id: "process".into(),
                    },
                    0,
                    FLOW_C,
                ),
                alias(
                    "switch.r-sp110-6f",
                    "R_SP110 one-zone switch 0x6f",
                    ComponentKind::StageMemory,
                    ComponentBinding::Stage {
                        stage: "R_SP110".into(),
                    },
                    22,
                    SWITCH_6F,
                ),
            ],
            derived_facts: Vec::new(),
        };
        let field_is =
            |component_id: &str, field: &str, value: StateValue| PredicateExpression::Compare {
                left: ValueReference::ComponentField {
                    component_id: component_id.into(),
                    field: field.into(),
                },
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal { value },
            };
        let room_is = |room: i8| PredicateExpression::Compare {
            left: ValueReference::LocationRoom,
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Signed(i64::from(room)),
            },
        };
        let layer_is = |layer: i8| PredicateExpression::Compare {
            left: ValueReference::LocationLayer,
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Signed(i64::from(layer)),
            },
        };
        let write_field =
            |component_id: &str, field: &str, value: StateValue| StateOperation::Write {
                target: crate::transition::ComponentFieldTarget {
                    component_id: component_id.into(),
                    field: field.into(),
                },
                value,
            };
        let write_raw =
            |component_id: &str, byte_offset: u32, mask: u8, value: u8| StateOperation::WriteRaw {
                component_id: component_id.into(),
                byte_offset,
                mask: vec![mask],
                value: vec![value],
            };
        let candidate = |id: &str,
                         label: &str,
                         kind: TransitionKind,
                         approach_id: &str,
                         guards: PredicateExpression,
                         obligations: Vec<&str>,
                         effects: Vec<StateOperation>,
                         truth: TruthStatus| CandidateTransition {
            id: id.into(),
            label: label.into(),
            scope: exact_scope.clone(),
            transition_kind: kind,
            approach_id: approach_id.into(),
            activation: ActivationContract {
                hard_guards: guards,
                physical_obligation_ids: obligations.into_iter().map(str::to_owned).collect(),
                effects,
                unknown_requirements: Vec::new(),
            },
            evidence: evidence(truth),
        };
        let producer = |id: &str, mask: u8| {
            candidate(
                id,
                &format!("Retain displaced message bit through {id}"),
                TransitionKind::MessageAction,
                &format!("approach.{id}"),
                stage_is("TEXT_SETUP"),
                Vec::new(),
                vec![write_raw("temporary.event-flags", 0, mask, mask)],
                TruthStatus::Established,
            )
        };
        let mut mechanics = catalog(vec![
            candidate(
                B_TO_C_ID,
                "Flow 6 reads displaced B and sets C before event 6 cut 4",
                TransitionKind::MessageAction,
                "approach.gor-coron-talk",
                PredicateExpression::All {
                    terms: vec![
                        stage_is("R_SP110"),
                        room_is(0),
                        layer_is(1),
                        PredicateExpression::Not {
                            term: Box::new(PredicateExpression::Fact {
                                fact_id: "event.goron-mines-clear".into(),
                            }),
                        },
                        PredicateExpression::Not {
                            term: Box::new(PredicateExpression::Fact {
                                fact_id: "message.flow-c".into(),
                            }),
                        },
                        PredicateExpression::Fact {
                            fact_id: "message.flow-b".into(),
                        },
                    ],
                },
                Vec::new(),
                vec![write_raw("temporary.event-flags", 0, FLOW_C, FLOW_C)],
                TruthStatus::Established,
            ),
            candidate(
                PRIME_A_ID,
                "Flow 6 sees C, jumps to flow 9, and primes A",
                TransitionKind::MessageAction,
                "approach.gor-coron-talk",
                PredicateExpression::All {
                    terms: vec![
                        stage_is("R_SP110"),
                        room_is(0),
                        layer_is(1),
                        PredicateExpression::Not {
                            term: Box::new(PredicateExpression::Fact {
                                fact_id: "event.goron-mines-clear".into(),
                            }),
                        },
                        PredicateExpression::Not {
                            term: Box::new(PredicateExpression::Fact {
                                fact_id: "message.flow-a".into(),
                            }),
                        },
                        PredicateExpression::Fact {
                            fact_id: "message.flow-c".into(),
                        },
                    ],
                },
                Vec::new(),
                vec![write_raw("temporary.event-flags", 0, FLOW_A, FLOW_A)],
                TruthStatus::Established,
            ),
            candidate(
                CONSUMER_ID,
                "Flow 6 sees C, flow 9 sees A, and event000 writes M029",
                TransitionKind::MessageAction,
                "approach.gor-coron-talk",
                PredicateExpression::All {
                    terms: vec![
                        stage_is("R_SP110"),
                        room_is(0),
                        layer_is(1),
                        PredicateExpression::Not {
                            term: Box::new(PredicateExpression::Fact {
                                fact_id: "event.goron-mines-clear".into(),
                            }),
                        },
                        PredicateExpression::Fact {
                            fact_id: "message.flow-c".into(),
                        },
                        PredicateExpression::Fact {
                            fact_id: "message.flow-a".into(),
                        },
                    ],
                },
                Vec::new(),
                vec![
                    write_raw("persistent.event-flags", 7, M029, M029),
                    write_raw("temporary.event-flags", 0, FLOW_A | FLOW_B, 0),
                    write_raw("temporary.event-flags", 0, FLOW_C, 0),
                ],
                TruthStatus::Established,
            ),
            candidate(
                "transition.enter-r-sp110-with-displaced-bit",
                "Enter the Goron Elder hall with displaced state",
                TransitionKind::EncodedMapExit,
                "approach.r-sp110",
                PredicateExpression::All {
                    terms: vec![
                        stage_is("TEXT_SETUP"),
                        PredicateExpression::Any {
                            terms: vec![
                                PredicateExpression::Fact {
                                    fact_id: "message.flow-b".into(),
                                },
                                PredicateExpression::Fact {
                                    fact_id: "message.flow-c".into(),
                                },
                            ],
                        },
                    ],
                },
                vec!["obligation.r-sp110-elevator-approach-complete"],
                vec![StateOperation::SetLocation {
                    location: SceneLocation {
                        stage: "R_SP110".into(),
                        room: 0,
                        layer: 1,
                        spawn: 0,
                    },
                }],
                TruthStatus::Established,
            ),
            candidate(
                "transition.r-sp110-unlock-elevator-guide",
                "Talk to the elevator guide after M029 and set switch 0x6f",
                TransitionKind::ActorDriven,
                "approach.r-sp110-elevator-guide",
                PredicateExpression::All {
                    terms: vec![
                        stage_is("R_SP110"),
                        PredicateExpression::Fact {
                            fact_id: "event.gor-coron-won".into(),
                        },
                        PredicateExpression::Not {
                            term: Box::new(PredicateExpression::Fact {
                                fact_id: "switch.r-sp110-6f".into(),
                            }),
                        },
                    ],
                },
                Vec::new(),
                vec![
                    write_raw("stage.r-sp110-switches", 22, SWITCH_6F, SWITCH_6F),
                    write_field(
                        "actor.elevator-guide-goron",
                        "gate_walk_complete",
                        StateValue::Boolean(true),
                    ),
                ],
                TruthStatus::Established,
            ),
            candidate(
                EXIT_ID,
                "Use R_SP110 SCLS exit 0 to enter Goron Mines",
                TransitionKind::EncodedMapExit,
                "approach.r-sp110-scls0",
                PredicateExpression::All {
                    terms: vec![stage_is("R_SP110"), room_is(0)],
                },
                vec![
                    "obligation.r-sp110-live-goron-clear",
                    "obligation.r-sp110-wall-clear",
                ],
                vec![StateOperation::SetLocation {
                    location: SceneLocation {
                        stage: "D_MN04".into(),
                        room: 1,
                        layer: 0,
                        spawn: 1,
                    },
                }],
                TruthStatus::Established,
            ),
            producer("transition.td-producer-auru", FLOW_B),
            producer("transition.td-producer-coro", FLOW_B),
            producer("transition.td-producer-ooccoo", FLOW_B),
            producer("transition.td-producer-yeta", FLOW_B),
        ]);
        mechanics
            .transitions
            .sort_by(|left, right| left.id.cmp(&right.id));
        let obligation =
            |id: &str, label: &str, predicate: PredicateExpression| FeasibilityObligation {
                id: id.into(),
                label: label.into(),
                scope: exact_scope.clone(),
                obligation_kind: ObligationKind::ActorState,
                detail: ObligationDetail::Predicate { predicate },
                evidence: evidence(TruthStatus::Established),
            };
        mechanics.obligations = vec![
            obligation(
                "obligation.r-sp110-elevator-approach-complete",
                "The independently resolved elevator approach has been completed",
                field_is(
                    "actor.dm-elevator",
                    "approach_complete",
                    StateValue::Boolean(true),
                ),
            ),
            obligation(
                "obligation.r-sp110-live-goron-clear",
                "Live blocking Goron no longer occupies the approach",
                field_is(
                    "actor.goron-blocker",
                    "collision_active",
                    StateValue::Boolean(false),
                ),
            ),
            obligation(
                "obligation.r-sp110-wall-clear",
                "GRA_WALL collision is absent",
                field_is(
                    "actor.gra-wall",
                    "collision_active",
                    StateValue::Boolean(false),
                ),
            ),
        ];
        let obstruction = |id: &str,
                           label: &str,
                           blocked_action_id: &str,
                           approach_id: &str,
                           active_when: PredicateExpression,
                           obligation_id: &str| Obstruction {
            id: id.into(),
            label: label.into(),
            scope: exact_scope.clone(),
            blocked_action_id: blocked_action_id.into(),
            approach_id: approach_id.into(),
            active_when,
            obligation_ids: vec![obligation_id.into()],
            evidence: evidence(TruthStatus::Established),
        };
        mechanics.obstructions = vec![
            obstruction(
                "obstruction.r-sp110-elevator",
                "The independently stateful elevator approach is incomplete",
                "transition.enter-r-sp110-with-displaced-bit",
                "approach.r-sp110",
                field_is(
                    "actor.dm-elevator",
                    "approach_complete",
                    StateValue::Boolean(false),
                ),
                "obligation.r-sp110-elevator-approach-complete",
            ),
            obstruction(
                "obstruction.r-sp110-live-goron",
                "A live Goron still occupies the approach",
                EXIT_ID,
                "approach.r-sp110-scls0",
                field_is(
                    "actor.goron-blocker",
                    "collision_active",
                    StateValue::Boolean(true),
                ),
                "obligation.r-sp110-live-goron-clear",
            ),
            obstruction(
                "obstruction.r-sp110-wall",
                "GRA_WALL collision blocks the exit approach",
                EXIT_ID,
                "approach.r-sp110-scls0",
                field_is(
                    "actor.gra-wall",
                    "collision_active",
                    StateValue::Boolean(true),
                ),
                "obligation.r-sp110-wall-clear",
            ),
        ];
        let resolver =
            |id: &str,
             label: &str,
             obstruction_id: &str,
             kind: ResolutionKind,
             extra: PredicateExpression,
             operations: Vec<StateOperation>| ObstructionResolver {
                id: id.into(),
                label: label.into(),
                scope: exact_scope.clone(),
                obstruction_id: obstruction_id.into(),
                resolution_kind: kind,
                applicable_when: PredicateExpression::All {
                    terms: vec![
                        PredicateExpression::Fact {
                            fact_id: "event.gor-coron-won".into(),
                        },
                        extra,
                    ],
                },
                operations,
                evidence: evidence(TruthStatus::Established),
            };
        mechanics.resolvers = vec![
            resolver(
                "resolver.r-sp110-npc-room-reload",
                "Reload the room so the blocking Goron reconstructs from M029",
                "obstruction.r-sp110-live-goron",
                ResolutionKind::Satisfy,
                PredicateExpression::True,
                vec![
                    write_field(
                        "actor.goron-blocker",
                        "collision_active",
                        StateValue::Boolean(false),
                    ),
                    write_field(
                        "actor.goron-blocker",
                        "room_generation",
                        StateValue::Unsigned(1),
                    ),
                ],
            ),
            resolver(
                "resolver.r-sp110-npc-roll-past",
                "Roll past the still-live blocking Goron",
                "obstruction.r-sp110-live-goron",
                ResolutionKind::Bypass,
                PredicateExpression::Compare {
                    left: ValueReference::PlayerAction,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text("roll".into()),
                    },
                },
                Vec::new(),
            ),
            resolver(
                "resolver.r-sp110-wall-execute-delete",
                "Let GRA_WALL Execute delete collision after M029",
                "obstruction.r-sp110-wall",
                ResolutionKind::Satisfy,
                PredicateExpression::True,
                vec![write_field(
                    "actor.gra-wall",
                    "collision_active",
                    StateValue::Boolean(false),
                )],
            ),
        ];
        mechanics
            .resolvers
            .sort_by(|left, right| left.id.cmp(&right.id));
        mechanics.reconstruction_rules = vec![
            ActorReconstructionRule {
                id: "reconstruct.r-sp110-goron-after-m029".into(),
                label: "Reconstruct the blocking Goron from M029".into(),
                scope: exact_scope.clone(),
                actor_type: "gra".into(),
                instantiate_when: PredicateExpression::Fact {
                    fact_id: "event.gor-coron-won".into(),
                },
                initialization_operations: vec![
                    write_field(
                        "actor.goron-blocker",
                        "collision_active",
                        StateValue::Boolean(false),
                    ),
                    write_field(
                        "actor.goron-blocker",
                        "room_generation",
                        StateValue::Unsigned(1),
                    ),
                ],
                evidence: evidence(TruthStatus::Established),
            },
            ActorReconstructionRule {
                id: "reconstruct.r-sp110-wall-before-m029".into(),
                label: "Instantiate GRA_WALL only while M029 is clear".into(),
                scope: exact_scope.clone(),
                actor_type: "gra_wall".into(),
                instantiate_when: PredicateExpression::Not {
                    term: Box::new(PredicateExpression::Fact {
                        fact_id: "event.gor-coron-won".into(),
                    }),
                },
                initialization_operations: vec![write_field(
                    "actor.gra-wall",
                    "collision_active",
                    StateValue::Boolean(true),
                )],
                evidence: evidence(TruthStatus::Established),
            },
        ];
        let raw_source = |component_id: &str, byte_offset: u32, mask: u8| ValueReference::RawBits {
            component_id: component_id.into(),
            byte_offset,
            byte_width: 1,
            mask: u64::from(mask),
        };
        mechanics.readers = vec![
            ReaderRule {
                id: "reader.gor-coron-flow6-b-path-m031".into(),
                scope: exact_scope.clone(),
                source: raw_source("persistent.event-flags", 7, M031),
                consuming_transition_id: B_TO_C_ID.into(),
                interpretation_fact_id: Some("event.goron-mines-clear".into()),
                evidence: evidence(TruthStatus::Established),
            },
            ReaderRule {
                id: "reader.gor-coron-flow6-b-path-b".into(),
                scope: exact_scope.clone(),
                source: raw_source("temporary.event-flags", 0, FLOW_B),
                consuming_transition_id: B_TO_C_ID.into(),
                interpretation_fact_id: Some("message.flow-b".into()),
                evidence: evidence(TruthStatus::Established),
            },
            ReaderRule {
                id: "reader.gor-coron-flow6-b-path-c".into(),
                scope: exact_scope.clone(),
                source: raw_source("temporary.event-flags", 0, FLOW_C),
                consuming_transition_id: B_TO_C_ID.into(),
                interpretation_fact_id: Some("message.flow-c".into()),
                evidence: evidence(TruthStatus::Established),
            },
            ReaderRule {
                id: "reader.gor-coron-flow9-prime-c".into(),
                scope: exact_scope.clone(),
                source: raw_source("temporary.event-flags", 0, FLOW_C),
                consuming_transition_id: PRIME_A_ID.into(),
                interpretation_fact_id: Some("message.flow-c".into()),
                evidence: evidence(TruthStatus::Established),
            },
            ReaderRule {
                id: "reader.gor-coron-flow9-prime-a".into(),
                scope: exact_scope.clone(),
                source: raw_source("temporary.event-flags", 0, FLOW_A),
                consuming_transition_id: PRIME_A_ID.into(),
                interpretation_fact_id: Some("message.flow-a".into()),
                evidence: evidence(TruthStatus::Established),
            },
            ReaderRule {
                id: "reader.gor-coron-flow9-win-c".into(),
                scope: exact_scope.clone(),
                source: raw_source("temporary.event-flags", 0, FLOW_C),
                consuming_transition_id: CONSUMER_ID.into(),
                interpretation_fact_id: Some("message.flow-c".into()),
                evidence: evidence(TruthStatus::Established),
            },
            ReaderRule {
                id: "reader.gor-coron-flow9-win-a".into(),
                scope: exact_scope.clone(),
                source: raw_source("temporary.event-flags", 0, FLOW_A),
                consuming_transition_id: CONSUMER_ID.into(),
                interpretation_fact_id: Some("message.flow-a".into()),
                evidence: evidence(TruthStatus::Established),
            },
            ReaderRule {
                id: "reader.r-sp110-scls0-m029".into(),
                scope: exact_scope.clone(),
                source: raw_source("persistent.event-flags", 7, M029),
                consuming_transition_id: EXIT_ID.into(),
                interpretation_fact_id: Some("event.gor-coron-won".into()),
                evidence: evidence(TruthStatus::Established),
            },
        ];
        mechanics
            .readers
            .sort_by(|left, right| left.id.cmp(&right.id));
        mechanics.validate().unwrap();
        let goal = stage_is("D_MN04");
        let solve =
            |snapshot: StateSnapshot, mechanics: &MechanicsCatalog, options: SolverOptions| {
                ForwardSolver::new(&facts, mechanics, &[], options)
                    .unwrap()
                    .solve(PlannerExecutionState::new(snapshot).unwrap(), &goal)
                    .unwrap()
            };

        let mut elevator_incomplete = start.clone();
        let ComponentPayload::Structured { fields } = &mut elevator_incomplete
            .environment
            .components
            .iter_mut()
            .find(|component| component.id == "actor.dm-elevator")
            .unwrap()
            .payload
        else {
            unreachable!()
        };
        fields.insert("approach_complete".into(), StateValue::Boolean(false));
        let ComponentPayload::Raw { bytes, .. } = &mut elevator_incomplete
            .environment
            .components
            .iter_mut()
            .find(|component| component.id == "persistent.event-flags")
            .unwrap()
            .payload
        else {
            unreachable!()
        };
        bytes[7] |= M029;
        let elevator_blocked = solve(elevator_incomplete, &mechanics, SolverOptions::default());
        assert_eq!(elevator_blocked.status, SearchStatus::UnreachableUnderModel);
        assert!(
            elevator_blocked
                .blocked_transition_witnesses
                .iter()
                .any(|witness| witness.transition_id
                    == "transition.enter-r-sp110-with-displaced-bit"
                    && witness.active_obstruction_ids == vec!["obstruction.r-sp110-elevator"])
        );

        let guide_transition = mechanics
            .transitions
            .iter()
            .find(|transition| transition.id == "transition.r-sp110-unlock-elevator-guide")
            .unwrap();
        assert!(guide_transition.activation.effects.iter().all(|operation| {
            !matches!(
                operation,
                StateOperation::Write { target, .. } if target.component_id == "actor.dm-elevator"
            )
        }));

        let mut encoded_but_blocked = start.clone();
        encoded_but_blocked.environment.location = SceneLocation {
            stage: "R_SP110".into(),
            room: 0,
            layer: 1,
            spawn: 0,
        };
        let blocked = solve(encoded_but_blocked, &mechanics, SolverOptions::default());
        assert_eq!(blocked.status, SearchStatus::UnreachableUnderModel);
        let exit_witness = blocked
            .blocked_transition_witnesses
            .iter()
            .find(|witness| witness.transition_id == EXIT_ID)
            .unwrap();
        assert_eq!(
            exit_witness.active_obstruction_ids,
            vec!["obstruction.r-sp110-live-goron", "obstruction.r-sp110-wall",]
        );

        let known_nonproducers = |transition: &CandidateTransition| {
            !transition.id.starts_with("transition.td-producer-")
        };
        for producer_id in PRODUCER_IDS {
            let mut isolated = mechanics.clone();
            isolated.transitions.retain(|transition| {
                known_nonproducers(transition) || transition.id == producer_id
            });
            let result = solve(start.clone(), &isolated, SolverOptions::default());
            assert_eq!(result.status, SearchStatus::Reached, "{producer_id}");
            let ids = result
                .steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<Vec<_>>();
            assert!(ids.contains(&producer_id));
            assert!(ids.contains(&B_TO_C_ID));
            assert!(ids.contains(&PRIME_A_ID));
            assert!(ids.contains(&CONSUMER_ID));
            assert!(ids.contains(&EXIT_ID));
            let index_of = |id: &str| ids.iter().position(|candidate| *candidate == id).unwrap();
            assert!(index_of(B_TO_C_ID) < index_of(PRIME_A_ID));
            assert!(index_of(PRIME_A_ID) < index_of(CONSUMER_ID));

            isolated
                .transitions
                .retain(|transition| transition.id != producer_id);
            let removed = solve(start.clone(), &isolated, SolverOptions::default());
            assert_eq!(
                removed.status,
                SearchStatus::UnreachableUnderModel,
                "removing {producer_id}"
            );
        }

        let full = solve(start.clone(), &mechanics, SolverOptions::default());
        assert_eq!(full.status, SearchStatus::Reached);
        for producer_id in PRODUCER_IDS {
            assert!(full.backward_relevance.contains_transition(producer_id));
        }
        assert!(full.backward_relevance.contains_transition(B_TO_C_ID));
        assert!(full.backward_relevance.contains_transition(PRIME_A_ID));
        assert!(full.backward_relevance.contains_transition(CONSUMER_ID));
        assert!(full.backward_relevance.contains_transition(EXIT_ID));

        let mut without_known_producers = mechanics.clone();
        without_known_producers
            .transitions
            .retain(known_nonproducers);
        let no_producer = solve(
            start.clone(),
            &without_known_producers,
            SolverOptions::default(),
        );
        assert_eq!(no_producer.status, SearchStatus::UnreachableUnderModel);
        let consumer_before = without_known_producers
            .transitions
            .iter()
            .find(|transition| transition.id == CONSUMER_ID)
            .unwrap()
            .clone();
        let exit_before = without_known_producers
            .transitions
            .iter()
            .find(|transition| transition.id == EXIT_ID)
            .unwrap()
            .clone();
        without_known_producers.transitions.push(producer(
            "transition.td-producer-hypothetical-new-interrupt",
            FLOW_B,
        ));
        let hypothetical = without_known_producers.transitions.last_mut().unwrap();
        hypothetical.evidence = evidence(TruthStatus::Hypothetical);
        without_known_producers
            .transitions
            .sort_by(|left, right| left.id.cmp(&right.id));
        let established_only = solve(
            start.clone(),
            &without_known_producers,
            SolverOptions::default(),
        );
        assert_ne!(established_only.status, SearchStatus::Reached);
        let research = solve(
            start.clone(),
            &without_known_producers,
            SolverOptions {
                evidence_policy: EvidencePolicy::RESEARCH,
                ..SolverOptions::default()
            },
        );
        assert_eq!(research.status, SearchStatus::Reached);
        assert!(
            research.steps.iter().any(|step| {
                step.action_id == "transition.td-producer-hypothetical-new-interrupt"
            })
        );
        assert_eq!(
            without_known_producers
                .transitions
                .iter()
                .find(|transition| transition.id == CONSUMER_ID)
                .unwrap(),
            &consumer_before
        );
        assert_eq!(
            without_known_producers
                .transitions
                .iter()
                .find(|transition| transition.id == EXIT_ID)
                .unwrap(),
            &exit_before
        );

        let mut roll_only = mechanics.clone();
        roll_only
            .resolvers
            .retain(|resolver| resolver.id != "resolver.r-sp110-npc-room-reload");
        let rolled = solve(start.clone(), &roll_only, SolverOptions::default());
        assert_eq!(rolled.status, SearchStatus::Reached);
        let rolled_exit = rolled
            .steps
            .iter()
            .find(|step| step.action_id == EXIT_ID)
            .unwrap();
        assert!(
            rolled_exit
                .selected_resolver_ids
                .contains(&"resolver.r-sp110-npc-roll-past".into())
        );

        let mut reload_only = mechanics.clone();
        reload_only
            .resolvers
            .retain(|resolver| resolver.id != "resolver.r-sp110-npc-roll-past");
        let reloaded = solve(start.clone(), &reload_only, SolverOptions::default());
        assert_eq!(reloaded.status, SearchStatus::Reached);
        let reloaded_exit = reloaded
            .steps
            .iter()
            .find(|step| step.action_id == EXIT_ID)
            .unwrap();
        assert!(
            reloaded_exit
                .selected_resolver_ids
                .contains(&"resolver.r-sp110-npc-room-reload".into())
        );

        let mut reconstructed = start;
        let ComponentPayload::Raw { bytes, .. } = &mut reconstructed
            .environment
            .components
            .iter_mut()
            .find(|component| component.id == "persistent.event-flags")
            .unwrap()
            .payload
        else {
            unreachable!()
        };
        bytes[7] |= M029;
        let mut reconstructed = PlannerExecutionState::new(reconstructed).unwrap();
        let goron_reconstruction = mechanics
            .reconstruction_rules
            .iter()
            .find(|rule| rule.id == "reconstruct.r-sp110-goron-after-m029")
            .unwrap();
        let wall_reconstruction = mechanics
            .reconstruction_rules
            .iter()
            .find(|rule| rule.id == "reconstruct.r-sp110-wall-before-m029")
            .unwrap();
        let evaluator = PredicateEvaluator::new(
            &reconstructed.snapshot,
            &facts,
            &[],
            &BTreeMap::new(),
            EvidencePolicy::ESTABLISHED_ONLY,
        )
        .unwrap();
        assert_eq!(
            evaluator.evaluate(&goron_reconstruction.instantiate_when),
            EvaluatedTruth::True
        );
        assert_eq!(
            evaluator.evaluate(&wall_reconstruction.instantiate_when),
            EvaluatedTruth::False
        );
        reconstructed
            .apply_operations(
                &goron_reconstruction.id,
                "snapshot.after-goron-reconstruction",
                &goron_reconstruction.initialization_operations,
            )
            .unwrap();
        let actor_field = |state: &PlannerExecutionState, id: &str, field: &str| {
            let component = state
                .snapshot
                .environment
                .components
                .iter()
                .find(|component| component.id == id)
                .unwrap();
            let ComponentPayload::Structured { fields } = &component.payload else {
                unreachable!()
            };
            fields[field].clone()
        };
        assert_eq!(
            actor_field(&reconstructed, "actor.gra-wall", "collision_active"),
            StateValue::Boolean(true)
        );
        assert_eq!(
            actor_field(&reconstructed, "actor.goron-blocker", "collision_active"),
            StateValue::Boolean(false)
        );
        assert_eq!(
            actor_field(&reconstructed, "actor.goron-blocker", "room_generation"),
            StateValue::Unsigned(1)
        );
        assert_eq!(
            actor_field(&reconstructed, "actor.dm-elevator", "heavy_plate_active"),
            StateValue::Boolean(false)
        );
        assert_eq!(
            actor_field(
                &reconstructed,
                "actor.elevator-guide-goron",
                "gate_walk_complete"
            ),
            StateValue::Boolean(false)
        );
    }
}
