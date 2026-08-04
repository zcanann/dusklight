//! Lossless sibling evaluation from one native checkpoint-backed request.

use super::controller_execution::{
    NativeControllerOutcomeKind, finish_native_controller_candidate,
    native_generic_controller_program_for_strategy, reactive_controller_uses_native_strategy,
};
use super::*;

enum PreparedBatchedOutcome {
    Static {
        prepared: Box<PreparedNativeTactic>,
        candidate_tape: InputTape,
    },
    Controller {
        duration: TacticDurationBounds,
        termination: OptionCondition,
        program: ControllerProgram,
        kind: NativeControllerOutcomeKind,
    },
}

pub(crate) fn selected_tactic_batch_is_compatible(
    selected: &[SelectedTactic],
    catalog: &TacticAssetCatalog,
    blueprints: &[TacticBlueprint],
    execution_strategy: NativeGenericExecutionStrategy,
) -> Result<bool, NativeTacticWorkerError> {
    if selected.len() < 2 {
        return Ok(false);
    }
    for selected in selected {
        match prepare_selected(selected, catalog, blueprints)? {
            PreparedNativeExecution::Static(_) => {}
            PreparedNativeExecution::NativeGeneric {
                stepper, duration, ..
            } => {
                if native_generic_controller_program_for_strategy(
                    stepper.plan(),
                    duration,
                    execution_strategy,
                )?
                .is_none()
                {
                    return Ok(false);
                }
            }
            PreparedNativeExecution::ReactiveController { cancellation, .. } => {
                if !reactive_controller_uses_native_strategy(execution_strategy, &cancellation) {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

/// Executes a complete compatible sibling set from one restored checkpoint.
///
/// `Ok(None)` is a lossless fallback signal: at least one tactic requires the
/// progressive per-tick executor, so the caller must preserve the ordinary
/// single-candidate path for the whole set.
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_selected_tactic_batch_if_compatible<W: PersistentTacticBatchWorker>(
    worker: &mut W,
    selected: &[SelectedTactic],
    catalog: &TacticAssetCatalog,
    blueprints: &[TacticBlueprint],
    before: &FactSnapshot,
    route_prefix: &InputTape,
    checkpoint_source: Option<&NativeTacticCheckpointSource>,
    paths: &NativeTacticWorkerPaths,
    retain_candidate_index: Option<usize>,
    execution_strategy: NativeGenericExecutionStrategy,
    checkpoint_cache_capacity_bytes: usize,
) -> Result<Option<Vec<NativeTacticWorkerOutcome>>, NativeTacticWorkerError> {
    if selected.len() < 2
        || checkpoint_cache_capacity_bytes == 0
        || retain_candidate_index.is_some_and(|index| index >= selected.len())
    {
        return Err(NativeTacticWorkerError::InvalidDuration);
    }
    let context = validate_tactic_execution_context(
        worker.identity(),
        selected,
        catalog,
        blueprints,
        before,
        route_prefix,
        checkpoint_source,
    )?;
    let prefix_frames =
        candidate_prefix_frames(route_prefix, context.source_frame, checkpoint_source);
    if prefix_frames.len() != context.replayed_prefix_ticks {
        return Err(NativeTacticWorkerError::DetachedSelection);
    }

    let mut candidates = Vec::with_capacity(selected.len());
    let mut prepared_outcomes = Vec::with_capacity(selected.len());
    for selected in selected {
        let id = tactic_candidate_id(selected)?;
        match prepare_selected(selected, catalog, blueprints)? {
            PreparedNativeExecution::Static(prepared) => {
                let mut candidate_tape = InputTape {
                    boot: route_prefix.boot.clone(),
                    tick_rate_numerator: route_prefix.tick_rate_numerator,
                    tick_rate_denominator: route_prefix.tick_rate_denominator,
                    frames: prefix_frames.clone(),
                };
                candidate_tape
                    .frames
                    .extend_from_slice(&prepared.option_tape.frames);
                let maximum_ticks = candidate_tape.frames.len();
                if maximum_ticks == 0 || maximum_ticks > 4_096 {
                    return Err(NativeTacticWorkerError::InvalidDuration);
                }
                candidates.push(NativeSuffixCandidate {
                    id,
                    actions: pad_runs(&candidate_tape.frames)?,
                    controller_program_hex: None,
                    maximum_ticks: Some(maximum_ticks),
                    cancellation_guard: prepared.cancellation_guard.clone(),
                });
                prepared_outcomes.push(PreparedBatchedOutcome::Static {
                    prepared: Box::new(prepared),
                    candidate_tape,
                });
            }
            PreparedNativeExecution::NativeGeneric {
                stepper,
                duration,
                termination,
            } => {
                let Some(program) = native_generic_controller_program_for_strategy(
                    stepper.plan(),
                    duration,
                    execution_strategy,
                )?
                else {
                    return Ok(None);
                };
                push_controller_candidate(
                    &mut candidates,
                    &mut prepared_outcomes,
                    id,
                    &prefix_frames,
                    duration,
                    termination,
                    program,
                    NativeControllerOutcomeKind::NativeGeneric,
                )?;
            }
            PreparedNativeExecution::ReactiveController {
                program,
                duration,
                termination,
                cancellation,
                ..
            } => {
                if !reactive_controller_uses_native_strategy(execution_strategy, &cancellation) {
                    return Ok(None);
                }
                push_controller_candidate(
                    &mut candidates,
                    &mut prepared_outcomes,
                    id,
                    &prefix_frames,
                    duration,
                    termination,
                    program,
                    NativeControllerOutcomeKind::Reactive,
                )?;
            }
        }
    }

    let maximum_ticks = candidates
        .iter()
        .filter_map(|candidate| candidate.maximum_ticks)
        .max()
        .ok_or(NativeTacticWorkerError::InvalidDuration)?;
    let mut checkpoint_cache = tactic_checkpoint_cache_request(
        checkpoint_source,
        NativeTacticCheckpointRetention::None,
        checkpoint_cache_capacity_bytes,
    );
    checkpoint_cache.retain_candidate_index = retain_candidate_index;
    let request = NativeSuffixBatch {
        schema: NATIVE_VARIABLE_CACHED_SUFFIX_BATCH_SCHEMA.into(),
        source_frame: usize::try_from(worker.identity().source_frame)
            .map_err(|_| NativeTacticWorkerError::InvalidDuration)?,
        source_boundary_fingerprint: checkpoint_source.map_or_else(
            || worker.identity().source_boundary_fingerprint.clone(),
            |source| source.boundary_fingerprint.clone(),
        ),
        checkpoint_validation: NativeCheckpointValidation {
            kind: worker.identity().checkpoint_validation_kind.clone(),
            ticks: usize::try_from(worker.identity().checkpoint_validation_ticks)
                .map_err(|_| NativeTacticWorkerError::InvalidDuration)?,
        },
        maximum_ticks,
        verify_state_hashes: false,
        checkpoint_cache: Some(checkpoint_cache),
        candidates,
    };
    write_new_compact_batch(&paths.request, &request)?;
    let validated = worker.run_tactic_batch(&paths.request, &paths.result, &request)?;
    let loaded = load_candidate_episodes(&request, &validated)?;

    let mut outcomes = Vec::with_capacity(selected.len());
    for (candidate_index, ((selected, prepared), loaded_episode)) in selected
        .iter()
        .zip(prepared_outcomes)
        .zip(loaded)
        .enumerate()
    {
        let outcome = match prepared {
            PreparedBatchedOutcome::Static {
                prepared,
                candidate_tape,
            } => observe_outcome(
                context.root_checkpoint_sha256,
                selected,
                before,
                route_prefix,
                *prepared,
                candidate_tape,
                context.replayed_prefix_ticks,
                &request,
                &validated,
                candidate_index,
                Some(loaded_episode),
                Vec::new(),
            )?,
            PreparedBatchedOutcome::Controller {
                duration,
                termination,
                program,
                kind,
            } => finish_native_controller_candidate(
                context.root_checkpoint_sha256,
                selected,
                before,
                route_prefix,
                context.replayed_prefix_ticks,
                &request,
                &validated,
                candidate_index,
                loaded_episode,
                duration,
                termination,
                program,
                kind,
            )?,
        };
        outcomes.push(outcome);
    }
    Ok(Some(outcomes))
}

#[allow(clippy::too_many_arguments)]
fn push_controller_candidate(
    candidates: &mut Vec<NativeSuffixCandidate>,
    prepared_outcomes: &mut Vec<PreparedBatchedOutcome>,
    id: String,
    prefix_frames: &[InputFrame],
    duration: TacticDurationBounds,
    termination: OptionCondition,
    program: ControllerProgram,
    kind: NativeControllerOutcomeKind,
) -> Result<(), NativeTacticWorkerError> {
    let maximum_ticks = prefix_frames
        .len()
        .checked_add(program.duration_frames as usize)
        .ok_or(NativeTacticWorkerError::InvalidDuration)?;
    if maximum_ticks == 0 || maximum_ticks > 4_096 {
        return Err(NativeTacticWorkerError::InvalidDuration);
    }
    let program_bytes = program
        .encode_compatible()
        .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?;
    candidates.push(NativeSuffixCandidate {
        id,
        actions: pad_runs(prefix_frames)?,
        controller_program_hex: Some(lower_hex_bytes(&program_bytes)),
        maximum_ticks: Some(maximum_ticks),
        cancellation_guard: None,
    });
    prepared_outcomes.push(PreparedBatchedOutcome::Controller {
        duration,
        termination,
        program,
        kind,
    });
    Ok(())
}
