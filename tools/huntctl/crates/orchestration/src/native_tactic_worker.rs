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
use dusklight_learning::fact_snapshot::{
    FactPhase, FactSnapshot, MAX_FACT_HISTORY, OptionTrajectoryFactSnapshot,
    RecentOptionFactSnapshot,
};
use dusklight_learning::learner_state::tactic_intrinsically_applicable;
use dusklight_learning::native_generic_tactic::{
    GenericTactic, NativeGenericTacticStepper, NativeTacticObservation, NativeTacticQueryRecord,
};
use dusklight_learning::option_transition::OptionIntermediateBoundary;
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
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

mod controller_execution;

#[cfg(test)]
use controller_execution::{
    controller_observation_from_facts, native_generic_controller_program,
    native_generic_controller_program_for_strategy,
};
use controller_execution::{
    execute_native_generic_tactic, execute_reactive_controller, execute_reactive_controller_native,
    reactive_controller_uses_native_strategy,
};

pub const NATIVE_TACTIC_WORKER_OUTCOME_SCHEMA_V2: &str =
    "dusklight-native-tactic-worker-outcome/v2";
pub(crate) const TACTIC_CHECKPOINT_CACHE_BYTES: usize = 640 * 1024 * 1024;
pub(crate) const TACTIC_CHECKPOINT_CACHE_ENTRIES: usize = 2;
const TACTIC_INTERMEDIATE_BOUNDARY_STRIDE: usize = 4;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeGenericExecutionStrategy {
    NativeController,
    ProgressiveAudit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeTacticCheckpointRetention {
    None,
    PortableImage,
    LiveEndpoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeTacticCheckpointStorage {
    PortableImage,
    LiveEndpoint,
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
    pub storage: NativeTacticCheckpointStorage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedNativeTacticFrontier {
    pub source: NativeTacticCheckpointSource,
    pub episode_shard_sha256: Digest,
    pub observed_state_sha256: Digest,
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
    #[serde(skip_serializing)]
    pub intermediate_boundaries: Vec<OptionIntermediateBoundary>,
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

struct LoadedCandidateEpisode {
    shard_content_sha256: Digest,
    episode: NativeEpisode,
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
        checkpoint_cache: Some(tactic_checkpoint_cache_request(
            None,
            NativeTacticCheckpointRetention::PortableImage,
        )),
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
    let observed_state_sha256 = validate_materialized_frontier_state(expected, &shard.episodes[0])?;
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
            storage: NativeTacticCheckpointStorage::PortableImage,
        },
        episode_shard_sha256: shard.content_sha256,
        observed_state_sha256,
    })
}

fn validate_materialized_frontier_state(
    expected: &FactSnapshot,
    episode: &NativeEpisode,
) -> Result<Digest, NativeTacticWorkerError> {
    let last = episode
        .steps
        .last()
        .ok_or(NativeTacticWorkerError::DetachedResult(
            "frontier materialization state",
        ))?;
    let mut boundary = last.post_simulation.clone();
    boundary.phase = NativeObservationPhase::PreInput;
    boundary.simulation_tick =
        boundary
            .simulation_tick
            .checked_add(1)
            .ok_or(NativeTacticWorkerError::DetachedResult(
                "frontier materialization state",
            ))?;
    boundary.tape_frame =
        boundary
            .tape_frame
            .checked_add(1)
            .ok_or(NativeTacticWorkerError::DetachedResult(
                "frontier materialization state",
            ))?;
    let mut prior = episode
        .steps
        .iter()
        .rev()
        .take(expected.recent_history.len())
        .map(|step| step.pre_input.clone())
        .collect::<Vec<_>>();
    prior.reverse();
    prior.retain(|observation| observation.boundary_index < boundary.boundary_index);
    let mut restored =
        FactSnapshot::from_native_learning(&boundary, &prior, None, expected.conditions.clone())
            .map_err(|error| NativeTacticWorkerError::Facts(error.to_string()))?;
    // These fields are derived from the graph-authenticated selected option
    // and condition program rather than native restore state. The complete
    // native observation and its bounded history above are reconstructed from
    // the fresh replay before they are compared with the graph state.
    restored.recent_option = expected.recent_option.clone();
    restored
        .validate()
        .map_err(|error| NativeTacticWorkerError::Facts(error.to_string()))?;
    if &restored != expected {
        return Err(NativeTacticWorkerError::DetachedResult(
            "frontier materialization typed state",
        ));
    }
    restored
        .content_sha256()
        .map_err(|error| NativeTacticWorkerError::Facts(error.to_string()))
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
        if retain_candidate_checkpoint {
            NativeTacticCheckpointRetention::PortableImage
        } else {
            NativeTacticCheckpointRetention::None
        },
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
    checkpoint_retention: NativeTacticCheckpointRetention,
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
            checkpoint_retention,
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
            checkpoint_retention,
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
                    checkpoint_retention,
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
                    checkpoint_retention,
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
    checkpoint_retention: NativeTacticCheckpointRetention,
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
        checkpoint_retention,
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
        None,
        Vec::new(),
    )
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
) -> Result<LoadedCandidateEpisode, NativeTacticWorkerError> {
    let loaded = load_candidate_episode(request, validated)?;
    if loaded.episode.steps.len() > candidate_tape.frames.len() {
        return Err(NativeTacticWorkerError::DetachedResult("episode shard"));
    }
    for (step, expected) in loaded.episode.steps.iter().zip(&candidate_tape.frames) {
        if !same_pad(step.chosen_pad, expected.pads[0])
            || !same_pad(step.consumed_pad, expected.pads[0])
        {
            return Err(NativeTacticWorkerError::PadMismatch);
        }
    }
    Ok(loaded)
}

fn load_candidate_episode(
    request: &NativeSuffixBatch,
    validated: &ValidatedNativeSuffixBatch,
) -> Result<LoadedCandidateEpisode, NativeTacticWorkerError> {
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
    let shard_content_sha256 = shard.content_sha256;
    let mut episodes = shard
        .episodes
        .into_iter()
        .filter(|episode| episode.id == validated.candidates[0].id);
    let episode = episodes
        .next()
        .ok_or(NativeTacticWorkerError::DetachedResult("episode id"))?;
    if episodes.next().is_some()
        || episode.steps.len() as u64 != validated.candidates[0].simulated_ticks
    {
        return Err(NativeTacticWorkerError::DetachedResult("episode shard"));
    }
    Ok(LoadedCandidateEpisode {
        shard_content_sha256,
        episode,
    })
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
    checkpoint_retention: NativeTacticCheckpointRetention,
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
            checkpoint_retention,
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
    checkpoint_retention: NativeTacticCheckpointRetention,
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
            checkpoint_retention,
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
    checkpoint_retention: NativeTacticCheckpointRetention,
) -> NativeCheckpointCacheRequest {
    NativeCheckpointCacheRequest {
        capacity_bytes: TACTIC_CHECKPOINT_CACHE_BYTES,
        capacity_entries: TACTIC_CHECKPOINT_CACHE_ENTRIES,
        source_identity: checkpoint_source.map(|source| source.restore_identity.clone()),
        source_route_ticks: checkpoint_source.map_or(0, |source| source.route_ticks),
        retain_candidate_checkpoints: matches!(
            checkpoint_retention,
            NativeTacticCheckpointRetention::PortableImage
        ),
        retain_live_endpoint: matches!(
            checkpoint_retention,
            NativeTacticCheckpointRetention::LiveEndpoint
        ),
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

pub(crate) fn pad_runs(frames: &[InputFrame]) -> Result<Vec<MacroAction>, NativeTacticWorkerError> {
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
    loaded_episode: Option<LoadedCandidateEpisode>,
    native_queries: Vec<TacticRuntimeQuery>,
) -> Result<NativeTacticWorkerOutcome, NativeTacticWorkerError> {
    let loaded = loaded_episode.map_or_else(|| load_candidate_episode(&request, &validated), Ok)?;
    let episode = &loaded.episode;
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
    let option_trajectory =
        OptionTrajectoryFactSnapshot::from_native_steps(&episode.steps[candidate_prefix_ticks..])
            .map_err(|error| NativeTacticWorkerError::Facts(error.to_string()))?;
    let next_facts = FactSnapshot::from_native_learning_with_option_trajectory(
        &next_boundary,
        &prior,
        Some(&execution),
        Some(option_trajectory),
        Vec::new(),
    )
    .map_err(|error| NativeTacticWorkerError::Facts(error.to_string()))?;
    if next_facts.tape_frame != end_frame_exclusive
        || next_facts.simulation_tick
            != before.simulation_tick + u64::try_from(realized_ticks).unwrap()
        || next_facts.terminal.reached != Some(terminal)
    {
        return Err(NativeTacticWorkerError::DetachedResult("next boundary"));
    }
    let intermediate_boundaries = capture_intermediate_boundaries(
        selected,
        &execution,
        loaded.shard_content_sha256,
        episode,
        candidate_prefix_ticks,
    )?;
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
        episode_shard_sha256: loaded.shard_content_sha256,
        selected: selected.clone(),
        execution,
        native_queries,
        route_tape,
        next_facts,
        intermediate_boundaries,
        terminal,
    })
}

fn capture_intermediate_boundaries(
    selected: &SelectedTactic,
    execution: &OptionExecution,
    episode_shard_sha256: Digest,
    episode: &NativeEpisode,
    candidate_prefix_ticks: usize,
) -> Result<Vec<OptionIntermediateBoundary>, NativeTacticWorkerError> {
    let realized_ticks = usize::try_from(execution.duration.realized_ticks)
        .map_err(|_| NativeTacticWorkerError::InvalidDuration)?;
    if realized_ticks <= TACTIC_INTERMEDIATE_BOUNDARY_STRIDE {
        return Ok(Vec::new());
    }
    let option_steps = episode
        .steps
        .get(candidate_prefix_ticks..candidate_prefix_ticks.saturating_add(realized_ticks))
        .ok_or(NativeTacticWorkerError::DetachedResult(
            "intermediate option steps",
        ))?;
    let mut boundaries = Vec::with_capacity(
        realized_ticks
            .saturating_sub(1)
            .div_ceil(TACTIC_INTERMEDIATE_BOUNDARY_STRIDE),
    );
    for offset in (TACTIC_INTERMEDIATE_BOUNDARY_STRIDE..realized_ticks)
        .step_by(TACTIC_INTERMEDIATE_BOUNDARY_STRIDE)
    {
        let episode_end = candidate_prefix_ticks
            .checked_add(offset)
            .ok_or(NativeTacticWorkerError::InvalidDuration)?;
        let last = episode.steps.get(episode_end.saturating_sub(1)).ok_or(
            NativeTacticWorkerError::DetachedResult("intermediate native boundary"),
        )?;
        let mut next_boundary = last.post_simulation.clone();
        next_boundary.phase = NativeObservationPhase::PreInput;
        next_boundary.simulation_tick = next_boundary.simulation_tick.checked_add(1).ok_or(
            NativeTacticWorkerError::DetachedResult("intermediate simulation boundary"),
        )?;
        next_boundary.tape_frame = next_boundary.tape_frame.checked_add(1).ok_or(
            NativeTacticWorkerError::DetachedResult("intermediate tape boundary"),
        )?;
        let prior = episode.steps[..episode_end]
            .iter()
            .rev()
            .take(MAX_FACT_HISTORY)
            .map(|step| step.pre_input.clone())
            .collect::<Vec<_>>();
        let mut prior = prior.into_iter().rev().collect::<Vec<_>>();
        prior.retain(|observation| observation.boundary_index < next_boundary.boundary_index);
        let trajectory = OptionTrajectoryFactSnapshot::from_native_steps(&option_steps[..offset])
            .map_err(|error| NativeTacticWorkerError::Facts(error.to_string()))?;
        let mut state =
            FactSnapshot::from_native_learning(&next_boundary, &prior, None, Vec::new())
                .map_err(|error| NativeTacticWorkerError::Facts(error.to_string()))?;
        let offset_ticks =
            u32::try_from(offset).map_err(|_| NativeTacticWorkerError::InvalidDuration)?;
        state.recent_option = Some(RecentOptionFactSnapshot {
            option_id: selected.descriptor.option_id.clone(),
            end_reason: OptionEndReason::MaximumDuration,
            realized_ticks: offset_ticks,
            tape_start: execution.realized_tape_range.start_frame,
            tape_end_exclusive: execution
                .realized_tape_range
                .start_frame
                .checked_add(u64::from(offset_ticks))
                .ok_or(NativeTacticWorkerError::InvalidDuration)?,
            trajectory: Some(trajectory),
        });
        state
            .validate()
            .map_err(|error| NativeTacticWorkerError::Facts(error.to_string()))?;
        boundaries.push(OptionIntermediateBoundary {
            episode_shard_sha256,
            offset_ticks,
            state_sha256: state
                .content_sha256()
                .map_err(|error| NativeTacticWorkerError::Facts(error.to_string()))?,
            state,
        });
    }
    Ok(boundaries)
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

impl NativeTacticWorkerError {
    pub(crate) fn is_missing_process_local_checkpoint(&self) -> bool {
        matches!(
            self,
            Self::Worker(error) if error.is_missing_process_local_checkpoint()
        )
    }
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
mod tests;
