//! Paired native evidence for every subsystem suppressed by the farming path.

use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::native_suffix_result::{
    NativeSuffixBatchResult, NativeSuffixCandidateResult, NativeTerminalBinding,
};
use crate::native_suffix_worker::{
    NativeHeadlessAuditComparators, NativeSuffixPrevalidatedFileIdentities,
    NativeSuffixWorkerLaunch, NativeSuffixWorkerLaunchTiming, NativeSuffixWorkerSession,
};
use crate::native_tactic_route_runner::{
    NativeTacticActionSurfaceAuditContext, native_tactic_action_surface_audit_context,
    native_tactic_applicable_action_surface_identity,
};
use crate::native_tactic_worker::pad_runs;
use crate::optimization_request::OptimizationRequest;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_evidence::native_episode_shard::{NativeEpisodeShard, NativeRawPad};
use dusklight_search::suffix_batch::{
    NATIVE_SUFFIX_BATCH_SCHEMA, NativeCheckpointValidation, NativeSuffixBatch,
    NativeSuffixCandidate,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const NATIVE_SUBSYSTEM_PARITY_SCHEMA_V2: &str = "dusklight-native-subsystem-parity/v2";
pub const NATIVE_SUBSYSTEM_PARITY_SCHEMA: &str = "dusklight-native-subsystem-parity/v3";
mod evidence_bundle;
mod legacy_v2;
pub use evidence_bundle::{
    NATIVE_SUBSYSTEM_PARITY_EVIDENCE_BUNDLE_SCHEMA_V1, NATIVE_SUBSYSTEM_PARITY_EVIDENCE_MANIFEST,
    NativeSubsystemParityBundleArtifact, NativeSubsystemParityConditionEvidence,
    NativeSubsystemParityEvidenceBundle,
};
const DISABLED_SUBSYSTEMS: [&str; 9] = [
    "gpu_frame_submission",
    "cpu_renderer_submission",
    "presentation_lifecycle",
    "imgui_frame_lifecycle",
    "host_pacing",
    "host_audio_device",
    "deterministic_audio_emulation",
    "game_audio_update",
    "state_hash_proof",
];
const MAX_CONCURRENT_PARITY_CONDITIONS: usize = 2;

pub struct NativeSubsystemParityConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub output_root: &'a Path,
    pub candidate_ticks: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSubsystemEvidenceProjection {
    pub source_boundary_fingerprint: String,
    pub simulated_ticks: u64,
    pub native_state_trajectory_sha256: Digest,
    pub episode_payload_xxh3_128: Vec<String>,
    pub applicable_action_surface_context: NativeTacticActionSurfaceAuditContext,
    pub applicable_action_surface_sha256: Digest,
    pub applicable_action_surface_boundaries: u64,
    pub applicable_action_descriptors: u64,
    pub controller_output_sha256: Digest,
    pub first_hit_ticks: Vec<Option<u64>>,
    pub terminal_evidence_sha256: Digest,
    pub terminal_boundary_fingerprints: Vec<String>,
    /// Diagnostic digest of the full state proof emitted inside one native process.
    ///
    /// This is deliberately not a cross-process parity identity: the proof includes
    /// process-local pointers and allocator state. Each proof-enabled condition must
    /// validate that state in-process; cross-condition parity uses the authenticated
    /// native episode payload and terminal boundaries below.
    pub process_local_state_proof_sha256: Option<Digest>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSubsystemConditionMeasurement {
    pub condition: String,
    pub reference_condition: String,
    pub comparators: NativeHeadlessAuditComparators,
    pub verify_state_hashes: bool,
    pub launch: NativeSuffixWorkerLaunchTiming,
    pub batch_wall_micros: u64,
    pub simulation_micros: u64,
    #[serde(default)]
    pub cpu_draw_traversal_micros: u64,
    pub cpu_renderer_submission_micros: u64,
    #[serde(default)]
    pub deterministic_audio_emulation_micros: u64,
    #[serde(default)]
    pub game_audio_update_micros: u64,
    pub headless_audit: Value,
    pub gpu_work: Value,
    pub state_validation: Value,
    pub evidence: NativeSubsystemEvidenceProjection,
    pub configuration_verified: bool,
    pub evidence_parity: bool,
    pub passed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSubsystemParityReport {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub execution_sha256: Digest,
    pub executable_sha256: Digest,
    pub game_data_sha256: Digest,
    pub platform_os: String,
    pub platform_arch: String,
    pub recorded_unix_millis: u64,
    pub source_frame: u64,
    pub candidate_ticks: u64,
    pub disabled_subsystems: Vec<String>,
    pub conditions: Vec<NativeSubsystemConditionMeasurement>,
    pub passed: bool,
}

#[derive(Clone, Copy)]
struct ConditionDefinition {
    name: &'static str,
    reference: &'static str,
    comparators: NativeHeadlessAuditComparators,
}

impl NativeSubsystemParityReport {
    pub fn to_pretty_json(&self) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn validate(&self) -> Result<(), Box<dyn Error>> {
        if self.schema == NATIVE_SUBSYSTEM_PARITY_SCHEMA_V2 {
            return legacy_v2::validate(self);
        }
        let expected_names = condition_definitions()
            .into_iter()
            .map(|definition| definition.name)
            .chain(std::iter::once("state_hash_proof_disabled"))
            .collect::<Vec<_>>();
        if self.schema != NATIVE_SUBSYSTEM_PARITY_SCHEMA
            || self.content_sha256 == Digest::ZERO
            || self.optimization_request_sha256 == Digest::ZERO
            || self.execution_sha256 == Digest::ZERO
            || self.executable_sha256 == Digest::ZERO
            || self.game_data_sha256 == Digest::ZERO
            || self.platform_os.is_empty()
            || self.platform_arch.is_empty()
            || self.recorded_unix_millis == 0
            || self.candidate_ticks == 0
            || self.disabled_subsystems
                != DISABLED_SUBSYSTEMS
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            || self
                .conditions
                .iter()
                .map(|condition| condition.condition.as_str())
                .ne(expected_names.iter().copied())
        {
            return Err("native subsystem parity report identity is invalid".into());
        }
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        if self.content_sha256 != digest_json(&canonical)?
            || self.passed != self.conditions.iter().all(|condition| condition.passed)
        {
            return Err("native subsystem parity report digest or pass bit is invalid".into());
        }
        let production = self
            .conditions
            .iter()
            .find(|condition| condition.condition == "production_all_disabled")
            .ok_or("native subsystem parity report lacks production treatment")?;
        for (index, condition) in self.conditions.iter().enumerate() {
            let expected_comparators = if index < expected_names.len() - 1 {
                condition_definitions()[index].comparators
            } else {
                NativeHeadlessAuditComparators::production()
            };
            let expected_reference = if index < expected_names.len() - 1 {
                condition_definitions()[index].reference
            } else {
                "production_all_disabled"
            };
            let expected_verify_state_hashes = condition.condition != "state_hash_proof_disabled";
            let reference = self
                .conditions
                .iter()
                .find(|candidate| candidate.condition == expected_reference)
                .ok_or("native subsystem parity condition reference is missing")?;
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
                self.candidate_ticks,
                &condition.headless_audit,
                condition.cpu_renderer_submission_micros,
                &condition.gpu_work,
                &condition.state_validation,
            );
            let evidence_valid = !condition.evidence.source_boundary_fingerprint.is_empty()
                && condition.evidence.simulated_ticks == self.candidate_ticks
                && condition.evidence.native_state_trajectory_sha256 != Digest::ZERO
                && !condition.evidence.episode_payload_xxh3_128.is_empty()
                && condition
                    .evidence
                    .episode_payload_xxh3_128
                    .iter()
                    .all(|digest| !digest.is_empty())
                && condition.evidence.applicable_action_surface_sha256 != Digest::ZERO
                && condition
                    .evidence
                    .applicable_action_surface_context
                    .validate()
                    .is_ok()
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
                || condition.passed
                    != (condition.configuration_verified && condition.evidence_parity)
            {
                return Err(format!(
                    "native subsystem parity condition {} is inconsistent",
                    condition.condition
                )
                .into());
            }
        }
        Ok(())
    }
}

pub fn run_native_subsystem_parity(
    config: &NativeSubsystemParityConfig<'_>,
) -> Result<NativeSubsystemParityReport, Box<dyn Error>> {
    validate_config(config)?;
    let root = config.repository_root.canonicalize()?;
    config
        .execution
        .validate_files(&root, config.optimization)?;
    let output_root = create_output_root(config.output_root)?;
    let process_tape_path = root.join(&config.execution.process_boot_tape.path);
    let process_tape = InputTape::decode(&fs::read(&process_tape_path)?)?.tape;
    let source_frame = usize::try_from(config.optimization.route.source_boundary_index)?;
    if source_frame
        .checked_add(config.candidate_ticks)
        .is_none_or(|end| end > process_tape.frames.len())
    {
        return Err("native subsystem parity candidate exceeds the process tape".into());
    }
    let terminal = NativeTerminalBinding {
        goal: config.optimization.terminal_predicate.goal.clone(),
        program_sha256: config.optimization.terminal_predicate.program_sha256,
        definition_sha256: config.optimization.terminal_predicate.definition_sha256,
    };
    let inputs = ConditionInputs {
        optimization: config.optimization,
        execution: config.execution,
        root: root.clone(),
        executable: root.join(&config.execution.executable.path),
        game_data: root.join(&config.execution.game_data.path),
        input_tape: process_tape_path,
        milestone_program: root.join(&config.execution.milestone_program.path),
        card_fixture: config
            .execution
            .card_fixture_root(&root, config.optimization)?,
        card_fixture_sha256: config.execution.card_fixture_manifest.sha256,
        world_context_sha256: config.execution.world_context.sha256,
        file_identities: NativeSuffixPrevalidatedFileIdentities {
            executable_sha256: config.execution.executable.sha256,
            game_data_sha256: config.execution.game_data.sha256,
        },
        terminal,
        batch: make_batch(config, &process_tape, true)?,
        proof_disabled_batch: make_batch(config, &process_tape, false)?,
    };

    let definitions = condition_definitions();
    let mut runs = Vec::with_capacity(definitions.len());
    for batch in definitions.chunks(MAX_CONCURRENT_PARITY_CONDITIONS) {
        let mut completed = std::thread::scope(|scope| {
            batch
                .iter()
                .copied()
                .map(|definition| {
                    let inputs = &inputs;
                    let output_root = &output_root;
                    scope.spawn(move || run_condition(inputs, output_root, definition))
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .map_err(|_| "native subsystem parity worker thread panicked".to_owned())?
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .map_err(|message| -> Box<dyn Error> { message.into() })?;
        runs.append(&mut completed);
    }

    let production_evidence = runs
        .iter()
        .find(|run| run.measurement.condition == "production_all_disabled")
        .ok_or("native subsystem parity produced no production treatment")?
        .measurement
        .evidence
        .clone();
    let reference_evidence = runs
        .iter()
        .map(|run| {
            (
                run.measurement.condition.clone(),
                run.measurement.evidence.clone(),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let mut conditions = Vec::with_capacity(runs.len() + 1);
    for run in &mut runs {
        let expected_evidence = reference_evidence
            .get(&run.measurement.reference_condition)
            .ok_or("native subsystem parity run reference is missing")?;
        run.measurement.evidence_parity =
            native_evidence_matches(&run.measurement.evidence, expected_evidence);
        run.measurement.passed =
            run.measurement.configuration_verified && run.measurement.evidence_parity;
        conditions.push(run.measurement.clone());
    }
    let proof_disabled = runs
        .into_iter()
        .find_map(|run| run.proof_disabled)
        .ok_or("production condition did not emit the proof-disabled treatment")?;
    let mut proof_disabled = proof_disabled;
    proof_disabled.evidence_parity =
        native_evidence_matches(&proof_disabled.evidence, &production_evidence)
            && proof_disabled
                .evidence
                .process_local_state_proof_sha256
                .is_none();
    proof_disabled.passed = proof_disabled.configuration_verified && proof_disabled.evidence_parity;
    conditions.push(proof_disabled);

    let mut report = NativeSubsystemParityReport {
        schema: NATIVE_SUBSYSTEM_PARITY_SCHEMA.into(),
        content_sha256: Digest::ZERO,
        optimization_request_sha256: config.optimization.content_sha256,
        execution_sha256: config.execution.content_sha256,
        executable_sha256: config.execution.executable.sha256,
        game_data_sha256: config.execution.game_data.sha256,
        platform_os: std::env::consts::OS.into(),
        platform_arch: std::env::consts::ARCH.into(),
        recorded_unix_millis: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64,
        source_frame: source_frame as u64,
        candidate_ticks: config.candidate_ticks as u64,
        disabled_subsystems: DISABLED_SUBSYSTEMS.into_iter().map(str::to_owned).collect(),
        passed: conditions.iter().all(|condition| condition.passed),
        conditions,
    };
    report.content_sha256 = digest_json(&report)?;
    report.validate()?;
    Ok(report)
}

struct ConditionInputs<'a> {
    optimization: &'a OptimizationRequest,
    execution: &'a NativeResidualExecutionBinding,
    root: PathBuf,
    executable: PathBuf,
    game_data: PathBuf,
    input_tape: PathBuf,
    milestone_program: PathBuf,
    card_fixture: PathBuf,
    card_fixture_sha256: Digest,
    world_context_sha256: Digest,
    file_identities: NativeSuffixPrevalidatedFileIdentities,
    terminal: NativeTerminalBinding,
    batch: NativeSuffixBatch,
    proof_disabled_batch: NativeSuffixBatch,
}

struct ConditionRun {
    measurement: NativeSubsystemConditionMeasurement,
    proof_disabled: Option<NativeSubsystemConditionMeasurement>,
}

fn run_condition(
    inputs: &ConditionInputs<'_>,
    output_root: &Path,
    definition: ConditionDefinition,
) -> Result<ConditionRun, String> {
    (|| -> Result<ConditionRun, Box<dyn Error>> {
        let condition_root = output_root.join(definition.name);
        fs::create_dir(&condition_root)?;
        let batch_path = condition_root.join("request.json");
        let result_path = condition_root.join("result.json");
        write_new_json(&batch_path, &inputs.batch)?;
        let launch = NativeSuffixWorkerLaunch {
            executable: inputs.executable.clone(),
            game_data: inputs.game_data.clone(),
            input_tape: inputs.input_tape.clone(),
            milestone_program: inputs.milestone_program.clone(),
            card_fixture: inputs.card_fixture.clone(),
            card_fixture_sha256: inputs.card_fixture_sha256,
            working_directory: inputs.root.clone(),
            state_root: condition_root.join("state"),
            world_context_sha256: inputs.world_context_sha256,
            terminal: inputs.terminal.clone(),
            initial_batch: batch_path,
            initial_result: result_path.clone(),
            initial_winner_tape: None,
        };
        let (mut worker, validated, launch_timing) =
            NativeSuffixWorkerSession::launch_profiled_with_audit_comparators(
                &launch,
                inputs.file_identities,
                definition.comparators,
            )?;
        let raw: NativeSuffixBatchResult = serde_json::from_slice(&fs::read(&result_path)?)?;
        let primary_measurement = measurement(
            inputs,
            definition.name,
            definition.reference,
            definition.comparators,
            true,
            launch_timing,
            &validated.episode_shard_path,
            &raw,
        )?;
        let proof_disabled = if definition.name == "production_all_disabled" {
            let batch_path = condition_root.join("proof-disabled.request.json");
            let result_path = condition_root.join("proof-disabled.result.json");
            write_new_json(&batch_path, &inputs.proof_disabled_batch)?;
            let validated = worker.run_batch(&batch_path, &result_path, None)?;
            let raw: NativeSuffixBatchResult = serde_json::from_slice(&fs::read(&result_path)?)?;
            Some(measurement(
                inputs,
                "state_hash_proof_disabled",
                "production_all_disabled",
                definition.comparators,
                false,
                NativeSuffixWorkerLaunchTiming {
                    spawn_call_micros: 0,
                    handshake_micros: 0,
                    initial_batch_wait_micros: 0,
                    artifact_validation_micros: 0,
                    total_micros: 0,
                },
                &validated.episode_shard_path,
                &raw,
            )?)
        } else {
            None
        };
        worker.shutdown()?;
        Ok(ConditionRun {
            measurement: primary_measurement,
            proof_disabled,
        })
    })()
    .map_err(|error| format!("condition {} failed: {error}", definition.name))
}

fn measurement(
    inputs: &ConditionInputs<'_>,
    condition: &str,
    reference_condition: &str,
    comparators: NativeHeadlessAuditComparators,
    verify_state_hashes: bool,
    launch: NativeSuffixWorkerLaunchTiming,
    episode_shard_path: &str,
    raw: &NativeSuffixBatchResult,
) -> Result<NativeSubsystemConditionMeasurement, Box<dyn Error>> {
    let shard = NativeEpisodeShard::read(Path::new(episode_shard_path))?;
    let phases = raw
        .timing
        .phases
        .as_object()
        .ok_or("native suffix timing phases are not an object")?;
    let cpu_renderer_submission = phases
        .get("cpu_renderer_submission")
        .ok_or("native suffix timing lacks CPU renderer submission")?
        .clone();
    let cpu_renderer_submission_micros = cpu_renderer_submission
        .get("micros")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let gpu_work = phases
        .get("gpu_work")
        .ok_or("native suffix timing lacks GPU work")?
        .clone();
    let state_validation = phases
        .get("state_validation")
        .ok_or("native suffix timing lacks state validation")?
        .clone();
    let evidence = evidence_projection(inputs, raw, &shard)?;
    let configuration_verified = validate_configuration_projection(
        comparators,
        verify_state_hashes,
        raw.timing.candidate_ticks,
        &raw.timing.headless_audit,
        cpu_renderer_submission_micros,
        &gpu_work,
        &state_validation,
    );
    Ok(NativeSubsystemConditionMeasurement {
        condition: condition.into(),
        reference_condition: reference_condition.into(),
        comparators,
        verify_state_hashes,
        launch,
        batch_wall_micros: raw.timing.batch_wall_micros,
        simulation_micros: phase_micros(raw, "simulation")?,
        cpu_draw_traversal_micros: phase_micros(raw, "cpu_draw_traversal")?,
        cpu_renderer_submission_micros,
        deterministic_audio_emulation_micros: phase_micros(raw, "audio_emulation")?,
        game_audio_update_micros: phase_micros(raw, "game_audio_update")?,
        headless_audit: raw.timing.headless_audit.clone(),
        gpu_work,
        state_validation,
        evidence,
        configuration_verified,
        evidence_parity: false,
        passed: false,
    })
}

fn evidence_projection(
    inputs: &ConditionInputs<'_>,
    raw: &NativeSuffixBatchResult,
    shard: &NativeEpisodeShard,
) -> Result<NativeSubsystemEvidenceProjection, Box<dyn Error>> {
    let context = native_tactic_action_surface_audit_context(
        &inputs.root,
        inputs.optimization,
        inputs.execution,
        shard,
    )?;
    evidence_projection_with_context(&context, raw, shard)
}

fn evidence_projection_with_context(
    applicable_action_surface_context: &NativeTacticActionSurfaceAuditContext,
    raw: &NativeSuffixBatchResult,
    shard: &NativeEpisodeShard,
) -> Result<NativeSubsystemEvidenceProjection, Box<dyn Error>> {
    let action_surface =
        native_tactic_applicable_action_surface_identity(applicable_action_surface_context, shard)?;
    evidence_projection_with_action_surface(
        applicable_action_surface_context,
        action_surface,
        raw,
        shard,
    )
}

fn evidence_projection_with_action_surface(
    applicable_action_surface_context: &NativeTacticActionSurfaceAuditContext,
    action_surface: (Digest, u64, u64),
    raw: &NativeSuffixBatchResult,
    shard: &NativeEpisodeShard,
) -> Result<NativeSubsystemEvidenceProjection, Box<dyn Error>> {
    if raw.source_frame != shard.source_frame
        || raw.maximum_ticks != u64::from(shard.maximum_ticks)
        || raw.episode_shard.observation_schema != shard.metadata.observation_schema
        || raw.episode_shard.action_schema != shard.metadata.action_schema
        || raw.episode_shard.episode_count != u64::try_from(shard.episodes.len())?
        || raw.episode_shard.uncompressed_bytes != shard.uncompressed_bytes
        || raw.episode_shard.compressed_bytes != shard.compressed_bytes
        || raw.candidates.len() != shard.episodes.len()
        || raw
            .candidates
            .iter()
            .zip(&shard.episodes)
            .any(|(candidate, episode)| {
                candidate.id != episode.id
                    || candidate.success != episode.success
                    || candidate.ticks_executed != u64::from(episode.ticks_executed)
                    || candidate.first_hit_tick != episode.first_hit_tick.map(u64::from)
            })
    {
        return Err("native subsystem result is detached from its episode shard".into());
    }
    let terminal_evidence_sha256 = digest_json(
        &raw.candidates
            .iter()
            .map(terminal_projection)
            .collect::<Vec<_>>(),
    )?;
    let (
        applicable_action_surface_sha256,
        applicable_action_surface_boundaries,
        applicable_action_descriptors,
    ) = action_surface;
    let process_local_state_proof_sha256 = raw
        .verify_state_hashes
        .then(|| {
            digest_json(
                &raw.candidates
                    .iter()
                    .map(|candidate| {
                        (
                            &candidate.state_sequence_digest,
                            &candidate.state_tick_digests,
                            &candidate.terminal_state_entry_digests,
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .transpose()?;
    Ok(NativeSubsystemEvidenceProjection {
        source_boundary_fingerprint: raw
            .source_boundary
            .actual_fingerprint
            .clone()
            .ok_or("native subsystem result lacks actual source fingerprint")?,
        simulated_ticks: raw
            .candidates
            .iter()
            .map(|candidate| candidate.ticks_executed)
            .sum(),
        native_state_trajectory_sha256: native_state_trajectory_sha256(shard),
        episode_payload_xxh3_128: shard
            .episodes
            .iter()
            .map(|episode| hex_bytes(&episode.payload_xxh3_128))
            .collect(),
        applicable_action_surface_context: applicable_action_surface_context.clone(),
        applicable_action_surface_sha256,
        applicable_action_surface_boundaries,
        applicable_action_descriptors,
        controller_output_sha256: controller_output_sha256(shard),
        first_hit_ticks: raw
            .candidates
            .iter()
            .map(|candidate| candidate.first_hit_tick)
            .collect(),
        terminal_evidence_sha256,
        terminal_boundary_fingerprints: raw
            .candidates
            .iter()
            .map(|candidate| candidate.terminal_boundary_fingerprint.clone())
            .collect(),
        process_local_state_proof_sha256,
    })
}

fn native_state_trajectory_sha256(shard: &NativeEpisodeShard) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-native-subsystem-state-trajectory/v1");
    hasher.update((shard.episodes.len() as u64).to_le_bytes());
    for episode in &shard.episodes {
        hasher.update((episode.id.len() as u64).to_le_bytes());
        hasher.update(episode.id.as_bytes());
        hasher.update((episode.steps.len() as u64).to_le_bytes());
        for step in &episode.steps {
            for observation in [&step.pre_input, &step.post_simulation] {
                hasher.update(observation.state_identity);
                hasher.update(observation.boundary_index.to_le_bytes());
                hasher.update(observation.simulation_tick.to_le_bytes());
                hasher.update(observation.tape_frame.to_le_bytes());
            }
        }
    }
    Digest(hasher.finalize().into())
}

fn controller_output_sha256(shard: &NativeEpisodeShard) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-native-subsystem-controller-output/v1");
    hasher.update((shard.episodes.len() as u64).to_le_bytes());
    for episode in &shard.episodes {
        hasher.update((episode.id.len() as u64).to_le_bytes());
        hasher.update(episode.id.as_bytes());
        hasher.update((episode.steps.len() as u64).to_le_bytes());
        for step in &episode.steps {
            hash_native_pad(&mut hasher, step.chosen_pad);
            hash_native_pad(&mut hasher, step.consumed_pad);
        }
    }
    Digest(hasher.finalize().into())
}

fn hash_native_pad(hasher: &mut Sha256, pad: NativeRawPad) {
    hasher.update(pad.buttons.to_le_bytes());
    hasher.update(pad.stick_x.to_le_bytes());
    hasher.update(pad.stick_y.to_le_bytes());
    hasher.update(pad.substick_x.to_le_bytes());
    hasher.update(pad.substick_y.to_le_bytes());
    hasher.update([
        pad.trigger_left,
        pad.trigger_right,
        pad.analog_a,
        pad.analog_b,
    ]);
    hasher.update([u8::from(pad.connected)]);
    hasher.update(pad.error.to_le_bytes());
}

fn terminal_projection(candidate: &NativeSuffixCandidateResult) -> Value {
    serde_json::json!({
        "id": candidate.id,
        "success": candidate.success,
        "ticks_executed": candidate.ticks_executed,
        "first_hit_tick": candidate.first_hit_tick,
        "predicate_evidence": candidate.predicate_evidence,
        "terminal_observation": candidate.terminal_observation,
        "terminal_boundary_fingerprint": candidate.terminal_boundary_fingerprint,
        "consumed_pad_states": candidate.consumed_pad_states,
    })
}

fn validate_configuration_projection(
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
    let expected_suppression = |suppressed| {
        if suppressed {
            "suppressed_on_candidate_ticks"
        } else {
            "retained"
        }
    };
    audit.get("active").and_then(Value::as_bool) == Some(true)
        && audit
            .get("deterministic_audio_emulation")
            .and_then(Value::as_str)
            == Some(expected_suppression(
                comparators.suppress_deterministic_audio_emulation,
            ))
        && audit.get("game_audio_update").and_then(Value::as_str)
            == Some(expected_suppression(comparators.suppress_game_audio_update))
        && audit.get("gameplay_draw_traversal").and_then(Value::as_str)
            == Some(expected_suppression(
                comparators.suppress_cpu_draw_traversal,
            ))
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

fn make_batch(
    config: &NativeSubsystemParityConfig<'_>,
    tape: &InputTape,
    verify_state_hashes: bool,
) -> Result<NativeSuffixBatch, Box<dyn Error>> {
    let source_frame = usize::try_from(config.optimization.route.source_boundary_index)?;
    let frames = &tape.frames[source_frame..source_frame + config.candidate_ticks];
    Ok(NativeSuffixBatch {
        schema: NATIVE_SUFFIX_BATCH_SCHEMA.into(),
        source_frame,
        source_boundary_fingerprint: config
            .optimization
            .route
            .native_source_boundary_fingerprint
            .clone(),
        checkpoint_validation: NativeCheckpointValidation {
            kind: "recorded_replay_window".into(),
            ticks: config.execution.checkpoint_validation_ticks as usize,
        },
        maximum_ticks: config.candidate_ticks,
        verify_state_hashes,
        checkpoint_cache: None,
        candidates: vec![NativeSuffixCandidate {
            id: "native-subsystem-parity".into(),
            actions: pad_runs(frames)?,
            controller_program_hex: None,
            maximum_ticks: None,
            cancellation_guard: None,
        }],
    })
}

fn condition_definitions() -> Vec<ConditionDefinition> {
    let suppressed = NativeHeadlessAuditComparators::production();
    let retained = NativeHeadlessAuditComparators {
        gpu_frame_submission: true,
        cpu_renderer_submission: true,
        presentation_lifecycle: true,
        imgui_frame_lifecycle: true,
        host_pacing: true,
        host_audio_device: true,
        ..NativeHeadlessAuditComparators::default()
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
            name: "deterministic_audio_emulation_retained",
            reference: "production_all_disabled",
            comparators: NativeHeadlessAuditComparators {
                suppress_deterministic_audio_emulation: false,
                ..suppressed
            },
        },
        ConditionDefinition {
            name: "game_audio_update_retained",
            reference: "production_all_disabled",
            comparators: NativeHeadlessAuditComparators {
                suppress_game_audio_update: false,
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

fn native_evidence_matches(
    left: &NativeSubsystemEvidenceProjection,
    right: &NativeSubsystemEvidenceProjection,
) -> bool {
    left.source_boundary_fingerprint == right.source_boundary_fingerprint
        && left.simulated_ticks == right.simulated_ticks
        && left.native_state_trajectory_sha256 == right.native_state_trajectory_sha256
        && left.episode_payload_xxh3_128 == right.episode_payload_xxh3_128
        && left.applicable_action_surface_context == right.applicable_action_surface_context
        && left.applicable_action_surface_sha256 == right.applicable_action_surface_sha256
        && left.applicable_action_surface_boundaries == right.applicable_action_surface_boundaries
        && left.applicable_action_descriptors == right.applicable_action_descriptors
        && left.controller_output_sha256 == right.controller_output_sha256
        && left.first_hit_ticks == right.first_hit_ticks
        && left.terminal_evidence_sha256 == right.terminal_evidence_sha256
        && left.terminal_boundary_fingerprints == right.terminal_boundary_fingerprints
}

fn validate_config(config: &NativeSubsystemParityConfig<'_>) -> Result<(), Box<dyn Error>> {
    if config.candidate_ticks == 0
        || config.candidate_ticks > 4_096
        || config.output_root.exists()
        || config.optimization.content_sha256 == Digest::ZERO
        || config.execution.content_sha256 == Digest::ZERO
        || config.execution.optimization_request_sha256 != config.optimization.content_sha256
    {
        return Err("native subsystem parity config is invalid".into());
    }
    Ok(())
}

fn create_output_root(path: &Path) -> Result<PathBuf, Box<dyn Error>> {
    fs::create_dir_all(path)?;
    Ok(path.canonicalize()?)
}

fn phase_micros(raw: &NativeSuffixBatchResult, phase: &str) -> Result<u64, Box<dyn Error>> {
    raw.timing
        .phases
        .get(phase)
        .and_then(|value| value.get("micros"))
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("native suffix timing phase {phase:?} lacks measured micros").into())
}

fn digest_json(value: &impl Serialize) -> Result<Digest, Box<dyn Error>> {
    Ok(Digest(Sha256::digest(serde_json::to_vec(value)?).into()))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|byte| [HEX[(byte >> 4) as usize], HEX[(byte & 0xf) as usize]])
        .map(char::from)
        .collect()
}

fn write_new_json(path: &Path, value: &impl Serialize) -> Result<(), Box<dyn Error>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn action_surface_context() -> NativeTacticActionSurfaceAuditContext {
        NativeTacticActionSurfaceAuditContext {
            schema: crate::native_tactic_route_runner::
                NATIVE_TACTIC_ACTION_SURFACE_AUDIT_CONTEXT_SCHEMA_V1
                .into(),
            action_schema_sha256:
                crate::native_tactic_route_runner::parameterized_policy_action_schema_sha256(None),
            goal_coordinate_f32_bits: [1.0_f32.to_bits(), 2.0_f32.to_bits(), 3.0_f32.to_bits()],
            maximum_ticks: 16,
            seed: 0,
        }
    }

    #[test]
    fn conditions_retain_each_subsystem_against_a_legal_reference() {
        let conditions = condition_definitions();
        assert_eq!(conditions.len(), 10);
        assert_eq!(conditions[0].name, "production_all_disabled");
        assert_eq!(conditions[9].name, "all_retained_composite");
        for condition in &conditions[1..9] {
            let reference = conditions
                .iter()
                .find(|candidate| candidate.name == condition.reference)
                .unwrap();
            let changed = [
                condition.comparators.gpu_frame_submission
                    != reference.comparators.gpu_frame_submission,
                condition.comparators.cpu_renderer_submission
                    != reference.comparators.cpu_renderer_submission,
                condition.comparators.presentation_lifecycle
                    != reference.comparators.presentation_lifecycle,
                condition.comparators.imgui_frame_lifecycle
                    != reference.comparators.imgui_frame_lifecycle,
                condition.comparators.host_pacing != reference.comparators.host_pacing,
                condition.comparators.host_audio_device != reference.comparators.host_audio_device,
                condition.comparators.suppress_cpu_draw_traversal
                    != reference.comparators.suppress_cpu_draw_traversal,
                condition.comparators.suppress_deterministic_audio_emulation
                    != reference.comparators.suppress_deterministic_audio_emulation,
                condition.comparators.suppress_game_audio_update
                    != reference.comparators.suppress_game_audio_update,
            ]
            .into_iter()
            .filter(|changed| *changed)
            .count();
            assert_eq!(changed, 1, "{}", condition.name);
        }
    }

    #[test]
    fn cross_process_evidence_excludes_only_the_process_local_state_proof() {
        let evidence = NativeSubsystemEvidenceProjection {
            source_boundary_fingerprint: "source".into(),
            simulated_ticks: 16,
            native_state_trajectory_sha256: Digest([1; 32]),
            episode_payload_xxh3_128: vec!["episode".into()],
            applicable_action_surface_context: action_surface_context(),
            applicable_action_surface_sha256: Digest([2; 32]),
            applicable_action_surface_boundaries: 16,
            applicable_action_descriptors: 64,
            controller_output_sha256: Digest([3; 32]),
            first_hit_ticks: vec![Some(15)],
            terminal_evidence_sha256: Digest([4; 32]),
            terminal_boundary_fingerprints: vec!["terminal".into()],
            process_local_state_proof_sha256: Some(Digest([5; 32])),
        };
        let mut other_process = evidence.clone();
        other_process.process_local_state_proof_sha256 = Some(Digest([6; 32]));
        assert!(native_evidence_matches(&evidence, &other_process));

        other_process.terminal_boundary_fingerprints = vec!["changed".into()];
        assert!(!native_evidence_matches(&evidence, &other_process));
    }

    #[test]
    fn cross_process_evidence_rejects_named_planner_and_controller_drift() {
        let evidence = NativeSubsystemEvidenceProjection {
            source_boundary_fingerprint: "source".into(),
            simulated_ticks: 1,
            native_state_trajectory_sha256: Digest([1; 32]),
            episode_payload_xxh3_128: vec!["episode".into()],
            applicable_action_surface_context: action_surface_context(),
            applicable_action_surface_sha256: Digest([2; 32]),
            applicable_action_surface_boundaries: 1,
            applicable_action_descriptors: 4,
            controller_output_sha256: Digest([3; 32]),
            first_hit_ticks: vec![None],
            terminal_evidence_sha256: Digest([4; 32]),
            terminal_boundary_fingerprints: vec!["terminal".into()],
            process_local_state_proof_sha256: Some(Digest([5; 32])),
        };
        let mut drifted = evidence.clone();
        drifted.applicable_action_surface_sha256 = Digest([9; 32]);
        assert!(!native_evidence_matches(&evidence, &drifted));

        drifted = evidence.clone();
        drifted.controller_output_sha256 = Digest([9; 32]);
        assert!(!native_evidence_matches(&evidence, &drifted));

        drifted = evidence.clone();
        drifted.first_hit_ticks = vec![Some(0)];
        assert!(!native_evidence_matches(&evidence, &drifted));

        drifted = evidence.clone();
        drifted.native_state_trajectory_sha256 = Digest([9; 32]);
        assert!(!native_evidence_matches(&evidence, &drifted));
    }

    #[test]
    fn output_root_creation_materializes_missing_parents() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temporary_root = std::env::temp_dir().join(format!(
            "dusklight-native-subsystem-parity-{}-{nonce}",
            std::process::id()
        ));
        let nested_output = temporary_root.join("missing").join("run");
        let resolved = create_output_root(&nested_output).unwrap();
        assert!(resolved.is_dir());
        assert_eq!(resolved, nested_output.canonicalize().unwrap());
        std::fs::remove_dir_all(&temporary_root).unwrap();
    }
}
