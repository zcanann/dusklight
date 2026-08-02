//! Complete process-boot replay proof for a minimized residual winner.

use crate::native_residual_campaign::{
    NativeResidualExecutionBinding, ValidatedNativeResidualExecution,
    materialize_native_residual_process_tape,
};
use crate::native_tactic_route_runner::{
    NativeTacticColdReplayArtifact, NativeTacticColdReplayAttempt, NativeTacticColdReplayFidelity,
    NativeTapeColdReplayConfig, exact_cold_replay_attempts,
    run_native_tape_cold_replay_after_execution_validation,
    validate_native_tape_cold_replay_artifacts,
};
use crate::optimization_request::OptimizationRequest;
use crate::residual_campaign::ResidualCampaignCandidate;
use crate::residual_winner_minimization::ResidualWinnerMinimizationSummary;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_search::residual_action::compile_residual_candidate_to_horizon;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path};
use std::time::Duration;

pub const RESIDUAL_WINNER_COLD_REPLAY_SCHEMA_V1: &str = "dusklight-residual-winner-cold-replay/v1";
pub const RESIDUAL_WINNER_COLD_REPLAY_PROOF_FILE: &str = "proof.json";
const REQUIRED_REPETITIONS: u32 = 2;
const MAXIMUM_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

pub struct ResidualWinnerColdReplayConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub minimization_summary: &'a ResidualWinnerMinimizationSummary,
    pub minimization_summary_artifact: ArtifactReference,
    pub timeout: Duration,
    pub output_root: &'a Path,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualWinnerColdReplayProof {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub minimization_summary: ArtifactReference,
    pub minimized_suffix_tape_sha256: Digest,
    pub source_boundary_index: u64,
    pub source_boundary_fingerprint: String,
    pub native_source_boundary_fingerprint: String,
    pub goal: String,
    pub terminal_program_sha256: Digest,
    pub terminal_definition_sha256: Digest,
    pub first_hit_tick: u64,
    pub controller_tape: NativeTacticColdReplayArtifact,
    pub controller_tape_frames: u64,
    pub fidelity: NativeTacticColdReplayFidelity,
    pub controller_in_loop: bool,
    pub learner_in_loop: bool,
    pub attempts: Vec<NativeTacticColdReplayAttempt>,
}

pub fn run_residual_winner_cold_replay(
    config: &ResidualWinnerColdReplayConfig<'_>,
) -> Result<ResidualWinnerColdReplayProof, ResidualWinnerColdReplayError> {
    let authority = ValidatedNativeResidualExecution::authenticate(
        config.repository_root,
        config.optimization,
        config.execution,
    )
    .map_err(cold_error)?;
    run_residual_winner_cold_replay_after_execution_validation(config, &authority)
}

/// Runs the cold proof after the caller authenticated the immutable execution
/// binding in this process. The minimized route lineage and every emitted
/// replay artifact remain fail-closed.
pub(crate) fn run_residual_winner_cold_replay_after_execution_validation(
    config: &ResidualWinnerColdReplayConfig<'_>,
    authority: &ValidatedNativeResidualExecution,
) -> Result<ResidualWinnerColdReplayProof, ResidualWinnerColdReplayError> {
    let (root, tape, tape_bytes) = validate_authority(
        config.repository_root,
        config.optimization,
        config.execution,
        config.minimization_summary,
        &config.minimization_summary_artifact,
        authority,
    )?;
    validate_new_build_output(&root, config.output_root)?;
    let replay_config = NativeTapeColdReplayConfig {
        repository_root: &root,
        optimization: config.optimization,
        execution: config.execution,
        tape: &tape,
        tape_bytes: &tape_bytes,
        first_hit_tick: config.minimization_summary.minimized_first_hit_tick,
        repetitions: REQUIRED_REPETITIONS,
        timeout: config.timeout,
        output_root: config.output_root,
    };
    let (controller_tape, attempts) =
        run_native_tape_cold_replay_after_execution_validation(&replay_config, authority)
            .map_err(cold_error)?;
    let mut proof = ResidualWinnerColdReplayProof {
        schema: RESIDUAL_WINNER_COLD_REPLAY_SCHEMA_V1.into(),
        content_sha256: Digest::ZERO,
        optimization_request_sha256: config.optimization.content_sha256,
        execution_binding_sha256: config.execution.content_sha256,
        minimization_summary: config.minimization_summary_artifact.clone(),
        minimized_suffix_tape_sha256: config.minimization_summary.minimized_tape_sha256,
        source_boundary_index: config.optimization.route.source_boundary_index,
        source_boundary_fingerprint: config
            .optimization
            .route
            .source_boundary_fingerprint
            .clone(),
        native_source_boundary_fingerprint: config
            .optimization
            .route
            .native_source_boundary_fingerprint
            .clone(),
        goal: config.optimization.terminal_predicate.goal.clone(),
        terminal_program_sha256: config.optimization.terminal_predicate.program_sha256,
        terminal_definition_sha256: config.optimization.terminal_predicate.definition_sha256,
        first_hit_tick: config.minimization_summary.minimized_first_hit_tick,
        controller_tape,
        controller_tape_frames: u64::try_from(tape.frames.len()).map_err(cold_error)?,
        fidelity: NativeTacticColdReplayFidelity::exact_headless(),
        controller_in_loop: false,
        learner_in_loop: false,
        attempts,
    };
    proof.content_sha256 = proof.identity()?;
    proof.validate_shape()?;
    write_new(
        &config
            .output_root
            .join(RESIDUAL_WINNER_COLD_REPLAY_PROOF_FILE),
        &proof.to_pretty_json()?,
    )?;
    validate_native_tape_cold_replay_artifacts(
        config.output_root,
        config.optimization,
        &tape,
        &tape_bytes,
        proof.first_hit_tick,
        &proof.controller_tape,
        &proof.attempts,
    )
    .map_err(cold_error)?;
    Ok(proof)
}

pub fn read_and_validate_residual_winner_cold_replay(
    repository_root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    proof_root: &Path,
) -> Result<ResidualWinnerColdReplayProof, ResidualWinnerColdReplayError> {
    let proof: ResidualWinnerColdReplayProof = serde_json::from_slice(&read_bounded(
        &proof_root.join(RESIDUAL_WINNER_COLD_REPLAY_PROOF_FILE),
    )?)
    .map_err(cold_error)?;
    proof.validate_files(repository_root, optimization, execution, proof_root)?;
    Ok(proof)
}

impl ResidualWinnerColdReplayProof {
    pub fn validate_files(
        &self,
        repository_root: &Path,
        optimization: &OptimizationRequest,
        execution: &NativeResidualExecutionBinding,
        proof_root: &Path,
    ) -> Result<(), ResidualWinnerColdReplayError> {
        self.validate_shape()?;
        let (root, tape, tape_bytes) = validate_authority_from_reference(
            repository_root,
            optimization,
            execution,
            &self.minimization_summary,
        )?;
        let summary = read_summary(&root, &self.minimization_summary)?;
        if self.optimization_request_sha256 != optimization.content_sha256
            || self.execution_binding_sha256 != execution.content_sha256
            || self.minimized_suffix_tape_sha256 != summary.minimized_tape_sha256
            || self.first_hit_tick != summary.minimized_first_hit_tick
            || self.source_boundary_index != optimization.route.source_boundary_index
            || self.source_boundary_fingerprint != optimization.route.source_boundary_fingerprint
            || self.native_source_boundary_fingerprint
                != optimization.route.native_source_boundary_fingerprint
            || self.goal != optimization.terminal_predicate.goal
            || self.terminal_program_sha256 != optimization.terminal_predicate.program_sha256
            || self.terminal_definition_sha256 != optimization.terminal_predicate.definition_sha256
            || self.controller_tape_frames
                != u64::try_from(tape.frames.len()).map_err(cold_error)?
        {
            return Err(cold_message(
                "residual winner cold replay belongs to another route authority",
            ));
        }
        validate_native_tape_cold_replay_artifacts(
            proof_root,
            optimization,
            &tape,
            &tape_bytes,
            self.first_hit_tick,
            &self.controller_tape,
            &self.attempts,
        )
        .map_err(cold_error)
    }

    pub(crate) fn validate_shape(&self) -> Result<(), ResidualWinnerColdReplayError> {
        let expected_frames = self
            .source_boundary_index
            .checked_add(self.first_hit_tick)
            .and_then(|value| value.checked_add(1));
        if self.schema != RESIDUAL_WINNER_COLD_REPLAY_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
            || self.optimization_request_sha256 == Digest::ZERO
            || self.execution_binding_sha256 == Digest::ZERO
            || !valid_reference(&self.minimization_summary)
            || self.minimized_suffix_tape_sha256 == Digest::ZERO
            || !native_fingerprint(&self.source_boundary_fingerprint)
            || !native_fingerprint(&self.native_source_boundary_fingerprint)
            || self.goal.is_empty()
            || self.terminal_program_sha256 == Digest::ZERO
            || self.terminal_definition_sha256 == Digest::ZERO
            || self.first_hit_tick == 0
            || expected_frames != Some(self.controller_tape_frames)
            || self.controller_tape.sha256 == Digest::ZERO
            || !confined_relative_path(&self.controller_tape.path)
            || !self.fidelity.is_exact_headless()
            || self.controller_in_loop
            || self.learner_in_loop
            || self.attempts.len() != REQUIRED_REPETITIONS as usize
            || !exact_cold_replay_attempts(
                &self.attempts,
                &self.controller_tape,
                self.source_boundary_index,
                self.first_hit_tick,
                self.controller_tape_frames,
            )
        {
            return Err(cold_message(
                "residual winner cold replay proof is invalid or detached",
            ));
        }
        Ok(())
    }

    fn identity(&self) -> Result<Digest, ResidualWinnerColdReplayError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.residual-winner-cold-replay/v1\0", &canonical)
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, ResidualWinnerColdReplayError> {
        let mut bytes = serde_json::to_vec_pretty(self).map_err(cold_error)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

fn validate_authority_from_reference(
    repository_root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    summary_reference: &ArtifactReference,
) -> Result<(std::path::PathBuf, InputTape, Vec<u8>), ResidualWinnerColdReplayError> {
    let root = repository_root.canonicalize().map_err(cold_error)?;
    let summary = read_summary(&root, summary_reference)?;
    let authority = ValidatedNativeResidualExecution::authenticate(&root, optimization, execution)
        .map_err(cold_error)?;
    summary.validate_files(&root).map_err(cold_error)?;
    validate_authority(
        &root,
        optimization,
        execution,
        &summary,
        summary_reference,
        &authority,
    )
}

fn validate_authority(
    repository_root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    summary: &ResidualWinnerMinimizationSummary,
    summary_reference: &ArtifactReference,
    authority: &ValidatedNativeResidualExecution,
) -> Result<(std::path::PathBuf, InputTape, Vec<u8>), ResidualWinnerColdReplayError> {
    let root = repository_root.canonicalize().map_err(cold_error)?;
    authority
        .validate_scope(&root, optimization, execution)
        .map_err(cold_error)?;
    optimization.validate().map_err(cold_error)?;
    execution.validate_seal(optimization).map_err(cold_error)?;
    summary.validate().map_err(cold_error)?;
    if &read_summary(&root, summary_reference)? != summary
        || summary.optimization_request_sha256 != optimization.content_sha256
        || summary.execution_binding_sha256 != execution.content_sha256
        || optimization
            .incumbent
            .as_ref()
            .is_none_or(|incumbent| summary.minimized_first_hit_tick >= incumbent.first_hit_tick)
    {
        return Err(cold_message(
            "residual winner selection requires a strict authenticated incumbent improvement",
        ));
    }
    let suffix_bytes = final_suffix_bytes(&root, optimization, summary)?;
    if Digest(Sha256::digest(&suffix_bytes).into()) != summary.minimized_tape_sha256 {
        return Err(cold_message(
            "residual winner suffix differs from the minimization summary",
        ));
    }
    let suffix = InputTape::decode(&suffix_bytes).map_err(cold_error)?.tape;
    let process =
        materialize_native_residual_process_tape(&root, optimization).map_err(cold_error)?;
    let tape = splice_complete_route(
        process,
        &suffix,
        optimization.route.source_boundary_index,
        summary.minimized_first_hit_tick,
    )?;
    let tape_bytes = tape.encode().map_err(cold_error)?;
    Ok((root, tape, tape_bytes))
}

fn final_suffix_bytes(
    root: &Path,
    optimization: &OptimizationRequest,
    summary: &ResidualWinnerMinimizationSummary,
) -> Result<Vec<u8>, ResidualWinnerColdReplayError> {
    if let Some(reference) = &summary.minimized_tape {
        return read_reference(root, reference);
    }
    let source: ResidualCampaignCandidate =
        serde_json::from_slice(&read_reference(root, &summary.source_candidate)?)
            .map_err(cold_error)?;
    let incumbent = optimization
        .incumbent
        .as_ref()
        .ok_or_else(|| cold_message("residual winner source has no incumbent"))?;
    let parent_bytes = read_reference(root, &incumbent.tape)?;
    let parent = InputTape::decode(&parent_bytes).map_err(cold_error)?.tape;
    let compiled = compile_residual_candidate_to_horizon(
        &parent,
        &parent_bytes,
        &source.candidate,
        optimization.budgets.exploration_horizon_ticks,
    )
    .map_err(cold_error)?;
    Ok(compiled.bytes)
}

fn splice_complete_route(
    mut process: InputTape,
    suffix: &InputTape,
    source_boundary_index: u64,
    first_hit_tick: u64,
) -> Result<InputTape, ResidualWinnerColdReplayError> {
    let source = usize::try_from(source_boundary_index).map_err(cold_error)?;
    let suffix_frames = usize::try_from(first_hit_tick)
        .map_err(cold_error)?
        .checked_add(1)
        .ok_or_else(|| cold_message("residual winner suffix length overflowed"))?;
    if process.frames.len() < source
        || suffix.frames.len() < suffix_frames
        || process.boot != suffix.boot
        || process.tick_rate_numerator != suffix.tick_rate_numerator
        || process.tick_rate_denominator != suffix.tick_rate_denominator
    {
        return Err(cold_message(
            "residual winner suffix cannot be spliced onto its process route",
        ));
    }
    process.frames.truncate(source);
    process
        .frames
        .extend_from_slice(&suffix.frames[..suffix_frames]);
    Ok(process)
}

fn read_summary(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<ResidualWinnerMinimizationSummary, ResidualWinnerColdReplayError> {
    serde_json::from_slice(&read_reference(root, reference)?).map_err(cold_error)
}

fn read_reference(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<Vec<u8>, ResidualWinnerColdReplayError> {
    if !valid_reference(reference) {
        return Err(cold_message(
            "residual winner artifact reference is invalid",
        ));
    }
    let path = root.join(&reference.path);
    let canonical = path.canonicalize().map_err(cold_error)?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(cold_message(
            "residual winner artifact escapes the repository",
        ));
    }
    let bytes = read_bounded(&canonical)?;
    if Digest(Sha256::digest(&bytes).into()) != reference.sha256 {
        return Err(cold_message("residual winner artifact digest differs"));
    }
    Ok(bytes)
}

fn valid_reference(reference: &ArtifactReference) -> bool {
    let path = Path::new(&reference.path);
    reference.sha256 != Digest::ZERO
        && !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn confined_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn native_fingerprint(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn validate_new_build_output(
    root: &Path,
    output: &Path,
) -> Result<(), ResidualWinnerColdReplayError> {
    if output.exists() {
        return Err(cold_message("residual winner cold replay output exists"));
    }
    let parent = output
        .parent()
        .ok_or_else(|| cold_message("residual winner cold replay output has no parent"))?
        .canonicalize()
        .map_err(cold_error)?;
    let build = root.join("build").canonicalize().map_err(cold_error)?;
    if !parent.starts_with(&build)
        || output.file_name().is_none()
        || output
            .file_name()
            .is_some_and(|name| Path::new(name).components().count() != 1)
    {
        return Err(cold_message(
            "residual winner cold replay output must be a new build directory",
        ));
    }
    Ok(())
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, ResidualWinnerColdReplayError> {
    let metadata = fs::symlink_metadata(path).map_err(cold_error)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAXIMUM_ARTIFACT_BYTES
    {
        return Err(cold_message(
            "residual winner artifact is invalid or oversized",
        ));
    }
    fs::read(path).map_err(cold_error)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ResidualWinnerColdReplayError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(cold_error)?;
    file.write_all(bytes).map_err(cold_error)?;
    file.sync_all().map_err(cold_error)
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, ResidualWinnerColdReplayError> {
    let bytes = serde_json::to_vec(value).map_err(cold_error)?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Debug)]
pub struct ResidualWinnerColdReplayError(String);

fn cold_message(message: impl Into<String>) -> ResidualWinnerColdReplayError {
    ResidualWinnerColdReplayError(message.into())
}

fn cold_error(error: impl fmt::Display) -> ResidualWinnerColdReplayError {
    cold_message(error.to_string())
}

impl fmt::Display for ResidualWinnerColdReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ResidualWinnerColdReplayError {}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_automation_contracts::tape::{InputFrame, RawPadState, TapeBoot};
    use dusklight_harness_contracts::evaluation::BoundaryFingerprint;

    fn tape(frames: usize) -> InputTape {
        InputTape {
            boot: TapeBoot::Process,
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            frames: (0..frames)
                .map(|index| InputFrame {
                    owned_ports: 1,
                    pads: [RawPadState {
                        stick_x: i8::try_from(index).unwrap_or(i8::MAX),
                        ..RawPadState::default()
                    }; 4],
                    ..InputFrame::default()
                })
                .collect(),
        }
    }

    fn digest(value: u8) -> Digest {
        Digest([value; 32])
    }

    fn artifact(path: &str, value: u8) -> NativeTacticColdReplayArtifact {
        NativeTacticColdReplayArtifact {
            path: path.into(),
            sha256: digest(value),
        }
    }

    fn attempt(repetition: u32) -> NativeTacticColdReplayAttempt {
        NativeTacticColdReplayAttempt {
            repetition,
            controller_tape: artifact(&format!("repeat-{repetition:03}/controller.tape"), 10),
            milestone_result: artifact(
                &format!("repeat-{repetition:03}/milestones.json"),
                20 + repetition as u8,
            ),
            stdout: artifact(
                &format!("repeat-{repetition:03}/stdout.txt"),
                30 + repetition as u8,
            ),
            stderr: artifact(
                &format!("repeat-{repetition:03}/stderr.txt"),
                40 + repetition as u8,
            ),
            sim_tick: 512,
            tape_frame: 12,
            boundary_index: 13,
            first_hit_tick: 2,
            boundary_fingerprint: BoundaryFingerprint {
                schema: "dusklight.milestone-boundary/v6".into(),
                algorithm: "xxh3-128".into(),
                canonical_encoding: "little-endian-fixed-v6".into(),
                digest: "a".repeat(32),
            },
        }
    }

    fn proof() -> ResidualWinnerColdReplayProof {
        let mut proof = ResidualWinnerColdReplayProof {
            schema: RESIDUAL_WINNER_COLD_REPLAY_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: digest(1),
            execution_binding_sha256: digest(2),
            minimization_summary: ArtifactReference {
                path: "build/minimization/summary.json".into(),
                sha256: digest(3),
            },
            minimized_suffix_tape_sha256: digest(4),
            source_boundary_index: 10,
            source_boundary_fingerprint: "1".repeat(32),
            native_source_boundary_fingerprint: "2".repeat(32),
            goal: "ordon-load-zone".into(),
            terminal_program_sha256: digest(5),
            terminal_definition_sha256: digest(6),
            first_hit_tick: 2,
            controller_tape: artifact("route.tape", 10),
            controller_tape_frames: 13,
            fidelity: NativeTacticColdReplayFidelity::exact_headless(),
            controller_in_loop: false,
            learner_in_loop: false,
            attempts: vec![attempt(1), attempt(2)],
        };
        proof.content_sha256 = proof.identity().unwrap();
        proof
    }

    #[test]
    fn complete_route_splice_preserves_prefix_and_uses_only_terminal_suffix() {
        let process = tape(20);
        let suffix = tape(10);
        let spliced = splice_complete_route(process.clone(), &suffix, 7, 3).unwrap();
        assert_eq!(spliced.frames.len(), 11);
        assert_eq!(&spliced.frames[..7], &process.frames[..7]);
        assert_eq!(&spliced.frames[7..], &suffix.frames[..4]);
    }

    #[test]
    fn complete_route_splice_rejects_detached_timebase_and_short_suffix() {
        let process = tape(20);
        let mut suffix = tape(3);
        assert!(splice_complete_route(process.clone(), &suffix, 7, 3).is_err());
        suffix = tape(10);
        suffix.tick_rate_numerator = 60;
        assert!(splice_complete_route(process, &suffix, 7, 3).is_err());
    }

    #[test]
    fn proof_requires_two_identical_full_root_terminal_attempts() {
        proof().validate_shape().unwrap();

        let mut one = proof();
        one.attempts.pop();
        one.content_sha256 = one.identity().unwrap();
        assert!(one.validate_shape().is_err());

        let mut drift = proof();
        drift.attempts[1].boundary_fingerprint.digest = "b".repeat(32);
        drift.content_sha256 = drift.identity().unwrap();
        assert!(drift.validate_shape().is_err());
    }
}
