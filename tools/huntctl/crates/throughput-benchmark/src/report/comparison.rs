use super::ColdProcessBenchmarkAttempt;
use dusklight_harness_contracts::artifact::Digest;
use dusklight_harness_contracts::run_contract::{
    HarnessBoundaryFingerprint, HarnessTerminalReason, SessionReuseAudit,
};

#[derive(Eq, PartialEq)]
struct ComparableResult<'a> {
    terminal: HarnessTerminalReason,
    objective_reached: bool,
    first_hit_tick: Option<u64>,
    boundary_fingerprint: &'a Option<HarnessBoundaryFingerprint>,
    realized_input_sha256: Option<Digest>,
    gameplay_trace_sha256: Option<Digest>,
    objective_evidence_sha256: Option<Digest>,
    logical_ticks: u64,
    consumed_input_ticks: u64,
    session_reuse_audit: &'a Option<SessionReuseAudit>,
}

fn comparable_result(attempt: &ColdProcessBenchmarkAttempt) -> ComparableResult<'_> {
    ComparableResult {
        terminal: attempt.terminal,
        objective_reached: attempt.objective_reached,
        first_hit_tick: attempt.first_hit_tick,
        boundary_fingerprint: &attempt.boundary_fingerprint,
        realized_input_sha256: attempt.realized_input_sha256,
        gameplay_trace_sha256: attempt.gameplay_trace_sha256,
        objective_evidence_sha256: attempt.objective_evidence_sha256,
        logical_ticks: attempt.logical_ticks,
        consumed_input_ticks: attempt.consumed_input_ticks,
        session_reuse_audit: &attempt.native_phases.session_reuse_audit,
    }
}

pub(crate) fn comparison_issue(attempts: &[ColdProcessBenchmarkAttempt]) -> Option<String> {
    let first = attempts.first()?;
    if let Some(incomplete) = attempts.iter().find(|attempt| !attempt.artifacts_complete) {
        return Some(format!(
            "attempt {} did not retain complete authenticated artifacts",
            incomplete.attempt
        ));
    }
    attempts
        .iter()
        .skip(1)
        .find(|attempt| comparable_result(attempt) != comparable_result(first))
        .map(|attempt| {
            format!(
                "attempt {} is not semantically and artifact-identical to attempt 1",
                attempt.attempt
            )
        })
}
