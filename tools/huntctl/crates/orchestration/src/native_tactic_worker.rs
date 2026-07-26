//! Execute one selected static tactic against an authenticated persistent
//! native checkpoint worker and recover its real option boundary.

use crate::native_suffix_result::{NativeRetainedCheckpointResult, ValidatedNativeSuffixBatch};
use crate::native_suffix_worker::{
    NativeSuffixWorkerError, NativeSuffixWorkerIdentity, NativeSuffixWorkerSession,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::{InputFrame, InputTape, RawPadState, WaitCondition};
use dusklight_control::controller_program::{
    ControllerProgram, CoordinateFrame, Layer, Operation, StickBlend,
};
use dusklight_control::controller_runtime::{
    ControllerProgramStepper, ControllerRuntimeActor, ControllerRuntimeEnd,
    ControllerRuntimeObservation, ControllerRuntimeQueryRecord, MAX_CONTROLLER_RUNTIME_ACTORS,
};
use dusklight_control::option_execution::{
    OptionCondition, OptionEndReason, OptionExecution, TapeRange,
};
use dusklight_evidence::native_episode_shard::{
    NativeEpisode, NativeEpisodeShard, NativeLearningObservation, NativeObservationPhase,
    NativeRawPad,
};
use dusklight_learning::fact_snapshot::{FactPhase, FactSnapshot};
use dusklight_learning::learner_state::tactic_intrinsically_applicable;
use dusklight_learning::native_generic_tactic::{
    GenericTactic, NativeGenericTacticStepper, NativeTacticObservation, NativeTacticQueryRecord,
};
use dusklight_learning::tactic_asset::{
    PreparedTacticExecution, TacticAssetCatalog, TacticDurationBounds,
};
use dusklight_learning::tactic_blueprint::{
    ApplicableTacticChoices, TacticBlueprint, TacticBlueprintError,
};
use dusklight_learning::tactic_exploration::SelectedTactic;
use dusklight_search::search::{MacroAction, SearchPadState};
use dusklight_search::suffix_batch::{
    NATIVE_CACHED_SUFFIX_BATCH_SCHEMA, NativeCheckpointCacheRequest, NativeCheckpointValidation,
    NativeSuffixBatch, NativeSuffixCandidate,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const NATIVE_TACTIC_WORKER_OUTCOME_SCHEMA_V2: &str =
    "dusklight-native-tactic-worker-outcome/v2";
const TACTIC_CHECKPOINT_CACHE_BYTES: usize = 640 * 1024 * 1024;
const TACTIC_CHECKPOINT_CACHE_ENTRIES: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeGenericExecutionStrategy {
    NativeController,
    ProgressiveAudit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeTacticWorkerPaths {
    pub request: PathBuf,
    pub result: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeTacticCheckpointSource {
    pub restore_identity: String,
    pub boundary_fingerprint: String,
    pub route_ticks: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedNativeTacticFrontier {
    pub source: NativeTacticCheckpointSource,
    pub episode_shard_sha256: Digest,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticWorkerOutcome {
    pub schema: String,
    pub source_checkpoint_sha256: Digest,
    pub checkpoint_identity: String,
    #[serde(skip_serializing)]
    pub retained_native_checkpoint: Option<NativeRetainedCheckpointResult>,
    #[serde(skip_serializing)]
    pub retained_native_boundary_fingerprint: Option<String>,
    pub episode_shard_sha256: Digest,
    pub selected: SelectedTactic,
    pub execution: OptionExecution,
    pub native_queries: Vec<TacticRuntimeQuery>,
    pub route_tape: InputTape,
    pub next_facts: FactSnapshot,
    pub terminal: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", content = "query", rename_all = "snake_case")]
pub enum TacticRuntimeQuery {
    NativeGeneric(NativeTacticQueryRecord),
    ReactiveController(ControllerRuntimeQueryRecord),
}

#[derive(Clone, Debug)]
struct PreparedNativeTactic {
    option_tape: InputTape,
    execution: OptionExecution,
    duration: TacticDurationBounds,
}

#[derive(Clone, Debug)]
enum PreparedNativeExecution {
    Static(PreparedNativeTactic),
    NativeGeneric {
        stepper: NativeGenericTacticStepper,
        duration: TacticDurationBounds,
        termination: OptionCondition,
    },
    ReactiveController {
        program: ControllerProgram,
        stepper: ControllerProgramStepper,
        duration: TacticDurationBounds,
        termination: OptionCondition,
        cancellation: Vec<OptionCondition>,
    },
}

pub trait PersistentTacticBatchWorker {
    fn identity(&self) -> &NativeSuffixWorkerIdentity;

    fn run_tactic_batch(
        &mut self,
        request: &Path,
        result: &Path,
    ) -> Result<ValidatedNativeSuffixBatch, NativeTacticWorkerError>;
}

impl PersistentTacticBatchWorker for NativeSuffixWorkerSession {
    fn identity(&self) -> &NativeSuffixWorkerIdentity {
        self.identity()
    }

    fn run_tactic_batch(
        &mut self,
        request: &Path,
        result: &Path,
    ) -> Result<ValidatedNativeSuffixBatch, NativeTacticWorkerError> {
        self.run_batch(request, result, None)
            .map_err(NativeTacticWorkerError::Worker)
    }
}

/// Stable identity of the authenticated native source across worker launches.
/// The emulator's internal restore handle is intentionally excluded because it
/// is process-local and changes after every cold launch.
pub fn tactic_root_checkpoint_sha256(
    identity: &NativeSuffixWorkerIdentity,
) -> Result<Digest, NativeTacticWorkerError> {
    let bytes = serde_json::to_vec(identity)
        .map_err(|error| NativeTacticWorkerError::Serialization(error.to_string()))?;
    Ok(sha256(&bytes))
}

pub fn materialize_tactic_frontier<W: PersistentTacticBatchWorker>(
    worker: &mut W,
    expected: &FactSnapshot,
    route: &InputTape,
    paths: &NativeTacticWorkerPaths,
) -> Result<MaterializedNativeTacticFrontier, NativeTacticWorkerError> {
    expected
        .validate()
        .map_err(|error| NativeTacticWorkerError::Facts(error.to_string()))?;
    route
        .validate()
        .map_err(|error| NativeTacticWorkerError::Tape(error.to_string()))?;
    let source_frame = usize::try_from(worker.identity().source_frame)
        .map_err(|_| NativeTacticWorkerError::InvalidDuration)?;
    if expected.tape_frame != route.frames.len() as u64
        || expected.terminal.reached != Some(false)
        || route.frames.len() <= source_frame
    {
        return Err(NativeTacticWorkerError::DetachedSelection);
    }
    let replay_frames = &route.frames[source_frame..];
    if replay_frames.len() > 4_096 {
        return Err(NativeTacticWorkerError::InvalidDuration);
    }
    let request = NativeSuffixBatch {
        schema: NATIVE_CACHED_SUFFIX_BATCH_SCHEMA.into(),
        source_frame,
        source_boundary_fingerprint: worker.identity().source_boundary_fingerprint.clone(),
        checkpoint_validation: NativeCheckpointValidation {
            kind: worker.identity().checkpoint_validation_kind.clone(),
            ticks: usize::try_from(worker.identity().checkpoint_validation_ticks)
                .map_err(|_| NativeTacticWorkerError::InvalidDuration)?,
        },
        maximum_ticks: replay_frames.len(),
        verify_state_hashes: false,
        checkpoint_cache: Some(tactic_checkpoint_cache_request(None, true)),
        candidates: vec![NativeSuffixCandidate {
            id: hex_digest(
                route
                    .encode()
                    .map_err(|error| NativeTacticWorkerError::Tape(error.to_string()))?,
            ),
            actions: pad_runs(replay_frames)?,
            controller_program_hex: None,
        }],
    };
    write_new_json(&paths.request, &request)?;
    let validated = worker.run_tactic_batch(&paths.request, &paths.result)?;
    if validated.candidates.len() != 1
        || validated.candidates[0].id != request.candidates[0].id
        || validated.candidates[0].simulated_ticks != replay_frames.len() as u64
        || validated.candidates[0].first_hit_tick.is_some()
    {
        return Err(NativeTacticWorkerError::DetachedResult(
            "frontier materialization",
        ));
    }
    let bytes = fs::read(&validated.episode_shard_path)
        .map_err(|error| NativeTacticWorkerError::Io(error.to_string()))?;
    let shard = NativeEpisodeShard::decode(&bytes)
        .map_err(|error| NativeTacticWorkerError::Evidence(error.to_string()))?;
    if shard.metadata.checkpoint_identity != validated.restore_identity
        || shard.source_frame != source_frame as u64
        || shard.episodes.len() != 1
        || shard.episodes[0].id != request.candidates[0].id
        || shard.episodes[0].steps.len() != replay_frames.len()
    {
        return Err(NativeTacticWorkerError::DetachedResult(
            "frontier materialization episode",
        ));
    }
    for (step, expected_frame) in shard.episodes[0].steps.iter().zip(replay_frames) {
        if !same_pad(step.chosen_pad, expected_frame.pads[0])
            || !same_pad(step.consumed_pad, expected_frame.pads[0])
        {
            return Err(NativeTacticWorkerError::PadMismatch);
        }
    }
    let endpoint =
        shard.episodes[0]
            .steps
            .last()
            .ok_or(NativeTacticWorkerError::DetachedResult(
                "frontier materialization endpoint",
            ))?;
    if endpoint.post_simulation.state_identity != expected.state_identity
        || endpoint.post_simulation.simulation_tick.checked_add(1) != Some(expected.simulation_tick)
        || endpoint.post_simulation.tape_frame.checked_add(1) != Some(expected.tape_frame)
    {
        return Err(NativeTacticWorkerError::DetachedResult(
            "frontier materialization state",
        ));
    }
    let retained = validated.candidates[0].retained_checkpoint.as_ref().ok_or(
        NativeTacticWorkerError::DetachedResult("frontier materialization checkpoint"),
    )?;
    if retained.route_ticks != replay_frames.len() as u64
        || !lower_hex_identity(&retained.restore_identity)
        || !lower_hex_identity(&validated.candidates[0].terminal_boundary_fingerprint)
    {
        return Err(NativeTacticWorkerError::DetachedResult(
            "frontier materialization checkpoint",
        ));
    }
    Ok(MaterializedNativeTacticFrontier {
        source: NativeTacticCheckpointSource {
            restore_identity: retained.restore_identity.clone(),
            boundary_fingerprint: validated.candidates[0]
                .terminal_boundary_fingerprint
                .clone(),
            route_ticks: replay_frames.len(),
        },
        episode_shard_sha256: shard.content_sha256,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn execute_selected_tactic<W: PersistentTacticBatchWorker>(
    worker: &mut W,
    selected: &SelectedTactic,
    catalog: &TacticAssetCatalog,
    blueprints: &[TacticBlueprint],
    before: &FactSnapshot,
    route_prefix: &InputTape,
    checkpoint_source: Option<&NativeTacticCheckpointSource>,
    paths: &NativeTacticWorkerPaths,
) -> Result<NativeTacticWorkerOutcome, NativeTacticWorkerError> {
    execute_selected_tactic_with_checkpoint_retention(
        worker,
        selected,
        catalog,
        blueprints,
        before,
        route_prefix,
        checkpoint_source,
        paths,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn execute_selected_tactic_with_checkpoint_retention<W: PersistentTacticBatchWorker>(
    worker: &mut W,
    selected: &SelectedTactic,
    catalog: &TacticAssetCatalog,
    blueprints: &[TacticBlueprint],
    before: &FactSnapshot,
    route_prefix: &InputTape,
    checkpoint_source: Option<&NativeTacticCheckpointSource>,
    paths: &NativeTacticWorkerPaths,
    retain_candidate_checkpoint: bool,
) -> Result<NativeTacticWorkerOutcome, NativeTacticWorkerError> {
    execute_selected_tactic_with_checkpoint_retention_and_strategy(
        worker,
        selected,
        catalog,
        blueprints,
        before,
        route_prefix,
        checkpoint_source,
        paths,
        retain_candidate_checkpoint,
        NativeGenericExecutionStrategy::NativeController,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn execute_selected_tactic_with_checkpoint_retention_and_strategy<
    W: PersistentTacticBatchWorker,
>(
    worker: &mut W,
    selected: &SelectedTactic,
    catalog: &TacticAssetCatalog,
    blueprints: &[TacticBlueprint],
    before: &FactSnapshot,
    route_prefix: &InputTape,
    checkpoint_source: Option<&NativeTacticCheckpointSource>,
    paths: &NativeTacticWorkerPaths,
    retain_candidate_checkpoint: bool,
    execution_strategy: NativeGenericExecutionStrategy,
) -> Result<NativeTacticWorkerOutcome, NativeTacticWorkerError> {
    let root_checkpoint_sha256 = tactic_root_checkpoint_sha256(worker.identity())?;
    before
        .validate()
        .map_err(|error| NativeTacticWorkerError::Facts(error.to_string()))?;
    route_prefix
        .validate()
        .map_err(|error| NativeTacticWorkerError::Tape(error.to_string()))?;
    if before.tape_frame != route_prefix.frames.len() as u64
        || selected.learner_snapshot_sha256
            != before
                .content_sha256()
                .map_err(|error| NativeTacticWorkerError::Facts(error.to_string()))?
    {
        return Err(NativeTacticWorkerError::DetachedSelection);
    }
    let applicable = ApplicableTacticChoices::enumerate(
        catalog,
        blueprints,
        |description| tactic_intrinsically_applicable(description, before),
        |_| None,
    )?;
    if !applicable
        .candidates
        .iter()
        .zip(&applicable.applicable_mask)
        .any(|(choice, applicable)| {
            *applicable
                && choice.choice_id == selected.descriptor.option_id
                && choice.descriptor == selected.descriptor
        })
    {
        return Err(NativeTacticWorkerError::DetachedSelection);
    }

    let source_frame = usize::try_from(worker.identity().source_frame)
        .map_err(|_| NativeTacticWorkerError::InvalidDuration)?;
    if source_frame > route_prefix.frames.len() {
        return Err(NativeTacticWorkerError::DetachedSelection);
    }
    let candidate_prefix_ticks = route_prefix.frames.len() - source_frame;
    if checkpoint_source.is_some_and(|source| {
        source.route_ticks != candidate_prefix_ticks
            || !lower_hex_identity(&source.restore_identity)
            || !lower_hex_identity(&source.boundary_fingerprint)
    }) {
        return Err(NativeTacticWorkerError::DetachedSelection);
    }
    let replayed_prefix_ticks = if checkpoint_source.is_some() {
        0
    } else {
        candidate_prefix_ticks
    };
    match prepare_selected(selected, catalog, blueprints)? {
        PreparedNativeExecution::Static(prepared) => execute_static_tactic(
            worker,
            root_checkpoint_sha256,
            selected,
            before,
            route_prefix,
            paths,
            source_frame,
            replayed_prefix_ticks,
            checkpoint_source,
            retain_candidate_checkpoint,
            prepared,
        ),
        PreparedNativeExecution::NativeGeneric {
            stepper,
            duration,
            termination,
        } => execute_native_generic_tactic(
            worker,
            root_checkpoint_sha256,
            selected,
            before,
            route_prefix,
            paths,
            source_frame,
            replayed_prefix_ticks,
            checkpoint_source,
            retain_candidate_checkpoint,
            stepper,
            duration,
            termination,
            execution_strategy,
        ),
        PreparedNativeExecution::ReactiveController {
            program,
            stepper,
            duration,
            termination,
            cancellation,
        } => {
            if reactive_controller_uses_native_strategy(execution_strategy, &cancellation) {
                execute_reactive_controller_native(
                    worker,
                    root_checkpoint_sha256,
                    selected,
                    before,
                    route_prefix,
                    paths,
                    source_frame,
                    replayed_prefix_ticks,
                    checkpoint_source,
                    retain_candidate_checkpoint,
                    stepper,
                    duration,
                    termination,
                    cancellation,
                    program,
                )
            } else {
                execute_reactive_controller(
                    worker,
                    root_checkpoint_sha256,
                    selected,
                    before,
                    route_prefix,
                    paths,
                    source_frame,
                    replayed_prefix_ticks,
                    checkpoint_source,
                    retain_candidate_checkpoint,
                    stepper,
                    duration,
                    termination,
                    cancellation,
                )
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_static_tactic<W: PersistentTacticBatchWorker>(
    worker: &mut W,
    root_checkpoint_sha256: Digest,
    selected: &SelectedTactic,
    before: &FactSnapshot,
    route_prefix: &InputTape,
    paths: &NativeTacticWorkerPaths,
    source_frame: usize,
    candidate_prefix_ticks: usize,
    checkpoint_source: Option<&NativeTacticCheckpointSource>,
    retain_candidate_checkpoint: bool,
    prepared: PreparedNativeTactic,
) -> Result<NativeTacticWorkerOutcome, NativeTacticWorkerError> {
    let mut candidate_tape = InputTape {
        boot: route_prefix.boot.clone(),
        tick_rate_numerator: route_prefix.tick_rate_numerator,
        tick_rate_denominator: route_prefix.tick_rate_denominator,
        frames: candidate_prefix_frames(route_prefix, source_frame, checkpoint_source),
    };
    candidate_tape
        .frames
        .extend_from_slice(&prepared.option_tape.frames);
    let request = tactic_batch(
        worker.identity(),
        selected,
        &candidate_tape,
        checkpoint_source,
        retain_candidate_checkpoint,
    )?;
    write_new_json(&paths.request, &request)?;
    let validated = worker.run_tactic_batch(&paths.request, &paths.result)?;
    observe_outcome(
        root_checkpoint_sha256,
        selected,
        before,
        route_prefix,
        prepared,
        candidate_tape,
        candidate_prefix_ticks,
        request,
        validated,
        Vec::new(),
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_native_generic_tactic<W: PersistentTacticBatchWorker>(
    worker: &mut W,
    root_checkpoint_sha256: Digest,
    selected: &SelectedTactic,
    before: &FactSnapshot,
    route_prefix: &InputTape,
    paths: &NativeTacticWorkerPaths,
    source_frame: usize,
    candidate_prefix_ticks: usize,
    checkpoint_source: Option<&NativeTacticCheckpointSource>,
    retain_candidate_checkpoint: bool,
    mut stepper: NativeGenericTacticStepper,
    duration: TacticDurationBounds,
    termination: OptionCondition,
    execution_strategy: NativeGenericExecutionStrategy,
) -> Result<NativeTacticWorkerOutcome, NativeTacticWorkerError> {
    if let Some(program) = native_generic_controller_program_for_strategy(
        stepper.plan(),
        duration,
        execution_strategy,
    )? {
        return execute_native_generic_controller(
            worker,
            root_checkpoint_sha256,
            selected,
            before,
            route_prefix,
            paths,
            source_frame,
            candidate_prefix_ticks,
            checkpoint_source,
            retain_candidate_checkpoint,
            stepper,
            duration,
            termination,
            program,
        );
    }
    let mut observation = before
        .to_native_tactic_observation()
        .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?;
    let mut option_tape = InputTape {
        boot: route_prefix.boot.clone(),
        tick_rate_numerator: route_prefix.tick_rate_numerator,
        tick_rate_denominator: route_prefix.tick_rate_denominator,
        frames: Vec::new(),
    };
    let mut queries = Vec::new();

    for local_tick in 0..duration.maximum_ticks {
        let step = stepper
            .step(observation)
            .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?;
        option_tape.frames.push(step.frame);
        queries.push(step.query);

        let mut candidate_tape = InputTape {
            boot: route_prefix.boot.clone(),
            tick_rate_numerator: route_prefix.tick_rate_numerator,
            tick_rate_denominator: route_prefix.tick_rate_denominator,
            frames: candidate_prefix_frames(route_prefix, source_frame, checkpoint_source),
        };
        candidate_tape.frames.extend_from_slice(&option_tape.frames);
        let request = tactic_batch(
            worker.identity(),
            selected,
            &candidate_tape,
            checkpoint_source,
            retain_candidate_checkpoint,
        )?;
        let iteration_paths = iteration_paths(paths, selected.decision_index, local_tick);
        write_new_json(&iteration_paths.request, &request)?;
        let validated =
            worker.run_tactic_batch(&iteration_paths.request, &iteration_paths.result)?;
        let episode = inspect_candidate_episode(&request, &validated, &candidate_tape)?;
        if episode.steps.len() <= candidate_prefix_ticks {
            return Err(NativeTacticWorkerError::DetachedResult(
                "route prefix terminated before the selected tactic",
            ));
        }
        let realized_ticks = episode.steps.len() - candidate_prefix_ticks;
        if realized_ticks != option_tape.frames.len() {
            return Err(NativeTacticWorkerError::DetachedResult(
                "observation loop replay diverged",
            ));
        }

        let end_reason = if episode.success && step.end_reason.is_none() {
            Some(OptionEndReason::Cancelled { condition_index: 0 })
        } else {
            step.end_reason
        };
        if let Some(end_reason) = end_reason {
            let cancellation_conditions = matches!(end_reason, OptionEndReason::Cancelled { .. })
                .then(|| {
                    vec![OptionCondition::TargetReached {
                        target: "authored_goal".into(),
                    }]
                })
                .unwrap_or_default();
            let local_execution = OptionExecution::capture(
                selected.descriptor.option_id.clone(),
                selected.descriptor.option_type.clone(),
                selected.descriptor.parameters.clone(),
                duration.minimum_ticks,
                duration.maximum_ticks,
                termination.clone(),
                cancellation_conditions,
                end_reason,
                &option_tape,
                TapeRange {
                    start_frame: 0,
                    end_frame_exclusive: option_tape.frames.len() as u64,
                },
            )
            .map_err(|error| NativeTacticWorkerError::Execution(error.to_string()))?;
            return observe_outcome(
                root_checkpoint_sha256,
                selected,
                before,
                route_prefix,
                PreparedNativeTactic {
                    option_tape,
                    execution: local_execution,
                    duration,
                },
                candidate_tape,
                candidate_prefix_ticks,
                request,
                validated,
                queries
                    .into_iter()
                    .map(TacticRuntimeQuery::NativeGeneric)
                    .collect(),
            );
        }

        let last = episode
            .steps
            .last()
            .ok_or(NativeTacticWorkerError::DetachedResult("empty episode"))?;
        observation = NativeTacticObservation::from_post_simulation_boundary(&last.post_simulation)
            .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?;
    }
    Err(NativeTacticWorkerError::InvalidDuration)
}

fn native_generic_controller_program_for_strategy(
    plan: &dusklight_learning::native_generic_tactic::NativeGenericTacticPlan,
    duration: TacticDurationBounds,
    execution_strategy: NativeGenericExecutionStrategy,
) -> Result<Option<ControllerProgram>, NativeTacticWorkerError> {
    match execution_strategy {
        NativeGenericExecutionStrategy::NativeController => {
            native_generic_controller_program(plan, duration)
        }
        NativeGenericExecutionStrategy::ProgressiveAudit => Ok(None),
    }
}

fn native_generic_controller_program(
    plan: &dusklight_learning::native_generic_tactic::NativeGenericTacticPlan,
    duration: TacticDurationBounds,
) -> Result<Option<ControllerProgram>, NativeTacticWorkerError> {
    if plan.maximum_ticks != duration.maximum_ticks
        || plan.minimum_ticks != duration.minimum_ticks
        || duration.maximum_ticks == 0
    {
        return Err(NativeTacticWorkerError::InvalidDuration);
    }
    let operation = match &plan.tactic {
        GenericTactic::MaintainRelativeHeading {
            heading_radians_f32_bits,
            magnitude,
        } => Operation::MaintainHeading {
            blend: StickBlend::Replace,
            // NativeGenericTactic treats the authored heading as camera-relative
            // and emits the player-relative steering error. The controller's
            // player-frame heading resolves to the same PAD vector when the
            // authored offset is negated:
            //
            // -sin(player - offset - camera) == sin(camera + offset - player)
            frame: CoordinateFrame::Player,
            heading_radians: canonical_controller_heading(-f32::from_bits(
                *heading_radians_f32_bits,
            )),
            magnitude: *magnitude,
        },
        GenericTactic::SeekCoordinate {
            coordinate_f32_bits,
            tolerance_f32_bits,
            magnitude,
        } => Operation::SeekCoordinate {
            blend: StickBlend::Replace,
            frame: CoordinateFrame::World,
            target: coordinate_f32_bits.map(f32::from_bits),
            offset: [0.0; 3],
            stop_radius: f32::from_bits(*tolerance_f32_bits),
            magnitude: *magnitude,
        },
        GenericTactic::SeekCoordinateSequence {
            coordinates_f32_bits,
            intermediate_tolerance_f32_bits,
            final_tolerance_f32_bits,
            stall_grace_ticks,
            magnitude,
            ..
        } if coordinates_f32_bits.len()
            <= dusklight_control::controller_program::MAX_SEEK_COORDINATE_SEQUENCE_POINTS
            && *stall_grace_ticks >= duration.maximum_ticks
            && duration.minimum_ticks <= 1 =>
        {
            Operation::SeekCoordinateSequence {
                blend: StickBlend::Replace,
                coordinates_xz: coordinates_f32_bits
                    .iter()
                    .map(|coordinate| {
                        [f32::from_bits(coordinate[0]), f32::from_bits(coordinate[2])]
                    })
                    .collect(),
                intermediate_stop_radius: f32::from_bits(*intermediate_tolerance_f32_bits),
                final_stop_radius: f32::from_bits(*final_tolerance_f32_bits),
                magnitude: *magnitude,
            }
        }
        GenericTactic::ShortCurve { control } => Operation::CubicBezier {
            blend: StickBlend::Replace,
            points: control.map(|point| point.map(i16::from)),
        },
        _ => return Ok(None),
    };
    let program = ControllerProgram {
        duration_frames: duration.maximum_ticks,
        layers: vec![Layer {
            start_frame: 0,
            duration_frames: duration.maximum_ticks,
            operation,
        }],
    };
    program
        .encode()
        .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?;
    Ok(Some(program))
}

fn canonical_controller_heading(heading: f32) -> f32 {
    let mut wrapped = heading;
    if wrapped >= std::f32::consts::PI {
        wrapped -= std::f32::consts::TAU;
    } else if wrapped < -std::f32::consts::PI {
        wrapped += std::f32::consts::TAU;
    }
    wrapped
}

#[allow(clippy::too_many_arguments)]
fn execute_native_generic_controller<W: PersistentTacticBatchWorker>(
    worker: &mut W,
    root_checkpoint_sha256: Digest,
    selected: &SelectedTactic,
    before: &FactSnapshot,
    route_prefix: &InputTape,
    paths: &NativeTacticWorkerPaths,
    source_frame: usize,
    candidate_prefix_ticks: usize,
    checkpoint_source: Option<&NativeTacticCheckpointSource>,
    retain_candidate_checkpoint: bool,
    mut stepper: NativeGenericTacticStepper,
    duration: TacticDurationBounds,
    termination: OptionCondition,
    program: ControllerProgram,
) -> Result<NativeTacticWorkerOutcome, NativeTacticWorkerError> {
    let prefix_frames = candidate_prefix_frames(route_prefix, source_frame, checkpoint_source);
    if prefix_frames.len() != candidate_prefix_ticks {
        return Err(NativeTacticWorkerError::DetachedSelection);
    }
    let request = tactic_controller_batch(
        worker.identity(),
        selected,
        &prefix_frames,
        &program,
        checkpoint_source,
        retain_candidate_checkpoint,
    )?;
    write_new_json(&paths.request, &request)?;
    let validated = worker.run_tactic_batch(&paths.request, &paths.result)?;
    let episode = load_candidate_episode(&request, &validated)?;
    if episode.steps.len() <= candidate_prefix_ticks
        || episode.steps.len()
            > candidate_prefix_ticks.saturating_add(duration.maximum_ticks as usize)
    {
        return Err(NativeTacticWorkerError::DetachedResult(
            "reactive controller episode length",
        ));
    }
    for (step, expected) in episode.steps.iter().zip(&prefix_frames) {
        if !same_pad(step.chosen_pad, expected.pads[0])
            || !same_pad(step.consumed_pad, expected.pads[0])
        {
            return Err(NativeTacticWorkerError::PadMismatch);
        }
    }

    let mut option_tape = InputTape {
        boot: route_prefix.boot.clone(),
        tick_rate_numerator: route_prefix.tick_rate_numerator,
        tick_rate_denominator: route_prefix.tick_rate_denominator,
        frames: Vec::with_capacity(duration.maximum_ticks as usize),
    };
    let mut queries = Vec::with_capacity(duration.maximum_ticks as usize);
    let option_steps = &episode.steps[candidate_prefix_ticks..];
    let mut stepper_end = None;
    for (index, native_step) in option_steps.iter().enumerate() {
        let observation = NativeTacticObservation::from_native(&native_step.pre_input)
            .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?;
        let realized = stepper
            .step(observation)
            .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?;
        if !same_pad(native_step.chosen_pad, realized.frame.pads[0])
            || !same_pad(native_step.consumed_pad, realized.frame.pads[0])
        {
            return Err(NativeTacticWorkerError::PadMismatch);
        }
        if realized.end_reason.is_some() && index + 1 != option_steps.len() {
            return Err(NativeTacticWorkerError::DetachedResult(
                "native controller continued after the tactic stopped",
            ));
        }
        stepper_end = realized.end_reason;
        option_tape.frames.push(realized.frame);
        queries.push(realized.query);
    }

    let (end_reason, cancellation_conditions) = if episode.success {
        (
            OptionEndReason::Cancelled { condition_index: 0 },
            vec![OptionCondition::TargetReached {
                target: "authored_goal".into(),
            }],
        )
    } else {
        (
            stepper_end.ok_or(NativeTacticWorkerError::DetachedResult(
                "native controller stopped before its bounded tactic",
            ))?,
            Vec::new(),
        )
    };
    let execution = capture_local_execution(
        selected,
        duration,
        termination,
        cancellation_conditions,
        end_reason,
        &option_tape,
    )?;
    let mut candidate_tape = InputTape {
        boot: route_prefix.boot.clone(),
        tick_rate_numerator: route_prefix.tick_rate_numerator,
        tick_rate_denominator: route_prefix.tick_rate_denominator,
        frames: prefix_frames,
    };
    candidate_tape.frames.extend_from_slice(&option_tape.frames);
    observe_outcome(
        root_checkpoint_sha256,
        selected,
        before,
        route_prefix,
        PreparedNativeTactic {
            option_tape,
            execution,
            duration,
        },
        candidate_tape,
        candidate_prefix_ticks,
        request,
        validated,
        queries
            .into_iter()
            .map(TacticRuntimeQuery::NativeGeneric)
            .collect(),
    )
}

fn reactive_controller_uses_native_strategy(
    execution_strategy: NativeGenericExecutionStrategy,
    cancellation: &[OptionCondition],
) -> bool {
    matches!(
        execution_strategy,
        NativeGenericExecutionStrategy::NativeController
    ) && cancellation.is_empty()
}

#[allow(clippy::too_many_arguments)]
fn execute_reactive_controller_native<W: PersistentTacticBatchWorker>(
    worker: &mut W,
    root_checkpoint_sha256: Digest,
    selected: &SelectedTactic,
    before: &FactSnapshot,
    route_prefix: &InputTape,
    paths: &NativeTacticWorkerPaths,
    source_frame: usize,
    candidate_prefix_ticks: usize,
    checkpoint_source: Option<&NativeTacticCheckpointSource>,
    retain_candidate_checkpoint: bool,
    mut stepper: ControllerProgramStepper,
    duration: TacticDurationBounds,
    termination: OptionCondition,
    mut cancellation: Vec<OptionCondition>,
    program: ControllerProgram,
) -> Result<NativeTacticWorkerOutcome, NativeTacticWorkerError> {
    if !cancellation.is_empty() {
        return Err(NativeTacticWorkerError::DetachedSelection);
    }
    let prefix_frames = candidate_prefix_frames(route_prefix, source_frame, checkpoint_source);
    if prefix_frames.len() != candidate_prefix_ticks {
        return Err(NativeTacticWorkerError::DetachedSelection);
    }
    let request = tactic_controller_batch(
        worker.identity(),
        selected,
        &prefix_frames,
        &program,
        checkpoint_source,
        retain_candidate_checkpoint,
    )?;
    write_new_json(&paths.request, &request)?;
    let validated = worker.run_tactic_batch(&paths.request, &paths.result)?;
    let episode = load_candidate_episode(&request, &validated)?;
    if episode.steps.len() <= candidate_prefix_ticks
        || episode.steps.len()
            > candidate_prefix_ticks.saturating_add(duration.maximum_ticks as usize)
    {
        return Err(NativeTacticWorkerError::DetachedResult(
            "reactive controller episode length",
        ));
    }
    for (step, expected) in episode.steps.iter().zip(&prefix_frames) {
        if !same_pad(step.chosen_pad, expected.pads[0])
            || !same_pad(step.consumed_pad, expected.pads[0])
        {
            return Err(NativeTacticWorkerError::PadMismatch);
        }
    }

    let mut option_tape = InputTape {
        boot: route_prefix.boot.clone(),
        tick_rate_numerator: route_prefix.tick_rate_numerator,
        tick_rate_denominator: route_prefix.tick_rate_denominator,
        frames: Vec::with_capacity(duration.maximum_ticks as usize),
    };
    let mut queries = Vec::with_capacity(duration.maximum_ticks as usize);
    let option_steps = &episode.steps[candidate_prefix_ticks..];
    let mut observation = controller_observation_from_facts(before)?;
    let mut stepper_end = None;
    for (index, native_step) in option_steps.iter().enumerate() {
        let realized = stepper
            .step(&observation)
            .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?;
        if matches!(realized.end, Some(ControllerRuntimeEnd::TargetLost { .. })) {
            return Err(NativeTacticWorkerError::DetachedResult(
                "native controller lost an undeclared target",
            ));
        }
        let frame = realized.frame.ok_or_else(|| {
            NativeTacticWorkerError::Observation(
                "reactive controller returned neither PAD nor stopping condition".into(),
            )
        })?;
        if !same_pad(native_step.chosen_pad, frame.pads[0])
            || !same_pad(native_step.consumed_pad, frame.pads[0])
        {
            return Err(NativeTacticWorkerError::PadMismatch);
        }
        if realized.end.is_some() && index + 1 != option_steps.len() {
            return Err(NativeTacticWorkerError::DetachedResult(
                "native controller continued after the tactic stopped",
            ));
        }
        stepper_end = realized.end;
        option_tape.frames.push(frame);
        queries.push(realized.query);
        if index + 1 != option_steps.len() {
            observation =
                controller_observation_from_post_simulation(&native_step.post_simulation)?;
        }
    }

    let end_reason = if episode.success {
        let condition_index = u32::try_from(cancellation.len())
            .map_err(|_| NativeTacticWorkerError::InvalidDuration)?;
        cancellation.push(OptionCondition::TargetReached {
            target: "authored_goal".into(),
        });
        OptionEndReason::Cancelled { condition_index }
    } else {
        if !matches!(stepper_end, Some(ControllerRuntimeEnd::MaximumDuration)) {
            return Err(NativeTacticWorkerError::DetachedResult(
                "native controller stopped before its bounded tactic",
            ));
        }
        OptionEndReason::Completed
    };
    let execution = capture_local_execution(
        selected,
        duration,
        termination,
        cancellation,
        end_reason,
        &option_tape,
    )?;
    let mut candidate_tape = InputTape {
        boot: route_prefix.boot.clone(),
        tick_rate_numerator: route_prefix.tick_rate_numerator,
        tick_rate_denominator: route_prefix.tick_rate_denominator,
        frames: prefix_frames,
    };
    candidate_tape.frames.extend_from_slice(&option_tape.frames);
    observe_outcome(
        root_checkpoint_sha256,
        selected,
        before,
        route_prefix,
        PreparedNativeTactic {
            option_tape,
            execution,
            duration,
        },
        candidate_tape,
        candidate_prefix_ticks,
        request,
        validated,
        queries
            .into_iter()
            .map(TacticRuntimeQuery::ReactiveController)
            .collect(),
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_reactive_controller<W: PersistentTacticBatchWorker>(
    worker: &mut W,
    root_checkpoint_sha256: Digest,
    selected: &SelectedTactic,
    before: &FactSnapshot,
    route_prefix: &InputTape,
    paths: &NativeTacticWorkerPaths,
    source_frame: usize,
    candidate_prefix_ticks: usize,
    checkpoint_source: Option<&NativeTacticCheckpointSource>,
    retain_candidate_checkpoint: bool,
    mut stepper: ControllerProgramStepper,
    duration: TacticDurationBounds,
    termination: OptionCondition,
    cancellation: Vec<OptionCondition>,
) -> Result<NativeTacticWorkerOutcome, NativeTacticWorkerError> {
    let mut observation = controller_observation_from_facts(before)?;
    let mut option_tape = InputTape {
        boot: route_prefix.boot.clone(),
        tick_rate_numerator: route_prefix.tick_rate_numerator,
        tick_rate_denominator: route_prefix.tick_rate_denominator,
        frames: Vec::new(),
    };
    let mut queries = Vec::new();
    let mut last_run: Option<(InputTape, NativeSuffixBatch, ValidatedNativeSuffixBatch)> = None;

    for local_tick in 0..duration.maximum_ticks {
        let step = stepper
            .step(&observation)
            .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?;
        queries.push(step.query);
        if matches!(step.end, Some(ControllerRuntimeEnd::TargetLost { .. })) {
            let Some((candidate_tape, request, validated)) = last_run else {
                return Err(NativeTacticWorkerError::Observation(
                    "reactive controller lost its exact target before emitting any input".into(),
                ));
            };
            if cancellation.is_empty() {
                return Err(NativeTacticWorkerError::DetachedSelection);
            }
            let local_execution = capture_local_execution(
                selected,
                duration,
                termination.clone(),
                cancellation.clone(),
                OptionEndReason::Cancelled { condition_index: 0 },
                &option_tape,
            )?;
            return observe_outcome(
                root_checkpoint_sha256,
                selected,
                before,
                route_prefix,
                PreparedNativeTactic {
                    option_tape,
                    execution: local_execution,
                    duration,
                },
                candidate_tape,
                candidate_prefix_ticks,
                request,
                validated,
                queries
                    .into_iter()
                    .map(TacticRuntimeQuery::ReactiveController)
                    .collect(),
            );
        }

        let frame = step.frame.ok_or_else(|| {
            NativeTacticWorkerError::Observation(
                "reactive controller returned neither PAD nor stopping condition".into(),
            )
        })?;
        option_tape.frames.push(frame);
        let mut candidate_tape = InputTape {
            boot: route_prefix.boot.clone(),
            tick_rate_numerator: route_prefix.tick_rate_numerator,
            tick_rate_denominator: route_prefix.tick_rate_denominator,
            frames: candidate_prefix_frames(route_prefix, source_frame, checkpoint_source),
        };
        candidate_tape.frames.extend_from_slice(&option_tape.frames);
        let request = tactic_batch(
            worker.identity(),
            selected,
            &candidate_tape,
            checkpoint_source,
            retain_candidate_checkpoint,
        )?;
        let iteration_paths = iteration_paths(paths, selected.decision_index, local_tick);
        write_new_json(&iteration_paths.request, &request)?;
        let validated =
            worker.run_tactic_batch(&iteration_paths.request, &iteration_paths.result)?;
        let episode = inspect_candidate_episode(&request, &validated, &candidate_tape)?;
        if episode.steps.len() <= candidate_prefix_ticks {
            return Err(NativeTacticWorkerError::DetachedResult(
                "route prefix terminated before the selected tactic",
            ));
        }
        let realized_ticks = episode.steps.len() - candidate_prefix_ticks;
        if realized_ticks != option_tape.frames.len() {
            return Err(NativeTacticWorkerError::DetachedResult(
                "reactive controller replay diverged",
            ));
        }

        let controller_complete = matches!(step.end, Some(ControllerRuntimeEnd::MaximumDuration));
        if episode.success || controller_complete {
            let mut final_cancellation = cancellation.clone();
            let end_reason = if episode.success && !controller_complete {
                let condition_index = u32::try_from(final_cancellation.len())
                    .map_err(|_| NativeTacticWorkerError::InvalidDuration)?;
                final_cancellation.push(OptionCondition::TargetReached {
                    target: "authored_goal".into(),
                });
                OptionEndReason::Cancelled { condition_index }
            } else {
                OptionEndReason::Completed
            };
            let local_execution = capture_local_execution(
                selected,
                duration,
                termination.clone(),
                final_cancellation,
                end_reason,
                &option_tape,
            )?;
            return observe_outcome(
                root_checkpoint_sha256,
                selected,
                before,
                route_prefix,
                PreparedNativeTactic {
                    option_tape,
                    execution: local_execution,
                    duration,
                },
                candidate_tape,
                candidate_prefix_ticks,
                request,
                validated,
                queries
                    .into_iter()
                    .map(TacticRuntimeQuery::ReactiveController)
                    .collect(),
            );
        }

        let last = episode
            .steps
            .last()
            .ok_or(NativeTacticWorkerError::DetachedResult("empty episode"))?;
        observation = controller_observation_from_post_simulation(&last.post_simulation)?;
        last_run = Some((candidate_tape, request, validated));
    }
    Err(NativeTacticWorkerError::InvalidDuration)
}

fn capture_local_execution(
    selected: &SelectedTactic,
    duration: TacticDurationBounds,
    termination: OptionCondition,
    cancellation: Vec<OptionCondition>,
    end_reason: OptionEndReason,
    option_tape: &InputTape,
) -> Result<OptionExecution, NativeTacticWorkerError> {
    OptionExecution::capture(
        selected.descriptor.option_id.clone(),
        selected.descriptor.option_type.clone(),
        selected.descriptor.parameters.clone(),
        duration.minimum_ticks,
        duration.maximum_ticks,
        termination,
        cancellation,
        end_reason,
        option_tape,
        TapeRange {
            start_frame: 0,
            end_frame_exclusive: option_tape.frames.len() as u64,
        },
    )
    .map_err(|error| NativeTacticWorkerError::Execution(error.to_string()))
}

fn controller_observation_from_facts(
    facts: &FactSnapshot,
) -> Result<ControllerRuntimeObservation, NativeTacticWorkerError> {
    facts
        .validate()
        .map_err(|error| NativeTacticWorkerError::Facts(error.to_string()))?;
    let offset = u64::from(facts.phase == FactPhase::PostSimulation);
    let simulation_tick = facts.simulation_tick.checked_add(offset).ok_or(
        NativeTacticWorkerError::DetachedResult("controller boundary"),
    )?;
    let tape_frame =
        facts
            .tape_frame
            .checked_add(offset)
            .ok_or(NativeTacticWorkerError::DetachedResult(
                "controller boundary",
            ))?;
    let player_yaw_radians = facts
        .player
        .current_angle
        .map(|angle| angle_to_radians(angle[1]));
    let player_velocity_xz = facts
        .player
        .velocity_f32_bits
        .map(|bits| [f32::from_bits(bits[0]), f32::from_bits(bits[2])]);
    let actors = facts
        .actors
        .iter()
        .map(|actor| ControllerRuntimeActor {
            actor_name: actor.actor_name,
            stable_id: actor.runtime_generation,
            set_id: actor.set_id,
            home_room: actor.home_room,
            position: actor.position_f32_bits.map(f32::from_bits),
        })
        .collect::<Vec<_>>();
    build_controller_observation(
        facts.boundary_index,
        simulation_tick,
        tape_frame,
        facts.state_identity,
        facts.player.present,
        facts.player.position_f32_bits.map(f32::from_bits),
        player_yaw_radians,
        player_velocity_xz,
        facts.player.camera_yaw_radians_f32_bits.map(f32::from_bits),
        facts.world.stage.clone(),
        facts.actors_complete,
        actors,
    )
}

fn controller_observation_from_post_simulation(
    value: &NativeLearningObservation,
) -> Result<ControllerRuntimeObservation, NativeTacticWorkerError> {
    if value.phase != NativeObservationPhase::PostSimulation {
        return Err(NativeTacticWorkerError::DetachedResult(
            "controller observation phase",
        ));
    }
    let simulation_tick =
        value
            .simulation_tick
            .checked_add(1)
            .ok_or(NativeTacticWorkerError::DetachedResult(
                "controller boundary",
            ))?;
    let tape_frame =
        value
            .tape_frame
            .checked_add(1)
            .ok_or(NativeTacticWorkerError::DetachedResult(
                "controller boundary",
            ))?;
    let actors = value
        .actors
        .iter()
        .map(|actor| ControllerRuntimeActor {
            actor_name: actor.actor_name,
            stable_id: actor.runtime_generation,
            set_id: actor.set_id,
            home_room: actor.home_room,
            position: actor.position,
        })
        .collect::<Vec<_>>();
    build_controller_observation(
        value.boundary_index,
        simulation_tick,
        tape_frame,
        value.state_identity,
        value.player_present,
        value.player_position,
        Some(angle_to_radians(value.player_current_angle[1])),
        Some([value.player_velocity[0], value.player_velocity[2]]),
        value.camera_yaw_radians,
        value.stage.clone(),
        !value.actors_truncated && value.actor_observed_count as usize == value.actors.len(),
        actors,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_controller_observation(
    boundary_index: u64,
    simulation_tick: u64,
    tape_frame: u64,
    state_identity: [u8; 16],
    player_present: bool,
    player_position: [f32; 3],
    player_yaw_radians: Option<f32>,
    player_velocity_xz: Option<[f32; 2]>,
    camera_yaw_radians: Option<f32>,
    stage: String,
    mut actors_complete: bool,
    mut actors: Vec<ControllerRuntimeActor>,
) -> Result<ControllerRuntimeObservation, NativeTacticWorkerError> {
    actors.sort_by_key(|actor| actor.stable_id);
    if actors.len() > MAX_CONTROLLER_RUNTIME_ACTORS {
        actors.truncate(MAX_CONTROLLER_RUNTIME_ACTORS);
        actors_complete = false;
    }
    let observation = ControllerRuntimeObservation {
        boundary_index,
        simulation_tick,
        tape_frame,
        state_identity,
        player_present,
        player_position,
        player_yaw_radians,
        player_velocity_xz,
        camera_yaw_radians,
        stage,
        actors_complete,
        actors,
    };
    observation
        .validate()
        .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?;
    Ok(observation)
}

fn angle_to_radians(angle: i16) -> f32 {
    f32::from(angle) * std::f32::consts::PI / 32_768.0
}

fn iteration_paths(
    paths: &NativeTacticWorkerPaths,
    decision_index: u64,
    local_tick: u32,
) -> NativeTacticWorkerPaths {
    let suffix = format!(".decision-{decision_index}.step-{local_tick:05}");
    let indexed = |path: &Path| {
        let mut name = path
            .file_name()
            .unwrap_or_else(|| path.as_os_str())
            .to_os_string();
        name.push(&suffix);
        path.with_file_name(name)
    };
    NativeTacticWorkerPaths {
        request: indexed(&paths.request),
        result: indexed(&paths.result),
    }
}

fn inspect_candidate_episode(
    request: &NativeSuffixBatch,
    validated: &ValidatedNativeSuffixBatch,
    candidate_tape: &InputTape,
) -> Result<NativeEpisode, NativeTacticWorkerError> {
    let episode = load_candidate_episode(request, validated)?;
    if episode.steps.len() > candidate_tape.frames.len() {
        return Err(NativeTacticWorkerError::DetachedResult("episode shard"));
    }
    for (step, expected) in episode.steps.iter().zip(&candidate_tape.frames) {
        if !same_pad(step.chosen_pad, expected.pads[0])
            || !same_pad(step.consumed_pad, expected.pads[0])
        {
            return Err(NativeTacticWorkerError::PadMismatch);
        }
    }
    Ok(episode)
}

fn load_candidate_episode(
    request: &NativeSuffixBatch,
    validated: &ValidatedNativeSuffixBatch,
) -> Result<NativeEpisode, NativeTacticWorkerError> {
    if validated.candidates.len() != 1
        || validated.candidates[0].id != request.candidates[0].id
        || validated.candidates[0].simulated_ticks == 0
        || validated.candidates[0].simulated_ticks > request.maximum_ticks as u64
    {
        return Err(NativeTacticWorkerError::DetachedResult("candidate summary"));
    }
    let bytes = fs::read(&validated.episode_shard_path)
        .map_err(|error| NativeTacticWorkerError::Io(error.to_string()))?;
    let shard = NativeEpisodeShard::decode(&bytes)
        .map_err(|error| NativeTacticWorkerError::Evidence(error.to_string()))?;
    if shard.metadata.checkpoint_identity != validated.restore_identity {
        return Err(NativeTacticWorkerError::DetachedResult("episode shard"));
    }
    let mut episodes = shard
        .episodes
        .iter()
        .filter(|episode| episode.id == validated.candidates[0].id);
    let episode = episodes
        .next()
        .ok_or(NativeTacticWorkerError::DetachedResult("episode id"))?;
    if episodes.next().is_some()
        || episode.steps.len() as u64 != validated.candidates[0].simulated_ticks
    {
        return Err(NativeTacticWorkerError::DetachedResult("episode shard"));
    }
    Ok(episode.clone())
}

fn prepare_selected(
    selected: &SelectedTactic,
    catalog: &TacticAssetCatalog,
    blueprints: &[TacticBlueprint],
) -> Result<PreparedNativeExecution, NativeTacticWorkerError> {
    if let Some(entry) = catalog.entry(&selected.descriptor.option_id) {
        if entry.description().option != selected.descriptor {
            return Err(NativeTacticWorkerError::DetachedSelection);
        }
        return match catalog.prepare_execution(&selected.descriptor.option_id)? {
            PreparedTacticExecution::Static(realized) => {
                Ok(PreparedNativeExecution::Static(PreparedNativeTactic {
                    option_tape: realized.tape,
                    execution: realized.execution,
                    duration: entry.description().duration,
                }))
            }
            PreparedTacticExecution::NativeGeneric(candidate) => {
                Ok(PreparedNativeExecution::NativeGeneric {
                    stepper: NativeGenericTacticStepper::new(candidate.plan().clone())
                        .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?,
                    duration: entry.description().duration,
                    termination: entry.description().stopping.termination.clone(),
                })
            }
            PreparedTacticExecution::ReactiveController(program) => {
                Ok(PreparedNativeExecution::ReactiveController {
                    program: program.clone(),
                    stepper: ControllerProgramStepper::new(program.clone())
                        .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?,
                    duration: entry.description().duration,
                    termination: entry.description().stopping.termination.clone(),
                    cancellation: entry.description().stopping.cancellation.clone(),
                })
            }
        };
    }

    let asset_id = selected
        .descriptor
        .option_id
        .strip_prefix("blueprint/")
        .ok_or(NativeTacticWorkerError::DetachedSelection)?;
    let blueprint = blueprints
        .iter()
        .find(|blueprint| blueprint.asset_id == asset_id)
        .ok_or(NativeTacticWorkerError::DetachedSelection)?;
    let compiled = blueprint.compile_static(catalog)?;
    let choices = ApplicableTacticChoices::enumerate(
        catalog,
        std::slice::from_ref(blueprint),
        |_| true,
        |_| Some(false),
    )?;
    let choice = choices
        .candidates
        .iter()
        .find(|choice| choice.descriptor == selected.descriptor)
        .ok_or(NativeTacticWorkerError::DetachedSelection)?;
    let execution = OptionExecution::capture(
        selected.descriptor.option_id.clone(),
        selected.descriptor.option_type.clone(),
        selected.descriptor.parameters.clone(),
        choice.duration.minimum_ticks,
        choice.duration.maximum_ticks,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &compiled.tape,
        TapeRange {
            start_frame: 0,
            end_frame_exclusive: compiled.tape.frames.len() as u64,
        },
    )
    .map_err(|error| NativeTacticWorkerError::Execution(error.to_string()))?;
    Ok(PreparedNativeExecution::Static(PreparedNativeTactic {
        option_tape: compiled.tape,
        execution,
        duration: choice.duration,
    }))
}

fn tactic_batch(
    identity: &NativeSuffixWorkerIdentity,
    selected: &SelectedTactic,
    tape: &InputTape,
    checkpoint_source: Option<&NativeTacticCheckpointSource>,
    retain_candidate_checkpoint: bool,
) -> Result<NativeSuffixBatch, NativeTacticWorkerError> {
    if tape.frames.is_empty() || tape.frames.len() > 4_096 {
        return Err(NativeTacticWorkerError::InvalidDuration);
    }
    let actions = pad_runs(&tape.frames)?;
    let id = hex_digest(
        serde_json::to_vec(&(
            selected.learner_snapshot_sha256,
            selected.decision_index,
            &selected.descriptor,
        ))
        .map_err(|error| NativeTacticWorkerError::Serialization(error.to_string()))?,
    );
    Ok(NativeSuffixBatch {
        schema: NATIVE_CACHED_SUFFIX_BATCH_SCHEMA.into(),
        source_frame: usize::try_from(identity.source_frame)
            .map_err(|_| NativeTacticWorkerError::InvalidDuration)?,
        source_boundary_fingerprint: checkpoint_source.map_or_else(
            || identity.source_boundary_fingerprint.clone(),
            |source| source.boundary_fingerprint.clone(),
        ),
        checkpoint_validation: NativeCheckpointValidation {
            kind: identity.checkpoint_validation_kind.clone(),
            ticks: usize::try_from(identity.checkpoint_validation_ticks)
                .map_err(|_| NativeTacticWorkerError::InvalidDuration)?,
        },
        maximum_ticks: tape.frames.len(),
        // These batches collect learning transitions, not promotion proof.
        // The persistent session already authenticates its source checkpoint
        // with a recorded replay window, while per-tick state hashing makes
        // failed exploration branches dramatically more expensive than the
        // simulation itself. Successful policies are state-hash verified by
        // the separate frozen-policy and cold-replay gates.
        verify_state_hashes: false,
        checkpoint_cache: Some(tactic_checkpoint_cache_request(
            checkpoint_source,
            retain_candidate_checkpoint,
        )),
        candidates: vec![NativeSuffixCandidate {
            id,
            actions,
            controller_program_hex: None,
        }],
    })
}

fn tactic_controller_batch(
    identity: &NativeSuffixWorkerIdentity,
    selected: &SelectedTactic,
    prefix_frames: &[InputFrame],
    program: &ControllerProgram,
    checkpoint_source: Option<&NativeTacticCheckpointSource>,
    retain_candidate_checkpoint: bool,
) -> Result<NativeSuffixBatch, NativeTacticWorkerError> {
    let program_bytes = program
        .encode_compatible()
        .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?;
    let maximum_ticks = prefix_frames
        .len()
        .checked_add(program.duration_frames as usize)
        .ok_or(NativeTacticWorkerError::InvalidDuration)?;
    if maximum_ticks == 0 || maximum_ticks > 4_096 {
        return Err(NativeTacticWorkerError::InvalidDuration);
    }
    let id = hex_digest(
        serde_json::to_vec(&(
            selected.learner_snapshot_sha256,
            selected.decision_index,
            &selected.descriptor,
        ))
        .map_err(|error| NativeTacticWorkerError::Serialization(error.to_string()))?,
    );
    Ok(NativeSuffixBatch {
        schema: NATIVE_CACHED_SUFFIX_BATCH_SCHEMA.into(),
        source_frame: usize::try_from(identity.source_frame)
            .map_err(|_| NativeTacticWorkerError::InvalidDuration)?,
        source_boundary_fingerprint: checkpoint_source.map_or_else(
            || identity.source_boundary_fingerprint.clone(),
            |source| source.boundary_fingerprint.clone(),
        ),
        checkpoint_validation: NativeCheckpointValidation {
            kind: identity.checkpoint_validation_kind.clone(),
            ticks: usize::try_from(identity.checkpoint_validation_ticks)
                .map_err(|_| NativeTacticWorkerError::InvalidDuration)?,
        },
        maximum_ticks,
        // Keep reactive exploration on the same evidence boundary as static
        // tactic exploration. Exact per-tick proof belongs to promotion.
        verify_state_hashes: false,
        checkpoint_cache: Some(tactic_checkpoint_cache_request(
            checkpoint_source,
            retain_candidate_checkpoint,
        )),
        candidates: vec![NativeSuffixCandidate {
            id,
            actions: pad_runs(prefix_frames)?,
            controller_program_hex: Some(lower_hex_bytes(&program_bytes)),
        }],
    })
}

fn tactic_checkpoint_cache_request(
    checkpoint_source: Option<&NativeTacticCheckpointSource>,
    retain_candidate_checkpoint: bool,
) -> NativeCheckpointCacheRequest {
    NativeCheckpointCacheRequest {
        capacity_bytes: TACTIC_CHECKPOINT_CACHE_BYTES,
        capacity_entries: TACTIC_CHECKPOINT_CACHE_ENTRIES,
        source_identity: checkpoint_source.map(|source| source.restore_identity.clone()),
        source_route_ticks: checkpoint_source.map_or(0, |source| source.route_ticks),
        retain_candidate_checkpoints: retain_candidate_checkpoint,
    }
}

fn candidate_prefix_frames(
    route_prefix: &InputTape,
    source_frame: usize,
    checkpoint_source: Option<&NativeTacticCheckpointSource>,
) -> Vec<InputFrame> {
    if checkpoint_source.is_some() {
        Vec::new()
    } else {
        route_prefix.frames[source_frame..].to_vec()
    }
}

fn lower_hex_identity(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn lower_hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn pad_runs(frames: &[InputFrame]) -> Result<Vec<MacroAction>, NativeTacticWorkerError> {
    let mut runs: Vec<(SearchPadState, u32)> = Vec::new();
    for frame in frames {
        if frame.owned_ports & 1 == 0
            || frame.wait_condition != WaitCondition::None
            || frame.wait_timeout_ticks != 0
        {
            return Err(NativeTacticWorkerError::ReactiveFrame);
        }
        let pad = SearchPadState::from(frame.pads[0]);
        if let Some((previous, count)) = runs.last_mut()
            && *previous == pad
        {
            *count = count
                .checked_add(1)
                .ok_or(NativeTacticWorkerError::InvalidDuration)?;
        } else {
            runs.push((pad, 1));
        }
    }
    Ok(runs
        .into_iter()
        .map(|(pad, frames)| MacroAction::PadRun { pad, frames })
        .collect())
}

fn observe_outcome(
    root_checkpoint_sha256: Digest,
    selected: &SelectedTactic,
    before: &FactSnapshot,
    route_prefix: &InputTape,
    prepared: PreparedNativeTactic,
    candidate_tape: InputTape,
    candidate_prefix_ticks: usize,
    request: NativeSuffixBatch,
    validated: ValidatedNativeSuffixBatch,
    native_queries: Vec<TacticRuntimeQuery>,
) -> Result<NativeTacticWorkerOutcome, NativeTacticWorkerError> {
    if validated.candidates.len() != 1
        || validated.candidates[0].id != request.candidates[0].id
        || validated.candidates[0].simulated_ticks == 0
        || validated.candidates[0].simulated_ticks > request.maximum_ticks as u64
    {
        return Err(NativeTacticWorkerError::DetachedResult("candidate summary"));
    }
    let bytes = fs::read(&validated.episode_shard_path)
        .map_err(|error| NativeTacticWorkerError::Io(error.to_string()))?;
    let shard = NativeEpisodeShard::decode(&bytes)
        .map_err(|error| NativeTacticWorkerError::Evidence(error.to_string()))?;
    if shard.metadata.checkpoint_identity != validated.restore_identity {
        return Err(NativeTacticWorkerError::DetachedResult("episode shard"));
    }
    let mut episodes = shard
        .episodes
        .iter()
        .filter(|episode| episode.id == validated.candidates[0].id);
    let episode = episodes
        .next()
        .ok_or(NativeTacticWorkerError::DetachedResult("episode id"))?;
    if episodes.next().is_some()
        || episode.steps.len() as u64 != validated.candidates[0].simulated_ticks
    {
        return Err(NativeTacticWorkerError::DetachedResult("episode shard"));
    }
    if episode.steps.len() <= candidate_prefix_ticks {
        return Err(NativeTacticWorkerError::DetachedResult(
            "route prefix terminated before the selected tactic",
        ));
    }
    let realized_ticks = episode.steps.len() - candidate_prefix_ticks;
    for (step, expected) in episode
        .steps
        .iter()
        .zip(&candidate_tape.frames[..episode.steps.len()])
    {
        if !same_pad(step.chosen_pad, expected.pads[0])
            || !same_pad(step.consumed_pad, expected.pads[0])
        {
            return Err(NativeTacticWorkerError::PadMismatch);
        }
    }

    let mut route_tape = route_prefix.clone();
    route_tape
        .frames
        .extend_from_slice(&prepared.option_tape.frames[..realized_ticks]);
    let start_frame = route_prefix.frames.len() as u64;
    let end_frame_exclusive = route_tape.frames.len() as u64;
    let terminal = episode.success;
    if realized_ticks < prepared.option_tape.frames.len() && !terminal {
        return Err(NativeTacticWorkerError::DetachedResult("early stop"));
    }
    let (end_reason, cancellation_conditions) =
        realized_option_end(&prepared, realized_ticks, terminal)?;
    let termination_condition = prepared.execution.termination_condition.clone();
    let duration = prepared.duration;
    let option_id = selected.descriptor.option_id.clone();
    let option_type = selected.descriptor.option_type.clone();
    let parameters = selected.descriptor.parameters.clone();
    let execution = OptionExecution::capture(
        option_id,
        option_type,
        parameters,
        duration.minimum_ticks,
        duration.maximum_ticks,
        termination_condition,
        cancellation_conditions,
        end_reason,
        &route_tape,
        TapeRange {
            start_frame,
            end_frame_exclusive,
        },
    )
    .map_err(|error| NativeTacticWorkerError::Execution(error.to_string()))?;
    let last = episode
        .steps
        .last()
        .ok_or(NativeTacticWorkerError::DetachedResult("empty episode"))?;
    let prior = episode
        .steps
        .iter()
        .rev()
        .take(dusklight_learning::fact_snapshot::MAX_FACT_HISTORY)
        .map(|step| step.pre_input.clone())
        .collect::<Vec<_>>();
    let mut prior = prior.into_iter().rev().collect::<Vec<_>>();
    prior.retain(|observation| observation.boundary_index < last.post_simulation.boundary_index);
    // The post-simulation row owns the next boundary's state identity, but its
    // simulation/tape coordinates still name the input that produced it.
    // Project it onto the immediately following pre-input boundary so another
    // tactic can extend the route without an off-by-one state/tape mismatch.
    let mut next_boundary = last.post_simulation.clone();
    next_boundary.phase = NativeObservationPhase::PreInput;
    next_boundary.simulation_tick = next_boundary
        .simulation_tick
        .checked_add(1)
        .ok_or(NativeTacticWorkerError::DetachedResult("next boundary"))?;
    next_boundary.tape_frame = next_boundary
        .tape_frame
        .checked_add(1)
        .ok_or(NativeTacticWorkerError::DetachedResult("next boundary"))?;
    let next_facts =
        FactSnapshot::from_native_learning(&next_boundary, &prior, Some(&execution), Vec::new())
            .map_err(|error| NativeTacticWorkerError::Facts(error.to_string()))?;
    if next_facts.tape_frame != end_frame_exclusive
        || next_facts.simulation_tick
            != before.simulation_tick + u64::try_from(realized_ticks).unwrap()
        || next_facts.terminal.reached != Some(terminal)
    {
        return Err(NativeTacticWorkerError::DetachedResult("next boundary"));
    }
    let retained_native_checkpoint = validated.candidates[0].retained_checkpoint.clone();
    let retained_route_ticks = request
        .checkpoint_cache
        .as_ref()
        .map_or(0, |cache| cache.source_route_ticks)
        .checked_add(episode.steps.len())
        .ok_or(NativeTacticWorkerError::InvalidDuration)?;
    if retained_native_checkpoint
        .as_ref()
        .is_some_and(|checkpoint| checkpoint.route_ticks != retained_route_ticks as u64)
    {
        return Err(NativeTacticWorkerError::DetachedResult(
            "retained checkpoint route boundary",
        ));
    }
    let retained_native_boundary_fingerprint = retained_native_checkpoint.as_ref().map(|_| {
        validated.candidates[0]
            .terminal_boundary_fingerprint
            .clone()
    });
    Ok(NativeTacticWorkerOutcome {
        schema: NATIVE_TACTIC_WORKER_OUTCOME_SCHEMA_V2.into(),
        source_checkpoint_sha256: root_checkpoint_sha256,
        checkpoint_identity: validated.restore_identity,
        retained_native_checkpoint,
        retained_native_boundary_fingerprint,
        episode_shard_sha256: shard.content_sha256,
        selected: selected.clone(),
        execution,
        native_queries,
        route_tape,
        next_facts,
        terminal,
    })
}

fn realized_option_end(
    prepared: &PreparedNativeTactic,
    realized_ticks: usize,
    terminal: bool,
) -> Result<(OptionEndReason, Vec<OptionCondition>), NativeTacticWorkerError> {
    if realized_ticks < prepared.option_tape.frames.len() {
        if !terminal {
            return Err(NativeTacticWorkerError::DetachedResult("early stop"));
        }
        let mut cancellation_conditions = prepared.execution.cancellation_conditions.clone();
        let condition_index = u32::try_from(cancellation_conditions.len())
            .map_err(|_| NativeTacticWorkerError::InvalidDuration)?;
        cancellation_conditions.push(OptionCondition::TargetReached {
            target: "authenticated_terminal".into(),
        });
        return Ok((
            OptionEndReason::Cancelled { condition_index },
            cancellation_conditions,
        ));
    }
    Ok((
        prepared.execution.end_reason,
        prepared.execution.cancellation_conditions.clone(),
    ))
}

fn same_pad(observed: NativeRawPad, expected: RawPadState) -> bool {
    observed.buttons == expected.buttons
        && observed.stick_x == expected.stick_x
        && observed.stick_y == expected.stick_y
        && observed.substick_x == expected.substick_x
        && observed.substick_y == expected.substick_y
        && observed.trigger_left == expected.trigger_left
        && observed.trigger_right == expected.trigger_right
        && observed.analog_a == expected.analog_a
        && observed.analog_b == expected.analog_b
        && observed.connected == expected.connected
        && observed.error == expected.error
}

fn write_new_json(path: &Path, value: &impl Serialize) -> Result<(), NativeTacticWorkerError> {
    let parent = path
        .parent()
        .ok_or_else(|| NativeTacticWorkerError::Io("request has no parent".into()))?;
    fs::create_dir_all(parent).map_err(|error| NativeTacticWorkerError::Io(error.to_string()))?;
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| NativeTacticWorkerError::Serialization(error.to_string()))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| NativeTacticWorkerError::Io(error.to_string()))?;
    file.write_all(&bytes)
        .map_err(|error| NativeTacticWorkerError::Io(error.to_string()))
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn hex_digest(bytes: Vec<u8>) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug)]
pub enum NativeTacticWorkerError {
    DetachedSelection,
    DetachedResult(&'static str),
    ObservationDriven(String),
    InvalidDuration,
    ReactiveFrame,
    PadMismatch,
    Facts(String),
    Tape(String),
    Execution(String),
    Observation(String),
    Evidence(String),
    Serialization(String),
    Io(String),
    Asset(dusklight_learning::tactic_asset::TacticAssetError),
    Blueprint(TacticBlueprintError),
    Worker(NativeSuffixWorkerError),
}

impl fmt::Display for NativeTacticWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DetachedSelection => {
                formatter.write_str("selected tactic is detached from its state or executor")
            }
            Self::DetachedResult(boundary) => {
                write!(
                    formatter,
                    "native tactic {boundary} is detached from its request"
                )
            }
            Self::ObservationDriven(id) => {
                write!(
                    formatter,
                    "tactic {id:?} requires the native observation-loop executor"
                )
            }
            Self::InvalidDuration => formatter.write_str("native tactic duration is invalid"),
            Self::ReactiveFrame => {
                formatter.write_str("native tactic batch contains a reactive or unowned frame")
            }
            Self::PadMismatch => formatter.write_str("native tactic PAD was not consumed exactly"),
            Self::Facts(message) => write!(formatter, "native tactic facts failed: {message}"),
            Self::Tape(message) => write!(formatter, "native tactic tape failed: {message}"),
            Self::Execution(message) => {
                write!(formatter, "native tactic execution failed: {message}")
            }
            Self::Observation(message) => {
                write!(
                    formatter,
                    "native tactic observation loop failed: {message}"
                )
            }
            Self::Evidence(message) => {
                write!(formatter, "native tactic evidence failed: {message}")
            }
            Self::Serialization(message) => {
                write!(formatter, "native tactic serialization failed: {message}")
            }
            Self::Io(message) => write!(formatter, "native tactic artifact I/O failed: {message}"),
            Self::Asset(error) => write!(formatter, "native tactic asset failed: {error}"),
            Self::Blueprint(error) => write!(formatter, "native tactic blueprint failed: {error}"),
            Self::Worker(error) => write!(formatter, "native tactic worker failed: {error}"),
        }
    }
}

impl Error for NativeTacticWorkerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Asset(error) => Some(error),
            Self::Blueprint(error) => Some(error),
            Self::Worker(error) => Some(error),
            _ => None,
        }
    }
}

impl From<dusklight_learning::tactic_asset::TacticAssetError> for NativeTacticWorkerError {
    fn from(value: dusklight_learning::tactic_asset::TacticAssetError) -> Self {
        Self::Asset(value)
    }
}

impl From<TacticBlueprintError> for NativeTacticWorkerError {
    fn from(value: TacticBlueprintError) -> Self {
        Self::Blueprint(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_suffix_result::ValidatedNativeSuffixCandidate;
    use dusklight_control::controller_program::ControllerProgram;
    use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
    use dusklight_control::option_execution::{OptionParameter, OptionType};
    use dusklight_learning::tactic_asset::{TacticAssetSource, TacticCatalogEntry};
    use dusklight_learning::tactic_exploration::TacticSelectionReason;
    use dusklight_learning::{
        native_generic_tactic::{
            GenericTactic, NATIVE_GENERIC_TACTIC_SCHEMA_V1, NativeGenericTacticPlan,
        },
        tactic_exploration::TACTIC_EXPLORATION_SCHEMA_V1,
    };
    use std::collections::BTreeMap;

    #[test]
    fn authenticated_terminal_can_cancel_a_static_tactic_before_its_minimum_duration() {
        let option_tape = InputTape {
            frames: vec![
                InputFrame {
                    owned_ports: 1,
                    ..InputFrame::default()
                };
                8
            ],
            ..InputTape::default()
        };
        let execution = OptionExecution::capture(
            "macro/example".into(),
            OptionType::Custom("macro".into()),
            BTreeMap::new(),
            8,
            8,
            OptionCondition::DurationElapsed,
            Vec::new(),
            OptionEndReason::Completed,
            &option_tape,
            TapeRange {
                start_frame: 0,
                end_frame_exclusive: 8,
            },
        )
        .unwrap();
        let prepared = PreparedNativeTactic {
            option_tape,
            execution,
            duration: TacticDurationBounds {
                minimum_ticks: 8,
                maximum_ticks: 8,
            },
        };

        let (end_reason, cancellation_conditions) =
            realized_option_end(&prepared, 6, true).unwrap();

        assert_eq!(
            end_reason,
            OptionEndReason::Cancelled { condition_index: 0 }
        );
        assert_eq!(
            cancellation_conditions,
            vec![OptionCondition::TargetReached {
                target: "authenticated_terminal".into(),
            }]
        );
        let realized_tape = InputTape {
            frames: prepared.option_tape.frames[..6].to_vec(),
            ..InputTape::default()
        };
        OptionExecution::capture(
            "macro/example".into(),
            OptionType::Custom("macro".into()),
            BTreeMap::new(),
            8,
            8,
            OptionCondition::DurationElapsed,
            cancellation_conditions,
            end_reason,
            &realized_tape,
            TapeRange {
                start_frame: 0,
                end_frame_exclusive: 6,
            },
        )
        .unwrap();
    }

    #[test]
    fn selected_static_tactic_becomes_one_exact_variable_horizon_batch() {
        let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new(
                "shield",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 3,
                })),
            )
            .unwrap(),
        ])
        .unwrap();
        let descriptor = catalog
            .entry("shield")
            .unwrap()
            .description()
            .option
            .clone();
        let selected = SelectedTactic {
            schema: dusklight_learning::tactic_exploration::TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: Digest([1; 32]),
            decision_index: 4,
            descriptor,
            reason: TacticSelectionReason::Greedy,
            exploration_draw: 900_000,
        };
        let PreparedNativeExecution::Static(prepared) =
            prepare_selected(&selected, &catalog, &[]).unwrap()
        else {
            panic!("shield must remain static");
        };
        let identity = NativeSuffixWorkerIdentity {
            executable_sha256: Digest([1; 32]),
            game_data_sha256: Digest([2; 32]),
            input_tape_sha256: Digest([3; 32]),
            milestone_program_sha256: Digest([4; 32]),
            card_fixture_sha256: Digest([5; 32]),
            world_context_sha256: Digest([6; 32]),
            source_frame: 440,
            source_boundary_fingerprint: "7".repeat(32),
            checkpoint_validation_kind: "recorded_replay_window".into(),
            checkpoint_validation_ticks: 2,
            maximum_ticks: 99,
            terminal: crate::native_suffix_result::NativeTerminalBinding {
                goal: "goal".into(),
                program_sha256: Digest([8; 32]),
                definition_sha256: Digest([9; 32]),
            },
        };
        let batch = tactic_batch(&identity, &selected, &prepared.option_tape, None, true).unwrap();
        assert_eq!(batch.maximum_ticks, 3);
        assert_eq!(batch.candidates.len(), 1);
        assert_eq!(
            batch.candidates[0]
                .actions
                .iter()
                .map(|action| match action {
                    MacroAction::PadRun { frames, .. } => *frames,
                    _ => 0,
                })
                .sum::<u32>(),
            3
        );
        assert!(!batch.verify_state_hashes);

        let checkpoint_source = NativeTacticCheckpointSource {
            restore_identity: "a".repeat(32),
            boundary_fingerprint: "b".repeat(32),
            route_ticks: 40,
        };
        let restored = tactic_batch(
            &identity,
            &selected,
            &prepared.option_tape,
            Some(&checkpoint_source),
            true,
        )
        .unwrap();
        let cache = restored
            .checkpoint_cache
            .expect("native tactic batches always declare their cache policy");
        assert_eq!(restored.schema, NATIVE_CACHED_SUFFIX_BATCH_SCHEMA);
        assert_eq!(restored.source_frame, 440);
        assert_eq!(restored.source_boundary_fingerprint, "b".repeat(32));
        assert_eq!(restored.maximum_ticks, 3);
        assert_eq!(
            cache.source_identity.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(cache.source_route_ticks, 40);
        assert!(cache.retain_candidate_checkpoints);

        let evaluation_only = tactic_batch(
            &identity,
            &selected,
            &prepared.option_tape,
            Some(&checkpoint_source),
            false,
        )
        .unwrap();
        assert!(
            !evaluation_only
                .checkpoint_cache
                .expect("evaluation batches still declare their cache policy")
                .retain_candidate_checkpoints
        );
        assert!(
            candidate_prefix_frames(
                &InputTape {
                    frames: vec![InputFrame::default(); 443],
                    ..InputTape::default()
                },
                440,
                Some(&checkpoint_source),
            )
            .is_empty()
        );
    }

    #[test]
    fn relative_heading_becomes_one_linear_native_controller_candidate() {
        let authored_heading = 0.375_f32;
        let plan = NativeGenericTacticPlan {
            schema: NATIVE_GENERIC_TACTIC_SCHEMA_V1.into(),
            tactic: GenericTactic::MaintainRelativeHeading {
                heading_radians_f32_bits: authored_heading.to_bits(),
                magnitude: 96,
            },
            minimum_ticks: 1,
            maximum_ticks: 16,
        };
        let duration = TacticDurationBounds {
            minimum_ticks: 1,
            maximum_ticks: 16,
        };
        let program = native_generic_controller_program(&plan, duration)
            .unwrap()
            .expect("relative heading has a native controller equivalent");
        let Operation::MaintainHeading {
            frame,
            heading_radians,
            magnitude,
            ..
        } = program.layers[0].operation
        else {
            panic!("relative heading must compile to maintain-heading");
        };
        assert_eq!(frame, CoordinateFrame::Player);
        assert_eq!(heading_radians.to_bits(), (-authored_heading).to_bits());
        assert_eq!(magnitude, 96);

        let endpoint_plan = NativeGenericTacticPlan {
            tactic: GenericTactic::MaintainRelativeHeading {
                heading_radians_f32_bits: (-std::f32::consts::PI).to_bits(),
                magnitude: 96,
            },
            ..plan.clone()
        };
        let endpoint_program = native_generic_controller_program(&endpoint_plan, duration)
            .unwrap()
            .expect("the -pi catalog endpoint must remain encodable");
        let Operation::MaintainHeading {
            heading_radians, ..
        } = endpoint_program.layers[0].operation
        else {
            panic!("relative heading must compile to maintain-heading");
        };
        assert_eq!(heading_radians.to_bits(), (-std::f32::consts::PI).to_bits());

        let descriptor = plan.descriptor("heading".into()).unwrap();
        let selected = SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: Digest([1; 32]),
            decision_index: 5,
            descriptor,
            reason: TacticSelectionReason::Epsilon,
            exploration_draw: 1,
        };
        let identity = NativeSuffixWorkerIdentity {
            executable_sha256: Digest([1; 32]),
            game_data_sha256: Digest([2; 32]),
            input_tape_sha256: Digest([3; 32]),
            milestone_program_sha256: Digest([4; 32]),
            card_fixture_sha256: Digest([5; 32]),
            world_context_sha256: Digest([6; 32]),
            source_frame: 440,
            source_boundary_fingerprint: "7".repeat(32),
            checkpoint_validation_kind: "recorded_replay_window".into(),
            checkpoint_validation_ticks: 2,
            maximum_ticks: 99,
            terminal: crate::native_suffix_result::NativeTerminalBinding {
                goal: "goal".into(),
                program_sha256: Digest([8; 32]),
                definition_sha256: Digest([9; 32]),
            },
        };
        let prefix = vec![
            InputFrame {
                owned_ports: 1,
                ..InputFrame::default()
            };
            3
        ];
        let batch =
            tactic_controller_batch(&identity, &selected, &prefix, &program, None, true).unwrap();

        assert_eq!(batch.schema, NATIVE_CACHED_SUFFIX_BATCH_SCHEMA);
        assert_eq!(batch.maximum_ticks, 19);
        assert_eq!(batch.candidates.len(), 1);
        assert_eq!(
            batch.candidates[0]
                .actions
                .iter()
                .map(|action| match action {
                    MacroAction::PadRun { frames, .. } => *frames,
                    _ => 0,
                })
                .sum::<u32>(),
            3
        );
        assert!(batch.candidates[0].controller_program_hex.is_some());
    }

    #[test]
    fn seek_coordinate_becomes_one_linear_native_controller_candidate() {
        let target = [-1842.25_f32, 717.0, -4739.5];
        let plan = NativeGenericTacticPlan::new(
            GenericTactic::SeekCoordinate {
                coordinate_f32_bits: target.map(f32::to_bits),
                tolerance_f32_bits: 24.0_f32.to_bits(),
                magnitude: 127,
            },
            80,
        );
        let program = native_generic_controller_program(
            &plan,
            TacticDurationBounds {
                minimum_ticks: 1,
                maximum_ticks: 80,
            },
        )
        .unwrap()
        .expect("seek-coordinate has a native controller equivalent");
        let Operation::SeekCoordinate {
            frame,
            target: actual,
            offset,
            stop_radius,
            magnitude,
            ..
        } = program.layers[0].operation
        else {
            panic!("seek coordinate must compile to the same controller operation");
        };
        assert_eq!(frame, CoordinateFrame::World);
        assert_eq!(actual, target);
        assert_eq!(offset, [0.0; 3]);
        assert_eq!(stop_radius.to_bits(), 24.0_f32.to_bits());
        assert_eq!(magnitude, 127);
        assert_eq!(program.duration_frames, 80);
    }

    #[test]
    fn seek_coordinate_sequence_becomes_one_linear_native_controller_candidate() {
        let coordinates = [
            [-2501.3477_f32, 717.0, -3931.8281],
            [-2534.9966, 717.0, -4164.2246],
            [-2568.6455, 717.0, -4396.6206],
            [-1842.2203, 717.0, -4739.0684],
        ];
        let plan = NativeGenericTacticPlan::new(
            GenericTactic::SeekCoordinateSequence {
                coordinates_f32_bits: coordinates
                    .map(|coordinate| coordinate.map(f32::to_bits))
                    .to_vec(),
                intermediate_tolerance_f32_bits: 96.0_f32.to_bits(),
                final_tolerance_f32_bits: 32.0_f32.to_bits(),
                magnitude: 127,
                stall_grace_ticks: 40,
                stationary_window_ticks: 16,
                stationary_window_distance_f32_bits: 1.0_f32.to_bits(),
            },
            40,
        );
        let program = native_generic_controller_program(
            &plan,
            TacticDurationBounds {
                minimum_ticks: 1,
                maximum_ticks: 40,
            },
        )
        .unwrap()
        .expect("a four-point coordinate sequence has a native controller equivalent");
        let Operation::SeekCoordinateSequence {
            coordinates_xz,
            intermediate_stop_radius,
            final_stop_radius,
            magnitude,
            ..
        } = &program.layers[0].operation
        else {
            panic!("coordinate sequence must compile to one sequence controller operation");
        };
        assert_eq!(
            coordinates_xz,
            &coordinates
                .iter()
                .map(|coordinate| [coordinate[0], coordinate[2]])
                .collect::<Vec<_>>()
        );
        assert_eq!(intermediate_stop_radius.to_bits(), 96.0_f32.to_bits());
        assert_eq!(final_stop_radius.to_bits(), 32.0_f32.to_bits());
        assert_eq!(*magnitude, 127);
        assert_eq!(program.duration_frames, 40);

        let mut early_stall_plan = plan.clone();
        let GenericTactic::SeekCoordinateSequence {
            stall_grace_ticks, ..
        } = &mut early_stall_plan.tactic
        else {
            unreachable!();
        };
        *stall_grace_ticks = 20;
        assert!(
            native_generic_controller_program(
                &early_stall_plan,
                TacticDurationBounds {
                    minimum_ticks: 1,
                    maximum_ticks: 40,
                },
            )
            .unwrap()
            .is_none(),
            "a sequence with an observable early-stall boundary must keep the Rust observation loop"
        );
        let delayed_stop_plan = NativeGenericTacticPlan {
            minimum_ticks: 2,
            ..plan.clone()
        };
        assert!(
            native_generic_controller_program(
                &delayed_stop_plan,
                TacticDurationBounds {
                    minimum_ticks: 2,
                    maximum_ticks: 40,
                },
            )
            .unwrap()
            .is_none(),
            "a sequence with a delayed stopping boundary must keep the Rust observation loop"
        );
        assert!(
            native_generic_controller_program_for_strategy(
                &plan,
                TacticDurationBounds {
                    minimum_ticks: 1,
                    maximum_ticks: 40,
                },
                NativeGenericExecutionStrategy::ProgressiveAudit,
            )
            .unwrap()
            .is_none(),
            "the audit strategy must force the progressive observation loop"
        );
    }

    #[test]
    fn selected_native_generic_tactic_dispatches_to_the_live_stepper() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let before = FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap();
        let plan = NativeGenericTacticPlan {
            schema: NATIVE_GENERIC_TACTIC_SCHEMA_V1.into(),
            tactic: GenericTactic::ShortCurve {
                control: [[37, -21]; 4],
            },
            minimum_ticks: 1,
            maximum_ticks: 1,
        };
        let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new("native.curve", TacticAssetSource::NativeGenericTactic(plan))
                .unwrap(),
        ])
        .unwrap();
        let selected = SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: before.content_sha256().unwrap(),
            decision_index: 2,
            descriptor: catalog
                .entry("native.curve")
                .unwrap()
                .description()
                .option
                .clone(),
            reason: TacticSelectionReason::Greedy,
            exploration_draw: 0,
        };

        let PreparedNativeExecution::NativeGeneric {
            mut stepper,
            duration,
            ..
        } = prepare_selected(&selected, &catalog, &[]).unwrap()
        else {
            panic!("native generic tactic must keep its observation loop");
        };
        let step = stepper
            .step(before.to_native_tactic_observation().unwrap())
            .unwrap();

        assert_eq!(duration.maximum_ticks, 1);
        assert_eq!(step.frame.pads[0].stick_x, 37);
        assert_eq!(step.frame.pads[0].stick_y, -21);
        assert_eq!(step.end_reason, Some(OptionEndReason::MaximumDuration));
        assert_eq!(step.query.local_tick, 0);
    }

    #[test]
    fn selected_reactive_controller_dispatches_to_the_observed_program_stepper() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let before = FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap();
        let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new(
                "controller.seek",
                TacticAssetSource::ReactiveController(
                    ControllerProgram::parse(
                        "duskcontrol 1\nframes 1\nseek coordinate replace from 0 for 1 frame world target 100 0 0 offset 0 0 0 magnitude 90 stop 1\n",
                    )
                    .unwrap(),
                ),
            )
            .unwrap(),
        ])
        .unwrap();
        let selected = SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: before.content_sha256().unwrap(),
            decision_index: 3,
            descriptor: catalog
                .entry("controller.seek")
                .unwrap()
                .description()
                .option
                .clone(),
            reason: TacticSelectionReason::Greedy,
            exploration_draw: 0,
        };
        let PreparedNativeExecution::ReactiveController {
            program,
            mut stepper,
            duration,
            ..
        } = prepare_selected(&selected, &catalog, &[]).unwrap()
        else {
            panic!("reactive controller must keep its observed program");
        };
        let step = stepper
            .step(&controller_observation_from_facts(&before).unwrap())
            .unwrap();

        assert_eq!(duration.maximum_ticks, 1);
        assert!(step.frame.is_some());
        assert_eq!(step.end, Some(ControllerRuntimeEnd::MaximumDuration));
        assert_eq!(step.query.controller_frame, 0);
        assert!(step.query.queried_fields.contains(
            &dusklight_control::controller_compilation::ControllerObservationField::PlayerPosition
        ));
        assert_eq!(program.duration_frames, 1);
        assert!(reactive_controller_uses_native_strategy(
            NativeGenericExecutionStrategy::NativeController,
            &[],
        ));
        assert!(!reactive_controller_uses_native_strategy(
            NativeGenericExecutionStrategy::ProgressiveAudit,
            &[],
        ));
        assert!(!reactive_controller_uses_native_strategy(
            NativeGenericExecutionStrategy::NativeController,
            &[OptionCondition::TargetLost {
                target: "controller_exact_actor".into(),
            }],
        ));
    }

    #[test]
    fn native_episode_observes_the_real_stop_and_next_fact_boundary() {
        let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../tests/fixtures/automation/native_episode_v28.dseps")
            .canonicalize()
            .unwrap();
        let bytes = fs::read(&fixture_path).unwrap();
        let shard = NativeEpisodeShard::decode(&bytes).unwrap();
        let episode = &shard.episodes[0];
        let step = &episode.steps[0];
        assert_eq!(step.chosen_pad, step.consumed_pad);

        let before =
            FactSnapshot::from_native_learning(&step.pre_input, &[], None, Vec::new()).unwrap();
        let selected = SelectedTactic {
            schema: dusklight_learning::tactic_exploration::TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: before.content_sha256().unwrap(),
            decision_index: 0,
            descriptor: dusklight_learning::option_values::OptionActionDescriptor {
                option_id: "fixture.tick".into(),
                option_type: OptionType::Neutral,
                parameters: BTreeMap::<String, OptionParameter>::new(),
            },
            reason: TacticSelectionReason::Greedy,
            exploration_draw: 999_999,
        };
        let mut frame = InputFrame {
            owned_ports: 1,
            ..InputFrame::default()
        };
        frame.pads[0] = RawPadState {
            buttons: step.chosen_pad.buttons,
            stick_x: step.chosen_pad.stick_x,
            stick_y: step.chosen_pad.stick_y,
            substick_x: step.chosen_pad.substick_x,
            substick_y: step.chosen_pad.substick_y,
            trigger_left: step.chosen_pad.trigger_left,
            trigger_right: step.chosen_pad.trigger_right,
            analog_a: step.chosen_pad.analog_a,
            analog_b: step.chosen_pad.analog_b,
            connected: step.chosen_pad.connected,
            error: step.chosen_pad.error,
        };
        let option_tape = InputTape {
            frames: vec![frame],
            ..InputTape::default()
        };
        let local_execution = OptionExecution::capture(
            selected.descriptor.option_id.clone(),
            selected.descriptor.option_type.clone(),
            selected.descriptor.parameters.clone(),
            1,
            1,
            OptionCondition::DurationElapsed,
            Vec::new(),
            OptionEndReason::Completed,
            &option_tape,
            TapeRange {
                start_frame: 0,
                end_frame_exclusive: 1,
            },
        )
        .unwrap();
        let prepared = PreparedNativeTactic {
            option_tape: option_tape.clone(),
            execution: local_execution,
            duration: TacticDurationBounds {
                minimum_ticks: 1,
                maximum_ticks: 1,
            },
        };
        let request = NativeSuffixBatch {
            schema: dusklight_search::suffix_batch::NATIVE_SUFFIX_BATCH_SCHEMA.into(),
            source_frame: before.tape_frame as usize,
            source_boundary_fingerprint: "a".repeat(32),
            checkpoint_validation: NativeCheckpointValidation {
                kind: "recorded_replay_window".into(),
                ticks: 1,
            },
            maximum_ticks: 1,
            verify_state_hashes: true,
            checkpoint_cache: None,
            candidates: vec![NativeSuffixCandidate {
                id: episode.id.clone(),
                actions: pad_runs(&option_tape.frames).unwrap(),
                controller_program_hex: None,
            }],
        };
        let validated = ValidatedNativeSuffixBatch {
            restore_identity: shard.metadata.checkpoint_identity.clone(),
            checkpoint_bytes: 1,
            simulated_ticks: 1,
            restore_micros: vec![1],
            checkpoint_cache: None,
            episode_shard_path: fixture_path.to_string_lossy().into_owned(),
            candidates: vec![ValidatedNativeSuffixCandidate {
                id: episode.id.clone(),
                simulated_ticks: 1,
                first_hit_tick: episode.first_hit_tick.map(u64::from),
                state_sequence_digest: Some("b".repeat(64)),
                terminal_boundary_fingerprint: "c".repeat(32),
                behavior_sha256: Digest([7; 32]),
                retained_checkpoint: None,
            }],
        };
        let route_prefix = InputTape {
            frames: vec![InputFrame::default(); before.tape_frame as usize],
            ..InputTape::default()
        };
        let outcome = observe_outcome(
            Digest([12; 32]),
            &selected,
            &before,
            &route_prefix,
            prepared,
            option_tape,
            0,
            request,
            validated,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(outcome.execution.duration.realized_ticks, 1);
        assert_eq!(
            outcome.execution.realized_tape_range,
            TapeRange {
                start_frame: before.tape_frame,
                end_frame_exclusive: before.tape_frame + 1,
            }
        );
        assert_eq!(outcome.next_facts.tape_frame, before.tape_frame + 1);
        assert_eq!(outcome.terminal, episode.success);
        assert_eq!(
            outcome.route_tape.frames.len(),
            before.tape_frame as usize + 1
        );
    }
}
