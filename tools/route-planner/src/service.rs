//! Typed request/response boundary for planner-owned editor and automation clients.

use crate::inspection::{StateInspection, inspect_state};
use crate::{
    RuntimeSolveOptions, SolveReport, solve_composed_catalog_goal, solve_composed_route_book_goal,
};
use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::execution::PlannerExecutionStateDocument;
use dusklight_route_planner::graph::PlannerGraph;
use dusklight_route_planner::identity::EquivalenceSet;
use dusklight_route_planner::logic::FactCatalog;
use dusklight_route_planner::refinement::{ComposedPlannerCatalog, RefinementPack};
use dusklight_route_planner::route_book::{RouteBook, RouteBookEditBatch};
use dusklight_route_planner::transition::MechanicsCatalog;
use serde::{Deserialize, Serialize};

pub const PLANNER_SERVICE_SCHEMA: &str = "dusklight.route-planner.service/v1";

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
    },
    ProjectGraph {
        request_id: String,
        catalog: Box<ComposedPlannerCatalog>,
        route_book: Option<Box<RouteBook>>,
    },
    InspectState {
        request_id: String,
        state: Box<PlannerExecutionStateDocument>,
        catalog: Box<ComposedPlannerCatalog>,
        equivalence_sets: Vec<EquivalenceSet>,
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
}

impl PlannerServiceRequest {
    pub fn request_id(&self) -> &str {
        match self {
            Self::ValidateRefinementPack { request_id, .. }
            | Self::ValidateRouteBook { request_id, .. }
            | Self::EditRouteBook { request_id, .. }
            | Self::Compose { request_id, .. }
            | Self::ProjectGraph { request_id, .. }
            | Self::InspectState { request_id, .. }
            | Self::Solve { request_id, .. } => request_id,
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
    StateInspection {
        inspection: Box<StateInspection>,
    },
    SolveReport {
        report: Box<SolveReport>,
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
            ..
        } => ComposedPlannerCatalog::compose(&facts, &mechanics, &packs).and_then(|catalog| {
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
mod tests {
    use super::*;
    use dusklight_route_planner::logic::FACT_CATALOG_SCHEMA;
    use dusklight_route_planner::transition::MECHANICS_CATALOG_SCHEMA;

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

    #[test]
    fn service_composes_then_projects_without_browser_or_huntctl_state() {
        let (facts, mechanics) = catalogs();
        let response = handle_request(PlannerServiceRequest::Compose {
            request_id: "request.compose".into(),
            facts: Box::new(facts),
            mechanics: Box::new(mechanics),
            packs: Vec::new(),
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
    fn malformed_catalog_error_keeps_request_identity() {
        let (facts, mut mechanics) = catalogs();
        mechanics.schema = "unsupported".into();
        let response = handle_request(PlannerServiceRequest::Compose {
            request_id: "request.bad".into(),
            facts: Box::new(facts),
            mechanics: Box::new(mechanics),
            packs: Vec::new(),
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
            },
        });
        assert!(matches!(
            response.outcome,
            PlannerServiceOutcome::Error { ref field, .. } if field == "schema"
        ));
    }
}
