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
    BackingAttachment, ComponentBinding, ComponentKind, ComponentPayload, ComponentProvenance,
    EXECUTION_ENVIRONMENT_SCHEMA, ExecutionContext, ExecutionEnvironment, PhysicalSlotId,
    PlayerForm, PlayerState, ProvenanceSourceKind, RuntimeFile, RuntimeFileLifecycle,
    RuntimeFileOrigin, SceneLocation, SemanticLifetime, SerializationOwner, StateComponent,
    StateValue,
};
use dusklight_route_planner::transition::{
    ActivationContract, CandidateTransition, FeasibilityObligation, Goal, MECHANICS_CATALOG_SCHEMA,
    ObligationDetail, ObligationKind, Obstruction, StateOperation, TransitionKind,
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
                attention_position: None,
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
fn solve_response_projects_the_authoritative_plan_into_proof_regions() {
    let (state, catalog) = executable_transition_fixture();
    let response = handle_request(PlannerServiceRequest::Solve {
        request_id: "request.solve-proof".into(),
        state: Box::new(state),
        catalog: Box::new(catalog),
        equivalence_sets: Vec::new(),
        goal_id: "goal.boss-room".into(),
        options: RuntimeSolveOptions {
            max_depth: 8,
            max_states: 256,
            max_resolution_combinations: 64,
            max_plans: 4,
            feasibility_mode: crate::RuntimeFeasibilityMode::Modeled,
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
        },
        route_book: None,
    });
    let PlannerServiceOutcome::Ok { payload } = response.outcome else {
        panic!(
            "solve should return a proof projection: {}",
            serde_json::to_string(&response).unwrap()
        );
    };
    let PlannerServicePayload::SolveReport {
        report,
        proof_graph,
        proof_graph_sha256,
    } = *payload
    else {
        panic!("solve should include its planner graph");
    };
    assert_eq!(
        report.result.status,
        dusklight_route_planner::solver::SearchStatus::Reached
    );
    assert_eq!(proof_graph.digest().unwrap(), proof_graph_sha256);
    assert!(proof_graph.regions.iter().any(|region| {
        region.id == "region.proof.plan.primary"
            && region.region_kind == dusklight_route_planner::graph::PlannerRegionKind::Proof
    }));
    assert!(proof_graph.nodes.iter().any(|node| {
        matches!(
            node.payload,
            dusklight_route_planner::graph::PlannerNodePayload::ProofPlan { primary: true, .. }
        )
    }));

    let (mut already_reached, catalog) = executable_transition_fixture();
    already_reached.snapshot.environment.location.stage = "D_MN06".into();
    let response = handle_request(PlannerServiceRequest::Solve {
        request_id: "request.solve-zero-step-proof".into(),
        state: Box::new(already_reached),
        catalog: Box::new(catalog),
        equivalence_sets: Vec::new(),
        goal_id: "goal.boss-room".into(),
        options: RuntimeSolveOptions {
            max_depth: 8,
            max_states: 256,
            max_resolution_combinations: 64,
            max_plans: 4,
            feasibility_mode: crate::RuntimeFeasibilityMode::Modeled,
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
        },
        route_book: None,
    });
    let PlannerServiceOutcome::Ok { payload } = response.outcome else {
        panic!("an already reached goal should project a zero-step proof");
    };
    let PlannerServicePayload::SolveReport {
        report,
        proof_graph,
        ..
    } = *payload
    else {
        panic!("zero-step solve should retain the proof graph");
    };
    assert!(report.result.steps.is_empty());
    assert!(proof_graph.nodes.iter().any(|node| {
        matches!(
            node.payload,
            dusklight_route_planner::graph::PlannerNodePayload::ProofState { ordinal: 0, .. }
        )
    }));
}

#[test]
fn service_evaluates_then_applies_only_an_executable_transition() {
    let (state, catalog) = executable_transition_fixture();
    let response = handle_request(PlannerServiceRequest::EvaluateTransition {
        request_id: "request.transition".into(),
        state: Box::new(state.clone()),
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
        state: Box::new(state.clone()),
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

    let exact_scope = catalog.mechanics.transitions[0].scope.clone();
    let blank = RouteBook {
        schema: ROUTE_BOOK_SCHEMA.into(),
        manifest: RouteBookManifest {
            id: "route.blank-workspace".into(),
            version: "1.0.0".into(),
            label: "Blank workspace route".into(),
            author: "Route Planner".into(),
            source: "Workspace-authored exact transition sequence".into(),
            scope: exact_scope,
            refinement_stack_sha256: Some(catalog.refinement_stack.digest().unwrap()),
        },
        goal_ids: vec![catalog.mechanics.goals[0].id.clone()],
        constraints: Vec::new(),
        directives: Vec::new(),
        steps: Vec::new(),
        methods: Vec::new(),
        regions: Vec::new(),
        annotations: Vec::new(),
    };
    let from_blank = handle_request(PlannerServiceRequest::AppendTransition {
        request_id: "request.append-blank-workspace".into(),
        state: Box::new(state.clone()),
        catalog: Box::new(catalog.clone()),
        equivalence_sets: Vec::new(),
        route_book: Some(Box::new(blank)),
        route_book_id: "route.blank-workspace".into(),
        route_book_label: "Blank workspace route".into(),
        transition_id: "transition.enter-forest".into(),
        evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
    });
    let PlannerServiceOutcome::Ok { payload } = from_blank.outcome else {
        panic!("a blank workspace Route Book should accept its first transition");
    };
    let PlannerServicePayload::AppendedTransition {
        book,
        previous_route_book_sha256,
        ..
    } = *payload
    else {
        panic!("blank workspace append should return an authored Route Book");
    };
    assert!(previous_route_book_sha256.is_some());
    assert_eq!(book.methods[0].step_ids, ["step.route-0000"]);

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

    let inserted = handle_request(PlannerServiceRequest::InsertTransitionAfter {
        request_id: "request.insert-after-first".into(),
        state: Box::new(state.clone()),
        catalog: Box::new(catalog.clone()),
        equivalence_sets: Vec::new(),
        route_book: Box::new((*book).clone()),
        after_step_id: "step.route-0000".into(),
        transition_id: "transition.enter-boss".into(),
        evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
    });
    let PlannerServiceOutcome::Ok { payload } = inserted.outcome else {
        panic!("insertion after an authored step should replay downstream state");
    };
    let PlannerServicePayload::InsertedTransition {
        book: inserted_book,
        step_id,
        after_step_id,
        after,
        ..
    } = *payload
    else {
        panic!("insertion should return the edited route and propagated state");
    };
    assert_eq!(step_id, "step.route-0001");
    assert_eq!(after_step_id, "step.route-0000");
    assert_eq!(after.snapshot.environment.location.stage, "D_MN06");
    assert_eq!(
        inserted_book.methods[0].step_ids,
        ["step.route-0000", "step.route-0001"]
    );

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
fn theorycraft_component_overlay_is_exact_scoped_and_reversible() {
    let (mut state, base) = executable_transition_fixture();
    state.snapshot.environment.components.push(StateComponent {
        id: "component.stage-bank".into(),
        component_kind: ComponentKind::StageMemory,
        payload: ComponentPayload::Unknown {
            expected_bytes: Some(32),
        },
        binding: ComponentBinding::Stage {
            stage: "F_SP103".into(),
        },
        lifetime: SemanticLifetime::StageLoad,
        serialization_owner: SerializationOwner::StageBank {
            runtime_file_id: "file-0".into(),
            stage: "F_SP103".into(),
        },
        provenance: vec![ComponentProvenance {
            source_kind: ProvenanceSourceKind::TraceObservation,
            source_id: "source.stage-bank".into(),
            source_sha256: Some(Digest([7; 32])),
            transition_id: None,
        }],
    });

    let response = handle_request(PlannerServiceRequest::EditTheorycraftOverlays {
        request_id: "request.theorycraft-add".into(),
        base_catalog: Box::new(base.clone()),
        overlays: Vec::new(),
        state: Box::new(state.clone()),
        route_book: None,
        edit: TheorycraftOverlayEdit::AddComponentTransfer {
            pack_id: "what-if.stage-bank-rebind".into(),
            label: "Rebind stage bank to Forest Temple".into(),
            source_component_id: "component.stage-bank".into(),
            destination: ComponentTransferDestination::Rebind {
                binding: ComponentBinding::Stage {
                    stage: "D_MN05".into(),
                },
            },
        },
    });
    let PlannerServiceOutcome::Ok { payload } = response.outcome else {
        panic!("valid theorycraft edit should succeed");
    };
    let PlannerServicePayload::TheorycraftOverlaysEdited {
        base_catalog,
        overlays,
        catalog,
        added_pack: Some(pack),
        ..
    } = *payload
    else {
        panic!("theorycraft edit should return its reversible composition");
    };
    assert_eq!(pack.rules[0].evidence.truth, TruthStatus::Hypothetical);
    assert_eq!(pack.manifest.scope.selectors.len(), 1);
    assert_eq!(overlays, vec![(*pack).clone()]);
    assert_eq!(catalog.mechanics.techniques.len(), 1);
    assert_eq!(base_catalog.as_ref(), &base);

    let response = handle_request(PlannerServiceRequest::EditTheorycraftOverlays {
        request_id: "request.theorycraft-remove".into(),
        base_catalog,
        overlays,
        state: Box::new(state.clone()),
        route_book: None,
        edit: TheorycraftOverlayEdit::Remove {
            pack_id: "what-if.stage-bank-rebind".into(),
        },
    });
    let PlannerServiceOutcome::Ok { payload } = response.outcome else {
        panic!("removing a theorycraft edit should succeed");
    };
    let PlannerServicePayload::TheorycraftOverlaysEdited {
        overlays,
        catalog,
        removed_pack_ids,
        ..
    } = *payload
    else {
        panic!("removal should return the recomposed base catalog");
    };
    assert!(overlays.is_empty());
    assert_eq!(*catalog, base);
    assert_eq!(removed_pack_ids, ["what-if.stage-bank-rebind"]);

    let mut mechanics = base.mechanics.clone();
    let obstruction_evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![EvidenceRecord {
            id: "source.test-obstruction".into(),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(Digest([8; 32])),
            note: "Test obstruction.".into(),
        }],
    };
    mechanics.obligations.push(FeasibilityObligation {
        id: "obligation.test-door".into(),
        label: "Reach test door".into(),
        scope: mechanics.transitions[0].scope.clone(),
        obligation_kind: ObligationKind::Geometry,
        stage: dusklight_route_planner::transition::ObligationStage::Reach,
        detail: ObligationDetail::Unresolved {
            research_question: "Can the test door be approached?".into(),
        },
        evidence: obstruction_evidence.clone(),
    });
    mechanics.obstructions.push(Obstruction {
        id: "obstruction.test-door".into(),
        label: "Test door obstruction".into(),
        scope: mechanics.transitions[0].scope.clone(),
        blocked_action_id: mechanics.transitions[0].id.clone(),
        approach_id: mechanics.transitions[0].approach_id.clone(),
        active_when: PredicateExpression::True,
        obligation_ids: vec!["obligation.test-door".into()],
        evidence: obstruction_evidence,
    });
    mechanics
        .obstructions
        .sort_by(|left, right| left.id.cmp(&right.id));
    let obstructed_base = ComposedPlannerCatalog::compose(&base.facts, &mechanics, &[]).unwrap();
    let response = handle_request(PlannerServiceRequest::EditTheorycraftOverlays {
        request_id: "request.theorycraft-bypass".into(),
        base_catalog: Box::new(obstructed_base),
        overlays: Vec::new(),
        state: Box::new(state),
        route_book: None,
        edit: TheorycraftOverlayEdit::AddObstructionBypass {
            pack_id: "what-if.test-door-absent".into(),
            label: "Assume test door absent".into(),
            obstruction_id: "obstruction.test-door".into(),
        },
    });
    let PlannerServiceOutcome::Ok { payload } = response.outcome else {
        panic!("catalogued obstruction bypass should succeed");
    };
    let PlannerServicePayload::TheorycraftOverlaysEdited { catalog, .. } = *payload else {
        panic!("bypass should return a recomposed catalog");
    };
    assert_eq!(catalog.mechanics.resolvers.len(), 1);
    assert_eq!(
        catalog.mechanics.resolvers[0].obstruction_id,
        "obstruction.test-door"
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
