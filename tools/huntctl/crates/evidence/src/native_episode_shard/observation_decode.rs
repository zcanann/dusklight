//! Assemble the versioned native learning observation payload.

use super::*;

pub(super) fn decode_observation(
    reader: &mut Reader<'_>,
    observation_version: u16,
) -> Result<NativeLearningObservation, NativeEpisodeShardError> {
    if observation_version == OBSERVATION_VERSION_V29 {
        return match reader.u8()? {
            0 => decode_observation(reader, OBSERVATION_VERSION_V28),
            1 => decode_tactic_observation(reader),
            _ => Err(NativeEpisodeShardError::new(
                "invalid learning observation detail profile",
            )),
        };
    }
    let phase = match reader.u8()? {
        1 => NativeObservationPhase::PreInput,
        2 => NativeObservationPhase::PostSimulation,
        _ => return Err(NativeEpisodeShardError::new("invalid observation phase")),
    };
    let actor_selection = match reader.u8()? {
        0 => NativeActorSelectionRule::Complete,
        1 => NativeActorSelectionRule::LowestRuntimeGeneration,
        _ => return Err(NativeEpisodeShardError::new("invalid actor selection rule")),
    };
    let terminal_reason = match reader.u8()? {
        0 => NativeTerminalReason::None,
        1 => NativeTerminalReason::GoalReached,
        2 => NativeTerminalReason::TickBudgetExhausted,
        _ => return Err(NativeEpisodeShardError::new("invalid terminal reason")),
    };
    if reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero observation reserved byte",
        ));
    }
    let actor_count = usize::from(reader.u16()?);
    let flags = reader.u32()?;
    let actor_observed_count = reader.u32()?;
    let remaining_ticks = reader.u32()?;
    let boundary_index = reader.u64()?;
    let simulation_tick = reader.u64()?;
    let tape_frame = reader.u64()?;
    let state_identity = reader.bytes(16)?.try_into().expect("exact length");
    if flags & !0x0fff != 0
        || actor_count > MAX_ACTORS
        || actor_observed_count < actor_count as u32
        || ((flags & (1 << 5) != 0) != (actor_observed_count > actor_count as u32))
        || (actor_selection == NativeActorSelectionRule::Complete) != (flags & (1 << 5) == 0)
    {
        return Err(NativeEpisodeShardError::new(
            "inconsistent observation header",
        ));
    }
    if observation_version >= OBSERVATION_VERSION_V4
        && (actor_selection != NativeActorSelectionRule::Complete
            || flags & (1 << 5) != 0
            || actor_observed_count != actor_count as u32)
    {
        return Err(NativeEpisodeShardError::new(
            "v4+ observation does not contain the complete actor set",
        ));
    }
    let stage = reader.fixed_name()?;
    let room = reader.i8()?;
    let layer = reader.i8()?;
    let point = reader.i16()?;
    let next_stage_raw = reader.fixed_name()?;
    let next_room = reader.i8()?;
    let next_layer = reader.i8()?;
    let next_point = reader.i16()?;
    let player_process_id = reader.u32()?;
    let player_actor_name = reader.i16()?;
    let player_procedure = reader.u16()?;
    let player_position = reader.f32x3()?;
    let player_velocity = reader.f32x3()?;
    let player_forward_speed = reader.f32()?;
    let player_current_angle = reader.i16x3()?;
    let player_shape_angle = reader.i16x3()?;
    let player_mode_flags = reader.u32()?;
    let player_damage_wait_timer = reader.i16()?;
    let player_ice_damage_wait_timer = reader.i16()?;
    let player_sword_change_wait_timer = reader.u8()?;
    let player_do_status = reader.u8()?;
    let player_contacts = reader.u8()?;
    if player_contacts & !0x1f != 0 || reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new("invalid player contact bits"));
    }
    let ground_height = reader.f32()?;
    let roof_height = reader.f32()?;
    let event_running = reader.bool()?;
    let event_id = reader.i16()?;
    let event_mode = reader.u8()?;
    let event_status = reader.u8()?;
    let event_map_tool_id = reader.u8()?;
    let event_name_hash_raw = reader.u32()?;
    let menu_flags = reader.u16()?;
    if menu_flags & !0x0fff != 0 {
        return Err(NativeEpisodeShardError::new("invalid menu flags"));
    }
    let menu_procedures = [
        reader.u8()?,
        reader.u8()?,
        reader.u8()?,
        reader.u8()?,
        reader.u8()?,
    ];
    if reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new("nonzero menu reserved byte"));
    }
    let camera = reader.f32()?;
    let correction = [reader.f32()?, reader.f32()?];
    let (
        camera_status,
        mechanics_camera,
        player_action_status,
        player_action,
        player_background_collision_status,
        player_background_collision,
        player_collision_surfaces_status,
        player_collision_surfaces,
        scene_exit_status,
        scene_exit,
        player_form_present,
        player_is_wolf,
    ) = match observation_version {
        OBSERVATION_VERSION_V2 => (
            NativeChannelStatus::NotSampled,
            None,
            NativeChannelStatus::NotSampled,
            None,
            NativeChannelStatus::NotSampled,
            None,
            NativeChannelStatus::NotSampled,
            None,
            NativeChannelStatus::NotSampled,
            None,
            false,
            false,
        ),
        OBSERVATION_VERSION_V3
        | OBSERVATION_VERSION_V4
        | OBSERVATION_VERSION_V5
        | OBSERVATION_VERSION_V6
        | OBSERVATION_VERSION_V7
        | OBSERVATION_VERSION_V8
        | OBSERVATION_VERSION_V9
        | OBSERVATION_VERSION_V10
        | OBSERVATION_VERSION_V11
        | OBSERVATION_VERSION_V12
        | OBSERVATION_VERSION_V13
        | OBSERVATION_VERSION_V14
        | OBSERVATION_VERSION_V15
        | OBSERVATION_VERSION_V16
        | OBSERVATION_VERSION_V17
        | OBSERVATION_VERSION_V18
        | OBSERVATION_VERSION_V19
        | OBSERVATION_VERSION_V20
        | OBSERVATION_VERSION_V21
        | OBSERVATION_VERSION_V22
        | OBSERVATION_VERSION_V23
        | OBSERVATION_VERSION_V24
        | OBSERVATION_VERSION_V25
        | OBSERVATION_VERSION_V26
        | OBSERVATION_VERSION_V27
        | OBSERVATION_VERSION_V28 => {
            let camera_status = decode_channel_status(reader)?;
            let action_status = decode_channel_status(reader)?;
            let background_status = decode_channel_status(reader)?;
            let surfaces_status = decode_channel_status(reader)?;
            let scene_exit_status = decode_channel_status(reader)?;
            let collision_plane_mask = reader.u8()?;
            let form_flags = reader.u8()?;
            if collision_plane_mask & !0x3f != 0
                || form_flags & !0x3 != 0
                || form_flags & 2 != 0 && form_flags & 1 == 0
                || reader.u8()? != 0
            {
                return Err(NativeEpisodeShardError::new(
                    "invalid mechanics observation header",
                ));
            }
            let mechanics_camera = decode_camera(reader)?;
            let player_action = decode_player_action(reader)?;
            let scene_exit = decode_scene_exit(reader, scene_exit_status)?;
            let background = decode_background_collision(reader)?;
            let surfaces = decode_collision_surfaces(
                reader,
                collision_plane_mask,
                surfaces_status,
                (flags & (1 << 2) != 0).then_some(next_stage_raw.as_str()),
                next_room,
                next_layer,
                next_point,
            )?;
            if surfaces_status != NativeChannelStatus::Present && collision_plane_mask != 0 {
                return Err(NativeEpisodeShardError::new(
                    "collision planes are present without a surface channel",
                ));
            }
            if background_status == NativeChannelStatus::Present
                && surfaces_status == NativeChannelStatus::Present
                && !collision_channels_agree(&background, &surfaces)
            {
                return Err(NativeEpisodeShardError::new(
                    "collision channels disagree on surface identities",
                ));
            }
            (
                camera_status,
                (camera_status == NativeChannelStatus::Present).then_some(mechanics_camera),
                action_status,
                (action_status == NativeChannelStatus::Present).then_some(player_action),
                background_status,
                (background_status == NativeChannelStatus::Present).then_some(background),
                surfaces_status,
                (surfaces_status == NativeChannelStatus::Present).then_some(surfaces),
                scene_exit_status,
                (scene_exit_status == NativeChannelStatus::Present).then_some(scene_exit),
                form_flags & 1 != 0,
                form_flags & 2 != 0,
            )
        }
        _ => {
            return Err(NativeEpisodeShardError::new(
                "unsupported observation schema version",
            ));
        }
    };
    let previous_input = decode_pad(reader)?;
    let rng_version = reader.u32()?;
    let rng_count = reader.u32()?;
    if rng_version != RNG_SNAPSHOT_VERSION || rng_count != 2 {
        return Err(NativeEpisodeShardError::new(
            "unsupported RNG snapshot identity",
        ));
    }
    let mut rng_streams = Vec::with_capacity(2);
    for expected_id in 0..2 {
        let id = reader.u8()?;
        if id != expected_id || reader.bytes(3)?.iter().any(|byte| *byte != 0) {
            return Err(NativeEpisodeShardError::new("noncanonical RNG stream"));
        }
        let algorithm_version = reader.u32()?;
        if algorithm_version != RNG_ALGORITHM_VERSION {
            return Err(NativeEpisodeShardError::new(
                "unsupported RNG algorithm identity",
            ));
        }
        rng_streams.push(NativeRngStream {
            id,
            algorithm_version,
            state: [reader.i32()?, reader.i32()?, reader.i32()?],
            call_count: reader.u64()?,
        });
    }
    let talk_partner = decode_actor_identity(reader)?;
    let grabbed_actor = decode_actor_identity(reader)?;
    let goal = NativeGoalObservation {
        configured: flags & (1 << 7) != 0,
        reached: flags & (1 << 8) != 0,
        requested_count: reader.u16()?,
        hit_count: reader.u16()?,
        stable_ticks: reader.u16()?,
        consecutive_ticks: reader.u16()?,
        sequence_steps: reader.u8()?,
        sequence_next_step: reader.u8()?,
        sequence_within_ticks: reader.u16()?,
        sequence_elapsed_ticks: reader.u16()?,
        first_hit_tick: match reader.u64()? {
            u64::MAX => None,
            tick => Some(tick),
        },
    };
    if goal.reached != goal.first_hit_tick.is_some() || goal.hit_count > goal.requested_count {
        return Err(NativeEpisodeShardError::new(
            "inconsistent goal observation",
        ));
    }
    let mut actors = Vec::with_capacity(actor_count);
    for _ in 0..actor_count {
        let mut actor = NativeActorObservation {
            runtime_generation: reader.u64()?,
            base_state_available: false,
            actor_type: 0,
            process_subtype: 0,
            parent_runtime_generation: reader.u32()?,
            parameters: reader.u32()?,
            status: reader.u32()?,
            condition: 0,
            actor_name: reader.i16()?,
            profile_name: reader.i16()?,
            set_id: reader.u16()?,
            home_room: reader.i8()?,
            old_room: -1,
            current_room: reader.i8()?,
            group: reader.u8()?,
            argument: reader.i8()?,
            pause_flag: 0,
            process_init_state: 0,
            process_create_phase: 0,
            cull_type: 0,
            demo_actor_id: 0,
            carry_type: 0,
            heap_present: false,
            model_present: false,
            joint_collision_present: false,
            health: reader.i16()?,
            position: reader.f32x3()?,
            home_position: reader.f32x3()?,
            old_position: [0.0; 3],
            velocity: reader.f32x3()?,
            forward_speed: reader.f32()?,
            scale: [0.0; 3],
            gravity: 0.0,
            max_fall_speed: 0.0,
            eye_position: [0.0; 3],
            home_angle: [0; 3],
            old_angle: [0; 3],
            current_angle: reader.i16x3()?,
            shape_angle: reader.i16x3()?,
            attention: None,
            event_participation: None,
            return_place_writer: None,
            enemy_base: None,
            trigger_volume: None,
            door20: None,
        };
        if observation_version >= OBSERVATION_VERSION_V6 {
            let component_mask = reader.u16()?;
            let known_component_mask = if observation_version >= OBSERVATION_VERSION_V27 {
                0x3f
            } else if observation_version >= OBSERVATION_VERSION_V17 {
                0x1f
            } else if observation_version >= OBSERVATION_VERSION_V15 {
                0xf
            } else if observation_version >= OBSERVATION_VERSION_V14 {
                0x7
            } else {
                0x3
            };
            if component_mask & !known_component_mask != 0 || reader.u16()? != 0 {
                return Err(NativeEpisodeShardError::new(
                    "invalid actor component header",
                ));
            }
            let attention_flags = reader.u32()?;
            let attention_position = reader.f32x3()?;
            let attention_distances: [u8; 9] = reader
                .bytes(9)?
                .try_into()
                .expect("exact attention-distance length");
            let attention_auxiliary = reader.i16()?;
            if reader.u8()? != 0 {
                return Err(NativeEpisodeShardError::new(
                    "nonzero actor attention reserved byte",
                ));
            }
            let event_command = reader.u16()?;
            let event_condition = reader.u16()?;
            let event_id = reader.i16()?;
            let event_map_tool_id = reader.u8()?;
            let event_index = reader.u8()?;
            if component_mask & 1 != 0 {
                if attention_flags == 0 {
                    return Err(NativeEpisodeShardError::new(
                        "present actor attention component has no flags",
                    ));
                }
                actor.attention = Some(NativeActorAttentionComponent {
                    flags: attention_flags,
                    position: attention_position,
                    distance_indices: attention_distances,
                    auxiliary: attention_auxiliary,
                });
            } else if attention_flags != 0
                || attention_position != [0.0; 3]
                || attention_distances != [0; 9]
                || attention_auxiliary != 0
            {
                return Err(NativeEpisodeShardError::new(
                    "absent actor attention component has a payload",
                ));
            }
            let event_is_nondefault = event_command != 0
                || event_condition != 2
                || event_id != -1
                || event_map_tool_id != 0xff
                || event_index != 0;
            if component_mask & 2 != 0 {
                if !event_is_nondefault {
                    return Err(NativeEpisodeShardError::new(
                        "present actor event component is constructor-default state",
                    ));
                }
                actor.event_participation = Some(NativeActorEventParticipationComponent {
                    command: event_command,
                    condition: event_condition,
                    event_id,
                    map_tool_id: event_map_tool_id,
                    index: event_index,
                });
            } else if event_command != 0
                || event_condition != 0
                || event_id != 0
                || event_map_tool_id != 0
                || event_index != 0
            {
                return Err(NativeEpisodeShardError::new(
                    "absent actor event component has a payload",
                ));
            }
            if observation_version >= OBSERVATION_VERSION_V14 {
                let save_room = reader.i8()?;
                let save_point = reader.u8()?;
                let switch_room = reader.i8()?;
                let guard_mask = reader.u8()?;
                let required_event_set = reader.u16()?;
                let required_event_unset = reader.u16()?;
                let required_switch_set = reader.u8()?;
                let required_switch_unset = reader.u8()?;
                if guard_mask & !0x3f != 0 || reader.u16()? != 0 {
                    return Err(NativeEpisodeShardError::new(
                        "invalid return-place writer payload",
                    ));
                }
                if component_mask & 4 != 0 {
                    let no_telop_clear = guard_mask & 1 != 0;
                    let event_set_satisfied = guard_mask & 2 != 0;
                    let event_unset_satisfied = guard_mask & 4 != 0;
                    let switch_set_satisfied = guard_mask & 8 != 0;
                    let switch_unset_satisfied = guard_mask & 16 != 0;
                    let eligible = guard_mask & 32 != 0;
                    if required_event_set == u16::MAX && !event_set_satisfied
                        || required_event_unset == u16::MAX && !event_unset_satisfied
                        || required_switch_set == u8::MAX && !switch_set_satisfied
                        || required_switch_unset == u8::MAX && !switch_unset_satisfied
                        || eligible
                            != (no_telop_clear
                                && event_set_satisfied
                                && event_unset_satisfied
                                && switch_set_satisfied
                                && switch_unset_satisfied)
                    {
                        return Err(NativeEpisodeShardError::new(
                            "inconsistent return-place writer guards",
                        ));
                    }
                    actor.return_place_writer = Some(NativeReturnPlaceWriterComponent {
                        save_room,
                        save_point,
                        switch_room,
                        required_event_set,
                        required_event_unset,
                        required_switch_set,
                        required_switch_unset,
                        no_telop_clear,
                        event_set_satisfied,
                        event_unset_satisfied,
                        switch_set_satisfied,
                        switch_unset_satisfied,
                        eligible,
                    });
                } else if save_room != 0
                    || save_point != 0
                    || switch_room != 0
                    || guard_mask != 0
                    || required_event_set != 0
                    || required_event_unset != 0
                    || required_switch_set != 0
                    || required_switch_unset != 0
                {
                    return Err(NativeEpisodeShardError::new(
                        "absent return-place writer component has a payload",
                    ));
                }
            }
            if observation_version >= OBSERVATION_VERSION_V15 {
                let enemy_flags = reader.u16()?;
                let enemy_throw_mode = reader.u8()?;
                if reader.u8()? != 0 {
                    return Err(NativeEpisodeShardError::new(
                        "nonzero enemy-base reserved byte",
                    ));
                }
                let down_position = reader.f32x3()?;
                let head_lock_position = reader.f32x3()?;
                if component_mask & 8 != 0 {
                    if actor.group != 2 {
                        return Err(NativeEpisodeShardError::new(
                            "enemy-base component belongs to a non-enemy actor",
                        ));
                    }
                    actor.enemy_base = Some(NativeEnemyBaseComponent {
                        flags: enemy_flags,
                        throw_mode: enemy_throw_mode,
                        down_position,
                        head_lock_position,
                    });
                } else if actor.group == 2
                    || enemy_flags != 0
                    || enemy_throw_mode != 0
                    || down_position != [0.0; 3]
                    || head_lock_position != [0.0; 3]
                {
                    return Err(NativeEpisodeShardError::new(
                        "absent enemy-base component has a payload or enemy owner",
                    ));
                }
            }
            if observation_version >= OBSERVATION_VERSION_V17 {
                let kind = reader.u8()?;
                let shape = reader.u8()?;
                let trigger_flags = reader.u8()?;
                if trigger_flags & !0x3 != 0 || reader.u8()? != 0 {
                    return Err(NativeEpisodeShardError::new(
                        "invalid trigger-volume header",
                    ));
                }
                let behavior = reader.u16()?;
                let yaw = reader.i16()?;
                let center = reader.f32x3()?;
                let half_extent = reader.f32x3()?;
                if component_mask & 16 != 0 {
                    let kind = match kind {
                        1 => NativeTriggerVolumeKind::SceneExit,
                        2 => NativeTriggerVolumeKind::SceneExitCylinder,
                        3 => NativeTriggerVolumeKind::EventArea,
                        4 => NativeTriggerVolumeKind::ScriptedEvent,
                        5 => NativeTriggerVolumeKind::MappedEvent,
                        _ => {
                            return Err(NativeEpisodeShardError::new(
                                "unknown trigger-volume kind",
                            ));
                        }
                    };
                    let shape = match shape {
                        1 => NativeTriggerVolumeShape::Box,
                        2 => NativeTriggerVolumeShape::EllipticCylinder,
                        _ => {
                            return Err(NativeEpisodeShardError::new(
                                "unknown trigger-volume shape",
                            ));
                        }
                    };
                    let vertical_unbounded = trigger_flags & 2 != 0;
                    if half_extent.iter().any(|value| *value < 0.0)
                        || vertical_unbounded && shape != NativeTriggerVolumeShape::EllipticCylinder
                    {
                        return Err(NativeEpisodeShardError::new(
                            "inconsistent trigger-volume geometry",
                        ));
                    }
                    actor.trigger_volume = Some(NativeTriggerVolumeComponent {
                        kind,
                        shape,
                        enabled: trigger_flags & 1 != 0,
                        vertical_unbounded,
                        behavior,
                        center,
                        half_extent,
                        yaw,
                    });
                } else if kind != 0
                    || shape != 0
                    || trigger_flags != 0
                    || behavior != 0
                    || yaw != 0
                    || center != [0.0; 3]
                    || half_extent != [0.0; 3]
                {
                    return Err(NativeEpisodeShardError::new(
                        "absent trigger-volume component has a payload",
                    ));
                }
            }
            if observation_version >= OBSERVATION_VERSION_V27 {
                let kind = reader.u8()?;
                let door_model = reader.u8()?;
                let front_option = reader.u8()?;
                let back_option = reader.u8()?;
                let front_room = reader.u8()?;
                let back_room = reader.u8()?;
                let exit_number = reader.u8()?;
                let action = reader.u8()?;
                let active_side = reader.u8()?;
                let event_variant = reader.u8()?;
                let key_type = reader.u8()?;
                let enemy_clear_debounce = reader.u8()?;
                let front_switch = reader.u8()?;
                let back_switch = reader.u8()?;
                let unlock_effect_switch = reader.u8()?;
                let stopper_side = reader.u8()?;
                let front_event = reader.u8()?;
                let back_event = reader.u8()?;
                let front_stopper_status = reader.i8()?;
                let back_stopper_status = reader.i8()?;
                let door_angle = reader.i16()?;
                let message_number = reader.u16()?;
                let door_flags = reader.u16()?;
                if door_flags & !0x01ff != 0 || reader.u16()? != 0 {
                    return Err(NativeEpisodeShardError::new("invalid DOOR20 header"));
                }
                if component_mask & 32 != 0 {
                    if actor.actor_name != ACTOR_NAME_DOOR20
                        || kind > 31
                        || door_model > 7
                        || front_option > 3
                        || back_option > 7
                        || front_room > 63
                        || back_room > 63
                        || exit_number > 63
                        || event_variant > 18
                        || key_type > 1
                        || enemy_clear_debounce > 65
                        || front_switch == u8::MAX && door_flags & (1 << 1) != 0
                        || back_switch == u8::MAX && door_flags & (1 << 2) != 0
                        || unlock_effect_switch == u8::MAX && door_flags & (1 << 3) != 0
                        || door_flags & (1 << 7) != 0 && door_flags & (1 << 8) != 0
                        || door_flags & (1 << 4) != 0 && front_option != 2 && back_option != 2
                        || key_type == 1 && kind != 9
                    {
                        return Err(NativeEpisodeShardError::new(
                            "inconsistent DOOR20 component state",
                        ));
                    }
                    let action = match action {
                        0 => NativeDoor20Action::Init,
                        1 => NativeDoor20Action::Wait,
                        2 => NativeDoor20Action::StopClose,
                        3 => NativeDoor20Action::Demo,
                        _ => {
                            return Err(NativeEpisodeShardError::new("unknown DOOR20 action"));
                        }
                    };
                    let active_side = match active_side {
                        0 => NativeDoor20Side::Front,
                        1 => NativeDoor20Side::Back,
                        2 => NativeDoor20Side::Neither,
                        _ => {
                            return Err(NativeEpisodeShardError::new("unknown DOOR20 active side"));
                        }
                    };
                    let stopper_side = match stopper_side {
                        0 => NativeDoor20Side::Front,
                        1 => NativeDoor20Side::Back,
                        _ => {
                            return Err(NativeEpisodeShardError::new(
                                "unknown DOOR20 stopper side",
                            ));
                        }
                    };
                    let stopper_status = |value| match value {
                        -1 => Ok(NativeDoor20StopperStatus::RoomUnavailable),
                        0 => Ok(NativeDoor20StopperStatus::Open),
                        1 => Ok(NativeDoor20StopperStatus::Closed),
                        _ => Err(NativeEpisodeShardError::new(
                            "unknown DOOR20 stopper status",
                        )),
                    };
                    actor.door20 = Some(NativeDoor20Component {
                        kind,
                        door_model,
                        front_option,
                        back_option,
                        front_room,
                        back_room,
                        exit_number,
                        message_door: door_flags & 1 != 0,
                        front_switch,
                        back_switch,
                        unlock_effect_switch,
                        front_switch_set: door_flags & (1 << 1) != 0,
                        back_switch_set: door_flags & (1 << 2) != 0,
                        unlock_effect_switch_set: door_flags & (1 << 3) != 0,
                        front_event,
                        back_event,
                        message_number,
                        action,
                        active_side,
                        event_variant,
                        locked: door_flags & (1 << 4) != 0,
                        background_collision_released: door_flags & (1 << 5) != 0,
                        unlock_effect_triggered: door_flags & (1 << 6) != 0,
                        key_type,
                        enemy_clear_debounce,
                        opening_active: door_flags & (1 << 7) != 0,
                        closing_active: door_flags & (1 << 8) != 0,
                        door_angle,
                        stopper_side,
                        front_stopper_status: stopper_status(front_stopper_status)?,
                        back_stopper_status: stopper_status(back_stopper_status)?,
                    });
                } else if actor.actor_name == ACTOR_NAME_DOOR20
                    || kind != 0
                    || door_model != 0
                    || front_option != 0
                    || back_option != 0
                    || front_room != 0
                    || back_room != 0
                    || exit_number != 0
                    || action != 0
                    || active_side != 0
                    || event_variant != 0
                    || key_type != 0
                    || enemy_clear_debounce != 0
                    || front_switch != 0
                    || back_switch != 0
                    || unlock_effect_switch != 0
                    || stopper_side != 0
                    || front_event != 0
                    || back_event != 0
                    || front_stopper_status != 0
                    || back_stopper_status != 0
                    || door_angle != 0
                    || message_number != 0
                    || door_flags != 0
                {
                    return Err(NativeEpisodeShardError::new(
                        "absent DOOR20 component has a payload or DOOR20 owner",
                    ));
                }
            }
        }
        if observation_version >= OBSERVATION_VERSION_V7 {
            let backing_mask = reader.u8()?;
            if backing_mask & !0x7 != 0 || reader.u8()? != 0 {
                return Err(NativeEpisodeShardError::new(
                    "invalid actor base-state header",
                ));
            }
            actor.base_state_available = true;
            actor.actor_type = reader.i32()?;
            actor.process_subtype = reader.i32()?;
            actor.condition = reader.u32()?;
            actor.pause_flag = reader.u8()?;
            actor.process_init_state = reader.i8()?;
            actor.process_create_phase = reader.u8()?;
            actor.cull_type = reader.u8()?;
            actor.demo_actor_id = reader.u8()?;
            actor.carry_type = reader.u8()?;
            actor.old_room = reader.i8()?;
            if reader.u8()? != 0 {
                return Err(NativeEpisodeShardError::new(
                    "nonzero actor base-state reserved byte",
                ));
            }
            actor.heap_present = backing_mask & 1 != 0;
            actor.model_present = backing_mask & 2 != 0;
            actor.joint_collision_present = backing_mask & 4 != 0;
            actor.old_position = reader.f32x3()?;
            actor.scale = reader.f32x3()?;
            actor.gravity = reader.f32()?;
            actor.max_fall_speed = reader.f32()?;
            actor.eye_position = reader.f32x3()?;
            actor.home_angle = reader.i16x3()?;
            actor.old_angle = reader.i16x3()?;
        }
        if let Some(door) = &actor.door20 {
            let home_angle_x = actor.home_angle[0] as u16;
            let home_angle_z = actor.home_angle[2] as u16;
            if door.kind != (actor.parameters & 0x1f) as u8
                || door.door_model != ((actor.parameters >> 5) & 0x7) as u8
                || door.front_option != ((actor.parameters >> 8) & 0x3) as u8
                || door.back_option != ((actor.parameters >> 10) & 0x7) as u8
                || door.front_room != ((actor.parameters >> 13) & 0x3f) as u8
                || door.back_room != ((actor.parameters >> 19) & 0x3f) as u8
                || door.exit_number != ((actor.parameters >> 25) & 0x3f) as u8
                || door.message_door != (actor.parameters >> 31 != 0)
                || door.front_switch != (home_angle_z & 0xff) as u8
                || door.back_switch != (home_angle_z >> 8) as u8
                || door.unlock_effect_switch != (home_angle_x >> 8) as u8
                || door.front_event != (home_angle_x & 0xff) as u8
                || door.back_event != (home_angle_x >> 8) as u8
                || door.message_number != home_angle_x
            {
                return Err(NativeEpisodeShardError::new(
                    "DOOR20 authored fields disagree with actor placement",
                ));
            }
        }
        actors.push(actor);
    }
    if actors
        .windows(2)
        .any(|pair| pair[0].runtime_generation >= pair[1].runtime_generation)
    {
        return Err(NativeEpisodeShardError::new(
            "actor set is not strictly ordered",
        ));
    }
    let (dynamic_colliders_status, dynamic_colliders) = if observation_version
        >= OBSERVATION_VERSION_V8
    {
        let status = decode_channel_status(reader)?;
        if reader.u8()? != 0 {
            return Err(NativeEpisodeShardError::new(
                "nonzero dynamic-collider reserved byte",
            ));
        }
        let count = usize::from(reader.u16()?);
        if status != NativeChannelStatus::Present && count != 0 {
            return Err(NativeEpisodeShardError::new(
                "dynamic-collider payload is present for an unavailable channel",
            ));
        }
        let mut colliders = Vec::with_capacity(count);
        for expected_index in 0..count {
            let registration_index = reader.u16()?;
            let collider_flags = reader.u16()?;
            if usize::from(registration_index) != expected_index || collider_flags & !0x0fff != 0 {
                return Err(NativeEpisodeShardError::new(
                    "dynamic-collider set is not canonical",
                ));
            }
            let owner = reader.u32()?;
            let attack_owner = reader.u32()?;
            let target_owner = reader.u32()?;
            let correction_owner = reader.u32()?;
            let optional_owner = |bit: u16, value: u32| {
                if collider_flags & bit != 0 {
                    Ok(Some(value))
                } else if value == u32::MAX {
                    Ok(None)
                } else {
                    Err(NativeEpisodeShardError::new(
                        "absent dynamic-collider owner has a payload",
                    ))
                }
            };
            let owner_runtime_generation = optional_owner(1 << 0, owner)?;
            let attack_hit_owner_runtime_generation = optional_owner(1 << 9, attack_owner)?;
            let target_hit_owner_runtime_generation = optional_owner(1 << 10, target_owner)?;
            let correction_hit_owner_runtime_generation =
                optional_owner(1 << 11, correction_owner)?;
            if collider_flags & (1 << 9) != 0 && collider_flags & (1 << 6) == 0
                || collider_flags & (1 << 10) != 0 && collider_flags & (1 << 7) == 0
                || collider_flags & (1 << 11) != 0 && collider_flags & (1 << 8) == 0
            {
                return Err(NativeEpisodeShardError::new(
                    "dynamic-collider hit owner is present without a hit",
                ));
            }
            let attack_type = reader.u32()?;
            let target_type = reader.u32()?;
            let attack_source_parameters = reader.u32()?;
            let attack_result_parameters = reader.u32()?;
            let target_source_parameters = reader.u32()?;
            let target_result_parameters = reader.u32()?;
            let correction_source_parameters = reader.u32()?;
            let correction_result_parameters = reader.u32()?;
            let attack_power = reader.u8()?;
            let weight = reader.u8()?;
            let damage = reader.u8()?;
            let shape = match reader.u8()? {
                0 => NativeDynamicColliderShape::Unknown,
                1 => NativeDynamicColliderShape::Sphere,
                2 => NativeDynamicColliderShape::Cylinder,
                _ => {
                    return Err(NativeEpisodeShardError::new(
                        "invalid dynamic-collider shape",
                    ));
                }
            };
            let center = reader.f32x3()?;
            let radius = reader.f32()?;
            let height = reader.f32()?;
            let aabb_min = reader.f32x3()?;
            let aabb_max = reader.f32x3()?;
            let correction = reader.f32x3()?;
            let shape_present = collider_flags & (1 << 2) != 0;
            if !shape_present
                && (shape != NativeDynamicColliderShape::Unknown
                    || center != [0.0; 3]
                    || radius != 0.0
                    || height != 0.0
                    || aabb_min != [0.0; 3]
                    || aabb_max != [0.0; 3])
            {
                return Err(NativeEpisodeShardError::new(
                    "absent dynamic-collider shape has a payload",
                ));
            }
            let status_present = collider_flags & (1 << 1) != 0;
            if !status_present && (weight != 0 || damage != 0 || correction != [0.0; 3]) {
                return Err(NativeEpisodeShardError::new(
                    "absent dynamic-collider status has a payload",
                ));
            }
            colliders.push(NativeDynamicColliderObservation {
                registration_index,
                owner_runtime_generation,
                attack_hit_owner_runtime_generation,
                target_hit_owner_runtime_generation,
                correction_hit_owner_runtime_generation,
                status_present,
                shape_present,
                attack_set: collider_flags & (1 << 3) != 0,
                target_set: collider_flags & (1 << 4) != 0,
                correction_set: collider_flags & (1 << 5) != 0,
                attack_hit: collider_flags & (1 << 6) != 0,
                target_hit: collider_flags & (1 << 7) != 0,
                correction_hit: collider_flags & (1 << 8) != 0,
                shape,
                attack_type,
                target_type,
                attack_source_parameters,
                attack_result_parameters,
                target_source_parameters,
                target_result_parameters,
                correction_source_parameters,
                correction_result_parameters,
                attack_power,
                weight,
                damage,
                center,
                radius,
                height,
                aabb_min,
                aabb_max,
                correction,
            });
        }
        (status, colliders)
    } else {
        (NativeChannelStatus::NotSampled, Vec::new())
    };
    let flags_present = flags & (1 << 6) != 0;
    let event_flags = flags_present.then(|| reader.vec(822)).transpose()?;
    let temporary_flags = flags_present.then(|| reader.vec(185)).transpose()?;
    let temporary_event_bytes = (flags_present && observation_version >= OBSERVATION_VERSION_V5)
        .then(|| reader.vec(256))
        .transpose()?;
    let dungeon_flags = flags_present.then(|| reader.vec(64)).transpose()?;
    let switch_flags = flags_present.then(|| reader.vec(240)).transpose()?;
    let switch_flag_room = reader.i8()?;
    let (player_resources_status, player_resources) =
        if observation_version >= OBSERVATION_VERSION_V9 {
            let (status, resources) = decode_player_resources(reader)?;
            let player_present = flags & 1 != 0;
            if (status == NativeChannelStatus::Present) != player_present {
                return Err(NativeEpisodeShardError::new(
                    "player-resources presence disagrees with player presence",
                ));
            }
            (
                status,
                (status == NativeChannelStatus::Present).then_some(resources),
            )
        } else {
            (NativeChannelStatus::NotSampled, None)
        };
    let (player_relationships_status, player_relationships) =
        if observation_version >= OBSERVATION_VERSION_V10 {
            let (status, relationships) = decode_player_relationships(reader)?;
            let player_present = flags & 1 != 0;
            let player_is_link = flags & (1 << 1) != 0;
            let expected_status = if player_is_link {
                NativeChannelStatus::Present
            } else if player_present {
                NativeChannelStatus::Unavailable
            } else {
                NativeChannelStatus::Absent
            };
            if status != expected_status {
                return Err(NativeEpisodeShardError::new(
                    "player-relationship status disagrees with player type",
                ));
            }
            validate_player_relationship_joins(&relationships, &actors)?;
            (
                status,
                (status == NativeChannelStatus::Present).then_some(relationships),
            )
        } else {
            (NativeChannelStatus::NotSampled, None)
        };
    let (player_collision_solver_status, player_collision_solver) =
        if observation_version >= OBSERVATION_VERSION_V11 {
            let (status, solver) = decode_player_collision_solver(reader)?;
            let player_present = flags & 1 != 0;
            let player_is_link = flags & (1 << 1) != 0;
            let expected_status = if player_is_link {
                NativeChannelStatus::Present
            } else if player_present {
                NativeChannelStatus::Unavailable
            } else {
                NativeChannelStatus::Absent
            };
            if status != expected_status {
                return Err(NativeEpisodeShardError::new(
                    "player-collision-solver status disagrees with player type",
                ));
            }
            (
                status,
                (status == NativeChannelStatus::Present).then_some(solver),
            )
        } else {
            (NativeChannelStatus::NotSampled, None)
        };
    let (
        runtime_file_status,
        runtime_file,
        return_place_status,
        return_place,
        restart_status,
        restart,
        event_handoff_status,
        event_handoff,
    ) = if observation_version >= OBSERVATION_VERSION_V12 {
        decode_planner_runtime_channels(reader, observation_version)?
    } else {
        (
            NativeChannelStatus::NotSampled,
            None,
            NativeChannelStatus::NotSampled,
            None,
            NativeChannelStatus::NotSampled,
            None,
            NativeChannelStatus::NotSampled,
            None,
        )
    };
    let (message_session_status, message_session) =
        if observation_version >= OBSERVATION_VERSION_V16 {
            decode_message_session(reader)?
        } else {
            (NativeChannelStatus::NotSampled, None)
        };
    let actor_identity_joins = |identity: &NativeActorIdentity| {
        if !identity.present {
            return true;
        }
        actors
            .binary_search_by_key(&u64::from(identity.runtime_generation), |actor| {
                actor.runtime_generation
            })
            .ok()
            .map(|index| &actors[index])
            .is_some_and(|actor| {
                actor.actor_name == identity.actor_name
                    && actor.set_id == identity.set_id
                    && actor.home_room == identity.home_room
                    && actor.current_room == identity.current_room
                    && identity.home_position == Some(actor.home_position)
            })
    };
    if message_session
        .as_ref()
        .is_some_and(|message| !actor_identity_joins(&message.talk_actor))
    {
        return Err(NativeEpisodeShardError::new(
            "message-session talk actor is outside the complete actor population",
        ));
    }
    let (event_queue_status, event_queue) = if observation_version >= OBSERVATION_VERSION_V18 {
        decode_event_queue(reader)?
    } else {
        (NativeChannelStatus::NotSampled, None)
    };
    if event_queue.as_ref().is_some_and(|queue| {
        let participants = [
            &queue.active_request_actor,
            &queue.active_target_actor,
            &queue.active_talk_actor,
            &queue.active_item_actor,
            &queue.active_door_actor,
            &queue.change_actor,
            &queue.skip_actor,
        ];
        participants.into_iter().any(|reference| {
            reference
                .actor
                .as_ref()
                .is_some_and(|identity| !actor_identity_joins(identity))
        }) || queue.pending_orders.iter().any(|order| {
            order
                .request_actor
                .actor
                .as_ref()
                .is_some_and(|identity| !actor_identity_joins(identity))
                || order
                    .target_actor
                    .actor
                    .as_ref()
                    .is_some_and(|identity| !actor_identity_joins(identity))
        })
    }) {
        return Err(NativeEpisodeShardError::new(
            "event-queue actor is outside the complete actor population",
        ));
    }
    let (process_lifecycle_status, mut process_lifecycle) =
        if observation_version >= OBSERVATION_VERSION_V19 {
            let status = decode_channel_status(reader)?;
            if reader.bytes(3)?.iter().any(|byte| *byte != 0) {
                return Err(NativeEpisodeShardError::new(
                    "nonzero process-lifecycle reserved bytes",
                ));
            }
            let lifecycle = NativeProcessLifecycleObservation {
                active_actor_count: reader.u32()?,
                pending_create_count: reader.u32()?,
                pending_delete_count: reader.u32()?,
                pending_creates: Vec::new(),
                pending_deletes: Vec::new(),
            };
            if status == NativeChannelStatus::Present {
                if usize::try_from(lifecycle.active_actor_count).ok() != Some(actors.len()) {
                    return Err(NativeEpisodeShardError::new(
                        "process-lifecycle actor count disagrees with complete actor population",
                    ));
                }
                (status, Some(lifecycle))
            } else if status == NativeChannelStatus::Unavailable
                && lifecycle.active_actor_count == 0
                && lifecycle.pending_create_count == 0
                && lifecycle.pending_delete_count == 0
            {
                (status, None)
            } else {
                return Err(NativeEpisodeShardError::new(
                    "process-lifecycle status or payload is inconsistent",
                ));
            }
        } else {
            (NativeChannelStatus::NotSampled, None)
        };
    let (attention_candidates_status, attention_candidates) =
        if observation_version >= OBSERVATION_VERSION_V20 {
            decode_attention_candidates(reader)?
        } else {
            (NativeChannelStatus::NotSampled, None)
        };
    let player_present = flags & 1 != 0;
    if (observation_version >= OBSERVATION_VERSION_V20
        && ((player_present
            && !matches!(
                attention_candidates_status,
                NativeChannelStatus::Present | NativeChannelStatus::Unavailable
            ))
            || (!player_present && attention_candidates_status != NativeChannelStatus::Absent)))
        || attention_candidates.as_ref().is_some_and(|attention| {
            attention
                .lock_candidates
                .iter()
                .chain(&attention.action_candidates)
                .chain(&attention.check_candidates)
                .any(|candidate| {
                    candidate
                        .actor
                        .actor
                        .as_ref()
                        .is_none_or(|identity| !actor_identity_joins(identity))
                })
        })
    {
        return Err(NativeEpisodeShardError::new(
            "attention-candidate actor or player availability is inconsistent",
        ));
    }
    if observation_version >= OBSERVATION_VERSION_V21
        && let Some(lifecycle) = process_lifecycle.as_mut()
    {
        decode_pending_process_records(reader, lifecycle)?;
    }
    let (event_transition_status, event_transition) =
        if observation_version >= OBSERVATION_VERSION_V22 {
            decode_event_transition(reader)?
        } else {
            (NativeChannelStatus::NotSampled, None)
        };
    if event_transition.as_ref().is_some_and(|transition| {
        transition.pending_stage.as_ref().is_some_and(|pending| {
            flags & (1 << 2) == 0
                || pending.stage != next_stage_raw
                || pending.room != next_room
                || pending.layer != next_layer
                || pending.point != next_point
        }) || (flags & (1 << 2) != 0 && transition.pending_stage.is_none())
    }) {
        return Err(NativeEpisodeShardError::new(
            "event-transition pending stage disagrees with core observation",
        ));
    }
    let (clock_domains_status, clock_domains) = if observation_version >= OBSERVATION_VERSION_V23 {
        decode_clock_domains(reader)?
    } else {
        (NativeChannelStatus::NotSampled, None)
    };
    let (room_load_status, room_load) = if observation_version >= OBSERVATION_VERSION_V24 {
        decode_room_load(reader)?
    } else {
        (NativeChannelStatus::NotSampled, None)
    };
    let (warp_session_status, warp_session) = if observation_version >= OBSERVATION_VERSION_V25 {
        decode_warp_session(reader)?
    } else {
        (NativeChannelStatus::NotSampled, None)
    };
    let (resource_load_status, resource_loads) = if observation_version >= OBSERVATION_VERSION_V26 {
        decode_resource_loads(reader)?
    } else {
        (NativeChannelStatus::NotSampled, None)
    };
    let (return_restart_write_trace_status, return_restart_write_trace) =
        if observation_version >= OBSERVATION_VERSION_V28 {
            decode_return_restart_write_trace(reader)?
        } else {
            (NativeChannelStatus::NotSampled, None)
        };
    Ok(NativeLearningObservation {
        phase,
        terminal_reason,
        actor_selection,
        actors_truncated: flags & (1 << 5) != 0,
        actor_observed_count,
        boundary_index,
        simulation_tick,
        tape_frame,
        remaining_ticks,
        state_identity,
        stage,
        room,
        layer,
        point,
        next_stage: (flags & (1 << 2) != 0).then_some(next_stage_raw),
        next_room,
        next_layer,
        next_point,
        player_present: flags & 1 != 0,
        player_is_link: flags & (1 << 1) != 0,
        player_process_id,
        player_actor_name,
        player_procedure,
        player_position,
        player_velocity,
        player_forward_speed,
        player_current_angle,
        player_shape_angle,
        player_mode_flags,
        player_damage_wait_timer,
        player_ice_damage_wait_timer,
        player_sword_change_wait_timer,
        player_do_status,
        player_contacts,
        player_ground_height: (flags & (1 << 9) != 0).then_some(ground_height),
        player_roof_height: (flags & (1 << 10) != 0).then_some(roof_height),
        event_running,
        event_id,
        event_mode,
        event_status,
        event_map_tool_id,
        event_name_hash: (flags & (1 << 11) != 0).then_some(event_name_hash_raw),
        menu_flags,
        menu_procedures,
        camera_yaw_radians: (flags & (1 << 3) != 0).then_some(camera),
        collision_correction: (flags & (1 << 4) != 0).then_some(correction),
        camera_status,
        camera: mechanics_camera,
        player_action_status,
        player_action,
        player_background_collision_status,
        player_background_collision,
        player_collision_surfaces_status,
        player_collision_surfaces,
        scene_exit_status,
        scene_exit,
        player_form_present,
        player_is_wolf,
        previous_input,
        rng_version,
        rng_streams,
        talk_partner,
        grabbed_actor,
        goal,
        actors,
        dynamic_colliders_status,
        dynamic_colliders,
        player_resources_status,
        player_resources,
        player_relationships_status,
        player_relationships,
        player_collision_solver_status,
        player_collision_solver,
        event_flags,
        temporary_flags,
        temporary_event_bytes,
        dungeon_flags,
        switch_flags,
        switch_flag_room,
        runtime_file_status,
        runtime_file,
        return_place_status,
        return_place,
        restart_status,
        restart,
        return_restart_write_trace_status,
        return_restart_write_trace,
        event_handoff_status,
        event_handoff,
        message_session_status,
        message_session,
        event_queue_status,
        event_queue,
        process_lifecycle_status,
        process_lifecycle,
        attention_candidates_status,
        attention_candidates,
        event_transition_status,
        event_transition,
        clock_domains_status,
        clock_domains,
        room_load_status,
        room_load,
        warp_session_status,
        warp_session,
        resource_load_status,
        resource_loads,
    })
}
