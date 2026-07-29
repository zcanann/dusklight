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
use dusklight_route_planner::identity::{ContextSelector, EquivalenceSet};
use dusklight_route_planner::logic::{
    ContextScope, EvidenceKind, EvidenceRecord, FactCatalog, PredicateExpression, RuleEvidence,
    TruthStatus,
};
use dusklight_route_planner::refinement::{
    ComposedPlannerCatalog, PackDependency, REFINEMENT_PACK_SCHEMA, RefinementLayers,
    RefinementOperation, RefinementPack, RefinementPackManifest, RefinementRule,
};
use dusklight_route_planner::route_book::{
    CollapsePolicy, PlanMethod, PlanRegion, ROUTE_BOOK_EDIT_BATCH_SCHEMA, ROUTE_BOOK_SCHEMA,
    ReferenceStep, RouteActionRef, RouteBook, RouteBookEdit, RouteBookEditBatch, RouteBookManifest,
};
use dusklight_route_planner::state::{BoundaryKind, ComponentBinding, ComponentSelector};
use dusklight_route_planner::transition::{MechanicsCatalog, StateOperation};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};

mod route_editing;
use route_editing::*;

pub const PLANNER_SERVICE_SCHEMA: &str = "dusklight.route-planner.service/v47";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerServiceEnvelope {
    pub schema: String,
    pub request: PlannerServiceRequest,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TheorycraftOverlayEdit {
    AddComponentTransfer {
        pack_id: String,
        label: String,
        source_component_id: String,
        destination: ComponentTransferDestination,
    },
    AddObstructionBypass {
        pack_id: String,
        label: String,
        obstruction_id: String,
    },
    Remove {
        pack_id: String,
    },
    Clear,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComponentTransferDestination {
    Rebind {
        binding: ComponentBinding,
    },
    Copy {
        destination_component_id: String,
        binding: ComponentBinding,
    },
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
    EditTheorycraftOverlays {
        request_id: String,
        base_catalog: Box<ComposedPlannerCatalog>,
        overlays: Vec<RefinementPack>,
        state: Box<PlannerExecutionStateDocument>,
        route_book: Option<Box<RouteBook>>,
        edit: TheorycraftOverlayEdit,
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
    InsertTransitionAfter {
        request_id: String,
        state: Box<PlannerExecutionStateDocument>,
        catalog: Box<ComposedPlannerCatalog>,
        equivalence_sets: Vec<EquivalenceSet>,
        route_book: Box<RouteBook>,
        after_step_id: String,
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
            | Self::EditTheorycraftOverlays { request_id, .. }
            | Self::ProjectGraph { request_id, .. }
            | Self::ProjectFeasibilityDiff { request_id, .. }
            | Self::InspectRouteFrontier { request_id, .. }
            | Self::InspectState { request_id, .. }
            | Self::DiffState { request_id, .. }
            | Self::EvaluateTransition { request_id, .. }
            | Self::SuggestTransitionChain { request_id, .. }
            | Self::AppendTransition { request_id, .. }
            | Self::InsertTransitionAfter { request_id, .. }
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
    TheorycraftOverlaysEdited {
        base_catalog: Box<ComposedPlannerCatalog>,
        overlays: Vec<RefinementPack>,
        catalog: Box<ComposedPlannerCatalog>,
        catalog_sha256: Digest,
        route_book: Option<Box<RouteBook>>,
        added_pack: Option<Box<RefinementPack>>,
        removed_pack_ids: Vec<String>,
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
    InsertedTransition {
        book: Box<RouteBook>,
        previous_route_book_sha256: Digest,
        route_book_sha256: Digest,
        step_id: String,
        after_step_id: String,
        transition_id: String,
        assessment: Box<TransitionAssessment>,
        after: Box<PlannerExecutionStateDocument>,
    },
    SolveReport {
        report: Box<SolveReport>,
        proof_graph: Box<PlannerGraph>,
        proof_graph_sha256: Digest,
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

fn edit_theorycraft_overlays(
    base_catalog: ComposedPlannerCatalog,
    mut overlays: Vec<RefinementPack>,
    state: PlannerExecutionStateDocument,
    route_book: Option<RouteBook>,
    edit: TheorycraftOverlayEdit,
) -> Result<PlannerServicePayload, dusklight_route_planner::PlannerContractError> {
    base_catalog.validate()?;
    state.clone().into_state()?;
    overlays.sort_by(|left, right| {
        left.manifest
            .precedence
            .cmp(&right.manifest.precedence)
            .then_with(|| left.manifest.id.cmp(&right.manifest.id))
    });
    let current = base_catalog.extend_ephemeral_what_if(&overlays)?;
    let mut added_pack = None;
    let mut removed_pack_ids = Vec::new();

    match edit {
        TheorycraftOverlayEdit::AddComponentTransfer {
            pack_id,
            label,
            source_component_id,
            destination,
        } => {
            let source = state
                .snapshot
                .environment
                .components
                .iter()
                .find(|component| component.id == source_component_id)
                .ok_or_else(|| {
                    dusklight_route_planner::PlannerContractError::new(
                        "edit.source_component_id",
                        format!("component {source_component_id} is absent from the start state"),
                    )
                })?;
            let operations = match destination {
                ComponentTransferDestination::Rebind { binding } => vec![
                    StateOperation::Preserve {
                        selector: ComponentSelector::Id {
                            component_id: source_component_id.clone(),
                        },
                    },
                    StateOperation::Rebind {
                        selector: ComponentSelector::Id {
                            component_id: source_component_id,
                        },
                        binding,
                    },
                ],
                ComponentTransferDestination::Copy {
                    destination_component_id,
                    binding,
                } => vec![StateOperation::Copy {
                    source: ComponentSelector::Id {
                        component_id: source_component_id,
                    },
                    destination_component_id,
                    binding,
                    serialization_owner: source.serialization_owner.clone(),
                }],
            };
            let pack = theorycraft_pack(
                &base_catalog,
                &current,
                &state,
                pack_id,
                label,
                RefinementOperation::ComponentTransform {
                    prerequisite: PredicateExpression::True,
                    operations,
                },
            )?;
            overlays.push(pack.clone());
            added_pack = Some(Box::new(pack));
        }
        TheorycraftOverlayEdit::AddObstructionBypass {
            pack_id,
            label,
            obstruction_id,
        } => {
            if !current
                .mechanics
                .obstructions
                .iter()
                .any(|obstruction| obstruction.id == obstruction_id)
            {
                return Err(dusklight_route_planner::PlannerContractError::new(
                    "edit.obstruction_id",
                    format!("obstruction {obstruction_id} is absent from the composed catalog"),
                ));
            }
            let pack = theorycraft_pack(
                &base_catalog,
                &current,
                &state,
                pack_id,
                label,
                RefinementOperation::AssumeObstructionAbsent {
                    obstruction_id,
                    when: PredicateExpression::True,
                },
            )?;
            overlays.push(pack.clone());
            added_pack = Some(Box::new(pack));
        }
        TheorycraftOverlayEdit::Remove { pack_id } => {
            let before = overlays.len();
            overlays.retain(|pack| pack.manifest.id != pack_id);
            if overlays.len() == before {
                return Err(dusklight_route_planner::PlannerContractError::new(
                    "edit.pack_id",
                    format!("theorycraft overlay {pack_id} is not active"),
                ));
            }
            removed_pack_ids.push(pack_id);
        }
        TheorycraftOverlayEdit::Clear => {
            removed_pack_ids = overlays
                .iter()
                .map(|pack| pack.manifest.id.clone())
                .collect();
            overlays.clear();
        }
    }

    overlays.sort_by(|left, right| {
        left.manifest
            .precedence
            .cmp(&right.manifest.precedence)
            .then_with(|| left.manifest.id.cmp(&right.manifest.id))
    });
    let catalog = base_catalog.extend_ephemeral_what_if(&overlays)?;
    let mut route_book = route_book;
    if let Some(book) = &mut route_book {
        if book.manifest.refinement_stack_sha256.is_some() {
            book.manifest.refinement_stack_sha256 = Some(catalog.refinement_stack.digest()?);
        }
        book.validate_against_composed(&catalog)?;
    }
    let catalog_sha256 = catalog.digest()?;
    Ok(PlannerServicePayload::TheorycraftOverlaysEdited {
        base_catalog: Box::new(base_catalog),
        overlays,
        catalog: Box::new(catalog),
        catalog_sha256,
        route_book: route_book.map(Box::new),
        added_pack,
        removed_pack_ids,
    })
}

fn theorycraft_pack(
    base_catalog: &ComposedPlannerCatalog,
    current: &ComposedPlannerCatalog,
    state: &PlannerExecutionStateDocument,
    pack_id: String,
    label: String,
    operation: RefinementOperation,
) -> Result<RefinementPack, dusklight_route_planner::PlannerContractError> {
    let runtime = &state.snapshot.environment.runtime_configuration;
    let scope = ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: runtime.exact_context()?,
        }],
    };
    let precedence = current
        .refinement_stack
        .entries
        .iter()
        .filter(|entry| {
            entry.layer == dusklight_route_planner::refinement::RefinementLayer::EphemeralWhatIf
        })
        .map(|entry| entry.precedence)
        .max()
        .unwrap_or(999)
        .checked_add(1)
        .ok_or_else(|| {
            dusklight_route_planner::PlannerContractError::new(
                "manifest.precedence",
                "ephemeral overlay precedence is exhausted",
            )
        })?;
    let dependencies = base_catalog
        .refinement_stack
        .entries
        .iter()
        .map(|entry| PackDependency {
            pack_id: entry.pack_id.clone(),
            pack_sha256: entry.pack_sha256,
        })
        .collect();
    let pack = RefinementPack {
        schema: REFINEMENT_PACK_SCHEMA.into(),
        manifest: RefinementPackManifest {
            id: pack_id.clone(),
            version: "1.0.0".into(),
            author: "Route Workbench".into(),
            source: "Explicit theorycraft editor assumption".into(),
            scope,
            precedence,
            dependencies,
            conflicts: Vec::new(),
        },
        rules: vec![RefinementRule {
            id: format!("{pack_id}.effect"),
            label,
            operation,
            evidence: RuleEvidence {
                truth: TruthStatus::Hypothetical,
                records: vec![EvidenceRecord {
                    id: format!("{pack_id}.evidence"),
                    kind: EvidenceKind::Theorycraft,
                    source_sha256: None,
                    note: "User-authored what-if; this is not a claim about game behavior.".into(),
                }],
            },
        }],
    };
    pack.validate()?;
    Ok(pack)
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
        PlannerServiceRequest::EditTheorycraftOverlays {
            base_catalog,
            overlays,
            state,
            route_book,
            edit,
            ..
        } => edit_theorycraft_overlays(
            *base_catalog,
            overlays,
            *state,
            route_book.map(|book| *book),
            edit,
        ),
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
        PlannerServiceRequest::InsertTransitionAfter {
            state,
            catalog,
            equivalence_sets,
            route_book,
            after_step_id,
            transition_id,
            evidence_mode,
            ..
        } => (*state).into_state().and_then(|state| {
            insert_transition_after_route_step(
                state,
                &catalog,
                &equivalence_sets,
                *route_book,
                &after_step_id,
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
            let mut proof_graph = if let Some(book) = route_book.as_deref() {
                PlannerGraph::project_composed_with_route_book(&catalog, book)?
            } else {
                PlannerGraph::project_composed(&catalog)?
            };
            let report = match route_book.as_deref() {
                Some(book) => solve_composed_route_book_goal(
                    state,
                    &catalog,
                    &equivalence_sets,
                    book,
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
            let proof_initial_state_sha256 =
                report
                    .result
                    .steps
                    .first()
                    .map(|step| step.source_state_sha256)
                    .or_else(|| {
                        report.result.alternative_plans.iter().find_map(|plan| {
                            plan.steps.first().map(|step| step.source_state_sha256)
                        })
                    })
                    .or_else(|| {
                        report
                            .result
                            .result_continuation
                            .as_ref()
                            .map(|continuation| continuation.state_sha256)
                    })
                    .unwrap_or(report.execution_state_sha256);
            proof_graph.attach_solver_proof(proof_initial_state_sha256, &report.result)?;
            let proof_graph_sha256 = proof_graph.digest()?;
            Ok(PlannerServicePayload::SolveReport {
                report: Box::new(report),
                proof_graph: Box::new(proof_graph),
                proof_graph_sha256,
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
#[path = "service_tests.rs"]
mod tests;
