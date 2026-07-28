use super::*;
use serde_json::json;

fn headless_audit() -> Value {
    json!({
        "active": true,
        "host_pacing": "disabled",
        "imgui_frame_lifecycle": "suppressed_on_candidate_ticks",
        "host_audio_device": "suppressed",
        "deterministic_audio_emulation": "retained",
        "game_audio_update": "retained",
        "gameplay_draw_traversal": "retained",
        "cpu_renderer_submission": "suppressed_on_candidate_ticks"
    })
}

fn batch(source_kind: &str, simulated_ticks: u64) -> NativeCheckpointBatchMeasurement {
    NativeCheckpointBatchMeasurement {
        host_wall_micros: 1,
        native_batch_wall_micros: 1,
        native_simulation_micros: 1,
        native_restore_micros: 1,
        simulated_ticks,
        source_kind: source_kind.into(),
        cpu_draw_traversal_micros: 1,
        cpu_renderer_submission_micros: 0,
        audio_emulation_micros: 1,
        game_audio_update_micros: 1,
        headless_audit: headless_audit(),
    }
}

fn live_frontier(label: &str, route_ticks: u64) -> NativeCheckpointFrontierMeasurement {
    NativeCheckpointFrontierMeasurement {
        label: label.into(),
        route_ticks,
        authenticated_root_replay: batch("authenticated_root_restore", route_ticks),
        process_local_follow_up: batch("direct_process_local_continuation", 1),
        authenticated_replay_fallback: batch("authenticated_root_restore", route_ticks + 1),
        endpoint_retention: NativeCheckpointCaptureMeasurement {
            storage_kind: "live_endpoint".into(),
            checkpoint_bytes: 0,
            host_snapshot_bytes: 296,
            machine_capture_micros: 0,
            host_snapshot_transfer_kind: "process_local_live_endpoint".into(),
            host_snapshot_capture_nanos: 100,
            total_capture_micros: 1,
        },
        evidence_projection: NativeEvidenceProjectionMeasurement {
            episode_decode_micros: 1,
            fact_extraction_micros: 1,
        },
        parity: NativeCheckpointParityMeasurement {
            source_state_exact: true,
            transition_exact: true,
            checkpoint_wide_semantic_digest_scope: "test".into(),
            semantic_state_digest_exact: true,
            checkpoint_entry_count: 1,
            divergent_checkpoint_entries: Vec::new(),
            terminal_evidence_bytes_exact: true,
            terminal_boundary_exact: true,
            passed: true,
        },
    }
}

fn live_report() -> NativeCheckpointBenchmarkReport {
    NativeCheckpointBenchmarkReport {
        schema: NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V4.into(),
        optimization_request_sha256: Digest([1; 32]),
        execution_sha256: Digest([2; 32]),
        executable_sha256: Digest([3; 32]),
        game_data_sha256: Digest([4; 32]),
        platform_os: "test".into(),
        platform_arch: "test".into(),
        source_frame: 1,
        cache_capacity_bytes: 1,
        cache_capacity_entries: 1,
        launch: NativeCheckpointLaunchMeasurement {
            phases: NativeSuffixWorkerLaunchTiming {
                spawn_call_micros: 1,
                handshake_micros: 1,
                initial_batch_wait_micros: 1,
                artifact_validation_micros: 1,
                total_micros: 4,
            },
            initial_batch_native_wall_micros: 1,
            initial_batch_native_simulation_micros: 1,
        },
        frontiers: ["early", "middle", "late"]
            .into_iter()
            .enumerate()
            .map(|(index, label)| live_frontier(label, index as u64 + 1))
            .collect(),
        throughput: NativeCheckpointThroughputMeasurement {
            useful_transition_definition: "test".into(),
            useful_transitions: 3,
            non_root_expansion_requests: 3,
            direct_restore_requests: 3,
            direct_restore_rate_millionths: 1_000_000,
            useful_transitions_per_direct_restore_millionths: 1_000_000,
            useful_transitions_per_native_sim_second_millionths: 1,
            useful_transitions_per_wall_second_millionths: 1,
            measured_wall_micros: 1,
            measured_native_simulation_micros: 1,
        },
        passed: true,
    }
}

#[test]
fn v4_requires_live_continuations_without_machine_capture() {
    let report = live_report();
    report.validate().unwrap();

    let mut portable = report.clone();
    portable.frontiers[0].endpoint_retention.checkpoint_bytes = 1;
    assert!(portable.validate().is_err());

    let mut restore = report;
    restore.frontiers[0].process_local_follow_up.source_kind =
        "direct_process_local_restore".into();
    assert!(restore.validate().is_err());
}

#[test]
fn legacy_v3_field_names_and_portable_capture_remain_readable() {
    let mut value = serde_json::to_value(live_report()).unwrap();
    value["schema"] = json!(NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V3);
    for frontier in value["frontiers"].as_array_mut().unwrap() {
        let object = frontier.as_object_mut().unwrap();
        let follow_up = object.remove("process_local_follow_up").unwrap();
        let fallback = object.remove("authenticated_replay_fallback").unwrap();
        let mut retention = object.remove("endpoint_retention").unwrap();
        retention.as_object_mut().unwrap().remove("storage_kind");
        retention["checkpoint_bytes"] = json!(295_000_000);
        retention["machine_capture_micros"] = json!(1);
        retention["host_snapshot_transfer_kind"] =
            json!("in_process_capture_and_move_into_resident_cache");
        let mut follow_up = follow_up;
        follow_up["source_kind"] = json!("direct_process_local_restore");
        object.insert("process_local_restore".into(), follow_up);
        object.insert("portable_reconstruction".into(), fallback);
        object.insert("checkpoint_capture".into(), retention);
    }
    let report: NativeCheckpointBenchmarkReport = serde_json::from_value(value).unwrap();
    report.validate().unwrap();
}
