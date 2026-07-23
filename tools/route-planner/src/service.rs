//! Typed request/response boundary for planner-owned editor and automation clients.

use crate::inspection::{StateInspection, StateInspectionDiff, inspect_state, inspect_state_diff};
use crate::{
    PortableSolveReport, RuntimeSolveOptions, SolveReport, SuspiciousStateQueryReport,
    query_composed_suspicious_state, solve_composed_catalog_goal,
    solve_composed_portable_route_book_goal, solve_composed_route_book_goal,
};
use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::evaluation::EvidencePolicy;
use dusklight_route_planner::evaluation::{
    FeasibilityMode, FeasibilitySelection, PredicateEvaluator, TransitionAssessment,
    TransitionClassification,
};
use dusklight_route_planner::execution::{PlannerExecutionState, PlannerExecutionStateDocument};
use dusklight_route_planner::graph::{
    PlannerExecutionPathState, PlannerFeasibilityGraphDiff, PlannerGraph,
};
use dusklight_route_planner::identity::EquivalenceSet;
use dusklight_route_planner::logic::{FactCatalog, PredicateExpression};
use dusklight_route_planner::refinement::{
    ComposedPlannerCatalog, RefinementLayers, RefinementPack,
};
use dusklight_route_planner::route_book::{
    CollapsePolicy, PlanMethod, PlanRegion, ROUTE_BOOK_EDIT_BATCH_SCHEMA, ROUTE_BOOK_SCHEMA,
    ReferenceStep, RouteActionRef, RouteBook, RouteBookEdit, RouteBookEditBatch, RouteBookManifest,
};
use dusklight_route_planner::state::BoundaryKind;
use dusklight_route_planner::transition::MechanicsCatalog;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};

pub const PLANNER_SERVICE_SCHEMA: &str = "dusklight.route-planner.service/v42";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerServiceEnvelope {
    pub schema: String,
    pub request: PlannerServiceRequest,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlannerServiceRequest {
    ValidateRefinementPack {
        request_id: String,
        pack: Box<RefinementPack>,
    },
    ValidateRouteBook {
        request_id: String,
        book: Box<RouteBook>,
        catalog: Box<ComposedPlannerCatalog>,
    },
    EditRouteBook {
        request_id: String,
        book: Box<RouteBook>,
        catalog: Box<ComposedPlannerCatalog>,
        edit_batch: Box<RouteBookEditBatch>,
    },
    Compose {
        request_id: String,
        facts: Box<FactCatalog>,
        mechanics: Box<MechanicsCatalog>,
        packs: Vec<RefinementPack>,
        #[serde(default)]
        route_local_overlays: Vec<RefinementPack>,
        #[serde(default)]
        ephemeral_what_if_overlays: Vec<RefinementPack>,
    },
    ProjectGraph {
        request_id: String,
        catalog: Box<ComposedPlannerCatalog>,
        route_book: Option<Box<RouteBook>>,
    },
    ProjectFeasibilityDiff {
        request_id: String,
        state: Box<PlannerExecutionStateDocument>,
        catalog: Box<ComposedPlannerCatalog>,
        equivalence_sets: Vec<EquivalenceSet>,
        evidence_mode: crate::RuntimeEvidenceMode,
    },
    InspectRouteFrontier {
        request_id: String,
        state: Box<PlannerExecutionStateDocument>,
        catalog: Box<ComposedPlannerCatalog>,
        equivalence_sets: Vec<EquivalenceSet>,
        route_book: Option<Box<RouteBook>>,
        evidence_mode: crate::RuntimeEvidenceMode,
    },
    InspectState {
        request_id: String,
        state: Box<PlannerExecutionStateDocument>,
        catalog: Box<ComposedPlannerCatalog>,
        equivalence_sets: Vec<EquivalenceSet>,
        evidence_mode: crate::RuntimeEvidenceMode,
    },
    DiffState {
        request_id: String,
        before: Box<PlannerExecutionStateDocument>,
        after: Box<PlannerExecutionStateDocument>,
        boundary: BoundaryKind,
        catalog: Box<ComposedPlannerCatalog>,
        equivalence_sets: Vec<EquivalenceSet>,
        evidence_mode: crate::RuntimeEvidenceMode,
    },
    EvaluateTransition {
        request_id: String,
        state: Box<PlannerExecutionStateDocument>,
        catalog: Box<ComposedPlannerCatalog>,
        equivalence_sets: Vec<EquivalenceSet>,
        transition_id: String,
        evidence_mode: crate::RuntimeEvidenceMode,
    },
    SuggestTransitionChain {
        request_id: String,
        state: Box<PlannerExecutionStateDocument>,
        catalog: Box<ComposedPlannerCatalog>,
        equivalence_sets: Vec<EquivalenceSet>,
        route_book: Option<Box<RouteBook>>,
        transition_id: String,
        evidence_mode: crate::RuntimeEvidenceMode,
        max_depth: usize,
        max_states: usize,
    },
    AppendTransition {
        request_id: String,
        state: Box<PlannerExecutionStateDocument>,
        catalog: Box<ComposedPlannerCatalog>,
        equivalence_sets: Vec<EquivalenceSet>,
        route_book: Option<Box<RouteBook>>,
        route_book_id: String,
        route_book_label: String,
        transition_id: String,
        evidence_mode: crate::RuntimeEvidenceMode,
    },
    RemoveAuthoredStep {
        request_id: String,
        state: Box<PlannerExecutionStateDocument>,
        catalog: Box<ComposedPlannerCatalog>,
        equivalence_sets: Vec<EquivalenceSet>,
        route_book: Box<RouteBook>,
        step_id: String,
        evidence_mode: crate::RuntimeEvidenceMode,
    },
    ReplaceAuthoredStep {
        request_id: String,
        state: Box<PlannerExecutionStateDocument>,
        catalog: Box<ComposedPlannerCatalog>,
        equivalence_sets: Vec<EquivalenceSet>,
        route_book: Box<RouteBook>,
        step_id: String,
        transition_id: String,
        evidence_mode: crate::RuntimeEvidenceMode,
    },
    InspectAuthoredRoute {
        request_id: String,
        state: Box<PlannerExecutionStateDocument>,
        catalog: Box<ComposedPlannerCatalog>,
        equivalence_sets: Vec<EquivalenceSet>,
        route_book: Box<RouteBook>,
        evidence_mode: crate::RuntimeEvidenceMode,
    },
    Solve {
        request_id: String,
        state: Box<PlannerExecutionStateDocument>,
        catalog: Box<ComposedPlannerCatalog>,
        equivalence_sets: Vec<EquivalenceSet>,
        goal_id: String,
        options: RuntimeSolveOptions,
        #[serde(default)]
        route_book: Option<Box<RouteBook>>,
    },
    SolvePortable {
        request_id: String,
        states: Vec<PlannerExecutionStateDocument>,
        catalog: Box<ComposedPlannerCatalog>,
        equivalence_sets: Vec<EquivalenceSet>,
        route_book: Box<RouteBook>,
        goal_id: String,
        options: RuntimeSolveOptions,
    },
    QuerySuspiciousState {
        request_id: String,
        state: Box<PlannerExecutionStateDocument>,
        catalog: Box<ComposedPlannerCatalog>,
        equivalence_sets: Vec<EquivalenceSet>,
        predicate: PredicateExpression,
        options: RuntimeSolveOptions,
    },
}

impl PlannerServiceRequest {
    pub fn request_id(&self) -> &str {
        match self {
            Self::ValidateRefinementPack { request_id, .. }
            | Self::ValidateRouteBook { request_id, .. }
            | Self::EditRouteBook { request_id, .. }
            | Self::Compose { request_id, .. }
            | Self::ProjectGraph { request_id, .. }
            | Self::ProjectFeasibilityDiff { request_id, .. }
            | Self::InspectRouteFrontier { request_id, .. }
            | Self::InspectState { request_id, .. }
            | Self::DiffState { request_id, .. }
            | Self::EvaluateTransition { request_id, .. }
            | Self::SuggestTransitionChain { request_id, .. }
            | Self::AppendTransition { request_id, .. }
            | Self::RemoveAuthoredStep { request_id, .. }
            | Self::ReplaceAuthoredStep { request_id, .. }
            | Self::InspectAuthoredRoute { request_id, .. }
            | Self::Solve { request_id, .. }
            | Self::SolvePortable { request_id, .. }
            | Self::QuerySuspiciousState { request_id, .. } => request_id,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerServiceResponse {
    pub schema: String,
    pub request_id: Option<String>,
    pub outcome: PlannerServiceOutcome,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlannerServiceOutcome {
    Ok { payload: Box<PlannerServicePayload> },
    Error { field: String, detail: String },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlannerServicePayload {
    RefinementPackValid {
        pack_id: String,
        pack_sha256: Digest,
    },
    RouteBookValid {
        route_book_id: String,
        route_book_sha256: Digest,
    },
    EditedRouteBook {
        book: Box<RouteBook>,
        previous_route_book_sha256: Digest,
        route_book_sha256: Digest,
    },
    ComposedCatalog {
        catalog: Box<ComposedPlannerCatalog>,
        catalog_sha256: Digest,
    },
    Graph {
        graph: Box<PlannerGraph>,
        graph_sha256: Digest,
    },
    FeasibilityGraphDiff {
        diff: Box<PlannerFeasibilityGraphDiff>,
        diff_sha256: Digest,
    },
    RouteFrontier {
        graph: Box<PlannerGraph>,
        graph_sha256: Digest,
        frontier_state: Box<PlannerExecutionStateDocument>,
        frontier: Box<StateInspection>,
        execution_states: Vec<StateInspection>,
        transitions: Vec<RouteFrontierTransition>,
    },
    StateInspection {
        inspection: Box<StateInspection>,
    },
    StateInspectionDiff {
        inspection_diff: Box<StateInspectionDiff>,
    },
    TransitionEvaluation {
        assessment: Box<TransitionAssessment>,
        diagnostics: Box<TransitionJoinDiagnostics>,
        after: Option<Box<PlannerExecutionStateDocument>>,
    },
    TransitionChainSuggestion {
        target_transition_id: String,
        transition_ids: Vec<String>,
        explored_states: usize,
        hit_search_limit: bool,
        assessment: Box<TransitionAssessment>,
        diagnostics: Box<TransitionJoinDiagnostics>,
        after: Option<Box<PlannerExecutionStateDocument>>,
    },
    RejectedTransitionJoin {
        assessment: Box<TransitionAssessment>,
        diagnostics: Box<TransitionJoinDiagnostics>,
        closest_before: Box<PlannerExecutionStateDocument>,
    },
    RemovedAuthoredStep {
        book: Option<Box<RouteBook>>,
        previous_route_book_sha256: Digest,
        route_book_sha256: Option<Digest>,
        step_id: String,
        after: Box<PlannerExecutionStateDocument>,
    },
    ReplacedAuthoredStep {
        book: Box<RouteBook>,
        previous_route_book_sha256: Digest,
        route_book_sha256: Digest,
        step_id: String,
        transition_id: String,
        assessment: Box<TransitionAssessment>,
        after: Box<PlannerExecutionStateDocument>,
    },
    RejectedRouteEdit {
        step_id: String,
        failed_step_id: String,
        assessment: Box<TransitionAssessment>,
        diagnostics: Box<TransitionJoinDiagnostics>,
        closest_before: Box<PlannerExecutionStateDocument>,
    },
    AuthoredRouteInspection {
        inspection: Box<AuthoredRouteInspection>,
    },
    AppendedTransition {
        book: Box<RouteBook>,
        previous_route_book_sha256: Option<Digest>,
        route_book_sha256: Digest,
        step_id: String,
        assessment: Box<TransitionAssessment>,
        after: Box<PlannerExecutionStateDocument>,
    },
    SolveReport {
        report: Box<SolveReport>,
    },
    PortableSolveReport {
        report: Box<PortableSolveReport>,
    },
    SuspiciousStateQueryReport {
        report: Box<SuspiciousStateQueryReport>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionJoinDiagnostics {
    pub active_obstruction_ids: Vec<String>,
    pub unknown_obstruction_ids: Vec<String>,
    pub applied_resolver_ids: Vec<String>,
    pub applicable_technique_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteFrontierTransition {
    pub transition_id: String,
    pub assessment: TransitionAssessment,
    pub diagnostics: TransitionJoinDiagnostics,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredRouteInspection {
    pub steps: Vec<AuthoredRouteStepInspection>,
    pub rejection: Option<AuthoredRouteRejectionInspection>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredRouteStepInspection {
    pub step_id: String,
    pub transition_id: String,
    pub assessment: TransitionAssessment,
    pub state_change: AuthoredRouteStateChange,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredRouteRejectionInspection {
    pub failed_step_id: String,
    pub transition_id: String,
    pub assessment: TransitionAssessment,
    pub diagnostics: TransitionJoinDiagnostics,
    pub prefix_state_change: AuthoredRouteStateChange,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredRouteStateChange {
    pub before: StateInspection,
    pub after: StateInspection,
    pub diff: StateInspectionDiff,
}

pub fn handle_request(request: PlannerServiceRequest) -> PlannerServiceResponse {
    let request_id = request.request_id().to_owned();
    if let Err(detail) = validate_request_id(&request_id) {
        return error_response(Some(request_id), "request_id", detail);
    }
    let result = match request {
        PlannerServiceRequest::ValidateRefinementPack { pack, .. } => {
            pack.digest()
                .map(|pack_sha256| PlannerServicePayload::RefinementPackValid {
                    pack_id: pack.manifest.id.clone(),
                    pack_sha256,
                })
        }
        PlannerServiceRequest::ValidateRouteBook { book, catalog, .. } => {
            book.validate_against_composed(&catalog).and_then(|()| {
                Ok(PlannerServicePayload::RouteBookValid {
                    route_book_id: book.manifest.id.clone(),
                    route_book_sha256: book.digest()?,
                })
            })
        }
        PlannerServiceRequest::EditRouteBook {
            book,
            catalog,
            edit_batch,
            ..
        } => book.digest().and_then(|previous_route_book_sha256| {
            edit_batch
                .apply_composed(&book, &catalog)
                .and_then(|edited| {
                    let route_book_sha256 = edited.digest()?;
                    Ok(PlannerServicePayload::EditedRouteBook {
                        book: Box::new(edited),
                        previous_route_book_sha256,
                        route_book_sha256,
                    })
                })
        }),
        PlannerServiceRequest::Compose {
            facts,
            mechanics,
            packs,
            route_local_overlays,
            ephemeral_what_if_overlays,
            ..
        } => ComposedPlannerCatalog::compose_layered(
            &facts,
            &mechanics,
            &RefinementLayers {
                enabled_packs: packs,
                route_local_overlays,
                ephemeral_what_if_overlays,
            },
        )
        .and_then(|catalog| {
            let catalog_sha256 = catalog.digest()?;
            Ok(PlannerServicePayload::ComposedCatalog {
                catalog: Box::new(catalog),
                catalog_sha256,
            })
        }),
        PlannerServiceRequest::ProjectGraph {
            catalog,
            route_book,
            ..
        } => {
            let graph = if let Some(book) = route_book {
                PlannerGraph::project_composed_with_route_book(&catalog, &book)
            } else {
                PlannerGraph::project_composed(&catalog)
            };
            graph.and_then(|graph| {
                let graph_sha256 = graph.digest()?;
                Ok(PlannerServicePayload::Graph {
                    graph: Box::new(graph),
                    graph_sha256,
                })
            })
        }
        PlannerServiceRequest::ProjectFeasibilityDiff {
            state,
            catalog,
            equivalence_sets,
            evidence_mode,
            ..
        } => (*state).into_state().and_then(|state| {
            let policy = match evidence_mode {
                crate::RuntimeEvidenceMode::EstablishedOnly => EvidencePolicy::ESTABLISHED_ONLY,
                crate::RuntimeEvidenceMode::Research => EvidencePolicy::RESEARCH,
            };
            PlannerFeasibilityGraphDiff::project_composed(
                &state,
                &catalog,
                &equivalence_sets,
                policy,
            )
            .and_then(|diff| {
                let diff_sha256 = diff.digest()?;
                Ok(PlannerServicePayload::FeasibilityGraphDiff {
                    diff: Box::new(diff),
                    diff_sha256,
                })
            })
        }),
        PlannerServiceRequest::InspectRouteFrontier {
            state,
            catalog,
            equivalence_sets,
            route_book,
            evidence_mode,
            ..
        } => (*state).into_state().and_then(|state| {
            inspect_route_frontier(
                state,
                &catalog,
                &equivalence_sets,
                route_book.map(|book| *book),
                evidence_mode,
            )
        }),
        PlannerServiceRequest::InspectState {
            state,
            catalog,
            equivalence_sets,
            evidence_mode,
            ..
        } => (*state).into_state().and_then(|state| {
            inspect_state(&state, &catalog.facts, &equivalence_sets, evidence_mode).map(
                |inspection| PlannerServicePayload::StateInspection {
                    inspection: Box::new(inspection),
                },
            )
        }),
        PlannerServiceRequest::DiffState {
            before,
            after,
            boundary,
            catalog,
            equivalence_sets,
            evidence_mode,
            ..
        } => (*before).into_state().and_then(|before| {
            (*after).into_state().and_then(|after| {
                inspect_state_diff(
                    &before,
                    &after,
                    boundary,
                    &catalog.facts,
                    &equivalence_sets,
                    evidence_mode,
                )
                .map(|inspection_diff| {
                    PlannerServicePayload::StateInspectionDiff {
                        inspection_diff: Box::new(inspection_diff),
                    }
                })
            })
        }),
        PlannerServiceRequest::EvaluateTransition {
            state,
            catalog,
            equivalence_sets,
            transition_id,
            evidence_mode,
            ..
        } => (*state).into_state().and_then(|mut state| {
            let evaluation = assess_and_apply_transition(
                &mut state,
                &catalog,
                &equivalence_sets,
                &transition_id,
                evidence_mode,
                "web.transition",
            )?;
            let after =
                if evaluation.assessment.classification == TransitionClassification::Executable {
                    Some(Box::new(state.to_document()?))
                } else {
                    None
                };
            Ok(PlannerServicePayload::TransitionEvaluation {
                assessment: Box::new(evaluation.assessment),
                diagnostics: Box::new(evaluation.diagnostics),
                after,
            })
        }),
        PlannerServiceRequest::SuggestTransitionChain {
            state,
            catalog,
            equivalence_sets,
            route_book,
            transition_id,
            evidence_mode,
            max_depth,
            max_states,
            ..
        } => (*state).into_state().and_then(|state| {
            suggest_transition_chain(
                state,
                &catalog,
                &equivalence_sets,
                route_book.map(|book| *book),
                &transition_id,
                evidence_mode,
                max_depth,
                max_states,
            )
        }),
        PlannerServiceRequest::AppendTransition {
            state,
            catalog,
            equivalence_sets,
            route_book,
            route_book_id,
            route_book_label,
            transition_id,
            evidence_mode,
            ..
        } => (*state).into_state().and_then(|state| {
            append_transition_to_route_book(
                state,
                &catalog,
                &equivalence_sets,
                route_book.map(|book| *book),
                route_book_id,
                route_book_label,
                &transition_id,
                evidence_mode,
            )
        }),
        PlannerServiceRequest::RemoveAuthoredStep {
            state,
            catalog,
            equivalence_sets,
            route_book,
            step_id,
            evidence_mode,
            ..
        } => (*state).into_state().and_then(|state| {
            remove_authored_step_from_route_book(
                state,
                &catalog,
                &equivalence_sets,
                *route_book,
                &step_id,
                evidence_mode,
            )
        }),
        PlannerServiceRequest::ReplaceAuthoredStep {
            state,
            catalog,
            equivalence_sets,
            route_book,
            step_id,
            transition_id,
            evidence_mode,
            ..
        } => (*state).into_state().and_then(|state| {
            replace_authored_step_in_route_book(
                state,
                &catalog,
                &equivalence_sets,
                *route_book,
                &step_id,
                &transition_id,
                evidence_mode,
            )
        }),
        PlannerServiceRequest::InspectAuthoredRoute {
            state,
            catalog,
            equivalence_sets,
            route_book,
            evidence_mode,
            ..
        } => (*state).into_state().and_then(|state| {
            inspect_authored_route(
                state,
                &catalog,
                &equivalence_sets,
                *route_book,
                evidence_mode,
            )
            .map(
                |inspection| PlannerServicePayload::AuthoredRouteInspection {
                    inspection: Box::new(inspection),
                },
            )
        }),
        PlannerServiceRequest::Solve {
            state,
            catalog,
            equivalence_sets,
            goal_id,
            options,
            route_book,
            ..
        } => (*state).into_state().and_then(|state| {
            let report = match route_book {
                Some(book) => solve_composed_route_book_goal(
                    state,
                    &catalog,
                    &equivalence_sets,
                    &book,
                    &goal_id,
                    options,
                ),
                None => solve_composed_catalog_goal(
                    state,
                    &catalog,
                    &equivalence_sets,
                    &goal_id,
                    options,
                ),
            }?;
            Ok(PlannerServicePayload::SolveReport {
                report: Box::new(report),
            })
        }),
        PlannerServiceRequest::SolvePortable {
            states,
            catalog,
            equivalence_sets,
            route_book,
            goal_id,
            options,
            ..
        } => states
            .into_iter()
            .map(PlannerExecutionStateDocument::into_state)
            .collect::<Result<Vec<_>, _>>()
            .and_then(|states| {
                solve_composed_portable_route_book_goal(
                    states,
                    &catalog,
                    &equivalence_sets,
                    &route_book,
                    &goal_id,
                    options,
                )
            })
            .map(|report| PlannerServicePayload::PortableSolveReport {
                report: Box::new(report),
            }),
        PlannerServiceRequest::QuerySuspiciousState {
            state,
            catalog,
            equivalence_sets,
            predicate,
            options,
            ..
        } => (*state).into_state().and_then(|state| {
            query_composed_suspicious_state(state, &catalog, &equivalence_sets, predicate, options)
                .map(|report| PlannerServicePayload::SuspiciousStateQueryReport {
                    report: Box::new(report),
                })
        }),
    };
    match result {
        Ok(payload) => success_response(Some(request_id), payload),
        Err(error) => error_response(
            Some(request_id),
            error.field().to_owned(),
            error.detail().to_owned(),
        ),
    }
}

const AUTHORED_REGION_ID: &str = "region.authored-route";
const AUTHORED_METHOD_ID: &str = "method.authored-route";

fn assess_and_apply_transition(
    state: &mut PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    transition_id: &str,
    evidence_mode: crate::RuntimeEvidenceMode,
    application_id: &str,
) -> Result<TransitionEvaluationResult, dusklight_route_planner::PlannerContractError> {
    let transition = catalog
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.id == transition_id)
        .ok_or_else(|| {
            dusklight_route_planner::PlannerContractError::new(
                "transition_id",
                "does not name a transition in the composed catalog",
            )
        })?;
    let policy = match evidence_mode {
        crate::RuntimeEvidenceMode::EstablishedOnly => EvidencePolicy::ESTABLISHED_ONLY,
        crate::RuntimeEvidenceMode::Research => EvidencePolicy::RESEARCH,
    };
    let empty = BTreeSet::new();
    let (assessment, diagnostics) = {
        let evaluator = PredicateEvaluator::new(
            &state.snapshot,
            &catalog.facts,
            equivalence_sets,
            &state.gate_states,
            policy,
        )?;
        let resolution = evaluator.resolve_feasibility(
            transition,
            &catalog.mechanics.obligations,
            &catalog.mechanics.obstructions,
            &catalog.mechanics.resolvers,
            &catalog.mechanics.techniques,
            FeasibilitySelection {
                resolver_ids: &empty,
                technique_ids: &empty,
                already_discharged: &empty,
                microtraces: &catalog.mechanics.microtraces,
            },
        );
        let assessment = evaluator.assess_transition(
            transition,
            &resolution.discharged_obligation_ids,
            &resolution.unknown_obligation_ids,
            FeasibilityMode::Modeled,
        );
        let diagnostics = TransitionJoinDiagnostics {
            active_obstruction_ids: resolution.active_obstruction_ids,
            unknown_obstruction_ids: resolution.unknown_obstruction_ids,
            applied_resolver_ids: resolution.applied_resolver_ids,
            applicable_technique_ids: resolution.applicable_technique_ids,
        };
        (assessment, diagnostics)
    };
    if assessment.classification == TransitionClassification::Executable {
        state.apply_operations(
            application_id,
            &format!("{application_id}.after"),
            &transition.activation.effects,
        )?;
    }
    Ok(TransitionEvaluationResult {
        assessment,
        diagnostics,
    })
}

struct TransitionEvaluationResult {
    assessment: TransitionAssessment,
    diagnostics: TransitionJoinDiagnostics,
}

fn inspect_route_frontier(
    mut state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: Option<RouteBook>,
    evidence_mode: crate::RuntimeEvidenceMode,
) -> Result<PlannerServicePayload, dusklight_route_planner::PlannerContractError> {
    let path_state = |state: &PlannerExecutionState,
                      route_step_id: Option<String>|
     -> Result<
        PlannerExecutionPathState,
        dusklight_route_planner::PlannerContractError,
    > {
        let location = &state.snapshot.environment.location;
        Ok(PlannerExecutionPathState {
            label: match &route_step_id {
                Some(step_id) => format!(
                    "After {step_id}: {} r{} l{} s{}",
                    location.stage, location.room, location.layer, location.spawn
                ),
                None => format!(
                    "Route start: {} r{} l{} s{}",
                    location.stage, location.room, location.layer, location.spawn
                ),
            },
            execution_state_sha256: state.digest()?,
            snapshot_sha256: state.snapshot.digest()?,
            route_step_id,
        })
    };
    let mut execution_path = vec![path_state(&state, None)?];
    let mut execution_states = vec![inspect_state(
        &state,
        &catalog.facts,
        equivalence_sets,
        evidence_mode,
    )?];
    if let Some(route_book) = &route_book {
        route_book.validate_against_composed(catalog)?;
        if let Some(method) = route_book
            .methods
            .iter()
            .find(|method| method.id == AUTHORED_METHOD_ID)
        {
            for (index, step_id) in method.step_ids.iter().enumerate() {
                let step = route_book
                    .steps
                    .iter()
                    .find(|step| &step.id == step_id)
                    .ok_or_else(|| {
                        dusklight_route_planner::PlannerContractError::new(
                            "route_book.methods.step_ids",
                            "references a missing authored step",
                        )
                    })?;
                let RouteActionRef::Transition { transition_id } = &step.action else {
                    return Err(dusklight_route_planner::PlannerContractError::new(
                        "route_book.steps.action",
                        "route-frontier inspection currently requires transition steps",
                    ));
                };
                let evaluation = assess_and_apply_transition(
                    &mut state,
                    catalog,
                    equivalence_sets,
                    transition_id,
                    evidence_mode,
                    &format!("route.frontier-replay-{index:04}"),
                )?;
                if evaluation.assessment.classification != TransitionClassification::Executable {
                    return Err(dusklight_route_planner::PlannerContractError::new(
                        "route_book.methods.step_ids",
                        format!(
                            "authored step {step_id} is {:?} at its replay boundary",
                            evaluation.assessment.classification
                        ),
                    ));
                }
                execution_path.push(path_state(&state, Some(step_id.clone()))?);
                execution_states.push(inspect_state(
                    &state,
                    &catalog.facts,
                    equivalence_sets,
                    evidence_mode,
                )?);
            }
        }
    }
    let frontier = execution_states
        .last()
        .cloned()
        .expect("start state inspected");
    let frontier_state = state.to_document()?;
    let mut transitions = Vec::with_capacity(catalog.mechanics.transitions.len());
    for transition in &catalog.mechanics.transitions {
        let mut candidate_state = state.clone();
        let evaluation = assess_and_apply_transition(
            &mut candidate_state,
            catalog,
            equivalence_sets,
            &transition.id,
            evidence_mode,
            &format!("route.frontier-candidate.{}", transition.id),
        )?;
        transitions.push(RouteFrontierTransition {
            transition_id: transition.id.clone(),
            assessment: evaluation.assessment,
            diagnostics: evaluation.diagnostics,
        });
    }
    let mut graph = if let Some(route_book) = &route_book {
        PlannerGraph::project_composed_with_route_book(catalog, route_book)?
    } else {
        PlannerGraph::project_composed(catalog)?
    };
    graph.attach_authored_execution_path(&execution_path)?;
    let graph_sha256 = graph.digest()?;
    Ok(PlannerServicePayload::RouteFrontier {
        graph: Box::new(graph),
        graph_sha256,
        frontier_state: Box::new(frontier_state),
        frontier: Box::new(frontier),
        execution_states,
        transitions,
    })
}

fn inspect_authored_route(
    mut state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: RouteBook,
    evidence_mode: crate::RuntimeEvidenceMode,
) -> Result<AuthoredRouteInspection, dusklight_route_planner::PlannerContractError> {
    route_book.validate_against_composed(catalog)?;
    let method = route_book
        .methods
        .iter()
        .find(|method| method.id == AUTHORED_METHOD_ID)
        .ok_or_else(|| {
            dusklight_route_planner::PlannerContractError::new(
                "route_book.methods",
                "does not contain the browser-authored route method",
            )
        })?;
    let initial = state.clone();
    let mut steps = Vec::with_capacity(method.step_ids.len());
    for (index, step_id) in method.step_ids.iter().enumerate() {
        let step = route_book
            .steps
            .iter()
            .find(|step| &step.id == step_id)
            .ok_or_else(|| {
                dusklight_route_planner::PlannerContractError::new(
                    "route_book.methods.step_ids",
                    "references a missing authored step",
                )
            })?;
        let RouteActionRef::Transition { transition_id } = &step.action else {
            return Err(dusklight_route_planner::PlannerContractError::new(
                "route_book.steps.action",
                "authored route inspection currently requires transition steps",
            ));
        };
        let before = state.clone();
        let evaluation = assess_and_apply_transition(
            &mut state,
            catalog,
            equivalence_sets,
            transition_id,
            evidence_mode,
            &format!("route.inspect-{index:04}"),
        )?;
        if evaluation.assessment.classification != TransitionClassification::Executable {
            return Ok(AuthoredRouteInspection {
                steps,
                rejection: Some(AuthoredRouteRejectionInspection {
                    failed_step_id: step_id.clone(),
                    transition_id: transition_id.clone(),
                    assessment: evaluation.assessment,
                    diagnostics: evaluation.diagnostics,
                    prefix_state_change: inspect_route_state_change(
                        &initial,
                        &before,
                        catalog,
                        equivalence_sets,
                        evidence_mode,
                        &format!("route.inspect-rejection-{index:04}"),
                    )?,
                }),
            });
        }
        steps.push(AuthoredRouteStepInspection {
            step_id: step_id.clone(),
            transition_id: transition_id.clone(),
            assessment: evaluation.assessment,
            state_change: inspect_route_state_change(
                &before,
                &state,
                catalog,
                equivalence_sets,
                evidence_mode,
                &format!("route.inspect-step-{index:04}"),
            )?,
        });
    }
    Ok(AuthoredRouteInspection {
        steps,
        rejection: None,
    })
}

fn inspect_route_state_change(
    before: &PlannerExecutionState,
    after: &PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    evidence_mode: crate::RuntimeEvidenceMode,
    boundary_id: &str,
) -> Result<AuthoredRouteStateChange, dusklight_route_planner::PlannerContractError> {
    Ok(AuthoredRouteStateChange {
        before: inspect_state(before, &catalog.facts, equivalence_sets, evidence_mode)?,
        after: inspect_state(after, &catalog.facts, equivalence_sets, evidence_mode)?,
        diff: inspect_state_diff(
            before,
            after,
            BoundaryKind::Custom {
                id: boundary_id.into(),
            },
            &catalog.facts,
            equivalence_sets,
            evidence_mode,
        )?,
    })
}

#[allow(clippy::too_many_arguments)]
fn suggest_transition_chain(
    state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: Option<RouteBook>,
    transition_id: &str,
    evidence_mode: crate::RuntimeEvidenceMode,
    max_depth: usize,
    max_states: usize,
) -> Result<PlannerServicePayload, dusklight_route_planner::PlannerContractError> {
    if max_depth == 0 || max_depth > 32 {
        return Err(dusklight_route_planner::PlannerContractError::new(
            "max_depth",
            "must be between 1 and 32",
        ));
    }
    if max_states == 0 || max_states > 100_000 {
        return Err(dusklight_route_planner::PlannerContractError::new(
            "max_states",
            "must be between 1 and 100000",
        ));
    }
    let frontier =
        inspect_route_frontier(state, catalog, equivalence_sets, route_book, evidence_mode)?;
    let PlannerServicePayload::RouteFrontier { frontier_state, .. } = frontier else {
        unreachable!("route-frontier inspection returns its typed payload")
    };
    let frontier_state = frontier_state.into_state()?;
    let mut initial_candidate = frontier_state.clone();
    let initial = assess_and_apply_transition(
        &mut initial_candidate,
        catalog,
        equivalence_sets,
        transition_id,
        evidence_mode,
        "route.suggest-initial",
    )?;

    let mut queue = VecDeque::from([(frontier_state.clone(), Vec::<String>::new())]);
    let mut visited = BTreeSet::from([frontier_state.digest()?]);
    let mut explored_states = 0usize;
    let mut hit_search_limit = false;
    while let Some((state, prefix)) = queue.pop_front() {
        if explored_states == max_states {
            hit_search_limit = true;
            break;
        }
        explored_states += 1;
        if prefix.len() < max_depth {
            let mut after = state.clone();
            let evaluation = assess_and_apply_transition(
                &mut after,
                catalog,
                equivalence_sets,
                transition_id,
                evidence_mode,
                &format!("route.suggest-target-{explored_states:06}"),
            )?;
            if evaluation.assessment.classification == TransitionClassification::Executable {
                let mut transition_ids = prefix;
                transition_ids.push(transition_id.into());
                return Ok(PlannerServicePayload::TransitionChainSuggestion {
                    target_transition_id: transition_id.into(),
                    transition_ids,
                    explored_states,
                    hit_search_limit: false,
                    assessment: Box::new(evaluation.assessment),
                    diagnostics: Box::new(evaluation.diagnostics),
                    after: Some(Box::new(after.to_document()?)),
                });
            }
        }
        if prefix.len() + 1 >= max_depth {
            continue;
        }
        for transition in &catalog.mechanics.transitions {
            if transition.id == transition_id {
                continue;
            }
            let mut next = state.clone();
            let evaluation = assess_and_apply_transition(
                &mut next,
                catalog,
                equivalence_sets,
                &transition.id,
                evidence_mode,
                &format!(
                    "route.suggest-producer-{explored_states:06}.{}",
                    transition.id
                ),
            )?;
            if evaluation.assessment.classification != TransitionClassification::Executable {
                continue;
            }
            let identity = next.digest()?;
            if visited.contains(&identity) {
                continue;
            }
            if visited.len() == max_states {
                hit_search_limit = true;
                break;
            }
            visited.insert(identity);
            let mut chain = prefix.clone();
            chain.push(transition.id.clone());
            queue.push_back((next, chain));
        }
    }
    if !queue.is_empty() {
        hit_search_limit = true;
    }
    Ok(PlannerServicePayload::TransitionChainSuggestion {
        target_transition_id: transition_id.into(),
        transition_ids: Vec::new(),
        explored_states,
        hit_search_limit,
        assessment: Box::new(initial.assessment),
        diagnostics: Box::new(initial.diagnostics),
        after: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn append_transition_to_route_book(
    mut state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: Option<RouteBook>,
    route_book_id: String,
    route_book_label: String,
    transition_id: &str,
    evidence_mode: crate::RuntimeEvidenceMode,
) -> Result<PlannerServicePayload, dusklight_route_planner::PlannerContractError> {
    let previous_route_book_sha256 = route_book.as_ref().map(RouteBook::digest).transpose()?;
    if let Some(book) = &route_book {
        book.validate_against_composed(catalog)?;
        let method = book
            .methods
            .iter()
            .find(|method| method.id == AUTHORED_METHOD_ID)
            .ok_or_else(|| {
                dusklight_route_planner::PlannerContractError::new(
                    "route_book.methods",
                    "does not contain the browser-authored route method",
                )
            })?;
        for (index, step_id) in method.step_ids.iter().enumerate() {
            let step = book
                .steps
                .iter()
                .find(|step| &step.id == step_id)
                .ok_or_else(|| {
                    dusklight_route_planner::PlannerContractError::new(
                        "route_book.methods.step_ids",
                        "references a missing authored step",
                    )
                })?;
            let RouteActionRef::Transition {
                transition_id: replay_id,
            } = &step.action
            else {
                return Err(dusklight_route_planner::PlannerContractError::new(
                    "route_book.steps.action",
                    "authored route propagation currently requires transition steps",
                ));
            };
            let evaluation = assess_and_apply_transition(
                &mut state,
                catalog,
                equivalence_sets,
                replay_id,
                evidence_mode,
                &format!("route.replay-{index:04}"),
            )?;
            if evaluation.assessment.classification != TransitionClassification::Executable {
                return Err(dusklight_route_planner::PlannerContractError::new(
                    "route_book.steps",
                    format!(
                        "existing step {step_id} no longer composes: {:?}",
                        evaluation.assessment.classification
                    ),
                ));
            }
        }
    }

    let evaluation = assess_and_apply_transition(
        &mut state,
        catalog,
        equivalence_sets,
        transition_id,
        evidence_mode,
        "route.append",
    )?;
    if evaluation.assessment.classification != TransitionClassification::Executable {
        return Ok(PlannerServicePayload::RejectedTransitionJoin {
            assessment: Box::new(evaluation.assessment),
            diagnostics: Box::new(evaluation.diagnostics),
            closest_before: Box::new(state.to_document()?),
        });
    }
    let transition = catalog
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.id == transition_id)
        .expect("assessment resolved the transition");
    let step_id = next_authored_step_id(route_book.as_ref());
    let step = ReferenceStep {
        id: step_id.clone(),
        label: transition.label.clone(),
        scope: transition.scope.clone(),
        action: RouteActionRef::Transition {
            transition_id: transition_id.into(),
        },
        precondition: None,
        postcondition: None,
        region_id: Some(AUTHORED_REGION_ID.into()),
        annotation_ids: Vec::new(),
    };
    let book = if let Some(book) = route_book {
        let mut method = book
            .methods
            .iter()
            .find(|method| method.id == AUTHORED_METHOD_ID)
            .expect("validated authored method")
            .clone();
        method.step_ids.push(step_id.clone());
        RouteBookEditBatch {
            schema: ROUTE_BOOK_EDIT_BATCH_SCHEMA.into(),
            expected_route_book_sha256: book.digest()?,
            edits: vec![
                RouteBookEdit::UpsertStep { step },
                RouteBookEdit::UpsertMethod { method },
            ],
        }
        .apply_composed(&book, catalog)?
    } else {
        let refinement_stack_sha256 = Some(catalog.refinement_stack.digest()?);
        let scope = transition.scope.clone();
        let goal_id = catalog
            .mechanics
            .goals
            .first()
            .map(|goal| goal.id.clone())
            .ok_or_else(|| {
                dusklight_route_planner::PlannerContractError::new(
                    "catalog.mechanics.goals",
                    "must contain a goal before creating an authored route",
                )
            })?;
        let book = RouteBook {
            schema: ROUTE_BOOK_SCHEMA.into(),
            manifest: RouteBookManifest {
                id: route_book_id,
                version: "1.0.0".into(),
                label: route_book_label,
                author: "Route Planner".into(),
                source: "Browser-authored exact transition sequence".into(),
                scope: scope.clone(),
                refinement_stack_sha256,
            },
            goal_ids: vec![goal_id],
            constraints: Vec::new(),
            directives: Vec::new(),
            steps: vec![step],
            methods: vec![PlanMethod {
                id: AUTHORED_METHOD_ID.into(),
                label: "Authored route".into(),
                scope: scope.clone(),
                region_id: AUTHORED_REGION_ID.into(),
                step_ids: vec![step_id.clone()],
            }],
            regions: vec![PlanRegion {
                id: AUTHORED_REGION_ID.into(),
                label: "Authored route".into(),
                scope,
                parent_region_id: None,
                entry_predicate: None,
                outcome_predicate: PredicateExpression::True,
                method_ids: vec![AUTHORED_METHOD_ID.into()],
                selected_method_id: Some(AUTHORED_METHOD_ID.into()),
                collapse_policy: CollapsePolicy::Never,
            }],
            annotations: Vec::new(),
        };
        book.validate_against_composed(catalog)?;
        book
    };
    let route_book_sha256 = book.digest()?;
    Ok(PlannerServicePayload::AppendedTransition {
        book: Box::new(book),
        previous_route_book_sha256,
        route_book_sha256,
        step_id,
        assessment: Box::new(evaluation.assessment),
        after: Box::new(state.to_document()?),
    })
}

fn remove_authored_step_from_route_book(
    mut state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: RouteBook,
    step_id: &str,
    evidence_mode: crate::RuntimeEvidenceMode,
) -> Result<PlannerServicePayload, dusklight_route_planner::PlannerContractError> {
    route_book.validate_against_composed(catalog)?;
    let previous_route_book_sha256 = route_book.digest()?;
    let method = route_book
        .methods
        .iter()
        .find(|method| method.id == AUTHORED_METHOD_ID)
        .ok_or_else(|| {
            dusklight_route_planner::PlannerContractError::new(
                "route_book.methods",
                "does not contain the browser-authored route method",
            )
        })?;
    if !method.step_ids.iter().any(|candidate| candidate == step_id) {
        return Err(dusklight_route_planner::PlannerContractError::new(
            "step_id",
            "does not name a step in the browser-authored route method",
        ));
    }

    for (index, surviving_step_id) in method
        .step_ids
        .iter()
        .filter(|candidate| candidate.as_str() != step_id)
        .enumerate()
    {
        let step = route_book
            .steps
            .iter()
            .find(|step| &step.id == surviving_step_id)
            .expect("validated route method references existing steps");
        let RouteActionRef::Transition { transition_id } = &step.action else {
            return Err(dusklight_route_planner::PlannerContractError::new(
                "route_book.steps.action",
                "authored route propagation currently requires transition steps",
            ));
        };
        let evaluation = assess_and_apply_transition(
            &mut state,
            catalog,
            equivalence_sets,
            transition_id,
            evidence_mode,
            &format!("route.remove-replay-{index:04}"),
        )?;
        if evaluation.assessment.classification != TransitionClassification::Executable {
            return Ok(PlannerServicePayload::RejectedRouteEdit {
                step_id: step_id.into(),
                failed_step_id: surviving_step_id.clone(),
                assessment: Box::new(evaluation.assessment),
                diagnostics: Box::new(evaluation.diagnostics),
                closest_before: Box::new(state.to_document()?),
            });
        }
    }
    let after = Box::new(state.to_document()?);
    if method.step_ids.len() == 1 {
        return Ok(PlannerServicePayload::RemovedAuthoredStep {
            book: None,
            previous_route_book_sha256,
            route_book_sha256: None,
            step_id: step_id.into(),
            after,
        });
    }

    let mut edited_method = method.clone();
    edited_method
        .step_ids
        .retain(|candidate| candidate != step_id);
    let book = RouteBookEditBatch {
        schema: ROUTE_BOOK_EDIT_BATCH_SCHEMA.into(),
        expected_route_book_sha256: previous_route_book_sha256,
        edits: vec![
            RouteBookEdit::UpsertMethod {
                method: edited_method,
            },
            RouteBookEdit::RemoveStep {
                step_id: step_id.into(),
            },
        ],
    }
    .apply_composed(&route_book, catalog)?;
    let route_book_sha256 = book.digest()?;
    Ok(PlannerServicePayload::RemovedAuthoredStep {
        book: Some(Box::new(book)),
        previous_route_book_sha256,
        route_book_sha256: Some(route_book_sha256),
        step_id: step_id.into(),
        after,
    })
}

#[allow(clippy::too_many_arguments)]
fn replace_authored_step_in_route_book(
    mut state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: RouteBook,
    step_id: &str,
    transition_id: &str,
    evidence_mode: crate::RuntimeEvidenceMode,
) -> Result<PlannerServicePayload, dusklight_route_planner::PlannerContractError> {
    route_book.validate_against_composed(catalog)?;
    let previous_route_book_sha256 = route_book.digest()?;
    let method = route_book
        .methods
        .iter()
        .find(|method| method.id == AUTHORED_METHOD_ID)
        .ok_or_else(|| {
            dusklight_route_planner::PlannerContractError::new(
                "route_book.methods",
                "does not contain the browser-authored route method",
            )
        })?;
    if !method.step_ids.iter().any(|candidate| candidate == step_id) {
        return Err(dusklight_route_planner::PlannerContractError::new(
            "step_id",
            "does not name a step in the browser-authored route method",
        ));
    }
    let transition = catalog
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.id == transition_id)
        .ok_or_else(|| {
            dusklight_route_planner::PlannerContractError::new(
                "transition_id",
                "does not name a transition in the composed catalog",
            )
        })?;
    let mut replacement = route_book
        .steps
        .iter()
        .find(|step| step.id == step_id)
        .expect("validated route method references existing steps")
        .clone();
    replacement.label = transition.label.clone();
    replacement.scope = transition.scope.clone();
    replacement.action = RouteActionRef::Transition {
        transition_id: transition_id.into(),
    };

    let mut replacement_assessment = None;
    for (index, replay_step_id) in method.step_ids.iter().enumerate() {
        let step = route_book
            .steps
            .iter()
            .find(|step| &step.id == replay_step_id)
            .expect("validated route method references existing steps");
        let replay_transition_id = if replay_step_id == step_id {
            transition_id
        } else {
            let RouteActionRef::Transition { transition_id } = &step.action else {
                return Err(dusklight_route_planner::PlannerContractError::new(
                    "route_book.steps.action",
                    "authored route propagation currently requires transition steps",
                ));
            };
            transition_id
        };
        let evaluation = assess_and_apply_transition(
            &mut state,
            catalog,
            equivalence_sets,
            replay_transition_id,
            evidence_mode,
            &format!("route.replace-replay-{index:04}"),
        )?;
        if evaluation.assessment.classification != TransitionClassification::Executable {
            return Ok(PlannerServicePayload::RejectedRouteEdit {
                step_id: step_id.into(),
                failed_step_id: replay_step_id.clone(),
                assessment: Box::new(evaluation.assessment),
                diagnostics: Box::new(evaluation.diagnostics),
                closest_before: Box::new(state.to_document()?),
            });
        }
        if replay_step_id == step_id {
            replacement_assessment = Some(evaluation.assessment);
        }
    }
    let assessment = replacement_assessment.expect("authored method contains replacement step");
    let after = Box::new(state.to_document()?);
    let book = RouteBookEditBatch {
        schema: ROUTE_BOOK_EDIT_BATCH_SCHEMA.into(),
        expected_route_book_sha256: previous_route_book_sha256,
        edits: vec![RouteBookEdit::UpsertStep { step: replacement }],
    }
    .apply_composed(&route_book, catalog)?;
    let route_book_sha256 = book.digest()?;
    Ok(PlannerServicePayload::ReplacedAuthoredStep {
        book: Box::new(book),
        previous_route_book_sha256,
        route_book_sha256,
        step_id: step_id.into(),
        transition_id: transition_id.into(),
        assessment: Box::new(assessment),
        after,
    })
}

fn next_authored_step_id(book: Option<&RouteBook>) -> String {
    let mut index = book.map_or(0, |book| book.steps.len());
    loop {
        let candidate = format!("step.route-{index:04}");
        if book.is_none_or(|book| book.steps.iter().all(|step| step.id != candidate)) {
            return candidate;
        }
        index += 1;
    }
}

pub fn handle_envelope(envelope: PlannerServiceEnvelope) -> PlannerServiceResponse {
    if envelope.schema != PLANNER_SERVICE_SCHEMA {
        return error_response(
            Some(envelope.request.request_id().to_owned()),
            "schema",
            "is unsupported",
        );
    }
    handle_request(envelope.request)
}

fn validate_request_id(value: &str) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > 128 {
        return Err("must contain between 1 and 128 characters");
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
    }) {
        return Err("contains unsupported characters");
    }
    Ok(())
}

pub fn success_response(
    request_id: Option<String>,
    payload: PlannerServicePayload,
) -> PlannerServiceResponse {
    PlannerServiceResponse {
        schema: PLANNER_SERVICE_SCHEMA.into(),
        request_id,
        outcome: PlannerServiceOutcome::Ok {
            payload: Box::new(payload),
        },
    }
}

pub fn error_response(
    request_id: Option<String>,
    field: impl Into<String>,
    detail: impl Into<String>,
) -> PlannerServiceResponse {
    PlannerServiceResponse {
        schema: PLANNER_SERVICE_SCHEMA.into(),
        request_id,
        outcome: PlannerServiceOutcome::Error {
            field: field.into(),
            detail: detail.into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_route_planner::artifact::Digest;
    use dusklight_route_planner::execution::PlannerExecutionState;
    use dusklight_route_planner::identity::{RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration};
    use dusklight_route_planner::logic::{
        ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA,
        PredicateExpression, RuleEvidence, TruthStatus, ValueReference,
    };
    use dusklight_route_planner::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
    use dusklight_route_planner::state::{
        BackingAttachment, EXECUTION_ENVIRONMENT_SCHEMA, ExecutionContext, ExecutionEnvironment,
        PhysicalSlotId, PlayerForm, PlayerState, RuntimeFile, RuntimeFileLifecycle,
        RuntimeFileOrigin, SceneLocation, StateValue,
    };
    use dusklight_route_planner::transition::{
        ActivationContract, CandidateTransition, Goal, MECHANICS_CATALOG_SCHEMA, StateOperation,
        TransitionKind,
    };
    use std::collections::BTreeMap;

    fn catalogs() -> (FactCatalog, MechanicsCatalog) {
        (
            FactCatalog {
                schema: FACT_CATALOG_SCHEMA.into(),
                aliases: Vec::new(),
                derived_facts: Vec::new(),
            },
            MechanicsCatalog {
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
                goals: Vec::new(),
            },
        )
    }

    fn executable_transition_fixture() -> (PlannerExecutionStateDocument, ComposedPlannerCatalog) {
        let runtime = RuntimeConfiguration {
            schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
            content_sha256: Digest([1; 32]),
            language: "en".into(),
            settings: BTreeMap::new(),
        };
        let scope = ContextScope {
            selectors: vec![dusklight_route_planner::identity::ContextSelector::Exact {
                context: runtime.exact_context().unwrap(),
            }],
        };
        let snapshot = StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.before".into(),
            sequence: 0,
            environment: ExecutionEnvironment {
                schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
                runtime_configuration: runtime,
                active_runtime_file: RuntimeFile {
                    id: "file-0".into(),
                    origin: RuntimeFileOrigin::TitleFile0,
                    backing: BackingAttachment::MemoryOnly,
                    allowed_serialization_targets: vec![PhysicalSlotId(1)],
                    lifecycle: RuntimeFileLifecycle::Active,
                },
                inactive_runtime_files: Vec::new(),
                physical_slots: Vec::new(),
                physical_slot_observations: Vec::new(),
                execution_context: ExecutionContext::World,
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
                spatial_volumes: Vec::new(),
                spatial_connections: Vec::new(),
                spatial_planes: Vec::new(),
                persisted_object_controls: Vec::new(),
                live_world_objects: Vec::new(),
            },
            semantic_observations: Vec::new(),
        };
        let state = PlannerExecutionState::new(snapshot)
            .unwrap()
            .to_document()
            .unwrap();
        let (facts, mut mechanics) = catalogs();
        mechanics.transitions.push(CandidateTransition {
            id: "transition.enter-forest".into(),
            label: "Enter Forest Temple".into(),
            scope,
            transition_kind: TransitionKind::Door,
            approach_id: "approach.front".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::True,
                physical_obligation_ids: Vec::new(),
                effects: vec![StateOperation::SetLocation {
                    location: SceneLocation {
                        stage: "D_MN05".into(),
                        room: 1,
                        layer: 0,
                        spawn: 2,
                    },
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: RuleEvidence {
                truth: TruthStatus::Established,
                records: vec![EvidenceRecord {
                    id: "source.test".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(Digest([2; 32])),
                    note: "Test transition.".into(),
                }],
            },
        });
        mechanics.transitions.push(CandidateTransition {
            id: "transition.enter-boss".into(),
            label: "Enter Boss Room".into(),
            scope: mechanics.transitions[0].scope.clone(),
            transition_kind: TransitionKind::Door,
            approach_id: "approach.boss".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::Compare {
                    left: ValueReference::LocationStage,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text("D_MN05".into()),
                    },
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![StateOperation::SetLocation {
                    location: SceneLocation {
                        stage: "D_MN06".into(),
                        room: 0,
                        layer: 0,
                        spawn: 0,
                    },
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: RuleEvidence {
                truth: TruthStatus::Established,
                records: vec![EvidenceRecord {
                    id: "source.test.boss".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(Digest([3; 32])),
                    note: "Test downstream transition.".into(),
                }],
            },
        });
        mechanics.transitions.push(CandidateTransition {
            id: "transition.enter-side-room".into(),
            label: "Enter Side Room".into(),
            scope: mechanics.transitions[0].scope.clone(),
            transition_kind: TransitionKind::Door,
            approach_id: "approach.side-room".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::Compare {
                    left: ValueReference::LocationStage,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text("D_MN05".into()),
                    },
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![StateOperation::SetLocation {
                    location: SceneLocation {
                        stage: "D_MN07".into(),
                        room: 2,
                        layer: 0,
                        spawn: 1,
                    },
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: RuleEvidence {
                truth: TruthStatus::Established,
                records: vec![EvidenceRecord {
                    id: "source.test.side-room".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(Digest([4; 32])),
                    note: "Test replacement transition.".into(),
                }],
            },
        });
        mechanics
            .transitions
            .sort_by(|left, right| left.id.cmp(&right.id));
        mechanics.goals.push(Goal {
            id: "goal.boss-room".into(),
            label: "Reach boss room".into(),
            predicate: PredicateExpression::Compare {
                left: ValueReference::LocationStage,
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Text("D_MN06".into()),
                },
            },
        });
        let catalog = ComposedPlannerCatalog::compose(&facts, &mechanics, &[]).unwrap();
        (state, catalog)
    }

    #[test]
    fn service_composes_then_projects_without_browser_or_huntctl_state() {
        let (facts, mechanics) = catalogs();
        let response = handle_request(PlannerServiceRequest::Compose {
            request_id: "request.compose".into(),
            facts: Box::new(facts),
            mechanics: Box::new(mechanics),
            packs: Vec::new(),
            route_local_overlays: Vec::new(),
            ephemeral_what_if_overlays: Vec::new(),
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("composition should succeed");
        };
        let PlannerServicePayload::ComposedCatalog { catalog, .. } = *payload else {
            panic!("composition should return a catalog");
        };
        let response = handle_request(PlannerServiceRequest::ProjectGraph {
            request_id: "request.graph".into(),
            catalog,
            route_book: None,
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("projection should succeed");
        };
        let PlannerServicePayload::Graph { graph, .. } = *payload else {
            panic!("projection should return a graph");
        };
        assert_eq!(response.request_id.as_deref(), Some("request.graph"));
        assert_eq!(graph.nodes.len(), 0);
        assert_eq!(graph.regions.len(), 2);
    }

    #[test]
    fn service_evaluates_then_applies_only_an_executable_transition() {
        let (state, catalog) = executable_transition_fixture();
        let response = handle_request(PlannerServiceRequest::EvaluateTransition {
            request_id: "request.transition".into(),
            state: Box::new(state),
            catalog: Box::new(catalog),
            equivalence_sets: Vec::new(),
            transition_id: "transition.enter-forest".into(),
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("transition evaluation should succeed");
        };
        let PlannerServicePayload::TransitionEvaluation {
            assessment,
            diagnostics,
            after,
        } = *payload
        else {
            panic!("transition evaluation should return its typed payload");
        };
        assert_eq!(
            assessment.classification,
            TransitionClassification::Executable
        );
        assert!(diagnostics.active_obstruction_ids.is_empty());
        let after = after.unwrap();
        assert_eq!(after.snapshot.environment.location.stage, "D_MN05");
        assert_eq!(after.snapshot.environment.location.room, 1);
    }

    #[test]
    fn route_frontier_replays_authored_steps_before_listing_applicable_transitions() {
        let (state, catalog) = executable_transition_fixture();
        let appended = handle_request(PlannerServiceRequest::AppendTransition {
            request_id: "request.frontier-append".into(),
            state: Box::new(state.clone()),
            catalog: Box::new(catalog.clone()),
            equivalence_sets: Vec::new(),
            route_book: None,
            route_book_id: "route.frontier".into(),
            route_book_label: "Frontier route".into(),
            transition_id: "transition.enter-forest".into(),
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = appended.outcome else {
            panic!("frontier producer should append");
        };
        let PlannerServicePayload::AppendedTransition { book, .. } = *payload else {
            panic!("append should return a route book");
        };
        let response = handle_request(PlannerServiceRequest::InspectRouteFrontier {
            request_id: "request.frontier".into(),
            state: Box::new(state),
            catalog: Box::new(catalog),
            equivalence_sets: Vec::new(),
            route_book: Some(book),
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("route frontier inspection should succeed");
        };
        let PlannerServicePayload::RouteFrontier {
            graph,
            frontier_state,
            frontier,
            execution_states,
            transitions,
            ..
        } = *payload
        else {
            panic!("frontier inspection should return its typed payload");
        };
        assert_eq!(frontier_state.snapshot.environment.location.stage, "D_MN05");
        assert_eq!(frontier.state.snapshot.environment.location.stage, "D_MN05");
        assert_eq!(execution_states.len(), 2);
        assert!(graph.nodes.iter().any(|node| {
            matches!(
                &node.payload,
                dusklight_route_planner::graph::PlannerNodePayload::ExecutionState {
                    route_step_id: Some(step_id),
                    ..
                } if step_id == "step.route-0000"
            )
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.relation == dusklight_route_planner::graph::PlannerGraphRelation::RouteResult
        }));
        for transition_id in ["transition.enter-boss", "transition.enter-side-room"] {
            assert_eq!(
                transitions
                    .iter()
                    .find(|record| record.transition_id == transition_id)
                    .unwrap()
                    .assessment
                    .classification,
                TransitionClassification::Executable
            );
        }
    }

    #[test]
    fn service_suggests_the_shortest_exact_transition_chain_to_a_rejected_join() {
        let (state, catalog) = executable_transition_fixture();
        let response = handle_request(PlannerServiceRequest::SuggestTransitionChain {
            request_id: "request.suggest-chain".into(),
            state: Box::new(state),
            catalog: Box::new(catalog),
            equivalence_sets: Vec::new(),
            route_book: None,
            transition_id: "transition.enter-boss".into(),
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
            max_depth: 4,
            max_states: 32,
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("chain suggestion should be a typed service result");
        };
        let PlannerServicePayload::TransitionChainSuggestion {
            target_transition_id,
            transition_ids,
            explored_states,
            hit_search_limit,
            assessment,
            after: Some(after),
            ..
        } = *payload
        else {
            panic!("a reachable rejected join should return its producer chain");
        };
        assert_eq!(target_transition_id, "transition.enter-boss");
        assert_eq!(
            transition_ids,
            ["transition.enter-forest", "transition.enter-boss"]
        );
        assert!(explored_states >= 2);
        assert!(!hit_search_limit);
        assert_eq!(
            assessment.classification,
            TransitionClassification::Executable
        );
        assert_eq!(after.snapshot.environment.location.stage, "D_MN06");
    }

    #[test]
    fn append_transition_replays_and_propagates_the_authored_route() {
        let (state, catalog) = executable_transition_fixture();
        let rejected = handle_request(PlannerServiceRequest::AppendTransition {
            request_id: "request.reject-join".into(),
            state: Box::new(state.clone()),
            catalog: Box::new(catalog.clone()),
            equivalence_sets: Vec::new(),
            route_book: None,
            route_book_id: "route.test".into(),
            route_book_label: "Test route".into(),
            transition_id: "transition.enter-boss".into(),
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = rejected.outcome else {
            panic!("a rejected join is a typed evaluation, not a service failure");
        };
        let PlannerServicePayload::RejectedTransitionJoin {
            assessment,
            diagnostics,
            closest_before,
        } = *payload
        else {
            panic!("non-executable append should return rejection diagnostics");
        };
        assert_eq!(
            assessment.classification,
            TransitionClassification::GuardBlocked
        );
        assert!(diagnostics.active_obstruction_ids.is_empty());
        assert_eq!(
            closest_before.snapshot.environment.location.stage,
            "F_SP103"
        );

        let first = handle_request(PlannerServiceRequest::AppendTransition {
            request_id: "request.append-first".into(),
            state: Box::new(state.clone()),
            catalog: Box::new(catalog.clone()),
            equivalence_sets: Vec::new(),
            route_book: None,
            route_book_id: "route.test".into(),
            route_book_label: "Test route".into(),
            transition_id: "transition.enter-forest".into(),
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = first.outcome else {
            panic!(
                "first append should succeed: {}",
                serde_json::to_string(&first).unwrap()
            );
        };
        let PlannerServicePayload::AppendedTransition {
            book,
            previous_route_book_sha256,
            after,
            ..
        } = *payload
        else {
            panic!("append should return route semantics and propagated state");
        };
        assert!(previous_route_book_sha256.is_none());
        assert_eq!(after.snapshot.environment.location.stage, "D_MN05");
        assert_eq!(book.methods[0].step_ids, ["step.route-0000"]);

        let second = handle_request(PlannerServiceRequest::AppendTransition {
            request_id: "request.append-second".into(),
            state: Box::new(state.clone()),
            catalog: Box::new(catalog.clone()),
            equivalence_sets: Vec::new(),
            route_book: Some(book),
            route_book_id: "route.test".into(),
            route_book_label: "Test route".into(),
            transition_id: "transition.enter-boss".into(),
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = second.outcome else {
            panic!("downstream append should succeed after replaying its producer");
        };
        let PlannerServicePayload::AppendedTransition {
            book,
            previous_route_book_sha256,
            after,
            ..
        } = *payload
        else {
            panic!("second append should return the edited route book");
        };
        assert!(previous_route_book_sha256.is_some());
        assert_eq!(after.snapshot.environment.location.stage, "D_MN06");
        assert_eq!(
            book.methods[0].step_ids,
            ["step.route-0000", "step.route-0001"]
        );

        let inspected = handle_request(PlannerServiceRequest::InspectAuthoredRoute {
            request_id: "request.inspect-route".into(),
            state: Box::new(state.clone()),
            catalog: Box::new(catalog.clone()),
            equivalence_sets: Vec::new(),
            route_book: Box::new((*book).clone()),
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = inspected.outcome else {
            panic!("authored route inspection should replay every accepted step");
        };
        let PlannerServicePayload::AuthoredRouteInspection { inspection } = *payload else {
            panic!("route inspection should return typed state changes");
        };
        assert!(inspection.rejection.is_none());
        assert_eq!(inspection.steps.len(), 2);
        assert_eq!(inspection.steps[0].step_id, "step.route-0000");
        assert_eq!(
            inspection.steps[0]
                .state_change
                .before
                .state
                .snapshot
                .environment
                .location
                .stage,
            "F_SP103"
        );
        assert_eq!(
            inspection.steps[0]
                .state_change
                .after
                .state
                .snapshot
                .environment
                .location
                .stage,
            "D_MN05"
        );
        assert_eq!(
            inspection.steps[1]
                .state_change
                .after
                .state
                .snapshot
                .environment
                .location
                .stage,
            "D_MN06"
        );
        assert!(
            inspection.steps[1]
                .state_change
                .diff
                .state_diff
                .location_changed
        );

        let replaced_consumer = handle_request(PlannerServiceRequest::ReplaceAuthoredStep {
            request_id: "request.replace-consumer".into(),
            state: Box::new(state.clone()),
            catalog: Box::new(catalog.clone()),
            equivalence_sets: Vec::new(),
            route_book: Box::new((*book).clone()),
            step_id: "step.route-0001".into(),
            transition_id: "transition.enter-side-room".into(),
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = replaced_consumer.outcome else {
            panic!("an executable replacement should replay the complete route");
        };
        let PlannerServicePayload::ReplacedAuthoredStep {
            book: replaced_book,
            step_id,
            transition_id,
            assessment,
            after,
            ..
        } = *payload
        else {
            panic!("replacement should return its edited route and propagated state");
        };
        assert_eq!(step_id, "step.route-0001");
        assert_eq!(transition_id, "transition.enter-side-room");
        assert_eq!(
            assessment.classification,
            TransitionClassification::Executable
        );
        assert_eq!(after.snapshot.environment.location.stage, "D_MN07");
        assert_eq!(
            replaced_book.methods[0].step_ids,
            ["step.route-0000", "step.route-0001"]
        );
        assert!(matches!(
            &replaced_book
                .steps
                .iter()
                .find(|step| step.id == "step.route-0001")
                .unwrap()
                .action,
            RouteActionRef::Transition { transition_id }
                if transition_id == "transition.enter-side-room"
        ));

        let rejected_replace = handle_request(PlannerServiceRequest::ReplaceAuthoredStep {
            request_id: "request.replace-producer".into(),
            state: Box::new(state.clone()),
            catalog: Box::new(catalog.clone()),
            equivalence_sets: Vec::new(),
            route_book: Box::new((*book).clone()),
            step_id: "step.route-0000".into(),
            transition_id: "transition.enter-boss".into(),
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = rejected_replace.outcome else {
            panic!("a rejected replacement should remain a typed edit result");
        };
        let PlannerServicePayload::RejectedRouteEdit {
            step_id,
            failed_step_id,
            assessment,
            closest_before,
            ..
        } = *payload
        else {
            panic!("replacement should identify the first non-executable join");
        };
        assert_eq!(step_id, "step.route-0000");
        assert_eq!(failed_step_id, "step.route-0000");
        assert_eq!(
            assessment.classification,
            TransitionClassification::GuardBlocked
        );
        assert_eq!(
            closest_before.snapshot.environment.location.stage,
            "F_SP103"
        );

        let rejected_remove = handle_request(PlannerServiceRequest::RemoveAuthoredStep {
            request_id: "request.remove-producer".into(),
            state: Box::new(state.clone()),
            catalog: Box::new(catalog.clone()),
            equivalence_sets: Vec::new(),
            route_book: Box::new((*book).clone()),
            step_id: "step.route-0000".into(),
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = rejected_remove.outcome else {
            panic!("a broken downstream join should be a typed edit rejection");
        };
        let PlannerServicePayload::RejectedRouteEdit {
            step_id,
            failed_step_id,
            assessment,
            closest_before,
            ..
        } = *payload
        else {
            panic!("producer removal should identify its broken consumer");
        };
        assert_eq!(step_id, "step.route-0000");
        assert_eq!(failed_step_id, "step.route-0001");
        assert_eq!(
            assessment.classification,
            TransitionClassification::GuardBlocked
        );
        assert_eq!(
            closest_before.snapshot.environment.location.stage,
            "F_SP103"
        );

        let removed_consumer = handle_request(PlannerServiceRequest::RemoveAuthoredStep {
            request_id: "request.remove-consumer".into(),
            state: Box::new(state.clone()),
            catalog: Box::new(catalog.clone()),
            equivalence_sets: Vec::new(),
            route_book: book,
            step_id: "step.route-0001".into(),
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = removed_consumer.outcome else {
            panic!("removing the terminal consumer should preserve its producer");
        };
        let PlannerServicePayload::RemovedAuthoredStep {
            book: Some(book),
            after,
            ..
        } = *payload
        else {
            panic!("one surviving step should retain the authored route book");
        };
        assert_eq!(book.methods[0].step_ids, ["step.route-0000"]);
        assert_eq!(after.snapshot.environment.location.stage, "D_MN05");

        let removed_last = handle_request(PlannerServiceRequest::RemoveAuthoredStep {
            request_id: "request.remove-last".into(),
            state: Box::new(state),
            catalog: Box::new(catalog),
            equivalence_sets: Vec::new(),
            route_book: book,
            step_id: "step.route-0000".into(),
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = removed_last.outcome else {
            panic!("removing the last authored step should restore an empty route");
        };
        let PlannerServicePayload::RemovedAuthoredStep {
            book: None, after, ..
        } = *payload
        else {
            panic!("an empty authored route should not preserve a hollow route book");
        };
        assert_eq!(after.snapshot.environment.location.stage, "F_SP103");
    }

    #[test]
    fn malformed_catalog_error_keeps_request_identity() {
        let (facts, mut mechanics) = catalogs();
        mechanics.schema = "unsupported".into();
        let response = handle_request(PlannerServiceRequest::Compose {
            request_id: "request.bad".into(),
            facts: Box::new(facts),
            mechanics: Box::new(mechanics),
            packs: Vec::new(),
            route_local_overlays: Vec::new(),
            ephemeral_what_if_overlays: Vec::new(),
        });
        assert_eq!(response.request_id.as_deref(), Some("request.bad"));
        assert!(matches!(
            response.outcome,
            PlannerServiceOutcome::Error { ref field, .. } if field == "schema"
        ));
    }

    #[test]
    fn envelope_rejects_unknown_protocol_versions_before_dispatch() {
        let (facts, mechanics) = catalogs();
        let response = handle_envelope(PlannerServiceEnvelope {
            schema: "dusklight.route-planner.service/v999".into(),
            request: PlannerServiceRequest::Compose {
                request_id: "request.version".into(),
                facts: Box::new(facts),
                mechanics: Box::new(mechanics),
                packs: Vec::new(),
                route_local_overlays: Vec::new(),
                ephemeral_what_if_overlays: Vec::new(),
            },
        });
        assert!(matches!(
            response.outcome,
            PlannerServiceOutcome::Error { ref field, .. } if field == "schema"
        ));
    }
}
