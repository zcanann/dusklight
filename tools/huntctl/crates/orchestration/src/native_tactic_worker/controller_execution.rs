//! Native generic and reactive controller execution.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_native_generic_tactic<W: PersistentTacticBatchWorker>(
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
    mut stepper: NativeGenericTacticStepper,
    duration: TacticDurationBounds,
    termination: OptionCondition,
    execution_strategy: NativeGenericExecutionStrategy,
    checkpoint_cache_capacity_bytes: usize,
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
            checkpoint_retention,
            duration,
            termination,
            program,
            checkpoint_cache_capacity_bytes,
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
            checkpoint_retention,
            checkpoint_cache_capacity_bytes,
            None,
        )?;
        let iteration_paths = iteration_paths(paths, selected.decision_index, local_tick);
        write_new_compact_batch(&iteration_paths.request, &request)?;
        let validated =
            worker.run_tactic_batch(&iteration_paths.request, &iteration_paths.result, &request)?;
        let loaded_episode = inspect_candidate_episode(&request, &validated, &candidate_tape)?;
        let episode = &loaded_episode.episode;
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
                    cancellation_guard: None,
                },
                candidate_tape,
                candidate_prefix_ticks,
                &request,
                &validated,
                0,
                Some(loaded_episode),
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

pub(super) fn native_generic_controller_program_for_strategy(
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

pub(super) fn native_generic_controller_program(
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
            // The descriptor is a camera-relative input direction, independent
            // of Link's current facing, just like the observation-driven path.
            frame: CoordinateFrame::Camera,
            heading_radians: canonical_controller_heading(f32::from_bits(
                *heading_radians_f32_bits,
            )),
            magnitude: *magnitude,
        },
        GenericTactic::SeekCoordinate {
            coordinate_f32_bits,
            tolerance_f32_bits,
            magnitude,
        } => Operation::SeekCoordinateSequence {
            blend: StickBlend::Replace,
            coordinates_xz: vec![[
                f32::from_bits(coordinate_f32_bits[0]),
                f32::from_bits(coordinate_f32_bits[2]),
            ]],
            intermediate_stop_radius: f32::from_bits(*tolerance_f32_bits),
            final_stop_radius: f32::from_bits(*tolerance_f32_bits),
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

pub(super) fn canonical_controller_heading(heading: f32) -> f32 {
    let mut wrapped = heading;
    if wrapped >= std::f32::consts::PI {
        wrapped -= std::f32::consts::TAU;
    } else if wrapped < -std::f32::consts::PI {
        wrapped += std::f32::consts::TAU;
    }
    wrapped
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NativeControllerOutcomeKind {
    NativeGeneric,
    Reactive,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_native_controller_candidate(
    root_checkpoint_sha256: Digest,
    selected: &SelectedTactic,
    before: &FactSnapshot,
    route_prefix: &InputTape,
    candidate_prefix_ticks: usize,
    request: &NativeSuffixBatch,
    validated: &ValidatedNativeSuffixBatch,
    candidate_index: usize,
    loaded_episode: LoadedCandidateEpisode,
    duration: TacticDurationBounds,
    termination: OptionCondition,
    program: ControllerProgram,
    kind: NativeControllerOutcomeKind,
) -> Result<NativeTacticWorkerOutcome, NativeTacticWorkerError> {
    let episode = &loaded_episode.episode;
    if episode.steps.len() <= candidate_prefix_ticks
        || episode.steps.len()
            > candidate_prefix_ticks.saturating_add(duration.maximum_ticks as usize)
    {
        return Err(NativeTacticWorkerError::DetachedResult(
            "reactive controller episode length",
        ));
    }
    let prefix_start = route_prefix
        .frames
        .len()
        .checked_sub(candidate_prefix_ticks)
        .ok_or(NativeTacticWorkerError::DetachedSelection)?;
    let prefix_frames = route_prefix.frames[prefix_start..].to_vec();
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
    // Audit the exact serialized controller that ran natively. Regenerating
    // controller PADs through a different tactic stepper can introduce a
    // one-unit rounding mismatch even when the native action is correct.
    let mut stepper = ControllerProgramStepper::new(program)
        .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?;
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
                "native controller returned neither PAD nor stopping condition".into(),
            )
        })?;
        if !same_pad(native_step.chosen_pad, frame.pads[0])
            || !same_pad(native_step.consumed_pad, frame.pads[0])
        {
            return Err(NativeTacticWorkerError::Observation(format!(
                "native controller PAD mismatch at local tick {index}: chosen {:?}, consumed {:?}, replayed {:?}",
                native_step.chosen_pad, native_step.consumed_pad, frame.pads[0]
            )));
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

    if kind == NativeControllerOutcomeKind::NativeGeneric
        && stepper_end == Some(ControllerRuntimeEnd::MaximumDuration)
    {
        let final_observation = controller_observation_from_post_simulation(
            &option_steps
                .last()
                .expect("controller candidate has at least one option step")
                .post_simulation,
        )?;
        if let Some(layer_index) = stepper
            .target_reached_after_last_frame(&final_observation)
            .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?
        {
            stepper_end = Some(ControllerRuntimeEnd::TargetReached { layer_index });
        }
    }

    let (end_reason, cancellation_conditions) = if episode.success {
        (
            OptionEndReason::Cancelled { condition_index: 0 },
            vec![OptionCondition::TargetReached {
                target: "authored_goal".into(),
            }],
        )
    } else {
        let end = stepper_end.ok_or(NativeTacticWorkerError::DetachedResult(
            "native controller stopped before its bounded tactic",
        ))?;
        match kind {
            NativeControllerOutcomeKind::NativeGeneric => (
                match end {
                    ControllerRuntimeEnd::TargetReached { .. } => OptionEndReason::Terminated,
                    ControllerRuntimeEnd::MaximumDuration => OptionEndReason::MaximumDuration,
                    ControllerRuntimeEnd::TargetLost { .. } => unreachable!(),
                },
                Vec::new(),
            ),
            NativeControllerOutcomeKind::Reactive => {
                if !matches!(
                    end,
                    ControllerRuntimeEnd::MaximumDuration
                        | ControllerRuntimeEnd::TargetReached { .. }
                ) {
                    return Err(NativeTacticWorkerError::DetachedResult(
                        "native controller stopped before its bounded tactic",
                    ));
                }
                (OptionEndReason::Completed, Vec::new())
            }
        }
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
            cancellation_guard: None,
        },
        candidate_tape,
        candidate_prefix_ticks,
        request,
        validated,
        candidate_index,
        Some(loaded_episode),
        queries
            .into_iter()
            .map(TacticRuntimeQuery::ReactiveController)
            .collect(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_native_generic_controller<W: PersistentTacticBatchWorker>(
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
    duration: TacticDurationBounds,
    termination: OptionCondition,
    program: ControllerProgram,
    checkpoint_cache_capacity_bytes: usize,
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
        checkpoint_retention,
        checkpoint_cache_capacity_bytes,
    )?;
    write_new_compact_batch(&paths.request, &request)?;
    let validated = worker.run_tactic_batch(&paths.request, &paths.result, &request)?;
    let loaded_episode = load_candidate_episode(&request, &validated)?;
    finish_native_controller_candidate(
        root_checkpoint_sha256,
        selected,
        before,
        route_prefix,
        candidate_prefix_ticks,
        &request,
        &validated,
        0,
        loaded_episode,
        duration,
        termination,
        program,
        NativeControllerOutcomeKind::NativeGeneric,
    )
}

pub(super) fn reactive_controller_uses_native_strategy(
    execution_strategy: NativeGenericExecutionStrategy,
    cancellation: &[OptionCondition],
) -> bool {
    matches!(
        execution_strategy,
        NativeGenericExecutionStrategy::NativeController
    ) && cancellation.is_empty()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_reactive_controller_native<W: PersistentTacticBatchWorker>(
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
    _stepper: ControllerProgramStepper,
    duration: TacticDurationBounds,
    termination: OptionCondition,
    cancellation: Vec<OptionCondition>,
    program: ControllerProgram,
    checkpoint_cache_capacity_bytes: usize,
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
        checkpoint_retention,
        checkpoint_cache_capacity_bytes,
    )?;
    write_new_compact_batch(&paths.request, &request)?;
    let validated = worker.run_tactic_batch(&paths.request, &paths.result, &request)?;
    let loaded_episode = load_candidate_episode(&request, &validated)?;
    finish_native_controller_candidate(
        root_checkpoint_sha256,
        selected,
        before,
        route_prefix,
        candidate_prefix_ticks,
        &request,
        &validated,
        0,
        loaded_episode,
        duration,
        termination,
        program,
        NativeControllerOutcomeKind::Reactive,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_reactive_controller<W: PersistentTacticBatchWorker>(
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
    mut stepper: ControllerProgramStepper,
    duration: TacticDurationBounds,
    termination: OptionCondition,
    cancellation: Vec<OptionCondition>,
    checkpoint_cache_capacity_bytes: usize,
) -> Result<NativeTacticWorkerOutcome, NativeTacticWorkerError> {
    let mut observation = controller_observation_from_facts(before)?;
    let mut option_tape = InputTape {
        boot: route_prefix.boot.clone(),
        tick_rate_numerator: route_prefix.tick_rate_numerator,
        tick_rate_denominator: route_prefix.tick_rate_denominator,
        frames: Vec::new(),
    };
    let mut queries = Vec::new();
    let mut last_run: Option<(
        InputTape,
        NativeSuffixBatch,
        ValidatedNativeSuffixBatch,
        LoadedCandidateEpisode,
    )> = None;

    for local_tick in 0..duration.maximum_ticks {
        let step = stepper
            .step(&observation)
            .map_err(|error| NativeTacticWorkerError::Observation(error.to_string()))?;
        queries.push(step.query);
        if matches!(step.end, Some(ControllerRuntimeEnd::TargetLost { .. })) {
            let Some((candidate_tape, request, validated, loaded_episode)) = last_run else {
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
                    cancellation_guard: None,
                },
                candidate_tape,
                candidate_prefix_ticks,
                &request,
                &validated,
                0,
                Some(loaded_episode),
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
            checkpoint_retention,
            checkpoint_cache_capacity_bytes,
            None,
        )?;
        let iteration_paths = iteration_paths(paths, selected.decision_index, local_tick);
        write_new_compact_batch(&iteration_paths.request, &request)?;
        let validated =
            worker.run_tactic_batch(&iteration_paths.request, &iteration_paths.result, &request)?;
        let loaded_episode = inspect_candidate_episode(&request, &validated, &candidate_tape)?;
        let episode = &loaded_episode.episode;
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

        let controller_complete = matches!(
            step.end,
            Some(
                ControllerRuntimeEnd::MaximumDuration | ControllerRuntimeEnd::TargetReached { .. }
            )
        );
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
                    cancellation_guard: None,
                },
                candidate_tape,
                candidate_prefix_ticks,
                &request,
                &validated,
                0,
                Some(loaded_episode),
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
        last_run = Some((candidate_tape, request, validated, loaded_episode));
    }
    Err(NativeTacticWorkerError::InvalidDuration)
}

pub(super) fn capture_local_execution(
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

pub(super) fn controller_observation_from_facts(
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

pub(super) fn controller_observation_from_post_simulation(
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
pub(super) fn build_controller_observation(
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

pub(super) fn angle_to_radians(angle: i16) -> f32 {
    f32::from(angle) * std::f32::consts::PI / 32_768.0
}
