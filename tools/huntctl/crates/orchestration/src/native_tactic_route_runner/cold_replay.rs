//! Exact, learner-free cold replay proof for a graph-selected tactic route.

use super::scratch_discovery::route_report_sha256;
use super::*;
use dusklight_automation_contracts::native_fidelity::FIXED_AUTOMATION_CVARS;
use dusklight_harness_contracts::evaluation::BoundaryFingerprint;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_harness_contracts::run_contract::HarnessFidelityMode;
use sha2::Sha256;
use std::path::Component;
use std::process::{Command, Stdio};
use std::thread;

pub const NATIVE_TACTIC_COLD_REPLAY_PROOF_SCHEMA_V1: &str =
    "dusklight-native-tactic-cold-replay-proof/v1";
pub const NATIVE_TACTIC_COLD_REPLAY_PROOF_FILE: &str = "proof.json";
const NATIVE_TACTIC_COLD_REPLAY_TAPE_FILE: &str = "route.tape";
const NATIVE_TACTIC_COLD_REPLAY_FIDELITY_PROFILE_V1: &str = "headless-fixed-step-unpaced-30hz/v1";
const MINIMUM_COLD_REPLAY_REPETITIONS: u32 = 2;
const MAXIMUM_COLD_REPLAY_REPETITIONS: u32 = 16;
const MAXIMUM_COLD_REPLAY_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct NativeTacticColdReplayConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub execution_plan: &'a NativeTacticExecutionPlan,
    pub route_report: &'a NativeTacticRouteReport,
    pub seed: u64,
    pub maximum_first_hit_tick: u64,
    pub repetitions: u32,
    pub timeout: Duration,
    pub output_root: &'a Path,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticColdReplayArtifact {
    pub path: String,
    pub sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticColdReplayFidelity {
    pub request_fidelity: HarnessFidelityMode,
    pub profile: String,
    pub headless: bool,
    pub fixed_step: bool,
    pub unpaced: bool,
    pub exit_after_tape: bool,
    pub input_tape_end: String,
    pub fixed_automation_cvars: Vec<String>,
}

impl NativeTacticColdReplayFidelity {
    fn exact_headless() -> Self {
        Self {
            request_fidelity: HarnessFidelityMode::Headless,
            profile: NATIVE_TACTIC_COLD_REPLAY_FIDELITY_PROFILE_V1.into(),
            headless: true,
            fixed_step: true,
            unpaced: true,
            exit_after_tape: true,
            input_tape_end: "hold".into(),
            fixed_automation_cvars: FIXED_AUTOMATION_CVARS
                .iter()
                .map(|value| (*value).into())
                .collect(),
        }
    }

    fn is_exact_headless(&self) -> bool {
        self == &Self::exact_headless()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticColdReplayAttempt {
    pub repetition: u32,
    pub controller_tape: NativeTacticColdReplayArtifact,
    pub milestone_result: NativeTacticColdReplayArtifact,
    pub stdout: NativeTacticColdReplayArtifact,
    pub stderr: NativeTacticColdReplayArtifact,
    pub sim_tick: u64,
    pub tape_frame: u64,
    pub boundary_index: u64,
    pub first_hit_tick: u64,
    pub boundary_fingerprint: BoundaryFingerprint,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticColdReplayProof {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub execution_plan_sha256: Digest,
    pub route_report_sha256: Digest,
    pub seed: u64,
    pub state_graph_sha256: Digest,
    pub terminal_result_sha256: Digest,
    pub terminal_state_sha256: Digest,
    pub objective_sha256: Digest,
    pub source_boundary_index: u64,
    pub source_boundary_fingerprint: String,
    pub native_source_boundary_fingerprint: String,
    pub goal: String,
    pub terminal_program_sha256: Digest,
    pub terminal_definition_sha256: Digest,
    pub first_hit_tick: u64,
    pub maximum_first_hit_tick: u64,
    pub controller_tape: NativeTacticColdReplayArtifact,
    pub controller_tape_frames: u64,
    pub executable: ArtifactReference,
    pub runtime_dependencies: Vec<ArtifactReference>,
    pub game_data: ArtifactReference,
    pub milestone_program: ArtifactReference,
    pub world_context: ArtifactReference,
    pub card_fixture_manifest: ArtifactReference,
    pub fidelity: NativeTacticColdReplayFidelity,
    pub controller_in_loop: bool,
    pub learner_in_loop: bool,
    pub attempts: Vec<NativeTacticColdReplayAttempt>,
}

struct ValidatedRouteAuthority {
    repository_root: PathBuf,
    route_report_sha256: Digest,
    execution_plan_sha256: Digest,
    seed_result: NativeTacticSeedResult,
    final_result: TacticQFinalResult,
    tape: InputTape,
    tape_bytes: Vec<u8>,
    first_hit_tick: u64,
}

impl NativeTacticColdReplayProof {
    fn seal(
        config: &NativeTacticColdReplayConfig<'_>,
        authority: &ValidatedRouteAuthority,
        controller_tape: NativeTacticColdReplayArtifact,
        attempts: Vec<NativeTacticColdReplayAttempt>,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let mut proof = Self {
            schema: NATIVE_TACTIC_COLD_REPLAY_PROOF_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: config.optimization.content_sha256,
            execution_binding_sha256: config.execution.content_sha256,
            execution_plan_sha256: authority.execution_plan_sha256,
            route_report_sha256: authority.route_report_sha256,
            seed: config.seed,
            state_graph_sha256: authority.seed_result.state_graph_sha256,
            terminal_result_sha256: authority.final_result.content_sha256,
            terminal_state_sha256: authority.final_result.terminal_state_sha256,
            objective_sha256: authority.final_result.objective_sha256,
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
            first_hit_tick: authority.first_hit_tick,
            maximum_first_hit_tick: config.maximum_first_hit_tick,
            controller_tape,
            controller_tape_frames: u64::try_from(authority.tape.frames.len())
                .map_err(route_error)?,
            executable: config.execution.executable.clone(),
            runtime_dependencies: config.execution.runtime_dependencies.clone(),
            game_data: config.execution.game_data.clone(),
            milestone_program: config.execution.milestone_program.clone(),
            world_context: config.execution.world_context.clone(),
            card_fixture_manifest: config.execution.card_fixture_manifest.clone(),
            fidelity: NativeTacticColdReplayFidelity::exact_headless(),
            controller_in_loop: false,
            learner_in_loop: false,
            attempts,
        };
        proof.content_sha256 = proof.identity()?;
        proof.validate_shape()?;
        Ok(proof)
    }

    pub(super) fn validate_shape(&self) -> Result<(), NativeTacticRouteRunError> {
        let first = self.attempts.first();
        let expected_tape_frame = self
            .source_boundary_index
            .checked_add(self.first_hit_tick)
            .ok_or_else(|| route_message("cold replay terminal tape frame overflowed"))?;
        let exact_attempts = self.attempts.len()
            >= usize::try_from(MINIMUM_COLD_REPLAY_REPETITIONS).map_err(route_error)?
            && self.attempts.len()
                <= usize::try_from(MAXIMUM_COLD_REPLAY_REPETITIONS).map_err(route_error)?
            && first.is_some()
            && self.attempts.iter().enumerate().all(|(index, attempt)| {
                usize::try_from(attempt.repetition).ok() == Some(index + 1)
                    && attempt.controller_tape.sha256 == self.controller_tape.sha256
                    && attempt.first_hit_tick == self.first_hit_tick
                    && attempt.tape_frame == expected_tape_frame
                    && attempt.tape_frame.checked_add(1) == Some(self.controller_tape_frames)
                    && attempt.boundary_index == self.controller_tape_frames
                    && attempt.milestone_result.sha256 != Digest::ZERO
                    && attempt.stdout.sha256 != Digest::ZERO
                    && attempt.stderr.sha256 != Digest::ZERO
                    && exact_boundary_fingerprint(&attempt.boundary_fingerprint)
                    && first.is_some_and(|first| {
                        attempt.sim_tick == first.sim_tick
                            && attempt.tape_frame == first.tape_frame
                            && attempt.boundary_index == first.boundary_index
                            && attempt.boundary_fingerprint == first.boundary_fingerprint
                    })
            });
        if self.schema != NATIVE_TACTIC_COLD_REPLAY_PROOF_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
            || self.optimization_request_sha256 == Digest::ZERO
            || self.execution_binding_sha256 == Digest::ZERO
            || self.execution_plan_sha256 == Digest::ZERO
            || self.route_report_sha256 == Digest::ZERO
            || self.state_graph_sha256 == Digest::ZERO
            || self.terminal_result_sha256 == Digest::ZERO
            || self.terminal_state_sha256 == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || self.terminal_program_sha256 == Digest::ZERO
            || self.terminal_definition_sha256 == Digest::ZERO
            || self.goal.is_empty()
            || self.first_hit_tick > self.maximum_first_hit_tick
            || self.controller_tape.sha256 == Digest::ZERO
            || self.controller_tape_frames
                != expected_tape_frame
                    .checked_add(1)
                    .ok_or_else(|| route_message("cold replay tape length overflowed"))?
            || !native_fingerprint(&self.source_boundary_fingerprint)
            || !native_fingerprint(&self.native_source_boundary_fingerprint)
            || !self.fidelity.is_exact_headless()
            || self.controller_in_loop
            || self.learner_in_loop
            || !exact_attempts
        {
            return Err(route_message(
                "native tactic cold replay proof is invalid, detached, or nonexact",
            ));
        }
        Ok(())
    }

    fn identity(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical).map_err(route_error)?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-tactic-cold-replay-proof/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, NativeTacticRouteRunError> {
        let mut bytes = serde_json::to_vec_pretty(self).map_err(route_error)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

pub fn run_native_tactic_cold_replay(
    config: &NativeTacticColdReplayConfig<'_>,
) -> Result<NativeTacticColdReplayProof, NativeTacticRouteRunError> {
    validate_run_config(config)?;
    let authority = validate_route_authority(
        config.repository_root,
        config.optimization,
        config.execution,
        config.execution_plan,
        config.route_report,
        config.seed,
        config.maximum_first_hit_tick,
    )?;
    if config.output_root.exists() {
        return Err(route_message(format!(
            "native tactic cold replay output already exists: {}",
            config.output_root.display()
        )));
    }
    fs::create_dir_all(config.output_root).map_err(route_error)?;
    let tape_path = config.output_root.join(NATIVE_TACTIC_COLD_REPLAY_TAPE_FILE);
    write_new(&tape_path, &authority.tape_bytes)?;
    let controller_tape =
        artifact_reference(NATIVE_TACTIC_COLD_REPLAY_TAPE_FILE, &authority.tape_bytes);
    let executable = authority
        .repository_root
        .join(&config.execution.executable.path);
    let game_data = authority
        .repository_root
        .join(&config.execution.game_data.path);
    let milestone_program = authority
        .repository_root
        .join(&config.execution.milestone_program.path);
    let card_fixture = config
        .execution
        .card_fixture_root(&authority.repository_root, config.optimization)
        .map_err(route_error)?;
    let logical_ticks = authority.tape.frames.len().to_string();
    let mut attempts =
        Vec::with_capacity(usize::try_from(config.repetitions).map_err(route_error)?);
    for repetition in 1..=config.repetitions {
        attempts.push(run_cold_replay_attempt(
            config,
            &authority,
            &executable,
            &game_data,
            &milestone_program,
            &card_fixture,
            &logical_ticks,
            repetition,
        )?);
    }
    let proof = NativeTacticColdReplayProof::seal(config, &authority, controller_tape, attempts)?;
    write_new(
        &config
            .output_root
            .join(NATIVE_TACTIC_COLD_REPLAY_PROOF_FILE),
        &proof.to_pretty_json()?,
    )?;
    validate_native_tactic_cold_replay_artifacts(
        config.output_root,
        &proof,
        config.optimization,
        &authority.tape,
        &authority.tape_bytes,
        authority.first_hit_tick,
    )?;
    Ok(proof)
}

#[allow(clippy::too_many_arguments)]
fn run_cold_replay_attempt(
    config: &NativeTacticColdReplayConfig<'_>,
    authority: &ValidatedRouteAuthority,
    executable: &Path,
    game_data: &Path,
    milestone_program: &Path,
    card_fixture: &Path,
    logical_ticks: &str,
    repetition: u32,
) -> Result<NativeTacticColdReplayAttempt, NativeTacticRouteRunError> {
    let trial_relative = format!("repeat-{repetition:03}");
    let trial = config.output_root.join(&trial_relative);
    let state = trial.join("state");
    let renderer_cache = trial.join("renderer-cache");
    let tape_path = trial.join("controller.tape");
    let result_path = trial.join("milestones.json");
    let stdout_path = trial.join("stdout.txt");
    let stderr_path = trial.join("stderr.txt");
    fs::create_dir_all(&state).map_err(route_error)?;
    fs::create_dir_all(&renderer_cache).map_err(route_error)?;
    write_new(&tape_path, &authority.tape_bytes)?;
    let stdout = fs::File::create(&stdout_path).map_err(route_error)?;
    let stderr = fs::File::create(&stderr_path).map_err(route_error)?;
    let mut command = Command::new(executable);
    command
        .current_dir(&authority.repository_root)
        .arg("--dvd")
        .arg(game_data)
        .arg("--input-tape")
        .arg(&tape_path)
        .arg("--input-tape-end")
        .arg("hold")
        .arg("--automation-tick-budget")
        .arg(logical_ticks)
        .arg("--automation-data-root")
        .arg(&state)
        .arg("--renderer-cache-root")
        .arg(&renderer_cache)
        .arg("--automation-card-fixture")
        .arg(card_fixture)
        .arg("--milestone-program")
        .arg(milestone_program)
        .arg("--milestones")
        .arg(&config.optimization.terminal_predicate.goal)
        .arg("--milestone-goal")
        .arg(&config.optimization.terminal_predicate.goal)
        .arg("--milestone-result")
        .arg(&result_path)
        .arg("--headless")
        .arg("--fixed-step")
        .arg("--unpaced")
        .arg("--exit-after-tape")
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    append_cold_replay_execution_authority(&mut command, config.execution);
    for cvar in FIXED_AUTOMATION_CVARS {
        command.arg("--cvar").arg(cvar);
    }
    let started = Instant::now();
    let mut child = command.spawn().map_err(route_error)?;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(route_error)? {
            break status;
        }
        if started.elapsed() >= config.timeout {
            child.kill().map_err(route_error)?;
            let _ = child.wait();
            return Err(route_message(format!(
                "native tactic cold replay {repetition} timed out"
            )));
        }
        thread::sleep(Duration::from_millis(10));
    };
    if !status.success() {
        return Err(route_message(format!(
            "native tactic cold replay {repetition} exited with {:?}",
            status.code()
        )));
    }
    let milestone_bytes = read_bounded_artifact(&result_path)?;
    let stdout_bytes = read_bounded_artifact(&stdout_path)?;
    let stderr_bytes = read_bounded_artifact(&stderr_path)?;
    parse_attempt(
        config.optimization,
        &authority.tape,
        authority.first_hit_tick,
        repetition,
        artifact_reference(
            &format!("{trial_relative}/controller.tape"),
            &authority.tape_bytes,
        ),
        artifact_reference(
            &format!("{trial_relative}/milestones.json"),
            &milestone_bytes,
        ),
        artifact_reference(&format!("{trial_relative}/stdout.txt"), &stdout_bytes),
        artifact_reference(&format!("{trial_relative}/stderr.txt"), &stderr_bytes),
        &milestone_bytes,
    )
}

/// Tape-mode cold replay authenticates the game image directly. World-context
/// identity remains bound by the sealed execution/proof, but the native CLI
/// accepts its runtime flag only for suffix-batch observation extraction.
fn append_cold_replay_execution_authority(
    command: &mut Command,
    execution: &NativeResidualExecutionBinding,
) {
    command
        .arg("--automation-game-data-sha256")
        .arg(execution.game_data.sha256.to_string());
}

pub fn read_and_validate_native_tactic_cold_replay(
    repository_root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    execution_plan: &NativeTacticExecutionPlan,
    route_report: &NativeTacticRouteReport,
    proof_root: &Path,
) -> Result<NativeTacticColdReplayProof, NativeTacticRouteRunError> {
    let proof_path = proof_root.join(NATIVE_TACTIC_COLD_REPLAY_PROOF_FILE);
    let proof: NativeTacticColdReplayProof = read_bounded_json(&proof_path)?;
    proof.validate_shape()?;
    let authority = validate_route_authority(
        repository_root,
        optimization,
        execution,
        execution_plan,
        route_report,
        proof.seed,
        proof.maximum_first_hit_tick,
    )?;
    validate_proof_authorities(&proof, optimization, execution, &authority, proof.seed)?;
    validate_native_tactic_cold_replay_artifacts(
        proof_root,
        &proof,
        optimization,
        &authority.tape,
        &authority.tape_bytes,
        authority.first_hit_tick,
    )?;
    Ok(proof)
}

fn validate_run_config(
    config: &NativeTacticColdReplayConfig<'_>,
) -> Result<(), NativeTacticRouteRunError> {
    if config.repetitions < MINIMUM_COLD_REPLAY_REPETITIONS
        || config.repetitions > MAXIMUM_COLD_REPLAY_REPETITIONS
        || config.timeout.is_zero()
    {
        return Err(route_message(
            "native tactic cold replay configuration is invalid",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_route_authority(
    repository_root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    execution_plan: &NativeTacticExecutionPlan,
    route_report: &NativeTacticRouteReport,
    seed: u64,
    maximum_first_hit_tick: u64,
) -> Result<ValidatedRouteAuthority, NativeTacticRouteRunError> {
    let root = repository_root.canonicalize().map_err(route_error)?;
    execution
        .validate_files(&root, optimization)
        .map_err(route_error)?;
    execution_plan.validate()?;
    let execution_plan_sha256 = execution_plan.identity()?;
    let route_report_sha256 = route_report_sha256(route_report)?;
    let seed_index = execution_plan
        .seeds
        .iter()
        .position(|candidate| *candidate == seed)
        .ok_or_else(|| route_message("cold replay seed is absent from the execution plan"))?;
    let reported = route_report
        .seeds
        .get(seed_index)
        .filter(|result| result.seed == seed)
        .ok_or_else(|| route_message("cold replay seed is absent from the route report"))?;
    let lane = execution_plan
        .lanes
        .get(seed_index)
        .ok_or_else(|| route_message("cold replay seed has no execution-plan lane"))?;
    let reported_terminal_tape = reported
        .best_terminal_tape
        .as_deref()
        .ok_or_else(|| route_message("cold replay seed has no best terminal tape"))?;
    let seed_root = Path::new(reported_terminal_tape)
        .parent()
        .ok_or_else(|| route_message("cold replay terminal tape has no seed root"))?;
    let seed_result = read_completed_seed_result(
        &seed_root.join("seed-result.json"),
        seed,
        execution_plan.budgets.decisions_per_lane,
        execution_plan_sha256,
        lane,
    )?;
    if serde_json::to_vec(&seed_result).map_err(route_error)?
        != serde_json::to_vec(reported).map_err(route_error)?
    {
        return Err(route_message(
            "cold replay seed differs from its validated route-report evidence",
        ));
    }
    let first_hit_tick = seed_result
        .best_authenticated_tick
        .ok_or_else(|| route_message("cold replay seed has no authenticated terminal"))?;
    let best_report_tick = route_report
        .best_authenticated_tick
        .ok_or_else(|| route_message("cold replay route report has no terminal"))?;
    let tape_path = seed_result
        .best_terminal_tape
        .as_deref()
        .ok_or_else(|| route_message("cold replay seed has no best terminal tape"))?;
    let result_path = seed_result
        .best_terminal_result
        .as_deref()
        .ok_or_else(|| route_message("cold replay seed has no best terminal result"))?;
    let tape_bytes = fs::read(tape_path).map_err(route_error)?;
    let tape = InputTape::decode(&tape_bytes).map_err(route_error)?.tape;
    let final_result = TacticQFinalResult::read(Path::new(result_path)).map_err(route_error)?;
    let tape_frames = u64::try_from(tape.frames.len()).map_err(route_error)?;
    let expected_frames = optimization
        .route
        .source_boundary_index
        .checked_add(first_hit_tick)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| route_message("cold replay route length overflowed"))?;
    if optimization.execution.fidelity != HarnessFidelityMode::Headless
        || !supports_current_route_report_schema(&route_report.schema)
        || route_report.optimization_request_sha256 != optimization.content_sha256
        || route_report.execution_binding_sha256 != execution.content_sha256
        || route_report.execution_plan_sha256 != execution_plan_sha256
        || route_report.objective_sha256 != optimization.terminal_predicate.definition_sha256
        || route_report.exploration_seeds != execution_plan.seeds
        || route_report.seeds.len() != execution_plan.seeds.len()
        || route_report.terminal_seeds == 0
        || first_hit_tick != best_report_tick
        || first_hit_tick > maximum_first_hit_tick
        || !seed_result.terminal_discovered
        || seed_result.state_graph_sha256 == Digest::ZERO
        || final_result.execution_authority_sha256 != execution_plan_sha256
        || final_result.objective_sha256 != optimization.terminal_predicate.definition_sha256
        || final_result.route_tape != tape
        || final_result.route_tape_sha256 != Digest(Sha256::digest(&tape_bytes).into())
        || final_result.terminal_state_sha256
            != seed_result
                .best_terminal_state_sha256
                .unwrap_or(Digest::ZERO)
        || tape.boot != dusklight_automation_contracts::tape::TapeBoot::Process
        || tape.tick_rate_numerator != 30
        || tape.tick_rate_denominator != 1
        || tape_frames != expected_frames
        || !native_fingerprint(&optimization.route.source_boundary_fingerprint)
        || !native_fingerprint(&optimization.route.native_source_boundary_fingerprint)
    {
        return Err(route_message(
            "native tactic cold replay route authority is invalid, detached, or above the accepted tick",
        ));
    }
    Ok(ValidatedRouteAuthority {
        repository_root: root,
        route_report_sha256,
        execution_plan_sha256,
        seed_result,
        final_result,
        tape,
        tape_bytes,
        first_hit_tick,
    })
}

fn validate_proof_authorities(
    proof: &NativeTacticColdReplayProof,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    authority: &ValidatedRouteAuthority,
    seed: u64,
) -> Result<(), NativeTacticRouteRunError> {
    if proof.optimization_request_sha256 != optimization.content_sha256
        || proof.execution_binding_sha256 != execution.content_sha256
        || proof.execution_plan_sha256 != authority.execution_plan_sha256
        || proof.route_report_sha256 != authority.route_report_sha256
        || proof.seed != seed
        || proof.state_graph_sha256 != authority.seed_result.state_graph_sha256
        || proof.terminal_result_sha256 != authority.final_result.content_sha256
        || proof.terminal_state_sha256 != authority.final_result.terminal_state_sha256
        || proof.objective_sha256 != authority.final_result.objective_sha256
        || proof.source_boundary_index != optimization.route.source_boundary_index
        || proof.source_boundary_fingerprint != optimization.route.source_boundary_fingerprint
        || proof.native_source_boundary_fingerprint
            != optimization.route.native_source_boundary_fingerprint
        || proof.goal != optimization.terminal_predicate.goal
        || proof.terminal_program_sha256 != optimization.terminal_predicate.program_sha256
        || proof.terminal_definition_sha256 != optimization.terminal_predicate.definition_sha256
        || proof.first_hit_tick != authority.first_hit_tick
        || proof.executable != execution.executable
        || proof.runtime_dependencies != execution.runtime_dependencies
        || proof.game_data != execution.game_data
        || proof.milestone_program != execution.milestone_program
        || proof.world_context != execution.world_context
        || proof.card_fixture_manifest != execution.card_fixture_manifest
    {
        return Err(route_message(
            "native tactic cold replay proof belongs to another route authority",
        ));
    }
    Ok(())
}

pub(super) fn validate_native_tactic_cold_replay_artifacts(
    proof_root: &Path,
    proof: &NativeTacticColdReplayProof,
    optimization: &OptimizationRequest,
    expected_tape: &InputTape,
    expected_tape_bytes: &[u8],
    expected_first_hit_tick: u64,
) -> Result<(), NativeTacticRouteRunError> {
    proof.validate_shape()?;
    let tape_bytes = read_proof_artifact(proof_root, &proof.controller_tape)?;
    let tape = InputTape::decode(&tape_bytes).map_err(route_error)?.tape;
    if tape_bytes != expected_tape_bytes || &tape != expected_tape {
        return Err(route_message(
            "cold replay controller bytes differ from the graph-selected route",
        ));
    }
    for retained in &proof.attempts {
        let attempt_tape_bytes = read_proof_artifact(proof_root, &retained.controller_tape)?;
        let milestone_bytes = read_proof_artifact(proof_root, &retained.milestone_result)?;
        let stdout_bytes = read_proof_artifact(proof_root, &retained.stdout)?;
        let stderr_bytes = read_proof_artifact(proof_root, &retained.stderr)?;
        let parsed = parse_attempt(
            optimization,
            &tape,
            expected_first_hit_tick,
            retained.repetition,
            retained.controller_tape.clone(),
            retained.milestone_result.clone(),
            retained.stdout.clone(),
            retained.stderr.clone(),
            &milestone_bytes,
        )?;
        if attempt_tape_bytes != tape_bytes
            || artifact_reference(&retained.stdout.path, &stdout_bytes) != retained.stdout
            || artifact_reference(&retained.stderr.path, &stderr_bytes) != retained.stderr
            || parsed != *retained
        {
            return Err(route_message(
                "cold replay attempt differs from its retained exact evidence",
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn parse_attempt(
    optimization: &OptimizationRequest,
    tape: &InputTape,
    first_hit_tick: u64,
    repetition: u32,
    controller_tape: NativeTacticColdReplayArtifact,
    milestone_result: NativeTacticColdReplayArtifact,
    stdout: NativeTacticColdReplayArtifact,
    stderr: NativeTacticColdReplayArtifact,
    bytes: &[u8],
) -> Result<NativeTacticColdReplayAttempt, NativeTacticRouteRunError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(route_error)?;
    if value
        .pointer("/schema/name")
        .and_then(serde_json::Value::as_str)
        != Some("dusklight.automation.milestones")
        || value
            .pointer("/schema/version")
            .and_then(serde_json::Value::as_u64)
            != Some(5)
        || value
            .get("boot_origin_established")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || serde_json::from_value::<dusklight_automation_contracts::tape::TapeBoot>(
            value["boot"].clone(),
        )
        .ok()
            != Some(tape.boot.clone())
        || value.get("goal").and_then(serde_json::Value::as_str)
            != Some(optimization.terminal_predicate.goal.as_str())
        || value
            .get("goal_reached")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || value
            .get("program_digest")
            .and_then(serde_json::Value::as_str)
            != Some(
                optimization
                    .terminal_predicate
                    .program_sha256
                    .to_string()
                    .as_str(),
            )
    {
        return Err(route_message(
            "cold replay returned unauthenticated milestone authority",
        ));
    }
    let matching = value
        .get("milestones")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| route_message("cold replay omitted milestones"))?
        .iter()
        .filter(|milestone| {
            milestone.get("id").and_then(serde_json::Value::as_str)
                == Some(optimization.terminal_predicate.goal.as_str())
        })
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        return Err(route_message(
            "cold replay did not return exactly one terminal goal",
        ));
    }
    let milestone = matching[0];
    let sim_tick = milestone
        .get("sim_tick")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| route_message("cold replay goal omitted sim_tick"))?;
    let tape_frame = milestone
        .get("tape_frame")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| route_message("cold replay goal omitted tape_frame"))?;
    let boundary_index = milestone
        .get("boundary_index")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| route_message("cold replay goal omitted boundary index"))?;
    let boundary_fingerprint: BoundaryFingerprint = serde_json::from_value(
        milestone
            .pointer("/evidence/boundary_fingerprint")
            .cloned()
            .ok_or_else(|| route_message("cold replay goal omitted boundary fingerprint"))?,
    )
    .map_err(route_error)?;
    let full_frames = u64::try_from(tape.frames.len()).map_err(route_error)?;
    let expected_tape_frame = optimization
        .route
        .source_boundary_index
        .checked_add(first_hit_tick)
        .ok_or_else(|| route_message("cold replay terminal frame overflowed"))?;
    if milestone.get("hit").and_then(serde_json::Value::as_bool) != Some(true)
        || milestone.get("phase").and_then(serde_json::Value::as_str) != Some("post_sim")
        || milestone
            .get("definition_digest")
            .and_then(serde_json::Value::as_str)
            != Some(
                optimization
                    .terminal_predicate
                    .definition_sha256
                    .to_string()
                    .as_str(),
            )
        || milestone
            .get("program_digest")
            .and_then(serde_json::Value::as_str)
            != Some(
                optimization
                    .terminal_predicate
                    .program_sha256
                    .to_string()
                    .as_str(),
            )
        || tape_frame != expected_tape_frame
        || tape_frame.checked_add(1) != Some(full_frames)
        || boundary_index != full_frames
        || !exact_boundary_fingerprint(&boundary_fingerprint)
    {
        return Err(route_message(
            "cold replay terminal proof differs from the graph-selected route",
        ));
    }
    Ok(NativeTacticColdReplayAttempt {
        repetition,
        controller_tape,
        milestone_result,
        stdout,
        stderr,
        sim_tick,
        tape_frame,
        boundary_index,
        first_hit_tick,
        boundary_fingerprint,
    })
}

fn artifact_reference(path: &str, bytes: &[u8]) -> NativeTacticColdReplayArtifact {
    NativeTacticColdReplayArtifact {
        path: path.replace('\\', "/"),
        sha256: Digest(Sha256::digest(bytes).into()),
    }
}

pub(super) fn read_proof_artifact(
    proof_root: &Path,
    artifact: &NativeTacticColdReplayArtifact,
) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    let relative = Path::new(&artifact.path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(route_message(
            "cold replay artifact path is not a confined relative path",
        ));
    }
    let bytes = read_bounded_artifact(&proof_root.join(relative))?;
    if artifact.sha256 == Digest::ZERO || artifact.sha256 != Digest(Sha256::digest(&bytes).into()) {
        return Err(route_message(
            "cold replay artifact content identity is invalid",
        ));
    }
    Ok(bytes)
}

fn read_bounded_artifact(path: &Path) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    let metadata = fs::symlink_metadata(path).map_err(route_error)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAXIMUM_COLD_REPLAY_ARTIFACT_BYTES
    {
        return Err(route_message(format!(
            "cold replay artifact is invalid or oversized: {}",
            path.display()
        )));
    }
    fs::read(path).map_err(route_error)
}

fn native_fingerprint(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn exact_boundary_fingerprint(fingerprint: &BoundaryFingerprint) -> bool {
    fingerprint.algorithm == "xxh3-128"
        && native_fingerprint(&fingerprint.digest)
        && matches!(
            (
                fingerprint.schema.as_str(),
                fingerprint.canonical_encoding.as_str()
            ),
            ("dusklight.milestone-boundary/v4", "little-endian-fixed-v4")
                | ("dusklight.milestone-boundary/v5", "little-endian-fixed-v5")
                | ("dusklight.milestone-boundary/v6", "little-endian-fixed-v6")
        )
}

#[cfg(test)]
#[path = "cold_replay_tests.rs"]
pub(crate) mod tests;
