//! Public search results, proofs, and validation contracts.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SolverOptions {
    pub max_depth: usize,
    pub max_states: usize,
    pub max_resolution_combinations: usize,
    pub feasibility_mode: FeasibilityMode,
    pub evidence_policy: EvidencePolicy,
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
    Reconstruction,
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
#[serde(deny_unknown_fields)]
pub struct BlockedTechniqueWitness {
    pub technique_id: String,
    pub source_state_sha256: Digest,
    pub classification: RuleClassification,
    pub prerequisites: EvaluatedTruth,
    pub prerequisites_expression: PredicateExpression,
    pub operations: Vec<StateOperation>,
    pub evidence_dependencies: Vec<EvidenceDependency>,
    pub weakest_evidence: Option<TruthStatus>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlockedResolverWitness {
    pub resolver_id: String,
    pub obstruction_id: String,
    pub source_state_sha256: Digest,
    pub classification: RuleClassification,
    pub applicability: EvaluatedTruth,
    pub applicability_expression: PredicateExpression,
    pub operations: Vec<StateOperation>,
    pub evidence_dependencies: Vec<EvidenceDependency>,
    pub weakest_evidence: Option<TruthStatus>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlockedReconstructionWitness {
    pub reconstruction_rule_id: String,
    pub source_state_sha256: Digest,
    pub classification: RuleClassification,
    pub activation: EvaluatedTruth,
    pub instantiate_when: PredicateExpression,
    pub initialization_operations: Vec<StateOperation>,
    pub evidence_dependencies: Vec<EvidenceDependency>,
    pub weakest_evidence: Option<TruthStatus>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FailedProducerAssumption {
    ReconstructionBoundary {
        reconstruction_rule_id: String,
        source_state_sha256: Digest,
        classification: RuleClassification,
    },
    NoCatalogProducer {
        source_state_sha256: Digest,
    },
}

impl FailedProducerAssumption {
    pub(super) fn identity(&self) -> &str {
        match self {
            Self::ReconstructionBoundary {
                reconstruction_rule_id,
                ..
            } => reconstruction_rule_id,
            Self::NoCatalogProducer { .. } => "no_catalog_producer",
        }
    }

    fn validate(&self) -> Result<(), PlannerContractError> {
        let (id, source): (&str, Digest) = match self {
            Self::ReconstructionBoundary {
                reconstruction_rule_id,
                source_state_sha256,
                ..
            } => (reconstruction_rule_id, *source_state_sha256),
            Self::NoCatalogProducer {
                source_state_sha256,
            } => ("no_catalog_producer", *source_state_sha256),
        };
        validate_stable_id("solver.failed_producer_assumption.id", id)?;
        if source == Digest::ZERO {
            return Err(PlannerContractError::new(
                "solver.failed_producer_assumption.source_state_sha256",
                "must be nonzero",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FailedProducerCutSet {
    pub cuts: Vec<FailedProducerCut>,
}

impl FailedProducerCutSet {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.cuts.is_empty() {
            return Err(PlannerContractError::new(
                "solver.failed_producer_cut_set",
                "must contain at least one dependency cut",
            ));
        }
        let mut prior = None;
        for cut in &self.cuts {
            cut.validate()?;
            if prior
                .as_ref()
                .is_some_and(|dependency| dependency >= &cut.dependency)
            {
                return Err(PlannerContractError::new(
                    "solver.failed_producer_cut_set.cuts",
                    "must be unique and sorted by dependency",
                ));
            }
            prior = Some(cut.dependency.clone());
        }
        Ok(())
    }
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
    Technique {
        technique_id: String,
        source_state_sha256: Digest,
        classification: RuleClassification,
    },
    Resolver {
        resolver_id: String,
        source_state_sha256: Digest,
        classification: RuleClassification,
        consumer_transition_id: Option<String>,
        consumer_classification: Option<TransitionClassification>,
    },
}

impl FailedProducerBlocker {
    pub(super) fn action(&self) -> RouteActionRef {
        match self {
            Self::Transition { transition_id, .. } => RouteActionRef::Transition {
                transition_id: transition_id.clone(),
            },
            Self::Writer { writer_id, .. } => RouteActionRef::Writer {
                writer_id: writer_id.clone(),
            },
            Self::Technique { technique_id, .. } => RouteActionRef::Technique {
                technique_id: technique_id.clone(),
            },
            Self::Resolver { resolver_id, .. } => RouteActionRef::Resolver {
                resolver_id: resolver_id.clone(),
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
            Self::Technique {
                source_state_sha256,
                classification,
                ..
            } => (
                self.action(),
                *source_state_sha256,
                *classification == RuleClassification::Active,
            ),
            Self::Resolver {
                source_state_sha256,
                classification,
                consumer_transition_id,
                consumer_classification,
                ..
            } => {
                let active_without_blocked_consumer = *classification == RuleClassification::Active
                    && (consumer_transition_id.is_none()
                        || consumer_classification.is_none_or(|classification| {
                            classification == TransitionClassification::Executable
                        }));
                if (*classification == RuleClassification::Active)
                    != consumer_transition_id.is_some()
                    || consumer_transition_id.is_some() != consumer_classification.is_some()
                {
                    return Err(PlannerContractError::new(
                        "solver.failed_producer_blocker.resolver",
                        "an active resolver requires one blocked consumer and an inactive resolver must not name one",
                    ));
                }
                (
                    self.action(),
                    *source_state_sha256,
                    active_without_blocked_consumer,
                )
            }
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
    pub missing_assumptions: Vec<FailedProducerAssumption>,
}

impl FailedProducerCut {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if (self.blocked_producers.is_empty() && self.missing_assumptions.is_empty())
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
        let mut prior_assumption = None;
        for assumption in &self.missing_assumptions {
            assumption.validate()?;
            if prior_assumption.is_some_and(|prior: &str| prior >= assumption.identity()) {
                return Err(PlannerContractError::new(
                    "solver.failed_producer_cut.missing_assumptions",
                    "must be unique and sorted by assumption identity",
                ));
            }
            prior_assumption = Some(assumption.identity());
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
    pub result_continuation: Option<ContinuationIdentity>,
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
    pub blocked_technique_witnesses: Vec<BlockedTechniqueWitness>,
    pub blocked_resolver_witnesses: Vec<BlockedResolverWitness>,
    pub blocked_reconstruction_witnesses: Vec<BlockedReconstructionWitness>,
    pub continuation_merge_proofs: Vec<ContinuationMergeProof>,
    pub failed_producer_cuts: Vec<FailedProducerCut>,
    pub failed_producer_cut_sets: Vec<FailedProducerCutSet>,
    pub failed_producer_cut_sets_complete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchPlan {
    pub result_state_sha256: Digest,
    pub continuation: ContinuationIdentity,
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
        self.continuation.validate()?;
        if self.continuation.state_sha256 != self.result_state_sha256
            || self.continuation.satisfied_preference_ids != self.satisfied_preference_ids
            || self.continuation.route_condition_unknown
        {
            return Err(PlannerContractError::new(
                "solver.search_plan.continuation",
                "must be the exact known terminal continuation for this reached plan",
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
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

pub(super) fn validate_route_action_ref(
    action: &RouteActionRef,
) -> Result<(), PlannerContractError> {
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
