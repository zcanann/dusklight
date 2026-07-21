//! Independent application boundary for the causal route planner.
//!
//! This crate intentionally does not depend on Huntctl's CLI, TAS timeline
//! workbench, playback graph, or browser protocol. It consumes the planner
//! engine's canonical artifacts and returns planner-specific reports.

pub mod service;

use dusklight_route_planner::PlannerContractError;
use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::evaluation::{EvidencePolicy, FeasibilityMode};
use dusklight_route_planner::execution::PlannerExecutionState;
use dusklight_route_planner::identity::EquivalenceSet;
use dusklight_route_planner::logic::FactCatalog;
use dusklight_route_planner::refinement::ComposedPlannerCatalog;
use dusklight_route_planner::solver::{ForwardSolver, SearchResult, SolverOptions};
use dusklight_route_planner::transition::MechanicsCatalog;
use serde::{Deserialize, Serialize};

pub const SOLVE_REPORT_SCHEMA: &str = "dusklight.route-planner.solve-report/v1";

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
    pub feasibility_mode: RuntimeFeasibilityMode,
    pub evidence_mode: RuntimeEvidenceMode,
}

impl Default for RuntimeSolveOptions {
    fn default() -> Self {
        let solver = SolverOptions::default();
        Self {
            max_depth: solver.max_depth,
            max_states: solver.max_states,
            max_resolution_combinations: solver.max_resolution_combinations,
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
    pub refinement_pack_ids: Vec<String>,
    pub equivalence_set_count: usize,
    pub feasibility_mode: RuntimeFeasibilityMode,
    pub evidence_mode: RuntimeEvidenceMode,
    pub result: SearchResult,
}

pub fn solve_catalog_goal(
    state: PlannerExecutionState,
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
    equivalence_sets: &[EquivalenceSet],
    goal_id: &str,
    options: RuntimeSolveOptions,
) -> Result<SolveReport, PlannerContractError> {
    state.validate()?;
    facts.validate()?;
    mechanics.validate()?;
    let goal = mechanics
        .goals
        .iter()
        .find(|goal| goal.id == goal_id)
        .ok_or_else(|| PlannerContractError::new("goal_id", "is absent from the catalog"))?;
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
    let result = ForwardSolver::new(facts, mechanics, equivalence_sets, solver_options)?
        .solve(state, &goal.predicate)?;
    Ok(SolveReport {
        schema: SOLVE_REPORT_SCHEMA.into(),
        goal_id: goal.id.clone(),
        execution_state_sha256,
        fact_catalog_sha256: facts.digest()?,
        mechanics_catalog_sha256: mechanics.digest()?,
        refinement_stack_sha256: None,
        refinement_pack_ids: Vec::new(),
        equivalence_set_count: equivalence_sets.len(),
        feasibility_mode: options.feasibility_mode,
        evidence_mode: options.evidence_mode,
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
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_route_planner::identity::{RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration};
    use dusklight_route_planner::logic::{
        ComparisonOperator, FACT_CATALOG_SCHEMA, PredicateExpression, ValueReference,
    };
    use dusklight_route_planner::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
    use dusklight_route_planner::solver::SearchStatus;
    use dusklight_route_planner::state::{
        BackingAttachment, EXECUTION_ENVIRONMENT_SCHEMA, ExecutionEnvironment, PlayerForm,
        PlayerState, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation,
        StateValue,
    };
    use dusklight_route_planner::transition::{Goal, MECHANICS_CATALOG_SCHEMA};
    use std::collections::BTreeMap;

    fn state() -> PlannerExecutionState {
        PlannerExecutionState::new(StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.runtime-test".into(),
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
                    stage: "F_SP103".into(),
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
        })
        .unwrap()
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
    }
}
