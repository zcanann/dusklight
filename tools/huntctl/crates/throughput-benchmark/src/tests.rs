use crate::report::{comparison_issue, summarize};
use crate::*;
use dusklight_harness_contracts::artifact::Digest;
use dusklight_harness_contracts::run_contract::{
    HarnessBoundaryFingerprint, HarnessTerminalReason,
};

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
        end_to_end_micros: duration,
        harness_outside_process_micros: duration.saturating_sub(8_000),
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
}
