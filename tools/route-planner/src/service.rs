//! Typed request/response boundary for planner-owned editor and automation clients.

use crate::inspection::{StateInspection, StateInspectionDiff, inspect_state, inspect_state_diff};
use crate::{
    PortableSolveReport, RuntimeSolveOptions, SolveReport, solve_composed_catalog_goal,
    solve_composed_portable_route_book_goal, solve_composed_route_book_goal,
};
use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::evaluation::EvidencePolicy;
use dusklight_route_planner::evaluation::{
    FeasibilityMode, FeasibilitySelection, PredicateEvaluator, TransitionAssessment,
    TransitionClassification,
};
use dusklight_route_planner::execution::{PlannerExecutionState, PlannerExecutionStateDocument};
use dusklight_route_planner::graph::{PlannerFeasibilityGraphDiff, PlannerGraph};
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
use std::collections::BTreeSet;

pub const PLANNER_SERVICE_SCHEMA: &str = "dusklight.route-planner.service/v30";

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
            | Self::InspectState { request_id, .. }
            | Self::DiffState { request_id, .. }
            | Self::EvaluateTransition { request_id, .. }
            | Self::AppendTransition { request_id, .. }
            | Self::Solve { request_id, .. }
            | Self::SolvePortable { request_id, .. } => request_id,
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
    StateInspection {
        inspection: Box<StateInspection>,
    },
    StateInspectionDiff {
        inspection_diff: Box<StateInspectionDiff>,
    },
    TransitionEvaluation {
        assessment: Box<TransitionAssessment>,
        after: Option<Box<PlannerExecutionStateDocument>>,
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
            let assessment = assess_and_apply_transition(
                &mut state,
                &catalog,
                &equivalence_sets,
                &transition_id,
                evidence_mode,
                "web.transition",
            )?;
            let after = if assessment.classification == TransitionClassification::Executable {
                Some(Box::new(state.to_document()?))
            } else {
                None
            };
            Ok(PlannerServicePayload::TransitionEvaluation {
                assessment: Box::new(assessment),
                after,
            })
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
) -> Result<TransitionAssessment, dusklight_route_planner::PlannerContractError> {
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
    let assessment = {
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
        evaluator.assess_transition(
            transition,
            &resolution.discharged_obligation_ids,
            &resolution.unknown_obligation_ids,
            FeasibilityMode::Modeled,
        )
    };
    if assessment.classification == TransitionClassification::Executable {
        state.apply_operations(
            application_id,
            &format!("{application_id}.after"),
            &transition.activation.effects,
        )?;
    }
    Ok(assessment)
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
            let assessment = assess_and_apply_transition(
                &mut state,
                catalog,
                equivalence_sets,
                replay_id,
                evidence_mode,
                &format!("route.replay-{index:04}"),
            )?;
            if assessment.classification != TransitionClassification::Executable {
                return Err(dusklight_route_planner::PlannerContractError::new(
                    "route_book.steps",
                    format!(
                        "existing step {step_id} no longer composes: {:?}",
                        assessment.classification
                    ),
                ));
            }
        }
    }

    let assessment = assess_and_apply_transition(
        &mut state,
        catalog,
        equivalence_sets,
        transition_id,
        evidence_mode,
        "route.append",
    )?;
    if assessment.classification != TransitionClassification::Executable {
        return Err(dusklight_route_planner::PlannerContractError::new(
            "transition_id",
            format!(
                "cannot append a non-executable join: {:?}",
                assessment.classification
            ),
        ));
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
        assessment: Box::new(assessment),
        after: Box::new(state.to_document()?),
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
        let PlannerServicePayload::TransitionEvaluation { assessment, after } = *payload else {
            panic!("transition evaluation should return its typed payload");
        };
        assert_eq!(
            assessment.classification,
            TransitionClassification::Executable
        );
        let after = after.unwrap();
        assert_eq!(after.snapshot.environment.location.stage, "D_MN05");
        assert_eq!(after.snapshot.environment.location.room, 1);
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
        assert!(matches!(
            rejected.outcome,
            PlannerServiceOutcome::Error { ref field, .. } if field == "transition_id"
        ));

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
            state: Box::new(state),
            catalog: Box::new(catalog),
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
