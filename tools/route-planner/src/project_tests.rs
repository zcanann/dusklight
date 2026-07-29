use super::*;
use crate::RuntimeEvidenceMode;
use crate::context_compare::{ContextRelation, compare_semantic_contexts};
use crate::service::{
    ComponentTransferDestination, PlannerServiceOutcome, PlannerServicePayload,
    PlannerServiceRequest, TheorycraftOverlayEdit, handle_request,
};
use dusklight_route_planner::evaluation::EvidencePolicy;
use dusklight_route_planner::logic::TruthStatus;
use dusklight_route_planner::route_evidence_coverage::RouteEvidenceCoverageReport;
use dusklight_route_planner::route_observation::{
    ObservationArtifact, ObservationArtifactKind, PLANNED_EDGE_OBSERVATION_MANIFEST_SCHEMA,
    PlannedEdgeObservation, PlannedEdgeObservationManifest, RouteObservationMatchReport,
};
use dusklight_route_planner::route_observation_validation::{
    ComponentDisposition, RouteObservationValidationReport, VerificationStatus,
};
use dusklight_route_planner::route_suite_coverage::{RouteSuiteCoverageReport, RouteSuiteKind};
use dusklight_route_planner::state::{
    ComponentBinding, ComponentPayload, ExecutionContext, RuntimeFileOrigin, SceneLocation,
    StateValue,
};
use dusklight_route_planner::witness_promotion::{
    RequestedActionPromotion, RequestedWitness, WITNESS_PROMOTION_REQUEST_SCHEMA,
    WitnessPromotionPackMetadata, WitnessPromotionRequest, promote_witnessed_actions,
};
use std::time::{SystemTime, UNIX_EPOCH};

fn temporary_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "dusklight-route-project-{label}-{}-{nonce}",
        std::process::id()
    ))
}

#[test]
fn builtins_are_valid_read_only_planner_artifacts() {
    let root = temporary_root("builtins");
    let store = ProjectStore::open(&root).unwrap();
    let list = store.list().unwrap();
    assert_eq!(list.projects.len(), 6);
    assert!(list.projects.iter().all(|project| project.read_only));
    assert!(
        list.projects
            .iter()
            .any(|project| project.id == "demo-fanadi-return-place")
    );
    let fanadi = store.load("demo-fanadi-return-place").unwrap();
    assert!(fanadi.project.start_state.is_some());
    assert_eq!(fanadi.project.catalog.mechanics.goals.len(), 1);
    let immutable_error = store
        .save(
            "demo-fanadi-return-place",
            ProjectSaveRequest {
                schema: WEB_PROJECT_SAVE_SCHEMA.into(),
                expected_revision_sha256: Some(fanadi.revision_sha256),
                project: fanadi.project.clone(),
            },
        )
        .unwrap_err();
    assert!(immutable_error.to_string().contains("read-only"));
    let opening = store.load("demo-opening-flow").unwrap();
    assert!(opening.project.start_state.is_some());
    assert_eq!(opening.project.catalog.mechanics.goals.len(), 2);
    assert_eq!(
        opening
            .project
            .catalog
            .mechanics
            .goals
            .iter()
            .map(|goal| goal.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "goal.enter-opening-process",
            dusklight_route_planner::title_boundary::GZ2E01_UNSAVED_FILE_ZERO_GOAL_ID,
        ]
    );
    let keyed_door = store.load("demo-forest-keyed-door").unwrap();
    assert!(keyed_door.project.start_state.is_some());
    assert_eq!(keyed_door.project.catalog.mechanics.transitions.len(), 9);
    assert_eq!(keyed_door.project.catalog.mechanics.goals.len(), 1);
    let rebind = store.load("demo-hypothetical-local-bank-rebind").unwrap();
    assert_eq!(rebind.project.evidence_mode, RuntimeEvidenceMode::Research);
    assert_eq!(rebind.project.catalog.mechanics.transitions.len(), 2);
    let auru = store.load("demo-auru-recent-item-transfer").unwrap();
    assert_eq!(auru.project.evidence_mode, RuntimeEvidenceMode::Research);
    assert_eq!(auru.project.catalog.mechanics.transitions.len(), 4);
    let text_displacement = store.load("demo-text-displacement-goron-mines").unwrap();
    assert_eq!(
        text_displacement
            .project
            .catalog
            .mechanics
            .transitions
            .len(),
        8
    );
    assert_eq!(text_displacement.project.catalog.mechanics.readers.len(), 4);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn semantic_context_comparison_never_falls_back_to_a_nearby_language() {
    let root = temporary_root("semantic-context-comparison");
    let store = ProjectStore::open(&root).unwrap();
    let project = store
        .load("demo-text-displacement-goron-mines")
        .unwrap()
        .project;
    let left = project.start_state.clone().unwrap().into_state().unwrap();
    let mut right_document = project.start_state.unwrap();
    right_document
        .snapshot
        .environment
        .runtime_configuration
        .language = "fr".into();
    let right = right_document.into_state().unwrap();
    let report = compare_semantic_contexts(
        &left,
        &project.catalog,
        &[],
        &right,
        &project.catalog,
        &[],
        RuntimeEvidenceMode::EstablishedOnly,
    )
    .unwrap();

    assert_eq!(
        report.relation,
        ContextRelation::SameContentDifferentRuntime
    );
    assert!(!report.fallback_used);
    assert_eq!(report.left.runtime_configuration.language, "en");
    assert_eq!(report.right.runtime_configuration.language, "fr");
    assert!(report.summary.left_inapplicable_fact_ids.is_empty());
    assert_eq!(
        report.summary.right_inapplicable_fact_ids.len(),
        project.catalog.facts.aliases.len() + project.catalog.facts.derived_facts.len()
    );
    assert!(report.mechanics.iter().all(|row| {
        row.comparison == crate::context_compare::MechanicsComparisonKind::Equivalent
    }));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn text_displacement_demo_replays_each_raw_consumer_in_order() {
    let root = temporary_root("text-displacement");
    let store = ProjectStore::open(&root).unwrap();
    let project = store
        .load("demo-text-displacement-goron-mines")
        .unwrap()
        .project;
    let start = project.start_state.unwrap();
    for producer in [
        "transition.td-producer-auru",
        "transition.td-producer-coro",
        "transition.td-producer-ooccoo",
        "transition.td-producer-yeta",
    ] {
        let response = handle_request(PlannerServiceRequest::EvaluateTransition {
            request_id: format!("request.evaluate-{producer}"),
            state: Box::new(start.clone()),
            catalog: Box::new(project.catalog.clone()),
            equivalence_sets: Vec::new(),
            transition_id: producer.into(),
            evidence_mode: project.evidence_mode,
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("{producer} should be independently executable");
        };
        let PlannerServicePayload::TransitionEvaluation { assessment, .. } = *payload else {
            panic!("producer evaluation should return a typed assessment");
        };
        assert_eq!(
            assessment.classification,
            dusklight_route_planner::evaluation::TransitionClassification::Executable
        );
    }
    let mut route_book = None;
    let mut final_state = None;
    for transition_id in [
        "transition.td-producer-coro",
        "transition.enter-r-sp110-with-displaced-bit",
        "transition.gor-coron-flow6-b-to-c",
        "transition.gor-coron-flow9-prime-a",
        "transition.gor-coron-flow9-write-m029",
    ] {
        let response = handle_request(PlannerServiceRequest::AppendTransition {
            request_id: format!("request.{transition_id}"),
            state: Box::new(start.clone()),
            catalog: Box::new(project.catalog.clone()),
            equivalence_sets: Vec::new(),
            route_book,
            route_book_id: "route.text-displacement-demo".into(),
            route_book_label: "Text Displacement demo route".into(),
            transition_id: transition_id.into(),
            evidence_mode: project.evidence_mode,
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("{transition_id} should append after replaying its raw-bit prefix");
        };
        let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
            panic!("Text Displacement demo should append an ordinary transition");
        };
        final_state = Some(after);
        route_book = Some(book);
    }
    let final_state = final_state.unwrap();
    let persistent = final_state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "persistent.event-flags")
        .unwrap();
    let ComponentPayload::Raw { bytes, .. } = &persistent.payload else {
        panic!("persistent events should remain raw byte-backed state");
    };
    assert_ne!(bytes[7] & 0x04, 0);
    let response = handle_request(PlannerServiceRequest::RemoveAuthoredStep {
        request_id: "request.remove-text-displacement-producer".into(),
        state: Box::new(start),
        catalog: Box::new(project.catalog),
        equivalence_sets: Vec::new(),
        route_book: route_book.unwrap(),
        step_id: "step.route-0000".into(),
        evidence_mode: project.evidence_mode,
    });
    let PlannerServiceOutcome::Ok { payload } = response.outcome else {
        panic!("removing the producer should preserve the rejected edit witness");
    };
    let PlannerServicePayload::RejectedRouteEdit { failed_step_id, .. } = *payload else {
        panic!("the hall entry must require an actual displaced bit producer");
    };
    assert_eq!(failed_step_id, "step.route-0001");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn auru_demo_preserves_session_item_across_runtime_file_lifetimes() {
    let root = temporary_root("auru-recent-item");
    let store = ProjectStore::open(&root).unwrap();
    let project = store
        .load("demo-auru-recent-item-transfer")
        .unwrap()
        .project;
    let start = project.start_state.unwrap();
    let mut route_book = None;
    let mut final_state = None;
    for transition_id in [
        "transition.auru-demo-01-present-fishing-rod",
        "transition.auru-demo-02-begin-file-b",
        "transition.auru-demo-03-hypothetical-gcn-geometry",
        "transition.auru-demo-04-generic-get-item",
    ] {
        let response = handle_request(PlannerServiceRequest::AppendTransition {
            request_id: format!("request.{transition_id}"),
            state: Box::new(start.clone()),
            catalog: Box::new(project.catalog.clone()),
            equivalence_sets: project.equivalence_sets.clone(),
            route_book,
            route_book_id: "route.auru-recent-item-demo".into(),
            route_book_label: "Auru recent-item demo route".into(),
            transition_id: transition_id.into(),
            evidence_mode: project.evidence_mode,
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("{transition_id} should append after replaying its prefix");
        };
        let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
            panic!("Auru demo should return an appended transition");
        };
        final_state = Some(after);
        route_book = Some(book);
    }
    let final_state = final_state.unwrap();
    assert_ne!(
        final_state.snapshot.environment.active_runtime_file.id,
        "file-a"
    );
    let recent_item = final_state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "event.recent-item")
        .unwrap();
    assert!(matches!(
        recent_item.binding,
        ComponentBinding::Session { .. }
    ));
    let inventory = final_state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "inventory.active")
        .unwrap();
    let ComponentPayload::Structured { fields } = &inventory.payload else {
        panic!("inventory should remain structured");
    };
    let StateValue::Bytes(items) = &fields["owned_item_ids"] else {
        panic!("owned item set should remain byte-backed");
    };
    assert_ne!(items[0x4a / 8] & (1 << (0x4a % 8)), 0);
    let response = handle_request(PlannerServiceRequest::RemoveAuthoredStep {
        request_id: "request.remove-auru-producer".into(),
        state: Box::new(start),
        catalog: Box::new(project.catalog),
        equivalence_sets: project.equivalence_sets,
        route_book: route_book.unwrap(),
        step_id: "step.route-0000".into(),
        evidence_mode: project.evidence_mode,
    });
    let PlannerServiceOutcome::Ok { payload } = response.outcome else {
        panic!("removing the recent-item producer should return a typed rejection");
    };
    let PlannerServicePayload::RejectedRouteEdit { failed_step_id, .. } = *payload else {
        panic!("file B must not inherit a Fishing Rod that was never presented");
    };
    assert_eq!(failed_step_id, "step.route-0001");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hypothetical_rebind_demo_changes_binding_without_changing_payload() {
    let root = temporary_root("hypothetical-rebind");
    let store = ProjectStore::open(&root).unwrap();
    let project = store
        .load("demo-hypothetical-local-bank-rebind")
        .unwrap()
        .project;
    let start = project.start_state.unwrap();
    let append = |route_book, transition_id: &str| {
        handle_request(PlannerServiceRequest::AppendTransition {
            request_id: format!("request.{transition_id}"),
            state: Box::new(start.clone()),
            catalog: Box::new(project.catalog.clone()),
            equivalence_sets: project.equivalence_sets.clone(),
            route_book,
            route_book_id: "route.hypothetical-rebind-demo".into(),
            route_book_label: "Hypothetical rebind demo route".into(),
            transition_id: transition_id.into(),
            evidence_mode: project.evidence_mode,
        })
    };
    let response = append(None, "transition.hypothetical-local-bank-rebind");
    let PlannerServiceOutcome::Ok { payload } = response.outcome else {
        panic!("research mode should admit the explicit hypothetical rebind");
    };
    let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
        panic!("rebind should append as an ordinary typed transition step");
    };
    let first_after_snapshot = after.snapshot.clone();
    let before = start.clone().into_state().unwrap();
    let after_state = after.clone().into_state().unwrap();
    let before_bank = before
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "stage.local-bank")
        .unwrap();
    let after_bank = after_state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "stage.local-bank")
        .unwrap();
    assert_eq!(before_bank.payload, after_bank.payload);
    assert_ne!(before_bank.binding, after_bank.binding);
    assert_eq!(
        after_bank.binding,
        ComponentBinding::Stage {
            stage: "D_MN06".into()
        }
    );
    let response = append(Some(book), "transition.enter-temple-path");
    let PlannerServiceOutcome::Ok { payload } = response.outcome else {
        panic!("the rebound alias should authorize the unchanged Temple path");
    };
    let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
        panic!("Temple path should append after replaying the rebind");
    };
    assert_eq!(after.snapshot.environment.location.stage, "STAGE_B");
    let mut weak_facts = project.catalog.facts.clone();
    weak_facts
        .derived_facts
        .iter_mut()
        .find(|fact| fact.id == "path.tot-open")
        .unwrap()
        .evidence
        .truth = TruthStatus::Contested;
    let weak_catalog =
        ComposedPlannerCatalog::compose(&weak_facts, &project.catalog.mechanics, &[]).unwrap();
    let mut second_book = book.clone();
    second_book.manifest.id = "route.hypothetical-rebind-demo-copy".into();
    let coverage =
        RouteEvidenceCoverageReport::build(&weak_catalog, &[*book.clone(), *second_book], 2)
            .unwrap();
    assert_eq!(coverage.weak_high_usage_fact_ids, ["path.tot-open"]);
    assert_eq!(
        coverage
            .facts
            .iter()
            .find(|fact| fact.fact_id == "local.tot-switch")
            .unwrap()
            .route_book_ids
            .len(),
        2
    );
    let suite_coverage = RouteSuiteCoverageReport::build(
        &weak_catalog,
        &[(RouteSuiteKind::Hypothetical, *book.clone())],
    )
    .unwrap();
    assert_eq!(suite_coverage.suites.len(), 4);
    assert!(
        suite_coverage.suites[..3]
            .iter()
            .all(|suite| !suite.reported)
    );
    assert_eq!(
        suite_coverage.suites[3].exercised_fact_ids,
        ["local.forest-switch", "local.tot-switch", "path.tot-open"]
    );
    let trace = ObservationArtifact {
        id: "trace.hypothetical-rebind".into(),
        kind: ObservationArtifactKind::Trace,
        sha256: Digest([0x41; 32]),
    };
    let tape = ObservationArtifact {
        id: "tape.hypothetical-rebind".into(),
        kind: ObservationArtifactKind::Tape,
        sha256: Digest([0x42; 32]),
    };
    let manifest = PlannedEdgeObservationManifest {
        schema: PLANNED_EDGE_OBSERVATION_MANIFEST_SCHEMA.into(),
        artifacts: vec![tape, trace],
        observations: vec![
            PlannedEdgeObservation {
                id: "observation.rebind".into(),
                step_id: "step.route-0000".into(),
                trace_artifact_id: "trace.hypothetical-rebind".into(),
                tape_artifact_id: Some("tape.hypothetical-rebind".into()),
                before_snapshot_sha256: before.snapshot.digest().unwrap(),
                after_snapshot_sha256: first_after_snapshot.digest().unwrap(),
                start_tick: 10,
                end_tick: 20,
                start_tape_frame: Some(9),
                end_tape_frame: Some(19),
            },
            PlannedEdgeObservation {
                id: "observation.temple-path".into(),
                step_id: "step.route-0001".into(),
                trace_artifact_id: "trace.hypothetical-rebind".into(),
                tape_artifact_id: Some("tape.hypothetical-rebind".into()),
                before_snapshot_sha256: first_after_snapshot.digest().unwrap(),
                after_snapshot_sha256: after.snapshot.digest().unwrap(),
                start_tick: 21,
                end_tick: 30,
                start_tape_frame: Some(20),
                end_tape_frame: Some(29),
            },
        ],
    };
    let observation_snapshots = vec![
        before.snapshot.clone(),
        first_after_snapshot,
        after.snapshot.clone(),
    ];
    let observation_report =
        RouteObservationMatchReport::build(&weak_catalog, &book, &manifest, &observation_snapshots)
            .unwrap();
    assert!(observation_report.steps.iter().all(|step| step.observed));
    assert_eq!(observation_report.steps[1].observations[0].start_tick, 21);
    let validation = RouteObservationValidationReport::build(
        &weak_catalog,
        &book,
        &observation_report,
        &observation_snapshots,
        &project.equivalence_sets,
        EvidencePolicy::RESEARCH,
    )
    .unwrap();
    assert_eq!(validation.validations.len(), 2);
    assert!(validation.validations.iter().all(|row| {
        row.model_replay_status == VerificationStatus::Verified
            && row.snapshot_effects_status == VerificationStatus::Verified
            && row.component_preservation_status == VerificationStatus::Verified
    }));
    let local_bank = validation.validations[0]
        .component_checks
        .iter()
        .find(|check| check.component_id == "stage.local-bank")
        .unwrap();
    assert_eq!(
        local_bank.modeled_disposition,
        ComponentDisposition::Changed
    );
    assert!(local_bank.matches_model);
    let source_transition = weak_catalog
        .mechanics
        .transitions
        .iter()
        .find(|record| record.id == "transition.hypothetical-local-bank-rebind")
        .unwrap()
        .clone();
    let untouched_transition = weak_catalog
        .mechanics
        .transitions
        .iter()
        .find(|record| record.id == "transition.enter-temple-path")
        .unwrap()
        .clone();
    let promotion_request = WitnessPromotionRequest {
        schema: WITNESS_PROMOTION_REQUEST_SCHEMA.into(),
        pack: WitnessPromotionPackMetadata {
            id: "pack.witnessed-rebind".into(),
            version: "1.0.0".into(),
            author: "Dusklight regression".into(),
            source: "Validated hypothetical rebind observation".into(),
            precedence: 100,
            conflicts: Vec::new(),
        },
        promotions: vec![RequestedActionPromotion {
            action: book.steps[0].action.clone(),
            promotion_rule_id: "rule.promote-rebind".into(),
            replacement_rule_id: "rule.replace-rebind-evidence".into(),
            witnesses: vec![RequestedWitness {
                observation_id: "observation.rebind".into(),
                evidence_id: "evidence.witnessed-rebind".into(),
            }],
        }],
    };
    let (promotion_pack, promotion_receipt) =
        promote_witnessed_actions(&weak_catalog, &validation, &promotion_request).unwrap();
    assert_eq!(
        promotion_receipt.action_ids_before,
        promotion_receipt.action_ids_after
    );
    let promoted_catalog = ComposedPlannerCatalog::compose(
        &weak_catalog.facts,
        &weak_catalog.mechanics,
        &[promotion_pack],
    )
    .unwrap();
    let promoted_transition = promoted_catalog
        .mechanics
        .transitions
        .iter()
        .find(|record| record.id == source_transition.id)
        .unwrap();
    assert_eq!(promoted_transition.evidence.truth, TruthStatus::Established);
    assert!(source_transition.evidence.records.iter().all(|prior| {
        promoted_transition
            .evidence
            .records
            .iter()
            .any(|record| record == prior)
    }));
    assert!(promoted_transition.evidence.records.iter().any(|record| {
        record.id == "evidence.witnessed-rebind"
            && record.kind == dusklight_route_planner::logic::EvidenceKind::RouteWitnessed
    }));
    assert_eq!(
        promoted_catalog
            .mechanics
            .transitions
            .iter()
            .find(|record| record.id == untouched_transition.id)
            .unwrap(),
        &untouched_transition
    );
    let response = handle_request(PlannerServiceRequest::RemoveAuthoredStep {
        request_id: "request.remove-hypothetical-rebind".into(),
        state: Box::new(start),
        catalog: Box::new(project.catalog),
        equivalence_sets: project.equivalence_sets,
        route_book: book,
        step_id: "step.route-0000".into(),
        evidence_mode: project.evidence_mode,
    });
    let PlannerServiceOutcome::Ok { payload } = response.outcome else {
        panic!("removing the rebind should return the broken downstream join");
    };
    let PlannerServicePayload::RejectedRouteEdit { failed_step_id, .. } = *payload else {
        panic!("the Temple path must remain causally dependent on the rebind");
    };
    assert_eq!(failed_step_id, "step.route-0001");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn keyed_door_demo_replays_every_audited_actor_phase() {
    let root = temporary_root("keyed-door-propagation");
    let store = ProjectStore::open(&root).unwrap();
    let project = store.load("demo-forest-keyed-door").unwrap().project;
    let start = project.start_state.unwrap();
    let mut route_book = None;
    let mut final_state = None;
    for transition_id in [
        "transition.gz2e01-door1-01-offer-event",
        "transition.gz2e01-door1-02-demo-action8",
        "transition.gz2e01-door1-03-finish-keyhole",
        "transition.gz2e01-door1-04-flush-key-delta",
        "transition.gz2e01-door1-05-open-init",
        "transition.gz2e01-door1-06-open-proc",
        "transition.gz2e01-door1-07-cross-room-adjacency",
        "transition.gz2e01-door1-08-close-init",
        "transition.gz2e01-door1-09-close-end",
    ] {
        let response = handle_request(PlannerServiceRequest::AppendTransition {
            request_id: format!("request.{transition_id}"),
            state: Box::new(start.clone()),
            catalog: Box::new(project.catalog.clone()),
            equivalence_sets: project.equivalence_sets.clone(),
            route_book,
            route_book_id: "route.keyed-door-demo".into(),
            route_book_label: "Forest keyed-door demo route".into(),
            transition_id: transition_id.into(),
            evidence_mode: RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("{transition_id} should append after replaying its exact prefix");
        };
        let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
            panic!("keyed-door demo should return an appended transition");
        };
        route_book = Some(book);
        final_state = Some(after);
    }
    let final_state = final_state.unwrap();
    assert_eq!(final_state.snapshot.environment.location.stage, "D_MN05");
    assert_eq!(final_state.snapshot.environment.location.room, 2);
    let dungeon = final_state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "dungeon.d-mn05-memory")
        .unwrap();
    let ComponentPayload::Structured { fields } = &dungeon.payload else {
        panic!("dungeon memory should remain structured");
    };
    assert_eq!(fields["small_keys"], StateValue::Unsigned(0));
    assert_eq!(fields["switch_0b"], StateValue::Boolean(true));
    let route_book = route_book.unwrap();
    assert_eq!(route_book.methods[0].step_ids.len(), 9);
    let response = handle_request(PlannerServiceRequest::RemoveAuthoredStep {
        request_id: "request.remove-keyed-action8".into(),
        state: Box::new(start),
        catalog: Box::new(project.catalog),
        equivalence_sets: project.equivalence_sets,
        route_book,
        step_id: "step.route-0001".into(),
        evidence_mode: RuntimeEvidenceMode::EstablishedOnly,
    });
    let PlannerServiceOutcome::Ok { payload } = response.outcome else {
        panic!("removing the unlock producer should return a typed rejection");
    };
    let PlannerServicePayload::RejectedRouteEdit {
        failed_step_id,
        assessment,
        ..
    } = *payload
    else {
        panic!("the keyed-door continuation should reject without action 8");
    };
    assert_eq!(failed_step_id, "step.route-0002");
    assert_eq!(
        assessment.classification,
        dusklight_route_planner::evaluation::TransitionClassification::GuardBlocked
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fanadi_demo_savewarp_propagates_from_its_exact_start_state() {
    let root = temporary_root("fanadi-propagation");
    let store = ProjectStore::open(&root).unwrap();
    let record = store.load("demo-fanadi-return-place").unwrap();
    let project = record.project;
    let response = handle_request(PlannerServiceRequest::AppendTransition {
        request_id: "request.fanadi-savewarp".into(),
        state: Box::new(project.start_state.unwrap()),
        catalog: Box::new(project.catalog),
        equivalence_sets: project.equivalence_sets,
        route_book: None,
        route_book_id: "route.fanadi-demo".into(),
        route_book_label: "Fanadi demo route".into(),
        transition_id: "transition.savewarp.from-player-return-place".into(),
        evidence_mode: RuntimeEvidenceMode::EstablishedOnly,
    });
    let PlannerServiceOutcome::Ok { payload } = response.outcome else {
        panic!("Fanadi demo savewarp should be executable from its checked start state");
    };
    let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
        panic!("Fanadi demo should append one propagated transition");
    };
    assert_eq!(after.snapshot.environment.location.stage, "R_SP107");
    assert_eq!(after.snapshot.environment.location.room, 3);
    assert_eq!(after.snapshot.environment.location.layer, -1);
    assert_eq!(after.snapshot.environment.location.spawn, 1);
    assert_eq!(book.methods[0].step_ids, ["step.route-0000"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn opening_demo_reset_propagates_into_the_pending_opening_process() {
    let root = temporary_root("opening-propagation");
    let store = ProjectStore::open(&root).unwrap();
    let record = store.load("demo-opening-flow").unwrap();
    let project = record.project;
    let response = handle_request(PlannerServiceRequest::AppendTransition {
        request_id: "request.opening-reset".into(),
        state: Box::new(project.start_state.unwrap()),
        catalog: Box::new(project.catalog),
        equivalence_sets: project.equivalence_sets,
        route_book: None,
        route_book_id: "route.opening-demo".into(),
        route_book_label: "Opening demo route".into(),
        transition_id: "transition.gz2e01.reset-to-opening".into(),
        evidence_mode: RuntimeEvidenceMode::EstablishedOnly,
    });
    let PlannerServiceOutcome::Ok { payload } = response.outcome else {
        panic!("opening reset should be executable from its checked start state");
    };
    let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
        panic!("opening demo should append one propagated transition");
    };
    assert_eq!(
        after.snapshot.environment.execution_context,
        ExecutionContext::Process {
            process_name: "PROC_OPENING_SCENE".into(),
            pending_world_load: Some(SceneLocation {
                stage: "F_SP102".into(),
                room: 0,
                layer: 10,
                spawn: 100,
            }),
        }
    );
    let restart = after
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "restart")
        .unwrap();
    let ComponentPayload::Structured { fields } = &restart.payload else {
        panic!("opening restart component should remain structured");
    };
    assert_eq!(fields["room_param"], StateValue::Unsigned(0));
    assert_eq!(book.methods[0].step_ids, ["step.route-0000"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn opening_demo_replays_through_blank_slot_selection() {
    let root = temporary_root("opening-file-select-propagation");
    let store = ProjectStore::open(&root).unwrap();
    let project = store.load("demo-opening-flow").unwrap().project;
    let start = project.start_state.unwrap();
    let mut route_book = None;
    let mut final_state = None;
    for transition_id in [
        "transition.gz2e01.reset-to-opening",
        "transition.gz2e01.observe-opening-phase-4",
        "transition.gz2e01.opening-enter-and-initialize-file0",
        "transition.gz2e01.title-key-accept",
        "transition.gz2e01.title-request-name-scene",
        "transition.gz2e01.observe-name-scene-create",
        "transition.gz2e01.name-scene-file-select-initialize",
        "transition.gz2e01.file-select-focus-blank-slot-1",
        "transition.gz2e01.file-select-blank-slot-1",
    ] {
        let response = handle_request(PlannerServiceRequest::AppendTransition {
            request_id: format!("request.{transition_id}"),
            state: Box::new(start.clone()),
            catalog: Box::new(project.catalog.clone()),
            equivalence_sets: project.equivalence_sets.clone(),
            route_book,
            route_book_id: "route.opening-file-select-demo".into(),
            route_book_label: "Opening file-select demo route".into(),
            transition_id: transition_id.into(),
            evidence_mode: project.evidence_mode,
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("{transition_id} should append after replaying its exact prefix");
        };
        let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
            panic!("opening demo should return an appended transition");
        };
        final_state = Some(after);
        route_book = Some(book);
    }
    let final_state = final_state.unwrap();
    assert_eq!(
        final_state.snapshot.environment.execution_context,
        ExecutionContext::Process {
            process_name: "PROC_NAME_SCENE".into(),
            pending_world_load: None,
        }
    );
    assert_eq!(
        final_state.snapshot.environment.active_runtime_file.origin,
        RuntimeFileOrigin::TitleFile0
    );
    let fields = |component_id: &str| {
        let component = final_state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == component_id)
            .unwrap();
        let ComponentPayload::Structured { fields } = &component.payload else {
            panic!("{component_id} should remain structured");
        };
        fields
    };
    assert_eq!(
        fields("runtime-file.header")["new_file_raw"],
        StateValue::Unsigned(128)
    );
    assert_eq!(
        fields("runtime-file.header")["data_num_raw"],
        StateValue::Unsigned(0)
    );
    assert_eq!(
        fields("name-scene-control")["phase"],
        StateValue::Text("name_entry".into())
    );
    assert_eq!(route_book.unwrap().methods[0].step_ids.len(), 9);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn presentation_regions_group_nodes_without_changing_the_planner_graph() {
    let root = temporary_root("presentation-region");
    let store = ProjectStore::open(&root).unwrap();
    let mut project = store.load("demo-forest-keyed-door").unwrap().project;
    let graph = PlannerGraph::project_composed(&project.catalog).unwrap();
    let transition_node = graph
        .nodes
        .iter()
        .find(|node| {
            matches!(
                &node.payload,
                dusklight_route_planner::graph::PlannerNodePayload::Transition {
                    transition_id,
                    ..
                } if transition_id == "transition.gz2e01-door1-09-close-end"
            )
        })
        .unwrap()
        .id
        .clone();
    let graph_sha256 = graph.digest().unwrap();
    project.presentation.regions.push(PresentationRegion {
        id: "region.presentation-shutter-close".into(),
        label: "Shutter close".into(),
        parent_region_id: None,
        version: 1,
        snapshot_node_ids: Vec::new(),
        derivation: None,
    });
    project
        .presentation
        .node_region_ids
        .insert(transition_node, "region.presentation-shutter-close".into());
    project.presentation.regions.push(PresentationRegion {
        id: "region.presentation-shutter-close-reference".into(),
        label: "Shutter close reference".into(),
        parent_region_id: None,
        version: 1,
        snapshot_node_ids: Vec::new(),
        derivation: Some(PresentationRegionDerivation {
            kind: PresentationRegionDerivationKind::Reference,
            source_region_id: "region.presentation-shutter-close".into(),
            source_version: 1,
        }),
    });
    project.validate().unwrap();
    let decoded: PlannerWebProject =
        serde_json::from_slice(&project.canonical_bytes().unwrap()).unwrap();
    assert_eq!(decoded.presentation, project.presentation);
    assert_eq!(
        PlannerGraph::project_composed(&decoded.catalog)
            .unwrap()
            .digest()
            .unwrap(),
        graph_sha256
    );
    let mut legacy = decoded.clone();
    legacy.schema = LEGACY_WEB_PROJECT_SCHEMAS[1].into();
    legacy.id = "legacy-presentation-region".into();
    fs::write(
        root.join("legacy-presentation-region.json"),
        serde_json::to_vec_pretty(&legacy).unwrap(),
    )
    .unwrap();
    assert_eq!(
        store
            .load("legacy-presentation-region")
            .unwrap()
            .project
            .schema,
        WEB_PROJECT_SCHEMA
    );

    project.presentation.regions[0].parent_region_id =
        Some("region.presentation-shutter-close".into());
    assert!(
        project
            .validate()
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn theorycraft_overlay_base_and_pack_survive_save_and_reload() {
    let root = temporary_root("theorycraft-save");
    let store = ProjectStore::open(&root).unwrap();
    let mut project = builtin_projects()
        .unwrap()
        .into_iter()
        .find(|project| project.id == "demo-hypothetical-local-bank-rebind")
        .unwrap();
    project.id = "theorycraft-save".into();
    project.label = "Theorycraft save".into();
    let base = project.catalog.clone();
    let state = project.start_state.clone().unwrap();
    let source = state.snapshot.environment.components[0].id.clone();
    let response = handle_request(PlannerServiceRequest::EditTheorycraftOverlays {
        request_id: "project.theorycraft-save".into(),
        base_catalog: Box::new(base.clone()),
        overlays: Vec::new(),
        state: Box::new(state),
        route_book: None,
        edit: TheorycraftOverlayEdit::AddComponentTransfer {
            pack_id: "what-if.saved-rebind".into(),
            label: "Saved exact-context rebind".into(),
            source_component_id: source,
            destination: ComponentTransferDestination::Rebind {
                binding: ComponentBinding::Stage {
                    stage: "D_MN06".into(),
                },
            },
        },
    });
    let PlannerServiceOutcome::Ok { payload } = response.outcome else {
        panic!("theorycraft overlay should compose");
    };
    let PlannerServicePayload::TheorycraftOverlaysEdited {
        base_catalog,
        overlays,
        catalog,
        ..
    } = *payload
    else {
        panic!("theorycraft edit should return persisted ingredients");
    };
    project.catalog = *catalog;
    project.theorycraft_base_catalog = Some(base_catalog);
    project.theorycraft_overlays = overlays;
    project.validate().unwrap();
    let created = store
        .save(
            &project.id.clone(),
            ProjectSaveRequest {
                schema: WEB_PROJECT_SAVE_SCHEMA.into(),
                expected_revision_sha256: None,
                project,
            },
        )
        .unwrap();
    let reloaded = store.load("theorycraft-save").unwrap().project;
    assert_eq!(reloaded, created.project);
    assert_eq!(reloaded.theorycraft_overlays.len(), 1);
    assert_eq!(
        reloaded.theorycraft_base_catalog.as_ref().unwrap().as_ref(),
        &base
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn save_is_atomic_revision_checked_and_path_confined() {
    let root = temporary_root("save");
    let store = ProjectStore::open(&root).unwrap();
    let mut project = PlannerWebProject::blank("my-route", "My route").unwrap();
    let created = store
        .save(
            "my-route",
            ProjectSaveRequest {
                schema: WEB_PROJECT_SAVE_SCHEMA.into(),
                expected_revision_sha256: None,
                project: project.clone(),
            },
        )
        .unwrap();
    assert_eq!(store.load("my-route").unwrap(), created);
    assert!(
        store
            .save(
                "my-route",
                ProjectSaveRequest {
                    schema: WEB_PROJECT_SAVE_SCHEMA.into(),
                    expected_revision_sha256: None,
                    project: project.clone(),
                },
            )
            .unwrap_err()
            .to_string()
            .contains("revision conflict")
    );
    project.label = "Renamed route".into();
    let updated = store
        .save(
            "my-route",
            ProjectSaveRequest {
                schema: WEB_PROJECT_SAVE_SCHEMA.into(),
                expected_revision_sha256: Some(created.revision_sha256),
                project,
            },
        )
        .unwrap();
    assert_ne!(updated.revision_sha256, created.revision_sha256);
    assert!(store.load("../escape").is_err());
    assert!(!root.parent().unwrap().join("escape.json").exists());
    fs::remove_dir_all(root).unwrap();
}
