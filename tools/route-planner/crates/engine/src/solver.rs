//! Bounded forward state search with explicit feasibility choices and proofs.

use crate::evaluation::{
    EvaluatedTruth, EvidencePolicy, FeasibilityMode, FeasibilityResolution, FeasibilitySelection,
    PredicateEvaluator, RuleClassification, TransitionAssessment, TransitionClassification,
};
use crate::execution::PlannerExecutionState;
use crate::identity::EquivalenceSet;
use crate::logic::{FactCatalog, PredicateExpression, RuleEvidence, TruthStatus};
use crate::route_book::{RouteActionRef, RouteBook, RouteDirectiveKind};
use crate::transition::{CandidateTransition, MechanicsCatalog, PathConstraint};
use crate::{PlannerContractError, artifact::Digest};
use serde::Serialize;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchActionKind {
    Transition,
    Technique,
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
    pub evidence_dependencies: Vec<EvidenceDependency>,
    pub weakest_evidence: Option<TruthStatus>,
    pub source_state_sha256: Digest,
    pub result_state_sha256: Digest,
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
    pub evidence_dependencies: Vec<EvidenceDependency>,
    pub weakest_evidence: Option<TruthStatus>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchResult {
    pub status: SearchStatus,
    pub steps: Vec<SearchStep>,
    pub explored_states: usize,
    pub hit_search_limit: bool,
    pub preference_score: u64,
    pub satisfied_preference_ids: Vec<String>,
    pub route_costs: BTreeMap<String, u64>,
    pub minimum_evidence: Option<TruthStatus>,
    pub unknown_transition_ids: Vec<String>,
    pub execution_error_ids: Vec<String>,
    pub blocked_transition_witnesses: Vec<BlockedTransitionWitness>,
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
    state_sha256: Digest,
    satisfied_required_actions: Vec<RouteActionRef>,
    required_sequence_progress: Vec<usize>,
    banned_sequence_progress: Vec<usize>,
    preferred_sequence_progress: Vec<usize>,
    satisfied_preference_ids: Vec<String>,
    route_condition_unknown: bool,
    route_costs: BTreeMap<String, u64>,
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
        let mut unknown_transition_ids = BTreeSet::new();
        let mut execution_error_ids = BTreeSet::new();
        let mut blocked_transition_witnesses = BTreeMap::new();
        let mut saw_unknown_goal = false;
        let mut hit_search_limit = false;
        let mut generated_id = 0_u64;

        while let Some(QueueEntry { node, .. }) = queue.pop() {
            let state_identity = node.state.semantic_digest()?;
            let search_identity = SearchIdentity {
                state_sha256: state_identity,
                satisfied_required_actions: node
                    .satisfied_required_actions
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>(),
                required_sequence_progress: node.required_sequence_progress.clone(),
                banned_sequence_progress: node.banned_sequence_progress.clone(),
                preferred_sequence_progress: node.preferred_sequence_progress.clone(),
                satisfied_preference_ids: node.satisfied_preference_ids.iter().cloned().collect(),
                route_condition_unknown: node.route_condition_unknown,
                route_costs: node.route_costs.clone(),
            };
            if !visited.insert(search_identity) {
                continue;
            }
            if visited.len() > self.options.max_states {
                hit_search_limit = true;
                break;
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
                    return Ok(SearchResult {
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
                        minimum_evidence: route_policy
                            .as_ref()
                            .and_then(|policy| policy.minimum_evidence),
                        unknown_transition_ids: unknown_transition_ids.into_iter().collect(),
                        execution_error_ids: execution_error_ids.into_iter().collect(),
                        blocked_transition_witnesses: Vec::new(),
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
                if !self.mechanics.transitions.is_empty() || !self.mechanics.techniques.is_empty() {
                    hit_search_limit = true;
                }
                continue;
            }

            // Techniques with concrete state operations are also standalone
            // actions. Their obligation annotations are action-local and are
            // considered separately when combining a technique with a target
            // transition below.
            for technique in &self.mechanics.techniques {
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
                    action,
                    before: node.state.clone(),
                    after: next.clone(),
                };
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
                        evidence_dependencies,
                        weakest_evidence,
                        source_state_sha256: state_identity,
                        result_state_sha256: Digest::ZERO,
                    },
                )?;
            }

            for transition in &self.mechanics.transitions {
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
                            let evidence_dependencies = transition_evidence_dependencies(
                                self.facts,
                                self.mechanics,
                                transition,
                                &resolution,
                                &preliminary,
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
                                    evidence_dependencies,
                                    weakest_evidence,
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
                        let evidence_dependencies = transition_evidence_dependencies(
                            self.facts,
                            self.mechanics,
                            transition,
                            &resolution,
                            &assessment,
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
                                evidence_dependencies,
                                weakest_evidence,
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

        let unknown = hit_search_limit
            || saw_unknown_goal
            || !unknown_transition_ids.is_empty()
            || !execution_error_ids.is_empty();
        Ok(SearchResult {
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
            minimum_evidence: route_policy
                .as_ref()
                .and_then(|policy| policy.minimum_evidence),
            unknown_transition_ids: unknown_transition_ids.into_iter().collect(),
            execution_error_ids: execution_error_ids.into_iter().collect(),
            blocked_transition_witnesses: blocked_transition_witnesses.into_values().collect(),
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
        let search_identity = SearchIdentity {
            state_sha256: result,
            satisfied_required_actions: satisfied_required_actions
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            required_sequence_progress: required_sequence_progress.clone(),
            banned_sequence_progress: banned_sequence_progress.clone(),
            preferred_sequence_progress: preferred_sequence_progress.clone(),
            satisfied_preference_ids: satisfied_preference_ids.iter().cloned().collect(),
            route_condition_unknown,
            route_costs: route_costs.clone(),
        };
        if visited.contains(&search_identity) {
            return Ok(saw_unknown_condition);
        }
        step.result_state_sha256 = result;
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
) -> Result<BlockedTransitionWitness, PlannerContractError> {
    let evidence_dependencies =
        transition_evidence_dependencies(facts, mechanics, transition, resolution, assessment);
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
        evidence_dependencies,
        weakest_evidence,
    })
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

fn transition_evidence_dependencies(
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
    transition: &CandidateTransition,
    resolution: &FeasibilityResolution,
    assessment: &TransitionAssessment,
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
        BackingAttachment, BoundaryKind, ComponentBinding, ComponentKind, ComponentPayload,
        ComponentProvenance, ComponentSelector, EXECUTION_ENVIRONMENT_SCHEMA, ExecutionEnvironment,
        PlayerForm, PlayerState, ProvenanceSourceKind, RuntimeFile, RuntimeFileLifecycle,
        RuntimeFileOrigin, SceneLocation, SemanticLifetime, SerializationOwner, StateComponent,
        StateValue,
    };
    use crate::transition::{
        ActivationContract, ActorReconstructionRule, CandidateTransition, FeasibilityObligation,
        Goal, MECHANICS_CATALOG_SCHEMA, MechanicsCatalog, ObligationDetail, ObligationKind,
        Obstruction, ObstructionResolver, ResolutionKind, RouteCost, StateOperation, Technique,
        TemporalRequirement, TemporalWindow, TransitionKind, WitnessedMicrotrace,
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
        let mechanics = catalog(vec![
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
            binding: ComponentBinding::Stage {
                stage: stage.into(),
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
    fn keyed_door_uses_bound_fungible_keys_and_oob_does_not_mutate_it() {
        let dungeon_field = |dungeon: &str, field: &str| ValueReference::BoundComponentField {
            component_kind: ComponentKind::DungeonMemory,
            binding: ComponentBinding::Dungeon {
                dungeon: dungeon.into(),
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
}
