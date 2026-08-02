use super::*;
use crate::native_tactic_route_runner::NATIVE_TACTIC_ACTION_SURFACE_AUDIT_CONTEXT_SCHEMA_V1;

const DISABLED_SUBSYSTEMS_V2: [&str; 7] = [
    "gpu_frame_submission",
    "cpu_renderer_submission",
    "presentation_lifecycle",
    "imgui_frame_lifecycle",
    "host_pacing",
    "host_audio_device",
    "state_hash_proof",
];

pub(super) fn validate(report: &NativeSubsystemParityReport) -> Result<(), Box<dyn Error>> {
    let definitions = condition_definitions_v2();
    let expected_names = definitions
        .iter()
        .map(|definition| definition.name)
        .chain(std::iter::once("state_hash_proof_disabled"))
        .collect::<Vec<_>>();
    if report.content_sha256 == Digest::ZERO
        || report.optimization_request_sha256 == Digest::ZERO
        || report.execution_sha256 == Digest::ZERO
        || report.executable_sha256 == Digest::ZERO
        || report.game_data_sha256 == Digest::ZERO
        || report.platform_os.is_empty()
        || report.platform_arch.is_empty()
        || report.recorded_unix_millis == 0
        || report.candidate_ticks == 0
        || report.disabled_subsystems
            != DISABLED_SUBSYSTEMS_V2
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        || report
            .conditions
            .iter()
            .map(|condition| condition.condition.as_str())
            .ne(expected_names.iter().copied())
    {
        return Err("native subsystem parity V2 report identity is invalid".into());
    }
    if report.content_sha256 != legacy_digest(report)?
        || report.passed != report.conditions.iter().all(|condition| condition.passed)
    {
        return Err("native subsystem parity V2 report digest or pass bit is invalid".into());
    }

    let production = report
        .conditions
        .iter()
        .find(|condition| condition.condition == "production_all_disabled")
        .ok_or("native subsystem parity V2 report lacks production treatment")?;
    for (index, condition) in report.conditions.iter().enumerate() {
        let expected_comparators = if index < definitions.len() {
            definitions[index].comparators
        } else {
            NativeHeadlessAuditComparators::default()
        };
        let expected_reference = if index < definitions.len() {
            definitions[index].reference
        } else {
            "production_all_disabled"
        };
        let expected_verify_state_hashes = condition.condition != "state_hash_proof_disabled";
        let reference = report
            .conditions
            .iter()
            .find(|candidate| candidate.condition == expected_reference)
            .ok_or("native subsystem parity V2 condition reference is missing")?;
        let expected_parity = if expected_verify_state_hashes {
            native_evidence_matches(&condition.evidence, &reference.evidence)
        } else {
            native_evidence_matches(&condition.evidence, &production.evidence)
                && condition
                    .evidence
                    .process_local_state_proof_sha256
                    .is_none()
        };
        let expected_configuration = validate_configuration_projection(
            expected_comparators,
            expected_verify_state_hashes,
            report.candidate_ticks,
            &condition.headless_audit,
            condition.cpu_renderer_submission_micros,
            &condition.gpu_work,
            &condition.state_validation,
        );
        let evidence_valid = !condition.evidence.source_boundary_fingerprint.is_empty()
            && condition.evidence.simulated_ticks == report.candidate_ticks
            && condition.evidence.native_state_trajectory_sha256 != Digest::ZERO
            && !condition.evidence.episode_payload_xxh3_128.is_empty()
            && condition
                .evidence
                .episode_payload_xxh3_128
                .iter()
                .all(|digest| !digest.is_empty())
            && condition.evidence.applicable_action_surface_sha256 != Digest::ZERO
            && action_surface_context_is_valid(
                &condition.evidence.applicable_action_surface_context,
            )
            && condition.evidence.applicable_action_surface_boundaries
                == condition.evidence.simulated_ticks
            && condition.evidence.applicable_action_descriptors
                >= condition.evidence.applicable_action_surface_boundaries
            && condition.evidence.controller_output_sha256 != Digest::ZERO
            && condition.evidence.first_hit_ticks.len()
                == condition.evidence.episode_payload_xxh3_128.len()
            && condition.evidence.terminal_evidence_sha256 != Digest::ZERO
            && !condition.evidence.terminal_boundary_fingerprints.is_empty()
            && condition
                .evidence
                .terminal_boundary_fingerprints
                .iter()
                .all(|fingerprint| !fingerprint.is_empty())
            && condition
                .evidence
                .process_local_state_proof_sha256
                .is_some()
                == expected_verify_state_hashes;
        if condition.reference_condition != expected_reference
            || condition.comparators != expected_comparators
            || condition.verify_state_hashes != expected_verify_state_hashes
            || !evidence_valid
            || condition.configuration_verified != expected_configuration
            || condition.evidence_parity != expected_parity
            || condition.passed != (condition.configuration_verified && condition.evidence_parity)
        {
            return Err(format!(
                "native subsystem parity V2 condition {} is inconsistent: reference={}, comparators={}, state_hashes={}, evidence={}, configuration={}=={}, parity={}=={}, passed={}=={}",
                condition.condition,
                condition.reference_condition == expected_reference,
                condition.comparators == expected_comparators,
                condition.verify_state_hashes == expected_verify_state_hashes,
                evidence_valid,
                condition.configuration_verified,
                expected_configuration,
                condition.evidence_parity,
                expected_parity,
                condition.passed,
                condition.configuration_verified && condition.evidence_parity,
            )
            .into());
        }
    }
    Ok(())
}

fn condition_definitions_v2() -> Vec<ConditionDefinition> {
    let suppressed = NativeHeadlessAuditComparators::default();
    let retained = NativeHeadlessAuditComparators {
        gpu_frame_submission: true,
        cpu_renderer_submission: true,
        presentation_lifecycle: true,
        imgui_frame_lifecycle: true,
        host_pacing: true,
        host_audio_device: true,
        ..suppressed
    };
    vec![
        ConditionDefinition {
            name: "production_all_disabled",
            reference: "production_all_disabled",
            comparators: suppressed,
        },
        ConditionDefinition {
            name: "gpu_frame_submission_retained",
            reference: "production_all_disabled",
            comparators: NativeHeadlessAuditComparators {
                gpu_frame_submission: true,
                ..suppressed
            },
        },
        ConditionDefinition {
            name: "presentation_lifecycle_retained",
            reference: "gpu_frame_submission_retained",
            comparators: NativeHeadlessAuditComparators {
                gpu_frame_submission: true,
                presentation_lifecycle: true,
                ..suppressed
            },
        },
        ConditionDefinition {
            name: "imgui_frame_lifecycle_retained",
            reference: "presentation_lifecycle_retained",
            comparators: NativeHeadlessAuditComparators {
                gpu_frame_submission: true,
                presentation_lifecycle: true,
                imgui_frame_lifecycle: true,
                ..suppressed
            },
        },
        ConditionDefinition {
            name: "cpu_renderer_submission_retained",
            reference: "imgui_frame_lifecycle_retained",
            comparators: NativeHeadlessAuditComparators {
                gpu_frame_submission: true,
                cpu_renderer_submission: true,
                presentation_lifecycle: true,
                imgui_frame_lifecycle: true,
                ..suppressed
            },
        },
        ConditionDefinition {
            name: "host_pacing_retained",
            reference: "production_all_disabled",
            comparators: NativeHeadlessAuditComparators {
                host_pacing: true,
                ..suppressed
            },
        },
        ConditionDefinition {
            name: "host_audio_device_retained",
            reference: "production_all_disabled",
            comparators: NativeHeadlessAuditComparators {
                host_audio_device: true,
                ..suppressed
            },
        },
        ConditionDefinition {
            name: "all_retained_composite",
            reference: "production_all_disabled",
            comparators: retained,
        },
    ]
}

pub(super) fn validate_configuration_projection(
    comparators: NativeHeadlessAuditComparators,
    verify_state_hashes: bool,
    candidate_ticks: u64,
    audit: &Value,
    cpu_renderer_submission_micros: u64,
    gpu_work: &Value,
    state_validation: &Value,
) -> bool {
    let expected = |enabled, retained, suppressed| if enabled { retained } else { suppressed };
    let gpu_submitted = gpu_work
        .get("submitted_command_buffers")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let discarded_frames = gpu_work
        .get("discarded_frames")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    audit.get("active").and_then(Value::as_bool) == Some(true)
        && audit.get("gpu_frame_submission").and_then(Value::as_str)
            == Some(expected(
                comparators.gpu_frame_submission,
                "retained_null_backend_comparator",
                "discarded_before_encoding",
            ))
        && audit.get("cpu_renderer_submission").and_then(Value::as_str)
            == Some(expected(
                comparators.cpu_renderer_submission,
                "retained_audit_comparator",
                "suppressed_on_candidate_ticks",
            ))
        && audit.get("presentation_lifecycle").and_then(Value::as_str)
            == Some(expected(
                comparators.presentation_lifecycle,
                "retained_audit_comparator",
                "suppressed",
            ))
        && audit.get("imgui_frame_lifecycle").and_then(Value::as_str)
            == Some(expected(
                comparators.imgui_frame_lifecycle,
                "retained_audit_comparator",
                "suppressed_on_candidate_ticks",
            ))
        && audit.get("host_pacing").and_then(Value::as_str)
            == Some(expected(comparators.host_pacing, "enabled", "disabled"))
        && audit.get("host_audio_device").and_then(Value::as_str)
            == Some(expected(
                comparators.host_audio_device,
                "active",
                "suppressed",
            ))
        && (!comparators.cpu_renderer_submission || cpu_renderer_submission_micros > 0)
        && (comparators.cpu_renderer_submission || cpu_renderer_submission_micros == 0)
        && if comparators.gpu_frame_submission {
            gpu_submitted > 0 && discarded_frames == 0
        } else {
            gpu_submitted == 0 && discarded_frames >= candidate_ticks
        }
        && state_validation.get("status").and_then(Value::as_str)
            == Some(if verify_state_hashes {
                "measured"
            } else {
                "disabled"
            })
}

fn action_surface_context_is_valid(context: &NativeTacticActionSurfaceAuditContext) -> bool {
    context.schema == NATIVE_TACTIC_ACTION_SURFACE_AUDIT_CONTEXT_SCHEMA_V1
        && context.action_schema_sha256 != Digest::ZERO
        && context
            .goal_coordinate_f32_bits
            .map(f32::from_bits)
            .iter()
            .all(|value| value.is_finite())
        && context.maximum_ticks > 0
        && context.maximum_ticks <= 40
        && context.seed == 0
}

#[derive(Serialize)]
struct LegacyComparatorProjection {
    gpu_frame_submission: bool,
    cpu_renderer_submission: bool,
    presentation_lifecycle: bool,
    imgui_frame_lifecycle: bool,
    host_pacing: bool,
    host_audio_device: bool,
}

#[derive(Serialize)]
struct LegacyConditionProjection<'a> {
    condition: &'a str,
    reference_condition: &'a str,
    comparators: LegacyComparatorProjection,
    verify_state_hashes: bool,
    launch: &'a NativeSuffixWorkerLaunchTiming,
    batch_wall_micros: u64,
    simulation_micros: u64,
    cpu_renderer_submission_micros: u64,
    headless_audit: &'a Value,
    gpu_work: &'a Value,
    state_validation: &'a Value,
    evidence: &'a NativeSubsystemEvidenceProjection,
    configuration_verified: bool,
    evidence_parity: bool,
    passed: bool,
}

#[derive(Serialize)]
struct LegacyReportProjection<'a> {
    schema: &'a str,
    content_sha256: Digest,
    optimization_request_sha256: Digest,
    execution_sha256: Digest,
    executable_sha256: Digest,
    game_data_sha256: Digest,
    platform_os: &'a str,
    platform_arch: &'a str,
    recorded_unix_millis: u64,
    source_frame: u64,
    candidate_ticks: u64,
    disabled_subsystems: &'a [String],
    conditions: Vec<LegacyConditionProjection<'a>>,
    passed: bool,
}

fn legacy_digest(report: &NativeSubsystemParityReport) -> Result<Digest, Box<dyn Error>> {
    let conditions = report
        .conditions
        .iter()
        .map(|condition| LegacyConditionProjection {
            condition: &condition.condition,
            reference_condition: &condition.reference_condition,
            comparators: LegacyComparatorProjection {
                gpu_frame_submission: condition.comparators.gpu_frame_submission,
                cpu_renderer_submission: condition.comparators.cpu_renderer_submission,
                presentation_lifecycle: condition.comparators.presentation_lifecycle,
                imgui_frame_lifecycle: condition.comparators.imgui_frame_lifecycle,
                host_pacing: condition.comparators.host_pacing,
                host_audio_device: condition.comparators.host_audio_device,
            },
            verify_state_hashes: condition.verify_state_hashes,
            launch: &condition.launch,
            batch_wall_micros: condition.batch_wall_micros,
            simulation_micros: condition.simulation_micros,
            cpu_renderer_submission_micros: condition.cpu_renderer_submission_micros,
            headless_audit: &condition.headless_audit,
            gpu_work: &condition.gpu_work,
            state_validation: &condition.state_validation,
            evidence: &condition.evidence,
            configuration_verified: condition.configuration_verified,
            evidence_parity: condition.evidence_parity,
            passed: condition.passed,
        })
        .collect();
    digest_json(&LegacyReportProjection {
        schema: &report.schema,
        content_sha256: Digest::ZERO,
        optimization_request_sha256: report.optimization_request_sha256,
        execution_sha256: report.execution_sha256,
        executable_sha256: report.executable_sha256,
        game_data_sha256: report.game_data_sha256,
        platform_os: &report.platform_os,
        platform_arch: &report.platform_arch,
        recorded_unix_millis: report.recorded_unix_millis,
        source_frame: report.source_frame,
        candidate_ticks: report.candidate_ticks,
        disabled_subsystems: &report.disabled_subsystems,
        conditions,
        passed: report.passed,
    })
}
