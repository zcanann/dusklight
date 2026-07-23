//! Independent application boundary for the causal route planner.
//!
//! This crate intentionally does not depend on Huntctl's CLI, TAS timeline
//! workbench, playback graph, or browser protocol. It consumes the planner
//! engine's canonical artifacts and returns planner-specific reports.

pub mod inspection;
pub mod project;
pub mod service;
pub mod web;

use dusklight_route_planner::PlannerContractError;
use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::evaluation::{EvidencePolicy, FeasibilityMode};
use dusklight_route_planner::execution::PlannerExecutionState;
use dusklight_route_planner::identity::{ContextSelector, EquivalenceSet, ExactContext};
use dusklight_route_planner::logic::{FactCatalog, TruthStatus};
use dusklight_route_planner::refinement::{ComposedPlannerCatalog, RefinementStackEntry};
use dusklight_route_planner::route_book::RouteBook;
use dusklight_route_planner::solver::{
    ForwardSolver, SearchActionKind, SearchPlan, SearchResult, SearchStatus, SolverOptions,
};
use dusklight_route_planner::transition::MechanicsCatalog;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SOLVE_REPORT_SCHEMA: &str = "dusklight.route-planner.solve-report/v11";
pub const PORTABLE_SOLVE_REPORT_SCHEMA: &str = "dusklight.route-planner.portable-solve-report/v10";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeFeasibilityMode {
    Modeled,
    UpperBound,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEvidenceMode {
    EstablishedOnly,
    Research,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSolveOptions {
    pub max_depth: usize,
    pub max_states: usize,
    pub max_resolution_combinations: usize,
    #[serde(default = "default_max_plans")]
    pub max_plans: usize,
    pub feasibility_mode: RuntimeFeasibilityMode,
    pub evidence_mode: RuntimeEvidenceMode,
}

const fn default_max_plans() -> usize {
    1
}

impl Default for RuntimeSolveOptions {
    fn default() -> Self {
        let solver = SolverOptions::default();
        Self {
            max_depth: solver.max_depth,
            max_states: solver.max_states,
            max_resolution_combinations: solver.max_resolution_combinations,
            max_plans: default_max_plans(),
            feasibility_mode: RuntimeFeasibilityMode::Modeled,
            evidence_mode: RuntimeEvidenceMode::EstablishedOnly,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SolveReport {
    pub schema: String,
    pub goal_id: String,
    pub execution_state_sha256: Digest,
    pub fact_catalog_sha256: Digest,
    pub mechanics_catalog_sha256: Digest,
    pub refinement_stack_sha256: Option<Digest>,
    pub route_book_sha256: Option<Digest>,
    pub refinement_pack_ids: Vec<String>,
    pub refinement_stack_entries: Vec<RefinementStackEntry>,
    pub equivalence_set_count: usize,
    pub feasibility_mode: RuntimeFeasibilityMode,
    pub evidence_mode: RuntimeEvidenceMode,
    /// Compact presentation derived only from the complete result below. The
    /// result remains the expanded research view and sole proof authority.
    pub summary: SolveSummary,
    pub result: SearchResult,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SolveSummary {
    pub status: SearchStatus,
    pub primary_plan: Option<PlanSummary>,
    pub alternative_plans: Vec<PlanSummary>,
    pub explored_states: usize,
    pub hit_search_limit: bool,
    pub unknown_transition_ids: Vec<String>,
    pub unknown_writer_ids: Vec<String>,
    pub execution_error_ids: Vec<String>,
    pub blocked_transition_count: usize,
    pub blocked_writer_count: usize,
    pub continuation_merge_count: usize,
    pub failed_producer_cut_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSummary {
    pub actions: Vec<ActionSummary>,
    pub preference_score: u64,
    pub satisfied_preference_ids: Vec<String>,
    pub route_costs: BTreeMap<String, u64>,
    pub weakest_evidence: Option<TruthStatus>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionSummary {
    pub action_kind: SearchActionKind,
    pub action_id: String,
}

impl SolveSummary {
    pub fn from_result(result: &SearchResult) -> Self {
        let primary_plan = (result.status == SearchStatus::Reached).then(|| PlanSummary {
            actions: result
                .steps
                .iter()
                .map(|step| ActionSummary {
                    action_kind: step.action_kind,
                    action_id: step.action_id.clone(),
                })
                .collect(),
            preference_score: result.preference_score,
            satisfied_preference_ids: result.satisfied_preference_ids.clone(),
            route_costs: result.route_costs.clone(),
            weakest_evidence: result
                .steps
                .iter()
                .filter_map(|step| step.weakest_evidence)
                .max(),
        });
        Self {
            status: result.status,
            primary_plan,
            alternative_plans: result
                .alternative_plans
                .iter()
                .map(PlanSummary::from_search_plan)
                .collect(),
            explored_states: result.explored_states,
            hit_search_limit: result.hit_search_limit,
            unknown_transition_ids: result.unknown_transition_ids.clone(),
            unknown_writer_ids: result.unknown_writer_ids.clone(),
            execution_error_ids: result.execution_error_ids.clone(),
            blocked_transition_count: result.blocked_transition_witnesses.len(),
            blocked_writer_count: result.blocked_writer_witnesses.len(),
            continuation_merge_count: result.continuation_merge_proofs.len(),
            failed_producer_cut_count: result.failed_producer_cuts.len(),
        }
    }
}

impl PlanSummary {
    fn from_search_plan(plan: &SearchPlan) -> Self {
        Self {
            actions: plan
                .steps
                .iter()
                .map(|step| ActionSummary {
                    action_kind: step.action_kind,
                    action_id: step.action_id.clone(),
                })
                .collect(),
            preference_score: plan.preference_score,
            satisfied_preference_ids: plan.satisfied_preference_ids.clone(),
            route_costs: plan.route_costs.clone(),
            weakest_evidence: plan
                .steps
                .iter()
                .filter_map(|step| step.weakest_evidence)
                .max(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortableSearchStatus {
    ReachedAll,
    UnreachableInSome,
    UnknownInSome,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortableContextSolveReport {
    pub exact_context: ExactContext,
    pub report: SolveReport,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortableSolveReport {
    pub schema: String,
    pub goal_id: String,
    pub route_book_sha256: Digest,
    pub status: PortableSearchStatus,
    pub contexts: Vec<PortableContextSolveReport>,
}

pub fn solve_catalog_goal(
    state: PlannerExecutionState,
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
    equivalence_sets: &[EquivalenceSet],
    goal_id: &str,
    options: RuntimeSolveOptions,
) -> Result<SolveReport, PlannerContractError> {
    solve_catalog_goal_inner(
        state,
        facts,
        mechanics,
        equivalence_sets,
        goal_id,
        options,
        None,
    )
}

pub fn solve_catalog_route_book_goal(
    state: PlannerExecutionState,
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: &RouteBook,
    goal_id: &str,
    options: RuntimeSolveOptions,
) -> Result<SolveReport, PlannerContractError> {
    solve_catalog_goal_inner(
        state,
        facts,
        mechanics,
        equivalence_sets,
        goal_id,
        options,
        Some(route_book),
    )
}

fn solve_catalog_goal_inner(
    state: PlannerExecutionState,
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
    equivalence_sets: &[EquivalenceSet],
    goal_id: &str,
    options: RuntimeSolveOptions,
    route_book: Option<&RouteBook>,
) -> Result<SolveReport, PlannerContractError> {
    state.validate()?;
    facts.validate()?;
    mechanics.validate()?;
    let goal = mechanics
        .goals
        .iter()
        .find(|goal| goal.id == goal_id)
        .ok_or_else(|| PlannerContractError::new("goal_id", "is absent from the catalog"))?;
    if let Some(book) = route_book {
        book.validate_against(facts, mechanics)?;
        if !book.goal_ids.iter().any(|id| id == goal_id) {
            return Err(PlannerContractError::new(
                "goal_id",
                "is not selected by the route book",
            ));
        }
    }
    let solver_options = SolverOptions {
        max_depth: options.max_depth,
        max_states: options.max_states,
        max_resolution_combinations: options.max_resolution_combinations,
        feasibility_mode: match options.feasibility_mode {
            RuntimeFeasibilityMode::Modeled => FeasibilityMode::Modeled,
            RuntimeFeasibilityMode::UpperBound => FeasibilityMode::UpperBound,
        },
        evidence_policy: match options.evidence_mode {
            RuntimeEvidenceMode::EstablishedOnly => EvidencePolicy::ESTABLISHED_ONLY,
            RuntimeEvidenceMode::Research => EvidencePolicy::RESEARCH,
        },
    };
    let execution_state_sha256 = state.digest()?;
    let result = match route_book {
        Some(book) => ForwardSolver::new_with_route_book(
            facts,
            mechanics,
            equivalence_sets,
            solver_options,
            book,
        )?
        .solve_alternatives(state, &goal.predicate, options.max_plans)?,
        None => ForwardSolver::new(facts, mechanics, equivalence_sets, solver_options)?
            .solve_alternatives(state, &goal.predicate, options.max_plans)?,
    };
    let summary = SolveSummary::from_result(&result);
    Ok(SolveReport {
        schema: SOLVE_REPORT_SCHEMA.into(),
        goal_id: goal.id.clone(),
        execution_state_sha256,
        fact_catalog_sha256: facts.digest()?,
        mechanics_catalog_sha256: mechanics.digest()?,
        refinement_stack_sha256: None,
        route_book_sha256: route_book.map(RouteBook::digest).transpose()?,
        refinement_pack_ids: Vec::new(),
        refinement_stack_entries: Vec::new(),
        equivalence_set_count: equivalence_sets.len(),
        feasibility_mode: options.feasibility_mode,
        evidence_mode: options.evidence_mode,
        summary,
        result,
    })
}

pub fn solve_composed_catalog_goal(
    state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    goal_id: &str,
    options: RuntimeSolveOptions,
) -> Result<SolveReport, PlannerContractError> {
    catalog.validate()?;
    let mut report = solve_catalog_goal(
        state,
        &catalog.facts,
        &catalog.mechanics,
        equivalence_sets,
        goal_id,
        options,
    )?;
    report.refinement_stack_sha256 = Some(catalog.refinement_stack.digest()?);
    report.refinement_pack_ids = catalog
        .refinement_stack
        .entries
        .iter()
        .map(|entry| entry.pack_id.clone())
        .collect();
    report.refinement_stack_entries = catalog.refinement_stack.entries.clone();
    Ok(report)
}

pub fn solve_composed_route_book_goal(
    state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: &RouteBook,
    goal_id: &str,
    options: RuntimeSolveOptions,
) -> Result<SolveReport, PlannerContractError> {
    catalog.validate()?;
    route_book.validate_against_composed(catalog)?;
    let mut report = solve_catalog_route_book_goal(
        state,
        &catalog.facts,
        &catalog.mechanics,
        equivalence_sets,
        route_book,
        goal_id,
        options,
    )?;
    report.refinement_stack_sha256 = Some(catalog.refinement_stack.digest()?);
    report.refinement_pack_ids = catalog
        .refinement_stack
        .entries
        .iter()
        .map(|entry| entry.pack_id.clone())
        .collect();
    report.refinement_stack_entries = catalog.refinement_stack.entries.clone();
    Ok(report)
}

pub fn solve_catalog_portable_route_book_goal(
    states: Vec<PlannerExecutionState>,
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: &RouteBook,
    goal_id: &str,
    options: RuntimeSolveOptions,
) -> Result<PortableSolveReport, PlannerContractError> {
    route_book.validate_against(facts, mechanics)?;
    let expected_contexts = expand_route_book_contexts(route_book, equivalence_sets)?;
    let mut states_by_context = BTreeMap::new();
    for state in states {
        state.validate()?;
        let exact_context = state
            .snapshot
            .environment
            .runtime_configuration
            .exact_context()?;
        if !expected_contexts.contains(&exact_context) {
            return Err(PlannerContractError::new(
                "states",
                "contains a context outside the route-book manifest scope",
            ));
        }
        if states_by_context.insert(exact_context, state).is_some() {
            return Err(PlannerContractError::new(
                "states",
                "contains duplicate start states for one exact context",
            ));
        }
    }
    if states_by_context.len() != expected_contexts.len() {
        return Err(PlannerContractError::new(
            "states",
            "must contain exactly one start state for every selected exact context",
        ));
    }

    let mut contexts = Vec::with_capacity(expected_contexts.len());
    for exact_context in expected_contexts {
        let state = states_by_context.remove(&exact_context).ok_or_else(|| {
            PlannerContractError::new("states", "is missing a selected exact context")
        })?;
        let report = solve_catalog_route_book_goal(
            state,
            facts,
            mechanics,
            equivalence_sets,
            route_book,
            goal_id,
            options,
        )?;
        contexts.push(PortableContextSolveReport {
            exact_context,
            report,
        });
    }
    Ok(PortableSolveReport {
        schema: PORTABLE_SOLVE_REPORT_SCHEMA.into(),
        goal_id: goal_id.to_owned(),
        route_book_sha256: route_book.digest()?,
        status: portable_status(&contexts),
        contexts,
    })
}

pub fn solve_composed_portable_route_book_goal(
    states: Vec<PlannerExecutionState>,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: &RouteBook,
    goal_id: &str,
    options: RuntimeSolveOptions,
) -> Result<PortableSolveReport, PlannerContractError> {
    catalog.validate()?;
    route_book.validate_against_composed(catalog)?;
    let mut portable = solve_catalog_portable_route_book_goal(
        states,
        &catalog.facts,
        &catalog.mechanics,
        equivalence_sets,
        route_book,
        goal_id,
        options,
    )?;
    for context in &mut portable.contexts {
        context.report.refinement_stack_sha256 = Some(catalog.refinement_stack.digest()?);
        context.report.refinement_pack_ids = catalog
            .refinement_stack
            .entries
            .iter()
            .map(|entry| entry.pack_id.clone())
            .collect();
        context.report.refinement_stack_entries = catalog.refinement_stack.entries.clone();
    }
    Ok(portable)
}

fn expand_route_book_contexts(
    route_book: &RouteBook,
    equivalence_sets: &[EquivalenceSet],
) -> Result<BTreeSet<ExactContext>, PlannerContractError> {
    let sets = equivalence_sets
        .iter()
        .map(|set| {
            set.validate()?;
            Ok((set.id.as_str(), set))
        })
        .collect::<Result<BTreeMap<_, _>, PlannerContractError>>()?;
    let mut contexts = BTreeSet::new();
    for selector in &route_book.manifest.scope.selectors {
        match selector {
            ContextSelector::Exact { context } => {
                contexts.insert(context.clone());
            }
            ContextSelector::Equivalent { equivalence_set_id } => {
                let set = sets.get(equivalence_set_id.as_str()).ok_or_else(|| {
                    PlannerContractError::new(
                        "route_book.manifest.scope",
                        format!("references unknown equivalence set {equivalence_set_id}"),
                    )
                })?;
                contexts.extend(set.contexts.iter().cloned());
            }
        }
    }
    if contexts.is_empty() {
        return Err(PlannerContractError::new(
            "route_book.manifest.scope",
            "does not expand to any exact contexts",
        ));
    }
    Ok(contexts)
}

fn portable_status(contexts: &[PortableContextSolveReport]) -> PortableSearchStatus {
    if contexts
        .iter()
        .all(|context| context.report.result.status == SearchStatus::Reached)
    {
        PortableSearchStatus::ReachedAll
    } else if contexts
        .iter()
        .any(|context| context.report.result.status == SearchStatus::Unknown)
    {
        PortableSearchStatus::UnknownInSome
    } else {
        PortableSearchStatus::UnreachableInSome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_route_planner::identity::{
        ContextSelector, EQUIVALENCE_SET_SCHEMA, EquivalenceEvidence, EquivalenceEvidenceKind,
        RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration,
    };
    use dusklight_route_planner::logic::{
        ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA,
        FriendlyAlias, PredicateExpression, RawFactBinding, RuleEvidence, TruthStatus,
        ValueReference,
    };
    use dusklight_route_planner::refinement::{
        REFINEMENT_PACK_SCHEMA, RefinementLayer, RefinementLayers, RefinementOperation,
        RefinementPack, RefinementPackManifest, RefinementRule,
    };
    use dusklight_route_planner::route_book::{ROUTE_BOOK_SCHEMA, RouteBookManifest};
    use dusklight_route_planner::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
    use dusklight_route_planner::solver::SearchStatus;
    use dusklight_route_planner::state::{
        BackingAttachment, ComponentBinding, ComponentBindingReference, ComponentKind,
        ComponentPayload, ComponentProvenance, ComponentSelector, EXECUTION_ENVIRONMENT_SCHEMA,
        ExecutionEnvironment, PlayerForm, PlayerState, ProvenanceSourceKind, RuntimeFile,
        RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation, SemanticLifetime,
        SerializationOwner, StateComponent, StateValue,
    };
    use dusklight_route_planner::transition::{
        ActivationContract, CandidateTransition, Goal, MECHANICS_CATALOG_SCHEMA, StateOperation,
        TransitionKind,
    };
    use std::collections::BTreeMap;

    fn state_for(content_byte: u8, stage: &str) -> PlannerExecutionState {
        PlannerExecutionState::new(StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.runtime-test".into(),
            sequence: 1,
            environment: ExecutionEnvironment {
                schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
                runtime_configuration: RuntimeConfiguration {
                    schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
                    content_sha256: Digest([content_byte; 32]),
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
                execution_context: dusklight_route_planner::state::ExecutionContext::World,
                location: SceneLocation {
                    stage: stage.into(),
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
        })
        .unwrap()
    }

    fn state() -> PlannerExecutionState {
        state_for(1, "F_SP103")
    }

    fn established_evidence(id: &str) -> RuleEvidence {
        RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![EvidenceRecord {
                id: id.into(),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(Digest([8; 32])),
                note: "Synthetic local-bank regression fixture.".into(),
            }],
        }
    }

    #[test]
    fn standalone_runtime_solves_catalog_goal_without_huntctl_types() {
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
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
            techniques: Vec::new(),
            microtraces: Vec::new(),
            goals: vec![Goal {
                id: "goal.ordon-spring".into(),
                label: "Reach Ordon Spring".into(),
                predicate: PredicateExpression::Compare {
                    left: ValueReference::LocationStage,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text("F_SP103".into()),
                    },
                },
            }],
        };
        let report = solve_catalog_goal(
            state(),
            &facts,
            &mechanics,
            &[],
            "goal.ordon-spring",
            RuntimeSolveOptions::default(),
        )
        .unwrap();
        assert_eq!(report.schema, SOLVE_REPORT_SCHEMA);
        assert_eq!(report.result.status, SearchStatus::Reached);
        assert!(report.result.steps.is_empty());
        assert_eq!(report.summary.status, SearchStatus::Reached);
        assert_eq!(
            report.summary.primary_plan.as_ref().unwrap().actions,
            vec![]
        );
        assert!(report.summary.alternative_plans.is_empty());
        assert_eq!(
            report.summary.explored_states,
            report.result.explored_states
        );
        assert_ne!(report.fact_catalog_sha256, Digest::ZERO);
        assert_eq!(report.refinement_stack_sha256, None);

        let composed = ComposedPlannerCatalog::compose(&facts, &mechanics, &[]).unwrap();
        let composed_report = solve_composed_catalog_goal(
            state(),
            &composed,
            &[],
            "goal.ordon-spring",
            RuntimeSolveOptions::default(),
        )
        .unwrap();
        assert_eq!(composed_report.result.status, SearchStatus::Reached);
        assert_eq!(
            composed_report.refinement_stack_sha256,
            Some(composed.refinement_stack.digest().unwrap())
        );
        assert!(composed_report.refinement_pack_ids.is_empty());

        let route_state = state();
        let route_book = RouteBook {
            schema: ROUTE_BOOK_SCHEMA.into(),
            manifest: RouteBookManifest {
                id: "route.runtime-test".into(),
                version: "1.0.0".into(),
                label: "Runtime route test".into(),
                author: "route planner tests".into(),
                source: "in-memory fixture".into(),
                scope: ContextScope {
                    selectors: vec![ContextSelector::Exact {
                        context: route_state
                            .snapshot
                            .environment
                            .runtime_configuration
                            .exact_context()
                            .unwrap(),
                    }],
                },
                refinement_stack_sha256: Some(composed.refinement_stack.digest().unwrap()),
            },
            goal_ids: vec!["goal.ordon-spring".into()],
            constraints: Vec::new(),
            directives: Vec::new(),
            steps: Vec::new(),
            methods: Vec::new(),
            regions: Vec::new(),
            annotations: Vec::new(),
        };
        let route_report = solve_composed_route_book_goal(
            route_state,
            &composed,
            &[],
            &route_book,
            "goal.ordon-spring",
            RuntimeSolveOptions::default(),
        )
        .unwrap();
        assert_eq!(
            route_report.route_book_sha256,
            Some(route_book.digest().unwrap())
        );
    }

    #[test]
    fn portable_solve_expands_scope_and_solves_every_exact_context_independently() {
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
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
            techniques: Vec::new(),
            microtraces: Vec::new(),
            goals: vec![Goal {
                id: "goal.ordon-spring".into(),
                label: "Reach Ordon Spring".into(),
                predicate: PredicateExpression::Compare {
                    left: ValueReference::LocationStage,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text("F_SP103".into()),
                    },
                },
            }],
        };
        let reached_state = state_for(1, "F_SP103");
        let unreachable_state = state_for(2, "F_SP00");
        let contexts = vec![
            reached_state
                .snapshot
                .environment
                .runtime_configuration
                .exact_context()
                .unwrap(),
            unreachable_state
                .snapshot
                .environment
                .runtime_configuration
                .exact_context()
                .unwrap(),
        ];
        let equivalence_set = EquivalenceSet {
            schema: EQUIVALENCE_SET_SCHEMA.into(),
            id: "equivalence.portable-test".into(),
            semantic_scope: "route.test".into(),
            contexts: contexts.clone(),
            evidence: vec![EquivalenceEvidence {
                kind: EquivalenceEvidenceKind::CommunityVerification,
                source_id: "fixture.portable-test".into(),
                source_sha256: Digest([9; 32]),
            }],
        };
        equivalence_set.validate().unwrap();
        let route_book = RouteBook {
            schema: ROUTE_BOOK_SCHEMA.into(),
            manifest: RouteBookManifest {
                id: "route.portable-test".into(),
                version: "1.0.0".into(),
                label: "Portable route test".into(),
                author: "route planner tests".into(),
                source: "in-memory fixture".into(),
                scope: ContextScope {
                    selectors: vec![ContextSelector::Equivalent {
                        equivalence_set_id: equivalence_set.id.clone(),
                    }],
                },
                refinement_stack_sha256: None,
            },
            goal_ids: vec!["goal.ordon-spring".into()],
            constraints: Vec::new(),
            directives: Vec::new(),
            steps: Vec::new(),
            methods: Vec::new(),
            regions: Vec::new(),
            annotations: Vec::new(),
        };

        let report = solve_catalog_portable_route_book_goal(
            vec![unreachable_state, reached_state],
            &facts,
            &mechanics,
            std::slice::from_ref(&equivalence_set),
            &route_book,
            "goal.ordon-spring",
            RuntimeSolveOptions::default(),
        )
        .unwrap();
        assert_eq!(report.schema, PORTABLE_SOLVE_REPORT_SCHEMA);
        assert_eq!(report.status, PortableSearchStatus::UnreachableInSome);
        assert_eq!(report.contexts.len(), 2);
        assert_eq!(report.contexts[0].exact_context, contexts[0]);
        assert_eq!(
            report.contexts[0].report.result.status,
            SearchStatus::Reached
        );
        assert_eq!(
            report.contexts[1].report.result.status,
            SearchStatus::UnreachableUnderModel
        );

        let error = solve_catalog_portable_route_book_goal(
            vec![state_for(1, "F_SP103")],
            &facts,
            &mechanics,
            &[equivalence_set],
            &route_book,
            "goal.ordon-spring",
            RuntimeSolveOptions::default(),
        )
        .unwrap_err();
        assert_eq!(error.field(), "states");
    }

    #[test]
    fn hypothetical_local_bank_rebind_is_typed_binding_sensitive_and_removable() {
        let mut start = state_for(1, "ForestTemple");
        start.snapshot.environment.components.push(StateComponent {
            id: "local.stage-bank".into(),
            component_kind: ComponentKind::StageMemory,
            payload: ComponentPayload::Raw {
                bytes: vec![0b0000_0100],
                known_mask: vec![0xff],
            },
            binding: ComponentBinding::Stage {
                stage: "ForestTemple".into(),
            },
            lifetime: SemanticLifetime::StageLoad,
            serialization_owner: SerializationOwner::StageBank {
                runtime_file_id: "file-0".into(),
                stage: "ForestTemple".into(),
            },
            provenance: vec![ComponentProvenance {
                source_kind: ProvenanceSourceKind::ExtractedFact,
                source_id: "fixture.forest-local-bank".into(),
                source_sha256: Some(Digest([7; 32])),
                transition_id: None,
            }],
        });
        start.validate().unwrap();
        let exact_scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: start
                    .snapshot
                    .environment
                    .runtime_configuration
                    .exact_context()
                    .unwrap(),
            }],
        };
        let alias = |id: &str, label: &str, stage: &str| FriendlyAlias {
            id: id.into(),
            label: label.into(),
            scope: exact_scope.clone(),
            raw: RawFactBinding {
                component_kind: ComponentKind::StageMemory,
                binding: ComponentBindingReference::Exact {
                    binding: ComponentBinding::Stage {
                        stage: stage.into(),
                    },
                },
                byte_offset: 0,
                mask: vec![0b0000_0100],
                expected: vec![0b0000_0100],
            },
            evidence: established_evidence(&format!("evidence.{id}")),
        };
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: vec![
                alias(
                    "fact.forest.local-switch",
                    "Forest Temple local switch",
                    "ForestTemple",
                ),
                alias(
                    "fact.temple.local-switch",
                    "Temple of Time local switch",
                    "TempleOfTime",
                ),
            ],
            derived_facts: Vec::new(),
        };
        let mechanics = MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions: vec![CandidateTransition {
                id: "transition.consume-temple-switch".into(),
                label: "Consume the rebound Temple of Time interpretation".into(),
                scope: exact_scope.clone(),
                transition_kind: TransitionKind::Other,
                approach_id: "approach.temple-switch".into(),
                activation: ActivationContract {
                    hard_guards: PredicateExpression::Fact {
                        fact_id: "fact.temple.local-switch".into(),
                    },
                    physical_obligation_ids: Vec::new(),
                    effects: vec![StateOperation::SetLocation {
                        location: SceneLocation {
                            stage: "RebindGoal".into(),
                            room: 0,
                            layer: 0,
                            spawn: 0,
                        },
                    }],
                    unknown_requirements: Vec::new(),
                },
                evidence: established_evidence("evidence.transition.temple-switch"),
            }],
            obligations: Vec::new(),
            writers: Vec::new(),
            gates: Vec::new(),
            readers: Vec::new(),
            reconstruction_rules: Vec::new(),
            obstructions: Vec::new(),
            resolvers: Vec::new(),
            techniques: Vec::new(),
            microtraces: Vec::new(),
            goals: vec![Goal {
                id: "goal.rebind-downstream".into(),
                label: "Reach a state gated by the rebound local switch".into(),
                predicate: PredicateExpression::Compare {
                    left: ValueReference::LocationStage,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text("RebindGoal".into()),
                    },
                },
            }],
        };
        let overlay = RefinementPack {
            schema: REFINEMENT_PACK_SCHEMA.into(),
            manifest: RefinementPackManifest {
                id: "what-if.local-bank-rebind".into(),
                version: "1.0.0".into(),
                author: "Route planner regression fixture".into(),
                source: "Explicit local what-if overlay".into(),
                scope: exact_scope,
                precedence: 1_000,
                dependencies: Vec::new(),
                conflicts: Vec::new(),
            },
            rules: vec![RefinementRule {
                id: "technique.preserve-and-rebind-local-bank".into(),
                label: "Hypothetically preserve and rebind the stage-local bank".into(),
                operation: RefinementOperation::ComponentTransform {
                    prerequisite: PredicateExpression::Compare {
                        left: ValueReference::LocationStage,
                        operator: ComparisonOperator::Equal,
                        right: ValueReference::Literal {
                            value: StateValue::Text("ForestTemple".into()),
                        },
                    },
                    operations: vec![
                        StateOperation::Preserve {
                            selector: ComponentSelector::Id {
                                component_id: "local.stage-bank".into(),
                            },
                        },
                        StateOperation::Rebind {
                            selector: ComponentSelector::Id {
                                component_id: "local.stage-bank".into(),
                            },
                            binding: ComponentBinding::Stage {
                                stage: "TempleOfTime".into(),
                            },
                        },
                    ],
                },
                evidence: RuleEvidence {
                    truth: TruthStatus::Hypothetical,
                    records: vec![EvidenceRecord {
                        id: "evidence.what-if.local-bank-rebind".into(),
                        kind: EvidenceKind::Theorycraft,
                        source_sha256: None,
                        note: "No transfer is claimed; this pack asks what follows if one exists."
                            .into(),
                    }],
                },
            }],
        };

        let base = ComposedPlannerCatalog::compose(&facts, &mechanics, &[]).unwrap();
        let baseline = solve_composed_catalog_goal(
            start.clone(),
            &base,
            &[],
            "goal.rebind-downstream",
            RuntimeSolveOptions {
                evidence_mode: RuntimeEvidenceMode::Research,
                ..RuntimeSolveOptions::default()
            },
        )
        .unwrap();
        assert_ne!(baseline.result.status, SearchStatus::Reached);

        let composed = ComposedPlannerCatalog::compose_layered(
            &facts,
            &mechanics,
            &RefinementLayers {
                enabled_packs: Vec::new(),
                route_local_overlays: Vec::new(),
                ephemeral_what_if_overlays: vec![overlay],
            },
        )
        .unwrap();
        let established_only = solve_composed_catalog_goal(
            start.clone(),
            &composed,
            &[],
            "goal.rebind-downstream",
            RuntimeSolveOptions::default(),
        )
        .unwrap();
        assert_ne!(established_only.result.status, SearchStatus::Reached);
        let research = solve_composed_catalog_goal(
            start.clone(),
            &composed,
            &[],
            "goal.rebind-downstream",
            RuntimeSolveOptions {
                evidence_mode: RuntimeEvidenceMode::Research,
                ..RuntimeSolveOptions::default()
            },
        )
        .unwrap();
        assert_eq!(research.result.status, SearchStatus::Reached);
        assert_eq!(
            research.refinement_pack_ids,
            vec!["what-if.local-bank-rebind"]
        );
        assert_eq!(research.refinement_stack_entries.len(), 1);
        assert_eq!(
            research.refinement_stack_entries[0].layer,
            RefinementLayer::EphemeralWhatIf
        );
        assert_eq!(
            research
                .result
                .steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "technique.preserve-and-rebind-local-bank",
                "transition.consume-temple-switch"
            ]
        );

        let transform = &composed.mechanics.techniques[0];
        assert_eq!(transform.evidence.truth, TruthStatus::Hypothetical);
        let mut rebound = start.clone();
        rebound
            .apply_operations(
                &transform.id,
                "snapshot.local-bank-rebound",
                &transform.operations,
            )
            .unwrap();
        let before_component = &start.snapshot.environment.components[0];
        let after_component = &rebound.snapshot.environment.components[0];
        assert_eq!(before_component.payload, after_component.payload);
        assert_eq!(
            after_component.binding,
            ComponentBinding::Stage {
                stage: "TempleOfTime".into()
            }
        );
        assert_eq!(
            after_component.provenance[0],
            before_component.provenance[0]
        );
        assert_eq!(after_component.provenance.len(), 2);
        assert_eq!(
            after_component.provenance[1].transition_id.as_deref(),
            Some("technique.preserve-and-rebind-local-bank")
        );

        let removed = ComposedPlannerCatalog::compose(&facts, &mechanics, &[]).unwrap();
        assert_eq!(removed, base);
        let after_removal = solve_composed_catalog_goal(
            start,
            &removed,
            &[],
            "goal.rebind-downstream",
            RuntimeSolveOptions {
                evidence_mode: RuntimeEvidenceMode::Research,
                ..RuntimeSolveOptions::default()
            },
        )
        .unwrap();
        assert_eq!(after_removal.result.status, baseline.result.status);
        assert_eq!(after_removal.result.steps, baseline.result.steps);
    }
}
