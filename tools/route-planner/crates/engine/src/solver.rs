//! Bounded forward state search with explicit feasibility choices and proofs.

use crate::evaluation::{
    EvaluatedTruth, EvidencePolicy, FeasibilityMode, FeasibilityResolution, FeasibilitySelection,
    PredicateEvaluator, RuleClassification, TransitionAssessment, TransitionClassification,
};
use crate::execution::PlannerExecutionState;
use crate::identity::EquivalenceSet;
use crate::logic::{FactCatalog, PredicateExpression, TruthStatus};
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
    pub introduced_obligation_ids: Vec<String>,
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
    pub unknown_requirement_ids: Vec<String>,
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
                        introduced_obligation_ids: technique.introduced_obligation_ids.clone(),
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
                                    unknown_requirement_ids: preliminary.unknown_requirement_ids,
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
                                    blocked_witness(transition, &next, &resolution, &assessment)?,
                                );
                                continue;
                            }
                            TransitionClassification::Inapplicable
                            | TransitionClassification::GuardBlocked
                            | TransitionClassification::Obstructed => {
                                record_blocked_transition_witness(
                                    &mut blocked_transition_witnesses,
                                    blocked_witness(transition, &next, &resolution, &assessment)?,
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
    transition: &CandidateTransition,
    source: &PlannerExecutionState,
    resolution: &FeasibilityResolution,
    assessment: &TransitionAssessment,
) -> Result<BlockedTransitionWitness, PlannerContractError> {
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
        unknown_requirement_ids: assessment.unknown_requirement_ids.clone(),
    })
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
        ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA,
        RuleEvidence, TruthStatus, ValueReference,
    };
    use crate::route_book::{
        CollapsePolicy, PlanMethod, PlanRegion, ROUTE_BOOK_SCHEMA, ReferenceStep,
        RouteBookManifest, RouteConstraint, RouteDirective, RouteDirectiveKind,
    };
    use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
    use crate::state::{
        BackingAttachment, EXECUTION_ENVIRONMENT_SCHEMA, ExecutionEnvironment, PlayerForm,
        PlayerState, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation,
        StateValue,
    };
    use crate::transition::{
        ActivationContract, CandidateTransition, FeasibilityObligation, Goal,
        MECHANICS_CATALOG_SCHEMA, MechanicsCatalog, ObligationDetail, ObligationKind, Obstruction,
        ObstructionResolver, ResolutionKind, RouteCost, StateOperation, Technique, TransitionKind,
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
