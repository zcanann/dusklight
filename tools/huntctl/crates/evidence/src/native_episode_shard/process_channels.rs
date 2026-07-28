//! Decode pending process lifecycle and tactic actor observations.

use super::*;

pub(super) fn decode_pending_process_state(
    reader: &mut Reader<'_>,
) -> Result<NativePendingProcessState, NativeEpisodeShardError> {
    let state = NativePendingProcessState {
        runtime_generation: reader.u32()?,
        process_name: reader.i16()?,
        profile_name: reader.i16()?,
        process_type: reader.i32()?,
        process_subtype: reader.i32()?,
        parameters: reader.u32()?,
        init_state: reader.i8()?,
        create_phase: reader.u8()?,
    };
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero pending-process reserved bytes",
        ));
    }
    Ok(state)
}

pub(super) fn pending_process_state_is_empty(state: &NativePendingProcessState) -> bool {
    state
        == &(NativePendingProcessState {
            runtime_generation: 0,
            process_name: -1,
            profile_name: -1,
            process_type: 0,
            process_subtype: 0,
            parameters: 0,
            init_state: 0,
            create_phase: 0,
        })
}

pub(super) fn decode_pending_process_records(
    reader: &mut Reader<'_>,
    lifecycle: &mut NativeProcessLifecycleObservation,
) -> Result<(), NativeEpisodeShardError> {
    let create_count = usize::try_from(lifecycle.pending_create_count)
        .map_err(|_| NativeEpisodeShardError::new("pending-create count overflow"))?;
    let delete_count = usize::try_from(lifecycle.pending_delete_count)
        .map_err(|_| NativeEpisodeShardError::new("pending-delete count overflow"))?;
    if create_count > MAX_PENDING_PROCESS_RECORDS || delete_count > MAX_PENDING_PROCESS_RECORDS {
        return Err(NativeEpisodeShardError::new(
            "pending-process record count exceeds decoder bound",
        ));
    }
    lifecycle.pending_creates.reserve(create_count);
    for _ in 0..create_count {
        let runtime_generation = reader.u32()?;
        let flags = reader.u8()?;
        let process_status = decode_channel_status(reader)?;
        if flags & !0x3 != 0 || reader.u16()? != 0 {
            return Err(NativeEpisodeShardError::new(
                "invalid pending-create header",
            ));
        }
        let process_state = decode_pending_process_state(reader)?;
        let process = match process_status {
            NativeChannelStatus::Present
                if process_state.runtime_generation == runtime_generation =>
            {
                Some(process_state)
            }
            NativeChannelStatus::Absent if pending_process_state_is_empty(&process_state) => None,
            _ => {
                return Err(NativeEpisodeShardError::new(
                    "pending-create process state is inconsistent",
                ));
            }
        };
        lifecycle
            .pending_creates
            .push(NativePendingCreateObservation {
                runtime_generation,
                doing: flags & 1 != 0,
                cancelled: flags & 2 != 0,
                process_status,
                process,
            });
    }
    lifecycle.pending_deletes.reserve(delete_count);
    for _ in 0..delete_count {
        let process = decode_pending_process_state(reader)?;
        let timer = reader.i16()?;
        if reader.u16()? != 0 {
            return Err(NativeEpisodeShardError::new(
                "nonzero pending-delete reserved bytes",
            ));
        }
        lifecycle
            .pending_deletes
            .push(NativePendingDeleteObservation { process, timer });
    }
    Ok(())
}

pub(super) fn empty_actor_identity() -> NativeActorIdentity {
    NativeActorIdentity {
        present: false,
        runtime_generation: 0,
        actor_name: 0,
        set_id: 0,
        home_room: 0,
        current_room: 0,
        home_position: None,
    }
}

pub(super) fn empty_trace_actor_identity() -> NativeTraceActorIdentity {
    NativeTraceActorIdentity {
        runtime_generation: 0,
        actor_name: 0,
        set_id: 0,
        home_room: 0,
        current_room: 0,
        home_position: [0.0; 3],
    }
}

pub(super) fn decode_tactic_actor(
    reader: &mut Reader<'_>,
) -> Result<NativeActorObservation, NativeEpisodeShardError> {
    Ok(NativeActorObservation {
        runtime_generation: reader.u64()?,
        base_state_available: true,
        actor_type: 0,
        process_subtype: 0,
        parent_runtime_generation: 0,
        parameters: 0,
        status: 0,
        condition: 0,
        actor_name: reader.i16()?,
        profile_name: 0,
        set_id: reader.u16()?,
        home_room: reader.i8()?,
        old_room: 0,
        current_room: reader.i8()?,
        group: 0,
        argument: 0,
        pause_flag: 0,
        process_init_state: 0,
        process_create_phase: 0,
        cull_type: 0,
        demo_actor_id: 0,
        carry_type: 0,
        heap_present: false,
        model_present: false,
        joint_collision_present: false,
        health: 0,
        position: reader.f32x3()?,
        home_position: [0.0; 3],
        old_position: [0.0; 3],
        velocity: [0.0; 3],
        forward_speed: 0.0,
        scale: [0.0; 3],
        gravity: 0.0,
        max_fall_speed: 0.0,
        eye_position: [0.0; 3],
        home_angle: [0; 3],
        old_angle: [0; 3],
        current_angle: [0; 3],
        shape_angle: [0; 3],
        attention: None,
        event_participation: None,
        return_place_writer: None,
        enemy_base: None,
        trigger_volume: None,
        door20: None,
    })
}

pub(super) fn decode_tactic_observation(
    reader: &mut Reader<'_>,
) -> Result<NativeLearningObservation, NativeEpisodeShardError> {
    let phase = match reader.u8()? {
        1 => NativeObservationPhase::PreInput,
        2 => NativeObservationPhase::PostSimulation,
        _ => {
            return Err(NativeEpisodeShardError::new(
                "invalid tactic observation phase",
            ));
        }
    };
    let terminal_reason = match reader.u8()? {
        0 => NativeTerminalReason::None,
        _ => {
            return Err(NativeEpisodeShardError::new(
                "compact tactic observation cannot be terminal",
            ));
        }
    };
    let actor_count = usize::from(reader.u16()?);
    let flags = reader.u16()?;
    let actor_observed_count = reader.u32()?;
    let remaining_ticks = reader.u32()?;
    let boundary_index = reader.u64()?;
    let simulation_tick = reader.u64()?;
    let tape_frame = reader.u64()?;
    let state_identity = reader.bytes(16)?.try_into().expect("exact length");
    let actors_truncated = flags & (1 << 6) != 0;
    if flags & !0x7f != 0
        || flags & (1 << 5) != 0 && flags & (1 << 4) == 0
        || actor_count > MAX_ACTORS
        || actor_observed_count < actor_count as u32
        || actors_truncated != (actor_observed_count > actor_count as u32)
    {
        return Err(NativeEpisodeShardError::new(
            "inconsistent compact tactic observation header",
        ));
    }
    let stage = reader.fixed_name()?;
    let room = reader.i8()?;
    let layer = reader.i8()?;
    let point = reader.i16()?;
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
        return Err(NativeEpisodeShardError::new(
            "invalid compact tactic player contacts",
        ));
    }
    let camera_yaw = reader.f32()?;
    let collision_correction = [reader.f32()?, reader.f32()?];
    let previous_input = decode_pad(reader)?;
    let player_action_status = decode_channel_status(reader)?;
    if reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero compact tactic action reserved byte",
        ));
    }
    let mut lanes = Vec::with_capacity(6);
    for _ in 0..6 {
        lanes.push(NativeAnimationLane {
            resource_id: reader.u16()?,
            frame: reader.f32()?,
            rate: 0.0,
        });
    }
    if player_action_status != NativeChannelStatus::Present
        && lanes
            .iter()
            .any(|lane| lane.resource_id != 0 || lane.frame != 0.0)
    {
        return Err(NativeEpisodeShardError::new(
            "compact tactic action payload exists for an unavailable channel",
        ));
    }
    let under_animations = lanes[..3].try_into().expect("exact animation lane count");
    let upper_animations = lanes[3..].try_into().expect("exact animation lane count");
    let player_action = (player_action_status == NativeChannelStatus::Present).then(|| {
        NativePlayerActionObservation {
            procedure_id: player_procedure,
            mode_flags: player_mode_flags,
            procedure_context_raw: [0; 6],
            damage_wait_timer: player_damage_wait_timer,
            sword_at_up_time: 0,
            ice_damage_wait_timer: player_ice_damage_wait_timer,
            sword_change_wait_timer: player_sword_change_wait_timer,
            under_animations,
            upper_animations,
            flags: 0,
            do_status: player_do_status,
            talk_partner: empty_trace_actor_identity(),
            grabbed_actor: empty_trace_actor_identity(),
        }
    });
    let mut actors = Vec::with_capacity(actor_count);
    for _ in 0..actor_count {
        actors.push(decode_tactic_actor(reader)?);
    }
    Ok(NativeLearningObservation {
        phase,
        terminal_reason,
        actor_selection: if actors_truncated {
            NativeActorSelectionRule::LowestRuntimeGeneration
        } else {
            NativeActorSelectionRule::Complete
        },
        actors_truncated,
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
        next_stage: None,
        next_room: 0,
        next_layer: 0,
        next_point: 0,
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
        player_ground_height: None,
        player_roof_height: None,
        event_running: false,
        event_id: 0,
        event_mode: 0,
        event_status: 0,
        event_map_tool_id: 0,
        event_name_hash: None,
        menu_flags: 0,
        menu_procedures: [0; 5],
        camera_yaw_radians: (flags & (1 << 2) != 0).then_some(camera_yaw),
        collision_correction: (flags & (1 << 3) != 0).then_some(collision_correction),
        camera_status: NativeChannelStatus::NotSampled,
        camera: None,
        player_action_status,
        player_action,
        player_background_collision_status: NativeChannelStatus::NotSampled,
        player_background_collision: None,
        player_collision_surfaces_status: NativeChannelStatus::NotSampled,
        player_collision_surfaces: None,
        scene_exit_status: NativeChannelStatus::NotSampled,
        scene_exit: None,
        player_form_present: flags & (1 << 4) != 0,
        player_is_wolf: flags & (1 << 5) != 0,
        previous_input,
        rng_version: 0,
        rng_streams: Vec::new(),
        talk_partner: empty_actor_identity(),
        grabbed_actor: empty_actor_identity(),
        goal: NativeGoalObservation {
            configured: false,
            reached: false,
            requested_count: 0,
            hit_count: 0,
            stable_ticks: 0,
            consecutive_ticks: 0,
            sequence_steps: 0,
            sequence_next_step: 0,
            sequence_within_ticks: 0,
            sequence_elapsed_ticks: 0,
            first_hit_tick: None,
        },
        actors,
        dynamic_colliders_status: NativeChannelStatus::NotSampled,
        dynamic_colliders: Vec::new(),
        player_resources_status: NativeChannelStatus::NotSampled,
        player_resources: None,
        player_relationships_status: NativeChannelStatus::NotSampled,
        player_relationships: None,
        player_collision_solver_status: NativeChannelStatus::NotSampled,
        player_collision_solver: None,
        event_flags: None,
        temporary_flags: None,
        temporary_event_bytes: None,
        dungeon_flags: None,
        switch_flags: None,
        switch_flag_room: room,
        runtime_file_status: NativeChannelStatus::NotSampled,
        runtime_file: None,
        return_place_status: NativeChannelStatus::NotSampled,
        return_place: None,
        restart_status: NativeChannelStatus::NotSampled,
        restart: None,
        return_restart_write_trace_status: NativeChannelStatus::NotSampled,
        return_restart_write_trace: None,
        event_handoff_status: NativeChannelStatus::NotSampled,
        event_handoff: None,
        message_session_status: NativeChannelStatus::NotSampled,
        message_session: None,
        event_queue_status: NativeChannelStatus::NotSampled,
        event_queue: None,
        process_lifecycle_status: NativeChannelStatus::NotSampled,
        process_lifecycle: None,
        attention_candidates_status: NativeChannelStatus::NotSampled,
        attention_candidates: None,
        event_transition_status: NativeChannelStatus::NotSampled,
        event_transition: None,
        clock_domains_status: NativeChannelStatus::NotSampled,
        clock_domains: None,
        room_load_status: NativeChannelStatus::NotSampled,
        room_load: None,
        warp_session_status: NativeChannelStatus::NotSampled,
        warp_session: None,
        resource_load_status: NativeChannelStatus::NotSampled,
        resource_loads: None,
    })
}
