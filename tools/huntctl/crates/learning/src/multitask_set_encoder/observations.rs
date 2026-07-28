use super::*;

pub(super) fn subtract3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

pub(super) fn push_continuous(
    values: &mut Vec<f32>,
    present: &mut Vec<bool>,
    value: f32,
    available: bool,
) {
    values.push(if available { value } else { 0.0 });
    present.push(available);
}

pub(super) fn push_continuous3(
    values: &mut Vec<f32>,
    present: &mut Vec<bool>,
    value: [f32; 3],
    available: bool,
) {
    for component in value {
        push_continuous(values, present, component, available);
    }
}

pub(super) fn length3(value: [f32; 3]) -> f32 {
    value
        .iter()
        .map(|component| component * component)
        .sum::<f32>()
        .sqrt()
}

pub(super) fn direction_yaw3(direction: [f32; 3], yaw: i16) -> [f32; 3] {
    let radians = f32::from(yaw) * std::f32::consts::PI / 32768.0;
    let (sin, cos) = radians.sin_cos();
    [
        cos * direction[0] - sin * direction[2],
        direction[1],
        sin * direction[0] + cos * direction[2],
    ]
}

pub(super) fn angle_pair(angle: i16) -> [f32; 2] {
    let radians = f32::from(angle) * std::f32::consts::PI / 32768.0;
    [radians.sin(), radians.cos()]
}

pub(super) fn native_target_names() -> Vec<String> {
    [
        "player_position_delta_x",
        "player_position_delta_y",
        "player_position_delta_z",
        "player_velocity_delta_x",
        "player_velocity_delta_y",
        "player_velocity_delta_z",
        "player_forward_speed_delta",
        "contact_changed",
        "procedure_changed",
        "mode_flags_changed",
        "actor_disappearance_occurred",
        "actor_disappearance_count",
        "inverse_stick_x",
        "inverse_stick_y",
        "inverse_button_0x0100",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

pub(super) fn native_target_conditioning() -> Vec<AuxiliaryHeadConditioning> {
    let mut conditioning = vec![AuxiliaryHeadConditioning::PreStateAndAction; 12];
    conditioning.extend([AuxiliaryHeadConditioning::PreAndPostState; 3]);
    conditioning
}

pub(super) fn target_objectives_for_names(names: &[String]) -> Vec<AuxiliaryHeadObjective> {
    names
        .iter()
        .map(|name| {
            if matches!(
                name.as_str(),
                "contact_changed"
                    | "procedure_changed"
                    | "mode_flags_changed"
                    | "actor_disappearance_occurred"
                    | "inverse_button_0x0100"
            ) {
                AuxiliaryHeadObjective::ClassBalancedBernoulli
            } else {
                AuxiliaryHeadObjective::NormalizedRegression
            }
        })
        .collect()
}

pub(super) fn target_conditioning_for_names(names: &[String]) -> Vec<AuxiliaryHeadConditioning> {
    names
        .iter()
        .map(|name| {
            if name.starts_with("inverse_") {
                AuxiliaryHeadConditioning::PreAndPostState
            } else {
                AuxiliaryHeadConditioning::PreStateAndAction
            }
        })
        .collect()
}

pub(super) fn native_action_context(example: &NativeAuxiliaryExample) -> Vec<f32> {
    let action = example.targets.inverse_action;
    pad_action_context(
        action.buttons,
        action.stick_x,
        action.stick_y,
        action.substick_x,
        action.substick_y,
        action.trigger_left,
        action.trigger_right,
        action.analog_a,
        action.analog_b,
    )
}

pub(super) fn episode_history_action_context(action: &EpisodeHistoryPad) -> Vec<f32> {
    pad_action_context(
        action.buttons,
        action.stick_x,
        action.stick_y,
        action.substick_x,
        action.substick_y,
        action.trigger_left,
        action.trigger_right,
        action.analog_a,
        action.analog_b,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn pad_action_context(
    buttons: u16,
    stick_x: i8,
    stick_y: i8,
    substick_x: i8,
    substick_y: i8,
    trigger_left: u8,
    trigger_right: u8,
    analog_a: u8,
    analog_b: u8,
) -> Vec<f32> {
    let mut context = [stick_x, stick_y, substick_x, substick_y]
        .map(|value| f32::from(value) / 128.0)
        .to_vec();
    context.extend(
        [trigger_left, trigger_right, analog_a, analog_b].map(|value| f32::from(value) / 255.0),
    );
    context.extend((0..16).map(|bit| f32::from(buttons & (1_u16 << bit) != 0)));
    debug_assert_eq!(context.len(), ACTION_CONTEXT_WIDTH);
    context
}

pub(super) fn append_episode_history_features(
    values: &mut Vec<f32>,
    present: &mut Vec<bool>,
    episode: &NativeEpisode,
    completed: &[&EpisodeHistoryTransition],
    spec: &NativeEncoderFeatureSpec,
) -> Result<(), TrainableSetError> {
    if completed.len() > spec.history_depth {
        return Err(TrainableSetError::new(
            "native episode history exceeds the declared feature depth",
        ));
    }
    let core_width = selected_feature_names(
        native_base_feature_names(),
        &native_base_feature_families(),
        spec,
    )
    .len();
    let missing = spec.history_depth - completed.len();
    for _ in 0..missing {
        values.push(0.0);
        present.push(true);
        values.extend(std::iter::repeat_n(0.0, ACTION_CONTEXT_WIDTH + core_width));
        present.extend(std::iter::repeat_n(
            false,
            ACTION_CONTEXT_WIDTH + core_width,
        ));
    }
    for transition in completed {
        if transition.episode_id != episode.id {
            return Err(TrainableSetError::new(
                "native episode history crosses an episode boundary",
            ));
        }
        let step = episode
            .steps
            .get(transition.step_index as usize)
            .ok_or_else(|| TrainableSetError::new("native episode history step is absent"))?;
        values.push(1.0);
        present.push(true);
        values.extend(episode_history_action_context(&transition.consumed_pad));
        present.extend(std::iter::repeat_n(true, ACTION_CONTEXT_WIDTH));

        let (mut state, mut state_present) = broad_base(&step.post_simulation);
        append_core_temporal_features(
            &mut state,
            &mut state_present,
            &step.post_simulation,
            Some(&step.pre_input),
        );
        retain_feature_families(
            &mut state,
            &mut state_present,
            &native_base_feature_families(),
            spec,
        );
        if state.len() != core_width || state_present.len() != core_width {
            return Err(TrainableSetError::new(
                "native episode history core width is inconsistent",
            ));
        }
        values.extend(state);
        present.extend(state_present);
    }
    Ok(())
}

pub(super) fn trainable_episode_history_steps(
    episode: &NativeEpisode,
    completed: &[&EpisodeHistoryTransition],
    spec: &NativeEncoderFeatureSpec,
    actor_feature_schema_sha256: Digest,
    states: &mut BTreeMap<(String, u32), Arc<TypedSetSample>>,
) -> Result<Vec<MultiTaskHistoryStep>, TrainableSetError> {
    if spec.history_encoding != NativeEncoderHistoryEncoding::TrainableGru {
        return Ok(Vec::new());
    }
    if completed.len() > spec.history_depth {
        return Err(TrainableSetError::new(
            "native trainable history exceeds the declared feature depth",
        ));
    }
    completed
        .iter()
        .map(|transition| {
            if transition.episode_id != episode.id {
                return Err(TrainableSetError::new(
                    "native trainable history crosses an episode boundary",
                ));
            }
            let transition_sha256 = canonical_digest(
                b"dusklight.native-trainable-history-transition/v1\0",
                transition,
            )?;
            let key = (transition.episode_id.clone(), transition.step_index);
            let state = if let Some(state) = states.get(&key) {
                Arc::clone(state)
            } else {
                let step = episode
                    .steps
                    .get(transition.step_index as usize)
                    .ok_or_else(|| {
                        TrainableSetError::new("native trainable history step is absent")
                    })?;
                let (mut base, mut base_present) = broad_base(&step.post_simulation);
                append_core_temporal_features(
                    &mut base,
                    &mut base_present,
                    &step.post_simulation,
                    Some(&step.pre_input),
                );
                suppress_base_family(
                    &mut base,
                    &mut base_present,
                    NativeEncoderChannelFamily::CorePreviousInput,
                );
                retain_feature_families(
                    &mut base,
                    &mut base_present,
                    &native_base_feature_families(),
                    spec,
                );
                let mut nodes = if spec.contains(NativeEncoderChannelFamily::ActorPopulation) {
                    native_actor_nodes(&step.post_simulation, Some(&step.pre_input))
                } else {
                    Vec::new()
                };
                for node in &mut nodes {
                    retain_node_feature_families(node, spec);
                }
                let sample_sha256 = canonical_digest(
                    b"dusklight.native-trainable-history-state/v1\0",
                    &(
                        transition_sha256,
                        hex_128(step.post_simulation.state_identity),
                        actor_feature_schema_sha256,
                    ),
                )?;
                let state = Arc::new(TypedSetSample {
                    sample_sha256,
                    actor_feature_schema_sha256,
                    base,
                    base_present,
                    nodes,
                    target: 0.0,
                });
                states.insert(key, Arc::clone(&state));
                state
            };
            Ok(MultiTaskHistoryStep {
                transition_sha256,
                state,
                action_context: episode_history_action_context(&transition.consumed_pad),
            })
        })
        .collect()
}

pub(super) fn append_encoded_episode_history_features(
    values: &mut Vec<f32>,
    present: &mut Vec<bool>,
    episode: &NativeEpisode,
    completed: &[&EpisodeHistoryTransition],
    spec: &NativeEncoderFeatureSpec,
    recurrent_reservoir: Option<&Reservoir>,
) -> Result<(), TrainableSetError> {
    match spec.history_encoding {
        NativeEncoderHistoryEncoding::None => {
            if !completed.is_empty() {
                return Err(TrainableSetError::new(
                    "native episode history is present for a history-free feature spec",
                ));
            }
            Ok(())
        }
        NativeEncoderHistoryEncoding::Stacked => {
            append_episode_history_features(values, present, episode, completed, spec)
        }
        NativeEncoderHistoryEncoding::RecurrentReservoir => {
            append_recurrent_episode_history_features(
                values,
                present,
                episode,
                completed,
                spec,
                recurrent_reservoir.ok_or_else(|| {
                    TrainableSetError::new("native recurrent history reservoir is absent")
                })?,
            )
        }
        NativeEncoderHistoryEncoding::TrainableGru => Ok(()),
    }
}

pub(super) fn native_recurrent_history_input_width(
    spec: &NativeEncoderFeatureSpec,
) -> Result<usize, TrainableSetError> {
    let core_width = selected_feature_names(
        native_base_feature_names(),
        &native_base_feature_families(),
        spec,
    )
    .len();
    ACTION_CONTEXT_WIDTH
        .checked_add(core_width.checked_mul(2).ok_or_else(|| {
            TrainableSetError::new("native recurrent history input width overflowed")
        })?)
        .ok_or_else(|| TrainableSetError::new("native recurrent history input width overflowed"))
}

pub(super) fn native_recurrent_history_reservoir(
    spec: &NativeEncoderFeatureSpec,
) -> Result<Option<Reservoir>, TrainableSetError> {
    if spec.history_encoding != NativeEncoderHistoryEncoding::RecurrentReservoir {
        return Ok(None);
    }
    Ok(Some(Reservoir::new(
        native_recurrent_history_input_width(spec)?,
        spec.history_recurrent_width,
        HISTORY_RESERVOIR_SEED,
    )))
}

pub(super) fn append_recurrent_episode_history_features(
    values: &mut Vec<f32>,
    present: &mut Vec<bool>,
    episode: &NativeEpisode,
    completed: &[&EpisodeHistoryTransition],
    spec: &NativeEncoderFeatureSpec,
    reservoir: &Reservoir,
) -> Result<(), TrainableSetError> {
    if completed.len() > spec.history_depth {
        return Err(TrainableSetError::new(
            "native episode history exceeds the declared feature depth",
        ));
    }
    let core_width = selected_feature_names(
        native_base_feature_names(),
        &native_base_feature_families(),
        spec,
    )
    .len();
    let input_width = native_recurrent_history_input_width(spec)?;
    let mut hidden = vec![0.0; spec.history_recurrent_width];
    for transition in completed {
        if transition.episode_id != episode.id {
            return Err(TrainableSetError::new(
                "native episode history crosses an episode boundary",
            ));
        }
        let step = episode
            .steps
            .get(transition.step_index as usize)
            .ok_or_else(|| TrainableSetError::new("native episode history step is absent"))?;
        let mut input = episode_history_action_context(&transition.consumed_pad);
        let (mut state, mut state_present) = broad_base(&step.post_simulation);
        append_core_temporal_features(
            &mut state,
            &mut state_present,
            &step.post_simulation,
            Some(&step.pre_input),
        );
        retain_feature_families(
            &mut state,
            &mut state_present,
            &native_base_feature_families(),
            spec,
        );
        if state.len() != core_width || state_present.len() != core_width {
            return Err(TrainableSetError::new(
                "native recurrent history core width is inconsistent",
            ));
        }
        input.extend(state.iter().zip(&state_present).map(|(value, available)| {
            if *available {
                (value.signum() * value.abs().ln_1p() / 32.0).clamp(-1.0, 1.0)
            } else {
                0.0
            }
        }));
        input.extend(state_present.iter().map(|available| f32::from(*available)));
        if input.len() != input_width {
            return Err(TrainableSetError::new(
                "native recurrent history observation width is inconsistent",
            ));
        }
        let input_scale = (input_width as f32).sqrt().recip();
        input.iter_mut().for_each(|value| *value *= input_scale);
        hidden = reservoir.step(&input, &hidden);
    }

    values.push(f32::from(!completed.is_empty()));
    values.push(completed.len() as f32 / spec.history_depth as f32);
    values.extend(hidden.into_iter().map(|value| value as f32));
    present.extend(std::iter::repeat_n(true, 2 + spec.history_recurrent_width));
    Ok(())
}

pub(super) fn native_targets(example: &NativeAuxiliaryExample) -> (Vec<f32>, Vec<bool>) {
    let mut targets = vec![0.0; 15];
    let mut present = vec![false; 15];
    if let Some(dynamics) = &example.targets.player_dynamics {
        targets[..3].copy_from_slice(&dynamics.position_delta);
        targets[3..6].copy_from_slice(&dynamics.velocity_delta);
        targets[6] = dynamics.forward_speed_delta;
        present[..7].fill(true);
    }
    if let Some(contacts) = &example.targets.contacts {
        targets[7] = f32::from(contacts.activated != 0 || contacts.cleared != 0);
        present[7] = true;
    }
    if let Some(action) = &example.targets.action_phase {
        targets[8] = f32::from(action.procedure_before != action.procedure_after);
        targets[9] = f32::from(action.mode_flags_activated != 0 || action.mode_flags_cleared != 0);
        present[8..10].fill(true);
    }
    if let Some(lifecycle) = &example.targets.actor_lifecycle {
        let count = lifecycle.disappeared_runtime_generations.len();
        targets[10] = f32::from(count != 0);
        targets[11] = count as f32;
        present[10..12].fill(true);
    }
    targets[12] = f32::from(example.targets.inverse_action.stick_x);
    targets[13] = f32::from(example.targets.inverse_action.stick_y);
    targets[14] = f32::from(example.targets.inverse_action.buttons & 0x0100 != 0);
    present[12..].fill(true);
    (targets, present)
}

pub(super) fn broad_base(observation: &NativeLearningObservation) -> (Vec<f32>, Vec<bool>) {
    let mut values = Vec::new();
    let mut present = Vec::new();
    let mut push = |value: f32, available: bool| {
        values.push(if available { value } else { 0.0 });
        present.push(available);
    };
    for value in observation.player_position {
        push(value, observation.player_present);
    }
    for value in observation.player_velocity {
        push(value, observation.player_present);
    }
    push(observation.player_forward_speed, observation.player_present);
    for angle in observation
        .player_current_angle
        .into_iter()
        .chain(observation.player_shape_angle)
    {
        push(f32::from(angle), observation.player_present);
    }
    push(
        f32::from(observation.player_procedure),
        observation.player_present,
    );
    for bit in 0..32 {
        push(
            f32::from(observation.player_mode_flags & (1_u32 << bit) != 0),
            observation.player_present,
        );
    }
    for bit in 0..8 {
        push(
            f32::from(observation.player_contacts & (1_u8 << bit) != 0),
            observation.player_present,
        );
    }
    push(f32::from(observation.event_running), true);
    push(f32::from(observation.event_id), true);
    push(f32::from(observation.event_mode), true);
    push(f32::from(observation.event_status), true);
    push(f32::from(observation.event_map_tool_id), true);
    push(f32::from(observation.room), true);
    push(f32::from(observation.layer), true);
    push(f32::from(observation.point), true);
    let transition = observation.event_transition.as_ref();
    push(
        transition.map_or(0.0, |value| f32::from(value.event_data_loaded)),
        transition.is_some(),
    );
    push(
        transition.map_or(0.0, |value| value.camera_play as f32),
        transition.is_some(),
    );
    let current_event = transition.and_then(|value| value.current_event.as_ref());
    push(
        current_event.map_or(0.0, |value| f32::from(value.event_id)),
        current_event.is_some(),
    );
    push(
        current_event.map_or(0.0, |value| value.event_type as f32),
        current_event.is_some(),
    );
    push(
        current_event.map_or(0.0, |value| value.room as f32),
        current_event.is_some(),
    );
    for index in 0..3 {
        push(
            current_event.map_or(0.0, |value| value.goal[index]),
            current_event.is_some(),
        );
    }
    let pending_stage = transition.and_then(|value| value.pending_stage.as_ref());
    push(f32::from(pending_stage.is_some()), transition.is_some());
    push(
        pending_stage.map_or(0.0, |value| f32::from(value.room)),
        pending_stage.is_some(),
    );
    push(
        pending_stage.map_or(0.0, |value| f32::from(value.layer)),
        pending_stage.is_some(),
    );
    push(
        pending_stage.map_or(0.0, |value| f32::from(value.point)),
        pending_stage.is_some(),
    );
    push(
        pending_stage.map_or(0.0, |value| f32::from(value.wipe)),
        pending_stage.is_some(),
    );
    push(
        pending_stage.map_or(0.0, |value| f32::from(value.wipe_speed)),
        pending_stage.is_some(),
    );
    let clocks = observation.clock_domains.as_ref();
    let clocks_present = clocks.is_some();
    push(
        clocks.map_or(0.0, |value| value.framework_frames as f32),
        clocks_present,
    );
    push(
        clocks.map_or(0.0, |value| value.gameplay_frames as f32),
        clocks_present,
    );
    push(
        clocks.map_or(0.0, |value| f32::from(value.global_pause)),
        clocks_present,
    );
    push(
        clocks.map_or(0.0, |value| f32::from(value.scene_paused)),
        clocks_present,
    );
    push(
        clocks.map_or(0.0, |value| value.scene_pause_timer as f32),
        clocks_present,
    );
    push(
        clocks.map_or(0.0, |value| value.scene_next_pause_timer as f32),
        clocks_present,
    );
    push(
        clocks.map_or(0.0, |value| f32::from(value.overlap_request_active)),
        clocks_present,
    );
    push(
        clocks.map_or(0.0, |value| f32::from(value.overlap_fadeout_peek)),
        clocks_present,
    );
    let demo_present =
        clocks.is_some_and(|value| value.demo_status == NativeChannelStatus::Present);
    push(f32::from(demo_present), clocks_present);
    push(
        clocks.map_or(0.0, |value| value.demo_mode as f32),
        demo_present,
    );
    push(
        clocks.map_or(0.0, |value| value.demo_frame as f32),
        demo_present,
    );
    push(
        clocks.map_or(0.0, |value| value.demo_frame_no_message as f32),
        demo_present,
    );
    push(
        clocks.map_or(0.0, |value| value.demo_flags as f32),
        demo_present,
    );
    let timer_present =
        clocks.is_some_and(|value| value.timer_status == NativeChannelStatus::Present);
    push(f32::from(timer_present), clocks_present);
    push(
        clocks.map_or(0.0, |value| value.timer_mode as f32),
        timer_present,
    );
    push(
        clocks.map_or(0.0, |value| value.timer_now_ms as f32),
        timer_present,
    );
    push(
        clocks.map_or(0.0, |value| value.timer_limit_ms as f32),
        timer_present,
    );
    let warp = observation.warp_session.as_ref();
    let warp_present = warp.is_some();
    push(
        warp.map_or(0.0, |value| f32::from(value.request_kind)),
        warp_present,
    );
    let selection = warp.and_then(|value| value.selection.as_ref());
    push(f32::from(selection.is_some()), warp_present);
    for index in 0..3 {
        push(
            selection.map_or(0.0, |value| value.position[index]),
            selection.is_some(),
        );
    }
    push(
        selection.map_or(0.0, |value| f32::from(value.angle)),
        selection.is_some(),
    );
    push(
        selection.map_or(0.0, |value| f32::from(value.room)),
        selection.is_some(),
    );
    push(
        selection.map_or(0.0, |value| f32::from(value.parameter)),
        selection.is_some(),
    );
    push(
        selection.map_or(0.0, |value| f32::from(value.player)),
        selection.is_some(),
    );
    push(
        selection.map_or(0.0, |value| f32::from(value.stage == observation.stage)),
        selection.is_some(),
    );
    let return_mark = warp.and_then(|value| value.return_mark.as_ref());
    push(f32::from(return_mark.is_some()), warp_present);
    for index in 0..3 {
        push(
            return_mark.map_or(0.0, |value| value.position[index]),
            return_mark.is_some(),
        );
    }
    push(
        return_mark.map_or(0.0, |value| f32::from(value.angle)),
        return_mark.is_some(),
    );
    push(
        return_mark.map_or(0.0, |value| f32::from(value.room)),
        return_mark.is_some(),
    );
    push(
        return_mark.map_or(0.0, |value| f32::from(value.accept_stage)),
        return_mark.is_some(),
    );
    push(
        return_mark.map_or(0.0, |value| f32::from(value.stage == observation.stage)),
        return_mark.is_some(),
    );
    let target_point = warp.and_then(|value| value.target_point);
    push(f32::from(target_point.is_some()), warp_present);
    push(target_point.map_or(0.0, f32::from), target_point.is_some());
    let selected_point = warp.and_then(|value| value.selected_point);
    push(f32::from(selected_point.is_some()), warp_present);
    push(
        selected_point.map_or(0.0, f32::from),
        selected_point.is_some(),
    );
    push(
        warp.map_or(0.0, |value| f32::from(value.transport_match)),
        warp_present,
    );
    for value in [
        observation.previous_input.stick_x,
        observation.previous_input.stick_y,
        observation.previous_input.substick_x,
        observation.previous_input.substick_y,
    ] {
        push(f32::from(value), true);
    }
    for value in [
        observation.previous_input.trigger_left,
        observation.previous_input.trigger_right,
        observation.previous_input.analog_a,
        observation.previous_input.analog_b,
    ] {
        push(f32::from(value), true);
    }
    for bit in 0..16 {
        push(
            f32::from(observation.previous_input.buttons & (1_u16 << bit) != 0),
            true,
        );
    }
    push(
        observation.camera_yaw_radians.unwrap_or(0.0),
        observation.camera_yaw_radians.is_some(),
    );
    for index in 0..9 {
        let camera_value = observation.camera.as_ref().map(|camera| match index {
            0 => f32::from(camera.view_yaw),
            1 => f32::from(camera.controlled_yaw),
            2 => f32::from(camera.bank),
            3..=5 => camera.eye[index - 3],
            _ => camera.center[index - 6],
        });
        push(camera_value.unwrap_or(0.0), camera_value.is_some());
    }
    push(
        observation.player_ground_height.unwrap_or(0.0),
        observation.player_ground_height.is_some(),
    );
    push(
        observation.player_roof_height.unwrap_or(0.0),
        observation.player_roof_height.is_some(),
    );
    let water_height = observation
        .player_background_collision
        .as_ref()
        .map(|collision| collision.water_height);
    push(water_height.unwrap_or(0.0), water_height.is_some());
    for index in 0..2 {
        let correction = observation.collision_correction.map(|value| value[index]);
        push(correction.unwrap_or(0.0), correction.is_some());
    }
    for index in 0..7 {
        let scene = observation.scene_exit.as_ref().map(|exit| match index {
            0 => exit.signed_distance_to_volume,
            1..=3 => exit.player_local_position[index - 1],
            _ => exit.volume_extent[index - 4],
        });
        push(scene.unwrap_or(0.0), scene.is_some());
    }
    for stream_index in 0..2 {
        let stream = observation.rng_streams.get(stream_index);
        push(
            stream.map_or(0.0, |value| f32::from(value.id)),
            stream.is_some(),
        );
        for state_index in 0..3 {
            push(
                stream.map_or(0.0, |value| value.state[state_index] as f32),
                stream.is_some(),
            );
        }
        push(
            stream.map_or(0.0, |value| value.call_count as f32),
            stream.is_some(),
        );
    }
    push(observation.rng_streams.len() as f32, true);
    push(observation.rng_streams.len().saturating_sub(2) as f32, true);
    for value in [
        observation.goal.requested_count,
        observation.goal.hit_count,
        observation.goal.stable_ticks,
        observation.goal.consecutive_ticks,
        u16::from(observation.goal.sequence_steps),
        u16::from(observation.goal.sequence_next_step),
        observation.goal.sequence_within_ticks,
        observation.goal.sequence_elapsed_ticks,
    ] {
        push(f32::from(value), observation.goal.configured);
    }
    push(
        f32::from(observation.goal.reached),
        observation.goal.configured,
    );
    let attention_available =
        observation.attention_candidates_status == NativeChannelStatus::Present;
    let attention = observation.attention_candidates.as_ref();
    for bit in 0..32 {
        push(
            attention.map_or(0.0, |value| {
                f32::from(value.player_attention_flags & (1_u32 << bit) != 0)
            }),
            attention_available,
        );
    }
    for value in [
        attention.map_or(0.0, |value| f32::from(value.attention_status)),
        attention.map_or(0.0, |value| value.attention_block_timer as f32),
        attention.map_or(0.0, |value| value.lock_candidates.len() as f32),
        attention.map_or(0.0, |value| f32::from(value.lock_offset)),
        attention.map_or(0.0, |value| value.action_candidates.len() as f32),
        attention.map_or(0.0, |value| f32::from(value.action_offset)),
        attention.map_or(0.0, |value| value.check_candidates.len() as f32),
        attention.map_or(0.0, |value| f32::from(value.check_offset)),
    ] {
        push(value, attention_available);
    }
    (values, present)
}

pub(super) fn append_core_temporal_features(
    values: &mut Vec<f32>,
    present: &mut Vec<bool>,
    current: &NativeLearningObservation,
    previous: Option<&NativeLearningObservation>,
) {
    let comparable = previous.is_some_and(|previous| {
        current.player_present
            && previous.player_present
            && current.player_is_link == previous.player_is_link
            && current.stage == previous.stage
            && current.room == previous.room
            && current.layer == previous.layer
    });
    let player_delta = |current: [f32; 3], select: fn(&NativeLearningObservation) -> [f32; 3]| {
        previous
            .filter(|_| comparable)
            .map_or([0.0; 3], |observation| {
                subtract3(current, select(observation))
            })
    };
    push_continuous3(
        values,
        present,
        player_delta(current.player_position, |value| value.player_position),
        comparable,
    );
    push_continuous3(
        values,
        present,
        player_delta(current.player_velocity, |value| value.player_velocity),
        comparable,
    );
    push_continuous(
        values,
        present,
        previous.map_or(0.0, |previous| {
            current.player_forward_speed - previous.player_forward_speed
        }),
        comparable,
    );
    let camera_pair = current
        .camera_yaw_radians
        .zip(previous.and_then(|previous| previous.camera_yaw_radians));
    push_continuous(
        values,
        present,
        camera_pair.map_or(0.0, |(current, previous)| current - previous),
        camera_pair.is_some() && comparable,
    );
    for (current_height, previous_height) in [
        (
            current.player_ground_height,
            previous.and_then(|value| value.player_ground_height),
        ),
        (
            current.player_roof_height,
            previous.and_then(|value| value.player_roof_height),
        ),
    ] {
        let pair = current_height.zip(previous_height);
        push_continuous(
            values,
            present,
            pair.map_or(0.0, |(current, previous)| current - previous),
            pair.is_some() && comparable,
        );
    }
    push_continuous(values, present, f32::from(previous.is_some()), true);
    push_continuous(values, present, f32::from(comparable), true);
    push_continuous(
        values,
        present,
        previous.map_or(0.0, |previous| {
            f32::from(current.player_procedure != previous.player_procedure)
        }),
        comparable,
    );
    push_continuous(
        values,
        present,
        previous.map_or(0.0, |previous| {
            f32::from(current.player_mode_flags != previous.player_mode_flags)
        }),
        comparable,
    );
    for bit in 0..8 {
        push_continuous(
            values,
            present,
            previous.map_or(0.0, |previous| {
                f32::from((current.player_contacts ^ previous.player_contacts) & (1_u8 << bit) != 0)
            }),
            comparable,
        );
    }
    push_continuous(
        values,
        present,
        previous.map_or(0.0, |previous| {
            f32::from(current.event_running != previous.event_running)
        }),
        previous.is_some(),
    );
    push_continuous(
        values,
        present,
        previous.map_or(0.0, |previous| {
            f32::from(current.event_id != previous.event_id)
        }),
        previous.is_some(),
    );
    push_continuous(
        values,
        present,
        previous.map_or(0.0, |previous| {
            f32::from(
                current.stage != previous.stage
                    || current.room != previous.room
                    || current.layer != previous.layer
                    || current.point != previous.point,
            )
        }),
        previous.is_some(),
    );
}
