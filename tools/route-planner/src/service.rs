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
            .find(|method| method.id == AUTHORED_METHOD_ID);
        if method.is_none() && (!book.steps.is_empty() || !book.methods.is_empty()) {
            return Err(dusklight_route_planner::PlannerContractError::new(
                "route_book.methods",
                "does not contain the browser-authored route method and is not empty",
            ));
        }
        if let Some(method) = method {
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
        let method = book
            .methods
            .iter()
            .find(|method| method.id == AUTHORED_METHOD_ID)
            .cloned();
        let scope = transition.scope.clone();
        let mut method = method.unwrap_or(PlanMethod {
            id: AUTHORED_METHOD_ID.into(),
            label: "Authored route".into(),
            scope: scope.clone(),
            region_id: AUTHORED_REGION_ID.into(),
            step_ids: Vec::new(),
        });
        method.step_ids.push(step_id.clone());
        let mut edits = vec![
            RouteBookEdit::UpsertStep { step },
            RouteBookEdit::UpsertMethod { method },
        ];
        if !book
            .regions
            .iter()
            .any(|region| region.id == AUTHORED_REGION_ID)
        {
            edits.push(RouteBookEdit::UpsertRegion {
                region: PlanRegion {
                    id: AUTHORED_REGION_ID.into(),
                    label: "Authored route".into(),
                    scope,
                    parent_region_id: None,
                    entry_predicate: None,
                    outcome_predicate: PredicateExpression::True,
                    method_ids: vec![AUTHORED_METHOD_ID.into()],
                    selected_method_id: Some(AUTHORED_METHOD_ID.into()),
                    collapse_policy: CollapsePolicy::Never,
                },
            });
        }
        RouteBookEditBatch {
            schema: ROUTE_BOOK_EDIT_BATCH_SCHEMA.into(),
            expected_route_book_sha256: book.digest()?,
            edits,
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

#[allow(clippy::too_many_arguments)]
fn insert_transition_after_route_step(
    mut state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: RouteBook,
    after_step_id: &str,
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
    let insertion_index = method
        .step_ids
        .iter()
        .position(|candidate| candidate == after_step_id)
        .ok_or_else(|| {
            dusklight_route_planner::PlannerContractError::new(
                "after_step_id",
                "does not name a step in the browser-authored route method",
            )
        })?;
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
    let step_id = next_authored_step_id(Some(&route_book));
    let mut insertion_assessment = None;
    for (index, replay_step_id) in method.step_ids.iter().enumerate() {
        let step = route_book
            .steps
            .iter()
            .find(|step| &step.id == replay_step_id)
            .expect("validated route method references existing steps");
        let RouteActionRef::Transition {
            transition_id: replay_transition_id,
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
            replay_transition_id,
            evidence_mode,
            &format!("route.insert-replay-{index:04}"),
        )?;
        if evaluation.assessment.classification != TransitionClassification::Executable {
            return Ok(PlannerServicePayload::RejectedRouteEdit {
                step_id: step_id.clone(),
                failed_step_id: replay_step_id.clone(),
                assessment: Box::new(evaluation.assessment),
                diagnostics: Box::new(evaluation.diagnostics),
                closest_before: Box::new(state.to_document()?),
            });
        }
        if index == insertion_index {
            let inserted = assess_and_apply_transition(
                &mut state,
                catalog,
                equivalence_sets,
                transition_id,
                evidence_mode,
                "route.insert",
            )?;
            if inserted.assessment.classification != TransitionClassification::Executable {
                return Ok(PlannerServicePayload::RejectedRouteEdit {
                    step_id: step_id.clone(),
                    failed_step_id: step_id,
                    assessment: Box::new(inserted.assessment),
                    diagnostics: Box::new(inserted.diagnostics),
                    closest_before: Box::new(state.to_document()?),
                });
            }
            insertion_assessment = Some(inserted.assessment);
        }
    }
    let assessment = insertion_assessment.expect("insertion anchor is in the authored method");
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
    let mut edited_method = method.clone();
    edited_method
        .step_ids
        .insert(insertion_index + 1, step_id.clone());
    let book = RouteBookEditBatch {
        schema: ROUTE_BOOK_EDIT_BATCH_SCHEMA.into(),
        expected_route_book_sha256: previous_route_book_sha256,
        edits: vec![
            RouteBookEdit::UpsertStep { step },
            RouteBookEdit::UpsertMethod {
                method: edited_method,
            },
        ],
    }
    .apply_composed(&route_book, catalog)?;
    let route_book_sha256 = book.digest()?;
    Ok(PlannerServicePayload::InsertedTransition {
        book: Box::new(book),
        previous_route_book_sha256,
        route_book_sha256,
        step_id,
        after_step_id: after_step_id.into(),
        transition_id: transition_id.into(),
        assessment: Box::new(assessment),
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
#[path = "service_tests.rs"]
mod tests;
