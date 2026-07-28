use super::*;
use crate::stage_survey::{
    STAGE_SURVEY_FIDELITY, StageSurveyAttempt, StageSurveyIdentity, StageSurveyObservedOrigin,
    StageSurveyPolicy, StageSurveyProbeKind,
};
use crate::stage_survey_artifact::compact_survey_artifact;
use dusklight_world::stage_boot_catalog::{
    BootLayerSource, BootLayerSourceKind, BootPointSource, BootPointSourceKind,
    STAGE_BOOT_CATALOG_SCHEMA, StageBootCandidate, StageCatalogStatus, StageInventoryStatus,
};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMPORARY_ROOT: AtomicU64 = AtomicU64::new(0);

fn digest(byte: u8) -> Digest {
    Digest([byte; 32])
}

fn catalog() -> StageBootCatalog {
    StageBootCatalog {
        schema: STAGE_BOOT_CATALOG_SCHEMA.into(),
        known_loader_sha256: None,
        stages: vec![StageCatalogStatus {
            stage: "F_SP103".into(),
            resources_present: true,
            inventory_status: StageInventoryStatus::Complete,
            inventory_sha256: Some(digest(7)),
            diagnostic: None,
            room_count: 1,
            player_spawn_count: 1,
            candidate_count: 1,
        }],
        candidates: vec![StageBootCandidate {
            id: "F_SP103/room/0/point/0/layer/-1".into(),
            stage: "F_SP103".into(),
            room: 0,
            point: 0,
            layer: -1,
            point_sources: vec![BootPointSource {
                kind: BootPointSourceKind::RetailPlayerSpawn,
                stable_id: Some("spawn-0".into()),
            }],
            layer_sources: vec![BootLayerSource {
                kind: BootLayerSourceKind::ResolvedDefault,
                chunk_tag: None,
            }],
        }],
    }
}

fn temporary_root() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = NEXT_TEMPORARY_ROOT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "stage-actor-coverage-{}-{nonce}-{sequence}",
        std::process::id()
    ))
}

fn fixture_with_learning_generation(
    first_learning_generation: u64,
) -> (StageBootCatalog, StageSurveyLedger, PathBuf) {
    let catalog = catalog();
    let mut ledger = StageSurveyLedger::new(
        &catalog,
        StageSurveyIdentity {
            catalog_sha256: catalog.digest().unwrap(),
            executable_sha256: digest(1),
            game_data_sha256: digest(2),
            card_fixture_sha256: digest(3),
            observation_schema_sha256: digest(4),
            settings_sha256: digest(5),
        },
        StageSurveyPolicy {
            probe_ticks: 30,
            probe: StageSurveyProbeKind::Neutral,
            native_stage_readiness_ticks: 30 * 60,
            host_timeout_millis: 1_000,
            maximum_attempts_per_case: 1,
            fidelity_profile: STAGE_SURVEY_FIDELITY.into(),
        },
    )
    .unwrap();
    let catalog_actors = vec![
        json!({"process_id": 4, "parent_process_id": 4294967295_u32,
                "actor_type": 1, "process_subtype": 2, "parameters": 1, "status": 2,
                "condition": 3, "actor_name": 253, "profile_name": 253,
                "symbolic_name": "fpcNm_ALINK_e", "set_id": 0, "health": 10,
                "home_room": 0, "old_room": 0, "current_room": 0, "group": 1,
                "argument": 0, "pause_flag": 0, "process_init_state": 1,
                "process_create_phase": 2, "cull_type": 3, "demo_actor_id": 4,
                "carry_type": 5, "heap_present": true, "model_present": true,
                "joint_collision_present": false, "home_position": [1.0, 2.0, 3.0],
                "old_position": [3.0, 4.0, 5.0], "current_position": [4.0, 5.0, 6.0],
                "scale": [1.0, 1.0, 1.0], "gravity": -1.0, "max_fall_speed": -20.0,
                "eye_position": [4.0, 7.0, 6.0], "home_angle": [1, 2, 3],
                "old_angle": [4, 5, 6], "is_enemy": false, "enemy_base": null,
                "trigger_volume": {"kind": "scene_exit", "shape": "box",
                    "enabled": true, "vertical_unbounded": false, "behavior": 0,
                    "center": [1.0, 2.0, 3.0], "half_extent": [4.0, 5.0, 6.0],
                    "yaw": 7}}),
        json!({"process_id": 8, "parent_process_id": 4, "actor_type": 6,
                "process_subtype": 7, "parameters": 3, "status": 4, "condition": 5,
                "actor_name": 291, "profile_name": 291, "symbolic_name": "fpcNm_NPC_e",
                "set_id": 1, "health": 5, "home_room": 0, "old_room": 0,
                "current_room": 0, "group": 2, "argument": -1, "pause_flag": 1,
                "process_init_state": 2, "process_create_phase": 3, "cull_type": 4,
                "demo_actor_id": 5, "carry_type": 6, "heap_present": false,
                "model_present": true, "joint_collision_present": true,
                "home_position": [7.0, 8.0, 9.0], "old_position": [9.0, 10.0, 11.0],
                "current_position": [10.0, 11.0, 12.0], "scale": [2.0, 2.0, 2.0],
                "gravity": -2.0, "max_fall_speed": -30.0,
                "eye_position": [10.0, 13.0, 12.0], "home_angle": [7, 8, 9],
                "old_angle": [10, 11, 12], "is_enemy": true,
                "enemy_base": {"flags": 137, "throw_mode": 4,
                    "down_position": [12.0, 3.5, -7.5],
                    "head_lock_position": [12.5, 7.0, -8.0]}}),
    ];
    let learning_actors = vec![
        json!({"runtime_generation": first_learning_generation,
                "parent_runtime_generation": 4294967295_u32, "actor_type": 1,
                "process_subtype": 2, "parameters": 1, "status": 2, "condition": 3,
                "actor_name": 253, "profile_name": 253, "set_id": 0, "health": 10,
                "home_room": 0, "old_room": 0, "current_room": 0, "group": 1,
                "argument": 0, "pause_flag": 0, "process_init_state": 1,
                "process_create_phase": 2, "cull_type": 3, "demo_actor_id": 4,
                "carry_type": 5, "heap_present": true, "model_present": true,
                "joint_collision_present": false, "home_position": [1.0, 2.0, 3.0],
                "old_position": [3.0, 4.0, 5.0], "current_position": [4.0, 5.0, 6.0],
                "velocity": [0.5, 0.0, 1.0], "forward_speed": 1.25,
                "scale": [1.0, 1.0, 1.0], "gravity": -1.0, "max_fall_speed": -20.0,
                "eye_position": [4.0, 7.0, 6.0], "home_angle": [1, 2, 3],
                "old_angle": [4, 5, 6], "current_angle": [5, 6, 7],
                "shape_angle": [6, 7, 8], "attention": null,
                "event_participation": null, "return_place_writer": null,
                "enemy_base": null,
                "trigger_volume": {"kind": "scene_exit", "shape": "box",
                    "enabled": true, "vertical_unbounded": false, "behavior": 0,
                    "center": [1.0, 2.0, 3.0], "half_extent": [4.0, 5.0, 6.0],
                    "yaw": 7}}),
        json!({"runtime_generation": 8, "parent_runtime_generation": 4,
                "actor_type": 6, "process_subtype": 7, "parameters": 3, "status": 4,
                "condition": 5, "actor_name": 291, "profile_name": 291, "set_id": 1,
                "health": 5, "home_room": 0, "old_room": 0, "current_room": 0,
                "group": 2, "argument": -1, "pause_flag": 1, "process_init_state": 2,
                "process_create_phase": 3, "cull_type": 4, "demo_actor_id": 5,
                "carry_type": 6, "heap_present": false, "model_present": true,
                "joint_collision_present": true, "home_position": [7.0, 8.0, 9.0],
                "old_position": [9.0, 10.0, 11.0], "current_position": [10.0, 11.0, 12.0],
                "velocity": [1.5, 2.0, 3.0], "forward_speed": 4.25,
                "scale": [2.0, 2.0, 2.0], "gravity": -2.0, "max_fall_speed": -30.0,
                "eye_position": [10.0, 13.0, 12.0], "home_angle": [7, 8, 9],
                "old_angle": [10, 11, 12], "current_angle": [11, 12, 13],
                "shape_angle": [12, 13, 14],
                "attention": {"flags": 3, "position": [10.0, 13.0, 12.0],
                    "distance_indices": [0,1,2,3,4,5,6,7,8], "auxiliary": 2},
                "event_participation": {"command": 1, "condition": 2,
                    "event_id": 3, "map_tool_id": 4, "index": 5},
                "return_place_writer": null,
                "enemy_base": {"flags": 137, "throw_mode": 4,
                    "down_position": [12.0, 3.5, -7.5],
                    "head_lock_position": [12.5, 7.0, -8.0]}}),
    ];
    let actor_bytes = serde_json::to_vec_pretty(&json!({
        "schema": "dusklight.actor-catalog.v10", "simulation_tick": 29,
        "stage": "F_SP103", "room": 0, "layer": 0, "observed_actor_count": 2,
        "retained_actor_count": 2, "truncated": false, "actors": catalog_actors,
        "learning_actor_population": {
            "source_schema": LEARNING_OBSERVATION_SCHEMA_V27,
            "observed_actor_count": 2, "retained_actor_count": 2,
            "truncated": false, "actors": learning_actors
        }
    }))
    .unwrap();
    let actor_digest = Digest(Sha256::digest(&actor_bytes).into());
    ledger
        .record_attempt(
            &catalog,
            &catalog.candidates[0].id,
            StageSurveyAttempt {
                number: 1,
                outcome: StageSurveyAttemptOutcome::Ready,
                exit_code: Some(0),
                elapsed_millis: 20,
                observation_sha256: Some(digest(8)),
                actor_catalog_sha256: Some(actor_digest),
                observed_actor_count: Some(2),
                retained_actor_count: Some(2),
                actor_catalog_truncated: Some(false),
                state_sequence_sha256: Some(digest(9)),
                observed_origin: Some(StageSurveyObservedOrigin {
                    stage: Some("F_SP103".into()),
                    room: 0,
                    point: 0,
                    layer: 0,
                    player_ready: true,
                }),
                observed_final: Some(StageSurveyObservedOrigin {
                    stage: Some("F_SP103".into()),
                    room: 0,
                    point: 0,
                    layer: 0,
                    player_ready: true,
                }),
                diagnostic_code: None,
            },
        )
        .unwrap();
    let root = temporary_root();
    let artifact_root = root
        .join("cases")
        .join(stage_survey_case_storage_id(&catalog.candidates[0].id).to_string())
        .join("attempt-001-run-00000");
    fs::create_dir_all(&artifact_root).unwrap();
    fs::write(artifact_root.join("actors.json"), actor_bytes).unwrap();
    (catalog, ledger, root)
}

fn fixture() -> (StageBootCatalog, StageSurveyLedger, PathBuf) {
    fixture_with_learning_generation(4)
}

#[test]
fn aggregates_verified_complete_actor_snapshots_by_stage_and_profile() {
    let (catalog, ledger, root) = fixture();
    let report = StageActorCoverageReport::build(&catalog, &ledger, &root).unwrap();
    assert_eq!(report.ready_case_count, 1);
    assert_eq!(report.verified_case_count, 1);
    assert_eq!(report.rejected_case_count, 0);
    assert_eq!(report.profiles.len(), 2);
    assert_eq!(report.cases[0].enemy_actor_count, 1);
    assert_eq!(report.stages[0].actor_instance_count, 2);
    let link = report
        .profiles
        .iter()
        .find(|profile| profile.profile_name == 253)
        .unwrap();
    assert!(link.fields.len() > 30);
    assert_eq!(
        link.fields
            .iter()
            .find(|field| field.path == "current_position[0]")
            .unwrap(),
        &StageActorProfileFieldCoverage {
            path: "current_position[0]".into(),
            status: StageActorFieldCoverageStatus::Present,
            sampled_actors: 1,
            missing_actors: 0,
            value_samples: 1,
            null_samples: 0,
            true_samples: 0,
            distinct_nonnull_values: 1,
        }
    );
    let trigger_enabled = link
        .fields
        .iter()
        .find(|field| field.path == "trigger_volume.enabled")
        .unwrap();
    assert_eq!(
        trigger_enabled.status,
        StageActorFieldCoverageStatus::Present
    );
    assert_eq!(trigger_enabled.true_samples, 1);
    assert_eq!(link.stages.len(), 1);
    assert_eq!(link.stages[0].stage, "F_SP103");
    assert_eq!(link.stages[0].verified_case_count, 1);
    assert_eq!(link.stages[0].actor_instance_count, 1);
    assert_eq!(
        link.stages[0]
            .fields
            .iter()
            .find(|field| field.path == "current_position[0]")
            .unwrap()
            .value_samples,
        1
    );
    assert_ne!(report.report_sha256, Digest::ZERO);
    assert!(report.canonical_bytes().unwrap().ends_with(b"\n"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn compressed_actor_artifact_reproduces_the_same_coverage_report() {
    let (catalog, ledger, root) = fixture();
    let raw_report = StageActorCoverageReport::build(&catalog, &ledger, &root).unwrap();
    let actor_path = root
        .join("cases")
        .join(stage_survey_case_storage_id(&catalog.candidates[0].id).to_string())
        .join("attempt-001-run-00000")
        .join("actors.json");
    let expected_digest = ledger.cases[0].attempts[0].actor_catalog_sha256.unwrap();
    assert!(compact_survey_artifact(&actor_path, expected_digest).unwrap());

    let compressed_report = StageActorCoverageReport::build(&catalog, &ledger, &root).unwrap();
    assert_eq!(compressed_report, raw_report);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_artifact_remains_explicit_instead_of_becoming_zero_actors() {
    let (catalog, ledger, root) = fixture();
    fs::remove_dir_all(root.join("cases")).unwrap();
    let report = StageActorCoverageReport::build(&catalog, &ledger, &root).unwrap();
    assert_eq!(report.verified_case_count, 0);
    assert_eq!(report.rejected_case_count, 1);
    assert_eq!(
        report.cases[0].status,
        StageActorEvidenceStatus::ArtifactMissing
    );
    assert!(report.profiles.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_a_complete_but_different_learning_actor_population() {
    let (catalog, ledger, root) = fixture_with_learning_generation(5);
    let report = StageActorCoverageReport::build(&catalog, &ledger, &root).unwrap();
    assert_eq!(report.verified_case_count, 0);
    assert_eq!(report.rejected_case_count, 1);
    assert_eq!(
        report.cases[0].status,
        StageActorEvidenceStatus::ArtifactRejected
    );
    assert_eq!(
        report.cases[0].diagnostic.as_deref(),
        Some("learning_actor_population_invariant_mismatch")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn profile_field_coverage_counts_variation_without_retaining_raw_values() {
    let mut fields = BTreeMap::new();
    accumulate_profile_field("health", &json!(3), 0, &mut fields).unwrap();
    accumulate_profile_field("health", &json!(4), 1, &mut fields).unwrap();
    accumulate_profile_field("model_present", &json!(true), 0, &mut fields).unwrap();
    accumulate_profile_field("model_present", &json!(false), 1, &mut fields).unwrap();
    accumulate_profile_field("attention", &Value::Null, 0, &mut fields).unwrap();
    accumulate_profile_field("attention", &json!({"flags": 3}), 1, &mut fields).unwrap();
    let fields = finish_profile_fields(fields, 2);

    let health = fields.iter().find(|field| field.path == "health").unwrap();
    assert_eq!(health.status, StageActorFieldCoverageStatus::Varying);
    assert_eq!(health.value_samples, 2);
    assert_eq!(health.distinct_nonnull_values, 2);
    let model = fields
        .iter()
        .find(|field| field.path == "model_present")
        .unwrap();
    assert_eq!(model.status, StageActorFieldCoverageStatus::Varying);
    assert_eq!(model.true_samples, 1);
    assert_eq!(
        fields
            .iter()
            .find(|field| field.path == "attention")
            .unwrap()
            .status,
        StageActorFieldCoverageStatus::Ambiguous
    );
    let flags = fields
        .iter()
        .find(|field| field.path == "attention.flags")
        .unwrap();
    assert_eq!(flags.sampled_actors, 1);
    assert_eq!(flags.missing_actors, 1);
    assert_eq!(flags.status, StageActorFieldCoverageStatus::Ambiguous);
    assert!(profile_identity_ambiguous(1, 2, 1, 1));
    assert!(!profile_identity_ambiguous(1, 0, 1, 1));
}

#[test]
fn door20_catalog_state_is_profile_bound_and_recomputed_from_placement() {
    let parameters = 9u32
        | (3u32 << 5)
        | (2u32 << 8)
        | (1u32 << 10)
        | (4u32 << 13)
        | (5u32 << 19)
        | (6u32 << 25)
        | (1u32 << 31);
    let mut door = LearningActorDoor20 {
        kind: 9,
        door_model: 3,
        front_option: 2,
        back_option: 1,
        front_room: 4,
        back_room: 5,
        exit_number: 6,
        message_door: true,
        front_switch: Some(LearningActorDoor20Switch {
            id: 0x11,
            set: true,
        }),
        back_switch: Some(LearningActorDoor20Switch {
            id: 0x22,
            set: false,
        }),
        unlock_effect_switch: Some(LearningActorDoor20Switch {
            id: 0x33,
            set: true,
        }),
        front_event: 0x44,
        back_event: 0x33,
        message_number: 0x3344,
        action: "demo".into(),
        active_side: "back".into(),
        event_variant: 13,
        locked: true,
        background_collision_released: false,
        unlock_effect_triggered: true,
        key_type: 1,
        enemy_clear_debounce: 42,
        opening_active: true,
        closing_active: false,
        door_angle: -1234,
        stopper_side: "back".into(),
        front_stopper_status: "room_unavailable".into(),
        back_stopper_status: "closed".into(),
    };
    assert!(valid_door20(
        0x0e8,
        parameters,
        [0x3344, 0, 0x2211],
        Some(&door)
    ));
    assert!(!valid_door20(
        0x0e7,
        parameters,
        [0x3344, 0, 0x2211],
        Some(&door)
    ));
    assert!(!valid_door20(0x0e8, parameters, [0x3344, 0, 0x2211], None));
    door.action = "invented".into();
    assert!(!valid_door20(
        0x0e8,
        parameters,
        [0x3344, 0, 0x2211],
        Some(&door)
    ));
}

#[test]
fn rejects_stale_stage_profile_field_counts_even_when_resealed() {
    let (catalog, ledger, root) = fixture();
    let mut report = StageActorCoverageReport::build(&catalog, &ledger, &root).unwrap();
    report.profiles[0].stages[0].actor_instance_count += 1;
    report.report_sha256 = report.compute_digest().unwrap();
    assert!(report.validate().is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_unaccounted_learner_actor_extension_fields() {
    let (catalog, mut ledger, root) = fixture();
    let actor_path = root
        .join("cases")
        .join(stage_survey_case_storage_id(&catalog.candidates[0].id).to_string())
        .join("attempt-001-run-00000")
        .join("actors.json");
    let mut document: Value = serde_json::from_slice(&fs::read(&actor_path).unwrap()).unwrap();
    document["learning_actor_population"]["actors"][0]["unaccounted_field"] = json!(7);
    let actor_bytes = serde_json::to_vec_pretty(&document).unwrap();
    fs::write(&actor_path, &actor_bytes).unwrap();
    ledger.cases[0].attempts[0].actor_catalog_sha256 =
        Some(Digest(Sha256::digest(&actor_bytes).into()));

    let report = StageActorCoverageReport::build(&catalog, &ledger, &root).unwrap();
    assert_eq!(report.verified_case_count, 0);
    assert_eq!(
        report.cases[0].status,
        StageActorEvidenceStatus::ArtifactRejected
    );
    assert_eq!(
        report.cases[0].diagnostic.as_deref(),
        Some("actor_catalog_decode_failed")
    );
    fs::remove_dir_all(root).unwrap();
}
