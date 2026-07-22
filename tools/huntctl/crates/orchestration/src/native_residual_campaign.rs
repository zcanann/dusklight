//! Sealed execution and evidence artifacts for persistent native residual campaigns.

use crate::optimization_request::OptimizationRequest;
use crate::residual_campaign::{ResidualCampaignCandidate, ResidualCampaignError};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::{
    InputFrame, InputTape, RawPadState, TapeBoot, WaitCondition,
};
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_routes::timeline::Timeline;
use dusklight_routes::timeline_materialization::materialize_segment_chain;
use dusklight_search::residual_retention::{ExactTerminalVerdict, ResidualEvaluationEvidence};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub const NATIVE_RESIDUAL_EXECUTION_SCHEMA_V1: &str = "dusklight-native-residual-execution/v1";
pub const NATIVE_RESIDUAL_EVALUATION_SCHEMA_V1: &str = "dusklight-native-residual-evaluation/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeResidualExecutionBinding {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub executable: ArtifactReference,
    pub game_data: ArtifactReference,
    pub process_boot_tape: ArtifactReference,
    pub milestone_program: ArtifactReference,
    pub world_context: ArtifactReference,
    pub checkpoint_validation_ticks: u64,
    pub verify_state_hashes: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeResidualExecutionValidationReport {
    pub schema: &'static str,
    pub execution_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub source_frame: u64,
    pub exploration_horizon_ticks: u64,
    pub process_boot_tape_frames: u64,
    pub materialized_route_frames: u64,
    pub checkpoint_validation_ticks: u64,
    pub workers: u16,
}

impl NativeResidualExecutionBinding {
    #[allow(clippy::too_many_arguments)]
    pub fn seal(
        repository_root: &Path,
        optimization: &OptimizationRequest,
        executable: &Path,
        game_data: &Path,
        process_boot_tape: &Path,
        milestone_program: &Path,
        world_context: &Path,
        checkpoint_validation_ticks: u64,
        verify_state_hashes: bool,
    ) -> Result<Self, NativeResidualCampaignError> {
        let root = repository_root.canonicalize().map_err(native_error)?;
        let mut binding = Self {
            schema: NATIVE_RESIDUAL_EXECUTION_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: optimization.content_sha256,
            executable: artifact_reference(&root, executable)?,
            game_data: artifact_reference(&root, game_data)?,
            process_boot_tape: artifact_reference(&root, process_boot_tape)?,
            milestone_program: artifact_reference(&root, milestone_program)?,
            world_context: artifact_reference(&root, world_context)?,
            checkpoint_validation_ticks,
            verify_state_hashes,
        };
        binding.content_sha256 = binding.identity()?;
        binding.validate_files(&root, optimization)?;
        Ok(binding)
    }

    pub fn validate_files(
        &self,
        repository_root: &Path,
        optimization: &OptimizationRequest,
    ) -> Result<NativeResidualExecutionValidationReport, NativeResidualCampaignError> {
        let root = repository_root.canonicalize().map_err(native_error)?;
        optimization.validate_files(&root).map_err(native_error)?;
        if self.schema != NATIVE_RESIDUAL_EXECUTION_SCHEMA_V1
            || self.optimization_request_sha256 != optimization.content_sha256
            || self.checkpoint_validation_ticks == 0
            || self.checkpoint_validation_ticks > 256
            || self.checkpoint_validation_ticks > optimization.budgets.exploration_horizon_ticks
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
        {
            return Err(native_message(
                "native residual execution binding is invalid or detached",
            ));
        }
        let _executable = validate_artifact(&root, "executable", &self.executable)?;
        let _game_data = validate_artifact(&root, "game data", &self.game_data)?;
        let tape_path = validate_artifact(&root, "process boot tape", &self.process_boot_tape)?;
        let program_path = validate_artifact(&root, "milestone program", &self.milestone_program)?;
        let world_context_path = validate_artifact(&root, "world context", &self.world_context)?;
        let world_context: serde_json::Value =
            serde_json::from_slice(&fs::read(world_context_path).map_err(native_error)?)
                .map_err(native_error)?;
        if world_context.get("schema").and_then(|value| value.as_str())
            != Some("dusklight-world-context/v1")
            || world_context
                .get("game_data_sha256")
                .and_then(|value| value.as_str())
                != Some(self.game_data.sha256.to_string().as_str())
        {
            return Err(native_message(
                "native residual world context is not bound to the authenticated game data",
            ));
        }

        let tape = InputTape::decode(&fs::read(tape_path).map_err(native_error)?)
            .map_err(native_error)?
            .tape;
        let materialized = materialized_route_authority(&root, optimization)?;
        let required_frames = optimization
            .route
            .source_boundary_index
            .checked_add(optimization.budgets.exploration_horizon_ticks)
            .ok_or_else(|| native_message("native residual tape horizon overflowed"))?;
        if tape.boot != TapeBoot::Process
            || tape.tick_rate_numerator != 30
            || tape.tick_rate_denominator != 1
            || u64::try_from(tape.frames.len()).map_err(native_error)? < required_frames
            || tape.frames.len() < materialized.tape.frames.len()
            || tape.frames[..materialized.tape.frames.len()] != materialized.tape.frames
        {
            return Err(native_message(
                "native residual execution requires the exact materialized 30 Hz process-boot route through the full source horizon",
            ));
        }
        let released = released_frame(
            materialized
                .tape
                .frames
                .last()
                .ok_or_else(|| native_message("materialized native route is empty"))?,
        );
        if tape.frames[materialized.tape.frames.len()..]
            .iter()
            .any(|frame| frame != &released)
        {
            return Err(native_message(
                "native residual process tape has a non-released tail after the materialized route",
            ));
        }

        let decoded = dusklight_objectives::milestone_dsl::decode(
            &fs::read(program_path).map_err(native_error)?,
        )
        .map_err(native_error)?;
        let definition = decoded
            .program
            .definitions
            .iter()
            .position(|definition| definition.name == optimization.terminal_predicate.goal)
            .ok_or_else(|| native_message("milestone program does not define the terminal goal"))?;
        if Digest(decoded.program_sha256) != optimization.terminal_predicate.program_sha256
            || Digest(decoded.definitions[definition].sha256)
                != optimization.terminal_predicate.definition_sha256
        {
            return Err(native_message(
                "native residual milestone program differs from the optimization terminal binding",
            ));
        }

        Ok(NativeResidualExecutionValidationReport {
            schema: NATIVE_RESIDUAL_EXECUTION_SCHEMA_V1,
            execution_sha256: self.content_sha256,
            optimization_request_sha256: optimization.content_sha256,
            source_frame: optimization.route.source_boundary_index,
            exploration_horizon_ticks: optimization.budgets.exploration_horizon_ticks,
            process_boot_tape_frames: tape.frames.len() as u64,
            materialized_route_frames: materialized.tape.frames.len() as u64,
            checkpoint_validation_ticks: self.checkpoint_validation_ticks,
            workers: optimization.execution.workers,
        })
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, NativeResidualCampaignError> {
        pretty_json(self)
    }

    fn identity(&self) -> Result<Digest, NativeResidualCampaignError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.native-residual-execution/v1\0", &canonical)
    }
}

/// Materializes the checked route and appends an authoritative released-PAD
/// tail through the optimization exploration horizon.
pub fn materialize_native_residual_process_tape(
    repository_root: &Path,
    optimization: &OptimizationRequest,
) -> Result<InputTape, NativeResidualCampaignError> {
    let root = repository_root.canonicalize().map_err(native_error)?;
    optimization.validate_files(&root).map_err(native_error)?;
    let mut materialized = materialized_route_authority(&root, optimization)?.tape;
    let required_frames = optimization
        .route
        .source_boundary_index
        .checked_add(optimization.budgets.exploration_horizon_ticks)
        .ok_or_else(|| native_message("native residual tape horizon overflowed"))?;
    let required_frames = usize::try_from(required_frames).map_err(native_error)?;
    if materialized.frames.len() < required_frames {
        let released = released_frame(
            materialized
                .frames
                .last()
                .ok_or_else(|| native_message("materialized native route is empty"))?,
        );
        materialized.frames.resize(required_frames, released);
    }
    Ok(materialized)
}

struct MaterializedRouteAuthority {
    tape: InputTape,
}

fn materialized_route_authority(
    root: &Path,
    optimization: &OptimizationRequest,
) -> Result<MaterializedRouteAuthority, NativeResidualCampaignError> {
    let timeline_path = root
        .join(&optimization.route.timeline.path)
        .canonicalize()
        .map_err(native_error)?;
    if !timeline_path.starts_with(root) || !timeline_path.is_file() {
        return Err(native_message(
            "optimization timeline is outside the repository",
        ));
    }
    let timeline = Timeline::parse(&fs::read_to_string(&timeline_path).map_err(native_error)?)
        .map_err(native_error)?;
    let segment = timeline
        .segments
        .get(&optimization.route.segment)
        .ok_or_else(|| native_message("optimization segment is absent from its timeline"))?;
    let parent_id = segment
        .parent
        .as_deref()
        .ok_or_else(|| native_message("native residual segment has no parent checkpoint"))?;
    let artifact_root = timeline_path
        .parent()
        .ok_or_else(|| native_message("optimization timeline has no artifact root"))?;
    let parent =
        materialize_segment_chain(&timeline, artifact_root, parent_id).map_err(native_error)?;
    let full = materialize_segment_chain(&timeline, artifact_root, &optimization.route.segment)
        .map_err(native_error)?;
    let source_frame = u64::try_from(parent.tape.frames.len()).map_err(native_error)?;
    let selected_start = full
        .steps
        .last()
        .filter(|step| step.segment == optimization.route.segment)
        .map(|step| step.chain_start_frame)
        .ok_or_else(|| native_message("materialized route lacks the selected segment"))?;
    if source_frame != selected_start
        || source_frame != optimization.route.source_boundary_index
        || full.tape.frames.get(..parent.tape.frames.len()) != Some(parent.tape.frames.as_slice())
    {
        return Err(native_message(
            "optimization source boundary is not the exact materialized parent checkpoint",
        ));
    }
    Ok(MaterializedRouteAuthority { tape: full.tape })
}

fn released_frame(source: &InputFrame) -> InputFrame {
    let mut released = source.clone();
    released.wait_condition = WaitCondition::None;
    released.wait_timeout_ticks = 0;
    for pad in &mut released.pads {
        let connected = pad.connected;
        let error = pad.error;
        *pad = RawPadState {
            connected,
            error,
            ..RawPadState::default()
        };
    }
    released
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeResidualAttempt {
    pub repetition: u16,
    pub worker_seed: u64,
    pub wire_candidate_id: String,
    pub batch_request: ArtifactReference,
    pub batch_result: ArtifactReference,
    pub episode_shard: ArtifactReference,
    pub restore_identity: String,
    pub checkpoint_bytes: u64,
    pub simulated_ticks: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_hit_tick: Option<u64>,
    pub behavior_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeResidualCampaignEvaluation {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub candidate_id: String,
    pub candidate_sha256: Digest,
    pub realized_tape_sha256: Digest,
    pub attempts: Vec<NativeResidualAttempt>,
    pub simulated_ticks: u64,
    pub evidence: ResidualEvaluationEvidence,
}

impl NativeResidualCampaignEvaluation {
    pub fn seal(
        optimization: &OptimizationRequest,
        execution: &NativeResidualExecutionBinding,
        candidate: &ResidualCampaignCandidate,
        attempts: Vec<NativeResidualAttempt>,
    ) -> Result<Self, NativeResidualCampaignError> {
        if attempts.is_empty() {
            return Err(native_message("native residual evaluation is empty"));
        }
        let first_hit_tick = attempts[0].first_hit_tick;
        if attempts
            .iter()
            .any(|attempt| attempt.first_hit_tick != first_hit_tick)
        {
            return Err(native_message(
                "native residual repetitions disagree on the exact terminal verdict",
            ));
        }
        let simulated_ticks = attempts.iter().try_fold(0_u64, |total, attempt| {
            total
                .checked_add(attempt.simulated_ticks)
                .ok_or_else(|| native_message("native residual simulated ticks overflowed"))
        })?;
        let evidence = ResidualEvaluationEvidence {
            candidate_sha256: candidate.candidate.content_sha256,
            realized_tape_sha256: candidate.compilation.realized_tape_sha256,
            terminal_program_sha256: optimization.terminal_predicate.program_sha256,
            terminal_definition_sha256: optimization.terminal_predicate.definition_sha256,
            evaluation_sha256: canonical_digest(
                b"dusklight.native-residual-attempts/v1\0",
                &attempts,
            )?,
            episode_sha256: canonical_digest(
                b"dusklight.native-residual-episodes/v1\0",
                &attempts
                    .iter()
                    .map(|attempt| attempt.episode_shard.sha256)
                    .collect::<Vec<_>>(),
            )?,
            behavior_sha256: canonical_digest(
                b"dusklight.native-residual-behavior/v1\0",
                &attempts
                    .iter()
                    .map(|attempt| attempt.behavior_sha256)
                    .collect::<Vec<_>>(),
            )?,
            verdict: first_hit_tick.map_or(ExactTerminalVerdict::Miss, |first_hit_tick| {
                ExactTerminalVerdict::Reached { first_hit_tick }
            }),
            shaped_progress_millionths: None,
            native_risk_events: None,
        };
        let mut value = Self {
            schema: NATIVE_RESIDUAL_EVALUATION_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: optimization.content_sha256,
            execution_binding_sha256: execution.content_sha256,
            candidate_id: candidate.id.clone(),
            candidate_sha256: candidate.candidate.content_sha256,
            realized_tape_sha256: candidate.compilation.realized_tape_sha256,
            attempts,
            simulated_ticks,
            evidence,
        };
        value.content_sha256 = value.identity()?;
        value.validate(optimization, execution, candidate)?;
        Ok(value)
    }

    pub fn validate(
        &self,
        optimization: &OptimizationRequest,
        execution: &NativeResidualExecutionBinding,
        candidate: &ResidualCampaignCandidate,
    ) -> Result<(), NativeResidualCampaignError> {
        let expected_attempts = usize::from(optimization.execution.repetitions);
        let verdict_tick = self
            .attempts
            .first()
            .and_then(|attempt| attempt.first_hit_tick);
        let attempts_valid = self.attempts.iter().enumerate().all(|(index, attempt)| {
            attempt.repetition as usize == index + 1
                && optimization
                    .execution
                    .deterministic_seeds
                    .contains(&attempt.worker_seed)
                && !attempt.wire_candidate_id.is_empty()
                && valid_reference(&attempt.batch_request)
                && valid_reference(&attempt.batch_result)
                && valid_reference(&attempt.episode_shard)
                && lower_hex(&attempt.restore_identity, 32)
                && attempt.checkpoint_bytes > 0
                && attempt.simulated_ticks > 0
                && attempt.simulated_ticks <= optimization.budgets.exploration_horizon_ticks
                && attempt
                    .first_hit_tick
                    .is_none_or(|tick| tick > 0 && tick == attempt.simulated_ticks)
                && attempt.first_hit_tick == verdict_tick
                && attempt.behavior_sha256 != Digest::ZERO
        });
        let charged = self.attempts.iter().try_fold(0_u64, |total, attempt| {
            total.checked_add(attempt.simulated_ticks)
        });
        let expected_verdict = verdict_tick.map_or(ExactTerminalVerdict::Miss, |first_hit_tick| {
            ExactTerminalVerdict::Reached { first_hit_tick }
        });
        if self.schema != NATIVE_RESIDUAL_EVALUATION_SCHEMA_V1
            || self.optimization_request_sha256 != optimization.content_sha256
            || self.execution_binding_sha256 != execution.content_sha256
            || self.candidate_id != candidate.id
            || self.candidate_sha256 != candidate.candidate.content_sha256
            || self.realized_tape_sha256 != candidate.compilation.realized_tape_sha256
            || self.attempts.len() != expected_attempts
            || !attempts_valid
            || charged != Some(self.simulated_ticks)
            || self.evidence.candidate_sha256 != self.candidate_sha256
            || self.evidence.realized_tape_sha256 != self.realized_tape_sha256
            || self.evidence.terminal_program_sha256
                != optimization.terminal_predicate.program_sha256
            || self.evidence.terminal_definition_sha256
                != optimization.terminal_predicate.definition_sha256
            || self.evidence.verdict != expected_verdict
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
        {
            return Err(native_message(
                "native residual evaluation is invalid or detached",
            ));
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, NativeResidualCampaignError> {
        pretty_json(self)
    }

    fn identity(&self) -> Result<Digest, NativeResidualCampaignError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.native-residual-evaluation/v1\0", &canonical)
    }
}

fn artifact_reference(
    root: &Path,
    path: &Path,
) -> Result<ArtifactReference, NativeResidualCampaignError> {
    let path = path.canonicalize().map_err(native_error)?;
    if !path.is_file() {
        return Err(native_message("native residual artifact is not a file"));
    }
    let relative = repository_relative(root, &path)?;
    Ok(ArtifactReference {
        path: relative,
        sha256: sha256_file(&path)?,
    })
}

fn validate_artifact(
    root: &Path,
    label: &str,
    reference: &ArtifactReference,
) -> Result<PathBuf, NativeResidualCampaignError> {
    if !valid_reference(reference) {
        return Err(native_message(format!(
            "invalid {label} artifact reference"
        )));
    }
    let path = root
        .join(&reference.path)
        .canonicalize()
        .map_err(native_error)?;
    if !path.starts_with(root) || !path.is_file() || sha256_file(&path)? != reference.sha256 {
        return Err(native_message(format!(
            "{label} artifact is missing, outside the repository, or digest-mismatched"
        )));
    }
    Ok(path)
}

fn valid_reference(reference: &ArtifactReference) -> bool {
    reference.sha256 != Digest::ZERO
        && !reference.path.is_empty()
        && !Path::new(&reference.path).is_absolute()
        && Path::new(&reference.path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn repository_relative(root: &Path, path: &Path) -> Result<String, NativeResidualCampaignError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| native_message("native residual artifact is outside the repository"))?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(native_message(
            "native residual artifact path is not canonical",
        ));
    }
    relative
        .to_str()
        .map(|value| value.replace(std::path::MAIN_SEPARATOR, "/"))
        .ok_or_else(|| native_message("native residual artifact path is not UTF-8"))
}

fn sha256_file(path: &Path) -> Result<Digest, NativeResidualCampaignError> {
    let mut file = File::open(path).map_err(native_error)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(native_error)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(Digest(hasher.finalize().into()))
}

fn pretty_json(value: &impl Serialize) -> Result<Vec<u8>, NativeResidualCampaignError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(native_error)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, NativeResidualCampaignError> {
    let bytes = serde_json::to_vec(value).map_err(native_error)?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeResidualCampaignError(String);

fn native_message(message: impl Into<String>) -> NativeResidualCampaignError {
    NativeResidualCampaignError(message.into())
}

fn native_error(error: impl fmt::Display) -> NativeResidualCampaignError {
    native_message(error.to_string())
}

impl fmt::Display for NativeResidualCampaignError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeResidualCampaignError {}

impl From<ResidualCampaignError> for NativeResidualCampaignError {
    fn from(error: ResidualCampaignError) -> Self {
        native_error(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    struct TestArtifacts(PathBuf);

    impl TestArtifacts {
        fn new(repository: &Path) -> Self {
            let path = repository.join("build").join(format!(
                "native-residual-binding-test-{}-{}",
                std::process::id(),
                NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
            ));
            if path.exists() {
                fs::remove_dir_all(&path).unwrap();
            }
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestArtifacts {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn repository() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap()
    }

    fn fixture() -> (
        PathBuf,
        TestArtifacts,
        OptimizationRequest,
        PathBuf,
        PathBuf,
        PathBuf,
        PathBuf,
        PathBuf,
    ) {
        let repository = repository();
        let artifacts = TestArtifacts::new(&repository);
        let request_path = repository.join(
            "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
        );
        let optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(request_path).unwrap()).unwrap();
        let executable = artifacts.0.join("Dusklight");
        let game_data = artifacts.0.join("game.iso");
        let tape_path = artifacts.0.join("full.tape");
        let program_path = artifacts.0.join("goal.dmsp");
        let world_context_path = artifacts.0.join("world.context.json");
        fs::write(&executable, b"executable").unwrap();
        fs::write(&game_data, b"game-data").unwrap();
        fs::write(
            &world_context_path,
            serde_json::to_vec(&serde_json::json!({
                "schema": "dusklight-world-context/v1",
                "game_data_sha256": sha256_file(&game_data).unwrap(),
                "stages": []
            }))
            .unwrap(),
        )
        .unwrap();
        let tape = materialize_native_residual_process_tape(&repository, &optimization).unwrap();
        fs::write(&tape_path, tape.encode().unwrap()).unwrap();
        let source =
            fs::read_to_string(repository.join(&optimization.terminal_predicate.source.path))
                .unwrap();
        let program = dusklight_objectives::milestone_dsl::parse(&source).unwrap();
        let compiled = dusklight_objectives::milestone_dsl::compile(&program).unwrap();
        fs::write(&program_path, compiled.bytes).unwrap();
        (
            repository,
            artifacts,
            optimization,
            executable,
            game_data,
            tape_path,
            program_path,
            world_context_path,
        )
    }

    #[test]
    fn execution_binding_seals_the_exact_native_checkpoint_authority() {
        let (root, _artifacts, optimization, executable, game_data, tape, program, world_context) =
            fixture();
        let binding = NativeResidualExecutionBinding::seal(
            &root,
            &optimization,
            &executable,
            &game_data,
            &tape,
            &program,
            &world_context,
            8,
            false,
        )
        .unwrap();
        let report = binding.validate_files(&root, &optimization).unwrap();
        assert_eq!(report.source_frame, 440);
        assert_eq!(report.exploration_horizon_ticks, 160);
        assert_eq!(report.process_boot_tape_frames, 600);
        assert_eq!(report.materialized_route_frames, 566);
        assert_eq!(report.workers, 4);

        fs::write(&program, b"tampered").unwrap();
        assert!(binding.validate_files(&root, &optimization).is_err());
    }

    #[test]
    fn checked_ordon_boundary_is_the_materialized_parent_checkpoint() {
        let (root, _artifacts, mut optimization, ..) = fixture();
        optimization.route.source_boundary_index = 500;
        optimization.refresh_content_sha256().unwrap();
        assert!(optimization.validate_files(&root).is_err());
        assert!(materialize_native_residual_process_tape(&root, &optimization).is_err());
    }
}
