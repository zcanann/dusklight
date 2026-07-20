use crate::report::{comparison_issue, summarize};
use crate::*;
use dusklight_harness_contracts::artifact::Digest;
use dusklight_harness_contracts::run_contract::{
    ENGINE_SESSION_REUSE_AUDIT_SCHEMA_V1, HarnessBoundaryFingerprint, HarnessNativePhaseTiming,
    HarnessTerminalReason, SessionReuseAudit, SessionReuseBlocker,
};

fn native_phases() -> HarnessNativePhaseTiming {
    HarnessNativePhaseTiming {
        schema: "dusklight-native-lifecycle-timing/v3".into(),
        clock: "steady_clock".into(),
        process_cpu_micros: Some(6_000),
        process_entry_micros: 0,
        cli_configured_micros: 500,
        aurora_initialized_micros: 1_000,
        engine_ready_micros: 2_000,
        stage_ready_micros: 3_000,
        first_simulation_tick_micros: 3_000,
        last_simulation_tick_micros: 5_000,
        proof_artifacts_written_micros: 6_000,
        engine_shutdown_micros: 7_000,
        exit_ready_micros: 8_000,
        session_reuse_audit: Some(SessionReuseAudit {
            schema: ENGINE_SESSION_REUSE_AUDIT_SCHEMA_V1.into(),
            reusable: false,
            evaluated_boundary: "post_authenticated_run".into(),
            target_boundary: "post_authenticated_run".into(),
            blockers: vec![SessionReuseBlocker {
                code: "game_global_reconstruction".into(),
                subsystem: "game_state".into(),
                required_guarantee: "game state reconstructs from a clean origin".into(),
            }],
        }),
    }
}

fn attempt(number: u32, trace: u8, duration: u128) -> ColdProcessBenchmarkAttempt {
    ColdProcessBenchmarkAttempt {
        attempt: number,
        request: format!("bench/requests/{number}.json"),
        request_sha256: Digest([number as u8; 32]),
        artifact_destination: format!("bench/attempt-{number}"),
        result: format!("bench/attempt-{number}/result.json"),
        result_sha256: Digest([number as u8 + 10; 32]),
        terminal: HarnessTerminalReason::Reached,
        objective_reached: true,
        first_hit_tick: Some(4),
        boundary_fingerprint: Some(HarnessBoundaryFingerprint {
            schema: "dusklight.milestone-boundary/v4".into(),
            algorithm: "xxh3-128".into(),
            canonical_encoding: "little-endian-fixed-v4".into(),
            digest: "12".repeat(16),
        }),
        realized_input_sha256: Some(Digest([3; 32])),
        gameplay_trace_sha256: Some(Digest([trace; 32])),
        objective_evidence_sha256: Some(Digest([5; 32])),
        artifacts_complete: true,
        logical_ticks: 5,
        consumed_input_ticks: 5,
        native_process_millis: 8,
        native_process_cpu_micros: Some(6_000),
        artifact_file_count: Some(10),
        artifact_bytes: Some(1_000),
        prefix_ticks: Some(2),
        candidate_ticks: Some(3),
        end_to_end_micros: duration,
        harness_outside_process_micros: duration.saturating_sub(8_000),
        native_phases: native_phases(),
    }
}

fn report(attempts: Vec<ColdProcessBenchmarkAttempt>) -> ColdProcessBenchmarkReport {
    let issue = comparison_issue(&attempts);
    let mut report = ColdProcessBenchmarkReport {
        schema: COLD_PROCESS_BENCHMARK_SCHEMA.into(),
        content_sha256: Digest::ZERO,
        mode: COLD_PROCESS_MODE.into(),
        recorded_unix_millis: 1,
        host: ColdProcessBenchmarkHost {
            operating_system: "macos".into(),
            architecture: "aarch64".into(),
            operating_system_version: Some("test".into()),
            hardware_model: Some("test-mac".into()),
            cpu_model: Some("test-cpu".into()),
            logical_cpu_count: 4,
            memory_bytes: Some(16 * 1024 * 1024 * 1024),
        },
        template_request_sha256: Digest([9; 32]),
        artifact_destination_root: "build/benchmark".into(),
        repetitions: attempts.len() as u32,
        comparable: issue.is_none(),
        comparison_issue: issue,
        summary: summarize(&attempts).unwrap(),
        attempts,
    };
    report.refresh_content_sha256().unwrap();
    report
}

#[test]
fn report_seals_exact_attempts_and_computes_throughput() {
    let report = report(vec![attempt(1, 4, 10_000), attempt(2, 4, 20_000)]);
    report.validate().unwrap();
    assert!(report.comparable);
    assert_eq!(report.summary.total_logical_ticks, 10);
    assert_eq!(report.summary.median_end_to_end_micros, 10_000);
    assert_eq!(report.summary.p95_end_to_end_micros, 20_000);
    assert_eq!(report.summary.candidates_per_second_millionths, 66_666_666);
    assert_eq!(report.summary.process_launches, Some(2));
    assert_eq!(report.summary.total_prefix_ticks, Some(4));
    assert_eq!(report.summary.total_candidate_ticks, Some(6));
    assert_eq!(
        report.summary.candidate_ticks_per_second_millionths,
        Some(200_000_000)
    );
    assert_eq!(report.summary.total_native_process_cpu_micros, Some(12_000));
    assert_eq!(
        report.summary.native_cpu_utilization_millionths,
        Some(400_000)
    );
    assert_eq!(report.summary.total_artifact_file_count, Some(20));
    assert_eq!(report.summary.total_artifact_bytes, Some(2_000));
    assert_eq!(report.summary.simulator_idle_micros, Some(26_000));
    let decoded: ColdProcessBenchmarkReport =
        serde_json::from_slice(&report.to_pretty_json().unwrap()).unwrap();
    decoded.validate().unwrap();

    let mut stale = report.clone();
    stale.attempts[0].logical_ticks += 1;
    assert!(stale.validate().is_err());
}

#[test]
fn changed_artifact_or_incomplete_proof_is_not_comparable() {
    let changed = report(vec![attempt(1, 4, 10_000), attempt(2, 7, 10_000)]);
    changed.validate().unwrap();
    assert!(!changed.comparable);
    assert!(
        changed
            .comparison_issue
            .as_deref()
            .unwrap()
            .contains("not semantically and artifact-identical")
    );

    let mut incomplete_attempt = attempt(2, 4, 10_000);
    incomplete_attempt.artifacts_complete = false;
    let incomplete = report(vec![attempt(1, 4, 10_000), incomplete_attempt]);
    incomplete.validate().unwrap();
    assert!(!incomplete.comparable);
    assert!(
        incomplete
            .comparison_issue
            .as_deref()
            .unwrap()
            .contains("complete authenticated artifacts")
    );

    let mut changed_audit_attempt = attempt(2, 4, 10_000);
    let mut changed_audit = changed_audit_attempt
        .native_phases
        .session_reuse_audit
        .take()
        .unwrap();
    changed_audit.blockers.clear();
    changed_audit.reusable = true;
    changed_audit_attempt.native_phases.session_reuse_audit = Some(changed_audit);
    let changed_audit = report(vec![attempt(1, 4, 10_000), changed_audit_attempt]);
    changed_audit.validate().unwrap();
    assert!(!changed_audit.comparable);
}

#[test]
fn nonmonotonic_native_phase_evidence_is_rejected() {
    let mut invalid = report(vec![attempt(1, 4, 10_000), attempt(2, 4, 10_000)]);
    invalid.attempts[0]
        .native_phases
        .last_simulation_tick_micros = 1_000;
    assert!(invalid.validate().is_err());
}
