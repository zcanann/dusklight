use super::*;
use dusklight_learning::factorized_policy_suffix_batch::NativeFactorizedPolicyBatchConfig;
use dusklight_learning::native_frozen_policy_suffix_batch::{
    NATIVE_POLICY_ROLLOUT_EXPLORATION_SCHEMA_V1, NativeFrozenPolicySuffixBatch,
    NativePolicyRolloutExploration, native_frozen_policy_probe_model,
};
use dusklight_search::search::MacroAction;
use dusklight_search::suffix_batch::{NativeCheckpointValidation, NativeSuffixCandidate};

#[path = "tests/live_endpoint.rs"]
mod live_endpoint;

fn request(verify_state_hashes: bool) -> NativeSuffixBatch {
    NativeSuffixBatch {
        schema: NATIVE_SUFFIX_BATCH_SCHEMA.into(),
        source_frame: 500,
        source_boundary_fingerprint: "1".repeat(32),
        checkpoint_validation: NativeCheckpointValidation {
            kind: "recorded_replay_window".into(),
            ticks: 2,
        },
        maximum_ticks: 2,
        verify_state_hashes,
        checkpoint_cache: None,
        candidates: vec![NativeSuffixCandidate {
            id: "candidate-0".into(),
            actions: vec![MacroAction::Neutral { frames: 2 }],
            controller_program_hex: None,
            maximum_ticks: None,
            cancellation_guard: None,
        }],
    }
}

fn terminal() -> NativeTerminalBinding {
    NativeTerminalBinding {
        goal: "goal".into(),
        program_sha256: Digest([2; 32]),
        definition_sha256: Digest([3; 32]),
    }
}

fn result(success: bool, verify_state_hashes: bool) -> NativeSuffixBatchResult {
    let terminal = terminal();
    let first_hit_tick = success.then_some(0);
    let ticks = first_hit_tick.map_or(2, |tick| tick + 1);
    NativeSuffixBatchResult {
        schema: NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V6.into(),
        status: "passed".into(),
        source_frame: 500,
        source_boundary: NativeSourceBoundaryResult {
            milestone: None,
            expected_fingerprint: "1".repeat(32),
            actual_fingerprint: Some("1".repeat(32)),
            fingerprint_verified: true,
            verified: true,
        },
        checkpoint_validation: NativeCheckpointValidationResult {
            kind: "recorded_replay_window".into(),
            ticks: 2,
            verified: true,
            source_semantic_digest: Some("4".repeat(32)),
            fresh_sequence_digest: Some("5".repeat(32)),
            restored_sequence_digest: Some("5".repeat(32)),
            first_divergence_tick: None,
            expected_tick_digest: None,
            actual_tick_digest: None,
        },
        maximum_ticks: 2,
        candidate_count: 1,
        completed_candidates: 1,
        verify_state_hashes,
        policy_model: None,
        checkpoint_bytes: 128,
        restore_identity: Some("6".repeat(32)),
        checkpoint_cache: None,
        capture_micros: 1,
        restore_micros: vec![1],
        timing: NativeSuffixTimingResult {
            schema: "dusklight-suffix-batch-timing/v1".into(),
            batch_wall_micros: 1,
            candidate_ticks: ticks,
            verified: true,
            accounting: Value::Object(Default::default()),
            phases: serde_json::json!({
                "checkpoint_restore": {
                    "status": "measured",
                    "micros": 1,
                },
                "simulation": {
                    "status": "measured",
                    "micros": 1,
                },
                "observation_capture": {
                    "status": "measured",
                    "micros": 1,
                },
                "corpus_encoding": {
                    "status": "measured",
                    "micros": 1,
                },
            }),
            headless_audit: Value::Object(Default::default()),
        },
        audio_callback_quiesced: true,
        episode_shard: NativeEpisodeShardResult {
            schema: NATIVE_EPISODE_SHARD_SCHEMA_V2.into(),
            path: "result.json.episodes.dseps".into(),
            observation_schema: "dusklight-learning-observation/v27".into(),
            action_schema: RAW_PAD_ACTION_SCHEMA_V2.into(),
            episode_count: 1,
            uncompressed_bytes: 10,
            compressed_bytes: 5,
        },
        winner_id: success.then(|| "candidate-0".into()),
        candidates: vec![NativeSuffixCandidateResult {
            id: "candidate-0".into(),
            success,
            ticks_executed: ticks,
            first_hit_tick,
            state_sequence_digest: verify_state_hashes.then(|| "7".repeat(32)),
            state_tick_digests: verify_state_hashes.then(|| vec!["8".repeat(32); ticks as usize]),
            terminal_state_entry_digests: None,
            terminal_boundary_fingerprint: "9".repeat(32),
            predicate_evidence: NativePredicateEvidence {
                schema: NativeMilestoneSchema {
                    name: "dusklight.automation.milestones".into(),
                    version: 5,
                },
                boot: NativeBootEvidence {
                    kind: "process".into(),
                },
                boot_origin_established: true,
                goal: terminal.goal.clone(),
                goal_reached: success,
                program_digest: Some(terminal.program_sha256.to_string()),
                milestones: vec![NativeMilestoneEvidence {
                    id: terminal.goal,
                    hit: success,
                    sim_tick: success.then_some(501),
                    tape_frame: success.then_some(500),
                    boundary_index: success.then_some(501),
                    phase: Some("post_sim".into()),
                    stable_ticks: Some(1),
                    definition_digest: Some(terminal.definition_sha256.to_string()),
                    program_digest: Some(terminal.program_sha256.to_string()),
                    evidence: None,
                    projections: None,
                }],
            },
            consumed_pad_states: success.then(|| vec![RawPadState::default(); ticks as usize]),
            retained_checkpoint: None,
            terminal_observation: Value::Object(Default::default()),
        }],
        error: None,
    }
}

fn frozen_request() -> (NativeFrozenPolicySuffixBatch, Vec<u8>) {
    let model = native_frozen_policy_probe_model(terminal().definition_sha256).unwrap();
    let bytes = model.to_bytes().unwrap();
    let request = NativeFrozenPolicySuffixBatch::build(
        &bytes,
        "policy.dsfrozen".into(),
        terminal().definition_sha256,
        "candidate-0".into(),
        NativeFactorizedPolicyBatchConfig {
            source_frame: 500,
            source_boundary_fingerprint: "1".repeat(32),
            checkpoint_validation_ticks: 2,
            maximum_ticks: 2,
            verify_state_hashes: false,
        },
    )
    .unwrap();
    (request, bytes)
}

fn exploratory_frozen_request() -> (NativeFrozenPolicySuffixBatch, Vec<u8>) {
    let model = native_frozen_policy_probe_model(terminal().definition_sha256).unwrap();
    let bytes = model.to_bytes().unwrap();
    let request = NativeFrozenPolicySuffixBatch::build_with_rollout_exploration(
        &bytes,
        "policy.dsfrozen".into(),
        terminal().definition_sha256,
        "candidate-0".into(),
        dusklight_learning::native_replay_corpus::DemonstrationMode::Absent,
        NativePolicyRolloutExploration {
            schema: NATIVE_POLICY_ROLLOUT_EXPLORATION_SCHEMA_V1.into(),
            seed: 17,
            stick_axis_delta_probability_millionths: 125_000,
            maximum_stick_axis_delta: 32,
            button_flip_probability_millionths: 2_000,
            button_flip_mask: 0x0f7f,
        },
        NativeFactorizedPolicyBatchConfig {
            source_frame: 500,
            source_boundary_fingerprint: "1".repeat(32),
            checkpoint_validation_ticks: 2,
            maximum_ticks: 2,
            verify_state_hashes: false,
        },
    )
    .unwrap();
    (request, bytes)
}

fn frozen_result(
    model_bytes: &[u8],
    request: &NativeFrozenPolicySuffixBatch,
) -> NativeSuffixBatchResult {
    let model = FrozenInferenceModel::from_bytes(model_bytes).unwrap();
    let mut result = result(false, false);
    result.schema = if request.frozen_policy.rollout_exploration.is_some() {
        NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V7
    } else {
        NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V6
    }
    .into();
    result.timing.schema = "dusklight-suffix-batch-timing/v2".into();
    result.episode_shard.schema = NATIVE_EPISODE_SHARD_SCHEMA_V3.into();
    result.policy_model = Some(serde_json::json!({
        "schema": request.frozen_policy.schema,
        "action_authority": "episode_policy",
        "policy_controlled_ticks": result.timing.candidate_ticks,
        "fallback_ticks": 0,
        "model_xxh3_128": request.frozen_policy.model_xxh3_128,
        "feature_schema_sha256": model.feature_schema_sha256,
        "action_schema_sha256": model.action_schema_sha256,
        "objective_sha256": model.objective_sha256,
        "parameter_count": model.layers.iter().map(|layer| layer.weights.len() + layer.biases.len()).sum::<usize>(),
        "rollout_exploration": request.frozen_policy.rollout_exploration,
    }));
    result
}

#[test]
fn accepts_exact_miss_and_success_evidence() {
    let miss = result(false, true)
        .validate_against(&request(true), &terminal())
        .unwrap();
    assert_eq!(miss.simulated_ticks, 2);
    assert_eq!(
        miss.timing,
        ValidatedNativeSuffixTiming {
            batch_wall_micros: 1,
            simulation_micros: 1,
            observation_capture_micros: 1,
            corpus_encoding_micros: 1,
        }
    );
    assert_eq!(miss.candidates[0].first_hit_tick, None);
    assert_eq!(
        miss.candidates[0].terminal_boundary_fingerprint,
        "9".repeat(32)
    );

    let success = result(true, false)
        .validate_against(&request(false), &terminal())
        .unwrap();
    assert_eq!(success.simulated_ticks, 1);
    assert_eq!(success.candidates[0].first_hit_tick, Some(0));
}

#[test]
fn rejects_missing_or_detached_native_phase_timing() {
    let mut missing = result(false, false);
    missing.timing.phases["simulation"] = Value::Null;
    assert!(
        missing
            .validate_against(&request(false), &terminal())
            .is_err()
    );

    let mut detached = result(false, false);
    detached.timing.phases["checkpoint_restore"]["micros"] = Value::from(2);
    assert!(
        detached
            .validate_against(&request(false), &terminal())
            .is_err()
    );
}

#[test]
fn accepts_early_unsuccessful_declared_controller_or_guard_completion_only() {
    let mut early = result(false, false);
    early.candidates[0].ticks_executed = 1;
    early.timing.candidate_ticks = 1;

    assert!(
        early
            .clone()
            .validate_against(&request(false), &terminal())
            .is_err()
    );

    let mut reactive = request(false);
    reactive.candidates[0].controller_program_hex = Some("00".into());
    let validated = early
        .clone()
        .validate_against(&reactive, &terminal())
        .unwrap();
    assert_eq!(validated.simulated_ticks, 1);
    assert_eq!(validated.candidates[0].first_hit_tick, None);

    let mut guarded = request(false);
    guarded.candidates[0].cancellation_guard = Some(
        dusklight_search::suffix_batch::NativeSuffixCancellationGuard {
            allowed_stage_rooms: vec![dusklight_search::suffix_batch::NativeSuffixStageRoom {
                stage: "F_SP103".into(),
                room: 1,
            }],
        },
    );
    let validated = early.validate_against(&guarded, &terminal()).unwrap();
    assert_eq!(validated.simulated_ticks, 1);
}

#[test]
fn equal_tick_winner_preserves_native_request_order() {
    let mut request = request(false);
    request.candidates[0].id = "candidate-z".into();
    let mut second_request = request.candidates[0].clone();
    second_request.id = "candidate-a".into();
    request.candidates.push(second_request);

    let mut result = result(true, false);
    result.candidates[0].id = "candidate-z".into();
    let mut second_result = result.candidates[0].clone();
    second_result.id = "candidate-a".into();
    result.candidates.push(second_result);
    result.candidate_count = 2;
    result.completed_candidates = 2;
    result.restore_micros.push(1);
    result.timing.phases["checkpoint_restore"]["micros"] = Value::from(2);
    result.timing.candidate_ticks = 2;
    result.episode_shard.episode_count = 2;
    result.winner_id = Some("candidate-z".into());

    let validated = result.validate_against(&request, &terminal()).unwrap();
    assert_eq!(validated.candidates.len(), 2);
}

#[test]
fn cached_result_binds_direct_source_and_retained_endpoint() {
    let mut request = request(false);
    request.schema = NATIVE_CACHED_SUFFIX_BATCH_SCHEMA.into();
    request.checkpoint_cache = Some(
        dusklight_search::suffix_batch::NativeCheckpointCacheRequest {
            capacity_bytes: 671_088_640,
            capacity_entries: 2,
            source_identity: Some("a".repeat(32)),
            source_route_ticks: 40,
            retain_candidate_checkpoints: true,
            retain_live_endpoint: false,
            retain_candidate_index: None,
        },
    );
    let mut result = result(false, false);
    result.schema = NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V9.into();
    result.restore_identity = Some("a".repeat(32));
    result.checkpoint_cache = Some(
        serde_json::from_value(serde_json::json!({
            "source_kind": "direct_process_local_restore",
            "source_identity": "a".repeat(32),
            "source_route_ticks": 40,
            "capacity_bytes": 671_088_640_u64,
            "capacity_entries": 2,
            "resident_bytes": 256,
            "resident_checkpoint_bytes": 240,
            "resident_host_snapshot_bytes": 16,
            "resident_entries": 2,
            "insertions": 2,
            "replacements": 0,
            "evictions": 0,
            "hits": 2,
            "misses": 0,
            "source_pinned": true,
            "batch_capture_attempts": 1,
            "batch_capture_successes": 1,
            "batch_capture_micros": 1,
            "checkpoint_image_reuse_enabled": true,
            "batch_image_reuse_attempts": 1,
            "batch_image_reuse_successes": 0,
            "live_endpoint_capacity_entries": 1,
            "live_endpoint_resident_entries": 0,
            "live_endpoint_resident_host_snapshot_bytes": 0,
            "batch_live_retention_attempts": 0,
            "batch_live_retention_successes": 0,
            "batch_live_retention_nanos": 0,
            "batch_live_consumptions": 0,
            "batch_live_invalidations": 0
        }))
        .unwrap(),
    );
    result.candidates[0].retained_checkpoint = Some(NativeRetainedCheckpointResult {
        storage_kind: "portable_image".into(),
        restore_identity: "b".repeat(32),
        image_digest: Some("c".repeat(32)),
        semantic_digest: Some("d".repeat(32)),
        checkpoint_bytes: 128,
        host_snapshot_bytes: 16,
        machine_capture_micros: 1,
        host_snapshot_capture_nanos: 1,
        capture_micros: 1,
        route_ticks: 42,
    });
    let validated = result.validate_against(&request, &terminal()).unwrap();
    assert_eq!(
        validated.candidates[0]
            .retained_checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.restore_identity.as_str()),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    assert_eq!(validated.restore_micros, vec![1]);
    assert_eq!(
        validated
            .checkpoint_cache
            .as_ref()
            .map(|cache| (cache.hits, cache.resident_bytes)),
        Some((2, 256))
    );

    let copy = |value: &NativeSuffixBatchResult| {
        serde_json::from_slice::<NativeSuffixBatchResult>(&serde_json::to_vec(value).unwrap())
            .unwrap()
    };
    let mut detached = copy(&result);
    detached.restore_identity = Some("e".repeat(32));
    assert!(detached.validate_against(&request, &terminal()).is_err());

    let mut detached = copy(&result);
    detached.checkpoint_cache.as_mut().unwrap().source_identity = Some("e".repeat(32));
    assert!(detached.validate_against(&request, &terminal()).is_err());

    let mut detached = copy(&result);
    detached.checkpoint_cache.as_mut().unwrap().source_kind = "authenticated_root_restore".into();
    assert!(detached.validate_against(&request, &terminal()).is_err());

    let mut detached = copy(&result);
    detached
        .checkpoint_cache
        .as_mut()
        .unwrap()
        .source_route_ticks = 39;
    assert!(detached.validate_against(&request, &terminal()).is_err());

    let mut detached = copy(&result);
    detached.checkpoint_cache.as_mut().unwrap().source_pinned = false;
    assert!(detached.validate_against(&request, &terminal()).is_err());

    let mut detached = copy(&result);
    detached.checkpoint_cache.as_mut().unwrap().resident_bytes += 1;
    assert!(detached.validate_against(&request, &terminal()).is_err());

    let mut detached = result;
    detached.candidates[0]
        .retained_checkpoint
        .as_mut()
        .unwrap()
        .route_ticks = 41;
    assert!(detached.validate_against(&request, &terminal()).is_err());
}

#[test]
fn rejects_boundary_checkpoint_terminal_and_tick_tampering() {
    let batch = request(true);
    let authority = terminal();

    let mut tampered = result(false, true);
    tampered.source_boundary.actual_fingerprint = Some("9".repeat(32));
    assert!(tampered.validate_against(&batch, &authority).is_err());

    let mut tampered = result(false, true);
    tampered.checkpoint_validation.restored_sequence_digest = Some("9".repeat(32));
    assert!(tampered.validate_against(&batch, &authority).is_err());

    let mut tampered = result(false, true);
    tampered.candidates[0].terminal_boundary_fingerprint = "9".repeat(31);
    assert!(tampered.validate_against(&batch, &authority).is_err());

    let mut tampered = result(false, true);
    tampered.candidates[0].predicate_evidence.milestones[0].definition_digest =
        Some("9".repeat(64));
    assert!(tampered.validate_against(&batch, &authority).is_err());

    let mut tampered = result(false, true);
    tampered.timing.candidate_ticks = 1;
    assert!(tampered.validate_against(&batch, &authority).is_err());
}

#[test]
fn accepts_exact_frozen_policy_identity_and_rejects_detachment() {
    let (request, bytes) = frozen_request();
    let result = frozen_result(&bytes, &request);
    let validated = result
        .validate_frozen_against(&request, &bytes, &terminal())
        .unwrap();
    assert_eq!(validated.simulated_ticks, 2);
    assert_eq!(validated.candidates[0].id, "candidate-0");

    let mut tampered = frozen_result(&bytes, &request);
    tampered.policy_model.as_mut().unwrap()["objective_sha256"] = Value::String("0".repeat(64));
    assert!(
        tampered
            .validate_frozen_against(&request, &bytes, &terminal())
            .is_err()
    );

    let mut tampered = frozen_result(&bytes, &request);
    tampered.episode_shard.schema = NATIVE_EPISODE_SHARD_SCHEMA_V2.into();
    assert!(
        tampered
            .validate_frozen_against(&request, &bytes, &terminal())
            .is_err()
    );

    let mut tampered = frozen_result(&bytes, &request);
    tampered.policy_model.as_mut().unwrap()["policy_controlled_ticks"] = Value::from(1);
    assert!(
        tampered
            .validate_frozen_against(&request, &bytes, &terminal())
            .is_err()
    );

    let mut tampered = frozen_result(&bytes, &request);
    tampered.policy_model.as_mut().unwrap()["fallback_ticks"] = Value::from(1);
    assert!(
        tampered
            .validate_frozen_against(&request, &bytes, &terminal())
            .is_err()
    );
}

#[test]
fn binds_v7_frozen_policy_result_to_exact_rollout_exploration() {
    let (request, bytes) = exploratory_frozen_request();
    let result = frozen_result(&bytes, &request);
    result
        .validate_frozen_against(&request, &bytes, &terminal())
        .unwrap();

    let mut tampered = frozen_result(&bytes, &request);
    tampered.policy_model.as_mut().unwrap()["rollout_exploration"]["seed"] = Value::from(18_u64);
    assert!(
        tampered
            .validate_frozen_against(&request, &bytes, &terminal())
            .is_err()
    );
}

#[test]
fn serde_contract_rejects_unknown_native_fields() {
    let mut value = serde_json::to_value(result(false, false)).unwrap();
    value["unreviewed_authority"] = Value::Bool(true);
    assert!(serde_json::from_value::<NativeSuffixBatchResult>(value).is_err());
}
