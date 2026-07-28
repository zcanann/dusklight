//! Decode planner-facing message, event, and attention channels.

use super::*;

pub(super) fn decode_planner_runtime_channels(
    reader: &mut Reader<'_>,
    observation_version: u16,
) -> Result<PlannerRuntimeChannels, NativeEpisodeShardError> {
    let runtime_status = decode_channel_status(reader)?;
    let backing_attachment_status = decode_channel_status(reader)?;
    let no_file_raw = reader.u8()?;
    let data_num_raw = reader.u8()?;
    let attached_slot_raw = reader.i8()?;
    let attached_mask = reader.u8()?;
    let slot_statuses = [
        decode_channel_status(reader)?,
        decode_channel_status(reader)?,
        decode_channel_status(reader)?,
    ];
    if reader.u8()? != 0
        || attached_mask & !0x07 != 0
        || runtime_status != NativeChannelStatus::Present
    {
        return Err(NativeEpisodeShardError::new(
            "invalid planner runtime-file header",
        ));
    }
    let attached_physical_slot = if backing_attachment_status == NativeChannelStatus::Present {
        if no_file_raw != 0
            || data_num_raw >= 3
            || attached_slot_raw != (data_num_raw + 1) as i8
            || attached_mask != 1 << data_num_raw
        {
            return Err(NativeEpisodeShardError::new(
                "inconsistent planner backing attachment",
            ));
        }
        Some(attached_slot_raw as u8)
    } else {
        if attached_slot_raw != -1 || attached_mask != 0 {
            return Err(NativeEpisodeShardError::new(
                "unavailable planner backing has an attached slot",
            ));
        }
        None
    };
    let physical_slots = std::array::from_fn(|index| NativePhysicalSlotObservation {
        number: index as u8 + 1,
        content_status: slot_statuses[index],
        attached_to_runtime: attached_mask & (1 << index) != 0,
    });

    let return_place_status = decode_channel_status(reader)?;
    let return_room = reader.i8()?;
    let return_player_status = reader.u8()?;
    if reader.u8()? != 0 || return_place_status != NativeChannelStatus::Present {
        return Err(NativeEpisodeShardError::new(
            "invalid planner return-place header",
        ));
    }
    let return_stage = reader.fixed_string(8)?;

    let restart_status = decode_channel_status(reader)?;
    let restart_room = reader.i8()?;
    let restart_start_point = reader.i16()?;
    let restart_angle_y = reader.i16()?;
    let restart_last_angle_y = reader.i16()?;
    let restart_position = reader.f32x3()?;
    let restart_last_speed = reader.f32()?;
    let restart_room_param = reader.u32()?;
    let restart_last_mode = reader.u32()?;
    if restart_status != NativeChannelStatus::Present {
        return Err(NativeEpisodeShardError::new(
            "planner restart channel is not present",
        ));
    }

    let event_handoff_status = decode_channel_status(reader)?;
    let event_name_status = decode_channel_status(reader)?;
    let message_flow_status = decode_channel_status(reader)?;
    let pending_cleanup_status = decode_channel_status(reader)?;
    let player_control_status = decode_channel_status(reader)?;
    let no_telop_status = decode_channel_status(reader)?;
    let pre_item_no = reader.u8()?;
    let get_item_no = reader.u8()?;
    let talk_xy_type = reader.u8()?;
    let compulsory = reader.u8()?;
    let room_info_set = reader.bool()?;
    let no_telop_raw = reader.bool()?;
    let event_flags = reader.u16()?;
    let secondary_flags = reader.u16()?;
    let hind_flags = reader.u16()?;
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero event-handoff reserved field",
        ));
    }
    let skip_timer = reader.i32()?;
    let skip_parameter = reader.i32()?;
    let message_flow_raw = NativeMessageFlowObservation {
        flow_id: reader.u16()?,
        node_index: reader.u16()?,
        cut_name_hash: reader.u32()?,
    };
    let pending_cleanup_raw = reader.u32()?;
    let player_control_raw = NativePlayerControlObservation {
        mode_flags: reader.u32()?,
        do_status: reader.u8()?,
    };
    let message_cut_status = if observation_version >= OBSERVATION_VERSION_V13 {
        decode_channel_status(reader)?
    } else {
        if reader.u8()? != 0 {
            return Err(NativeEpisodeShardError::new(
                "nonzero legacy planner event reserved byte",
            ));
        }
        NativeChannelStatus::NotSampled
    };
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero planner event reserved field",
        ));
    }
    let item_partner = decode_actor_identity(reader)?;
    let event_name_raw = reader.fixed_string(64)?;
    if (message_flow_status != NativeChannelStatus::Present
        && message_flow_raw
            != (NativeMessageFlowObservation {
                flow_id: 0,
                node_index: 0,
                cut_name_hash: 0,
            }))
        || (pending_cleanup_status != NativeChannelStatus::Present && pending_cleanup_raw != 0)
        || (message_cut_status != NativeChannelStatus::Present
            && message_flow_raw.cut_name_hash != 0)
        || (player_control_status != NativeChannelStatus::Present
            && player_control_raw
                != (NativePlayerControlObservation {
                    mode_flags: 0,
                    do_status: 0,
                }))
        || (event_name_status != NativeChannelStatus::Present && !event_name_raw.is_empty())
        || (no_telop_status != NativeChannelStatus::Present && no_telop_raw)
    {
        return Err(NativeEpisodeShardError::new(
            "payload is present for an unavailable planner channel",
        ));
    }

    Ok((
        runtime_status,
        Some(NativeRuntimeFileObservation {
            no_file_raw,
            data_num_raw,
            backing_attachment_status,
            attached_physical_slot,
            physical_slots,
        }),
        return_place_status,
        Some(NativeReturnPlaceObservation {
            stage: return_stage,
            room: return_room,
            player_status: return_player_status,
        }),
        restart_status,
        Some(NativeRestartObservation {
            room: restart_room,
            start_point: restart_start_point,
            angle_y: restart_angle_y,
            position: restart_position,
            room_param: restart_room_param,
            last_speed: restart_last_speed,
            last_mode: restart_last_mode,
            last_angle_y: restart_last_angle_y,
        }),
        event_handoff_status,
        Some(NativeEventHandoffObservation {
            pre_item_no,
            get_item_no,
            event_flags,
            secondary_flags,
            hind_flags,
            talk_xy_type,
            compulsory,
            room_info_set,
            skip_timer,
            skip_parameter,
            item_partner,
            event_name_status,
            event_name: (event_name_status == NativeChannelStatus::Present)
                .then_some(event_name_raw),
            message_flow_status,
            message_flow: (message_flow_status == NativeChannelStatus::Present)
                .then_some(message_flow_raw),
            message_cut_status,
            pending_cleanup_status,
            pending_cleanup_flags: (pending_cleanup_status == NativeChannelStatus::Present)
                .then_some(pending_cleanup_raw),
            player_control_status,
            player_control: (player_control_status == NativeChannelStatus::Present)
                .then_some(player_control_raw),
            no_telop_status,
            no_telop: (no_telop_status == NativeChannelStatus::Present).then_some(no_telop_raw),
        }),
    ))
}

pub(super) fn decode_message_session(
    reader: &mut Reader<'_>,
) -> Result<(NativeChannelStatus, Option<NativeMessageSessionObservation>), NativeEpisodeShardError>
{
    const TALK_NOW: u16 = 1 << 0;
    const TALK_MESSAGE: u16 = 1 << 1;
    const AUTO_MESSAGE: u16 = 1 << 2;
    const KILL_PENDING: u16 = 1 << 3;
    const CAMERA_CANCEL: u16 = 1 << 4;
    const SEND: u16 = 1 << 5;
    const SEND_CONTROL: u16 = 1 << 6;
    const KNOWN_FLAGS: u16 =
        TALK_NOW | TALK_MESSAGE | AUTO_MESSAGE | KILL_PENDING | CAMERA_CANCEL | SEND | SEND_CONTROL;

    let status = decode_channel_status(reader)?;
    if reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero message-session reserved byte",
        ));
    }
    let procedure = reader.u16()?;
    let message_id = reader.u32()?;
    let message_index = reader.i32()?;
    let node_index = reader.u16()?;
    let flow_id = reader.i16()?;
    let selection_count = reader.u8()?;
    let selection_cursor = reader.u8()?;
    let selection_push = reader.u8()?;
    let output_type = reader.u8()?;
    let flags = reader.u16()?;
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero message-session reserved field",
        ));
    }
    let talk_actor = decode_actor_identity(reader)?;
    if flags & !KNOWN_FLAGS != 0
        || matches!(status, NativeChannelStatus::NotSampled)
        || (status != NativeChannelStatus::Present
            && (procedure != 0
                || message_id != 0
                || message_index != 0
                || node_index != 0
                || flow_id != 0
                || selection_count != 0
                || selection_cursor != 0
                || selection_push != 0
                || output_type != 0
                || flags != 0
                || talk_actor.present))
    {
        return Err(NativeEpisodeShardError::new(
            "message-session status and payload disagree",
        ));
    }
    Ok((
        status,
        (status == NativeChannelStatus::Present).then_some(NativeMessageSessionObservation {
            procedure,
            message_id,
            message_index,
            node_index,
            flow_id,
            selection_count,
            selection_cursor,
            selection_push,
            output_type,
            talk_now: flags & TALK_NOW != 0,
            talk_message: flags & TALK_MESSAGE != 0,
            auto_message: flags & AUTO_MESSAGE != 0,
            kill_pending: flags & KILL_PENDING != 0,
            camera_cancel: flags & CAMERA_CANCEL != 0,
            send: flags & SEND != 0,
            send_control: flags & SEND_CONTROL != 0,
            talk_actor,
        }),
    ))
}

pub(super) fn decode_event_actor_reference(
    reader: &mut Reader<'_>,
) -> Result<NativeEventActorReferenceObservation, NativeEpisodeShardError> {
    let status = decode_channel_status(reader)?;
    if reader.u8()? != 0 || reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero event-actor-reference reserved field",
        ));
    }
    let identity = decode_actor_identity(reader)?;
    if (status == NativeChannelStatus::Present) != identity.present {
        return Err(NativeEpisodeShardError::new(
            "event actor-reference status and identity disagree",
        ));
    }
    Ok(NativeEventActorReferenceObservation {
        status,
        actor: identity.present.then_some(identity),
    })
}

pub(super) fn decode_event_queue(
    reader: &mut Reader<'_>,
) -> Result<(NativeChannelStatus, Option<NativeEventQueueObservation>), NativeEpisodeShardError> {
    const MAXIMUM_PENDING_ORDERS: usize = 8;
    let status = decode_channel_status(reader)?;
    let pending_count = usize::from(reader.u8()?);
    let skip_registered = reader.bool()?;
    if reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero event-queue reserved byte",
        ));
    }
    if pending_count > MAXIMUM_PENDING_ORDERS {
        return Err(NativeEpisodeShardError::new(
            "event-queue pending count exceeds the native queue capacity",
        ));
    }
    let mut pending_orders = Vec::with_capacity(pending_count);
    for _ in 0..pending_count {
        let event_type = reader.u16()?;
        let flags = reader.u16()?;
        let hind_flags = reader.u16()?;
        let event_id = reader.i16()?;
        let priority = reader.u16()?;
        let map_tool_id = reader.u8()?;
        if reader.u8()? != 0 {
            return Err(NativeEpisodeShardError::new(
                "nonzero pending-event-order reserved byte",
            ));
        }
        if !(event_type <= 7 || (10..=13).contains(&event_type)) || priority == 0 {
            return Err(NativeEpisodeShardError::new(
                "pending event order has an unknown type or zero priority",
            ));
        }
        pending_orders.push(NativePendingEventOrderObservation {
            event_type,
            flags,
            hind_flags,
            event_id,
            priority,
            map_tool_id,
            request_actor: decode_event_actor_reference(reader)?,
            target_actor: decode_event_actor_reference(reader)?,
        });
    }
    if pending_orders
        .windows(2)
        .any(|pair| pair[0].priority > pair[1].priority)
    {
        return Err(NativeEpisodeShardError::new(
            "event-queue orders are not in semantic priority order",
        ));
    }
    let event_queue = NativeEventQueueObservation {
        pending_orders,
        active_request_actor: decode_event_actor_reference(reader)?,
        active_target_actor: decode_event_actor_reference(reader)?,
        active_talk_actor: decode_event_actor_reference(reader)?,
        active_item_actor: decode_event_actor_reference(reader)?,
        active_door_actor: decode_event_actor_reference(reader)?,
        change_actor: decode_event_actor_reference(reader)?,
        skip_registered,
        skip_actor: decode_event_actor_reference(reader)?,
    };
    let participant_references = [
        &event_queue.active_request_actor,
        &event_queue.active_target_actor,
        &event_queue.active_talk_actor,
        &event_queue.active_item_actor,
        &event_queue.active_door_actor,
        &event_queue.change_actor,
        &event_queue.skip_actor,
    ];
    let references = participant_references.into_iter().chain(
        event_queue
            .pending_orders
            .iter()
            .flat_map(|order| [&order.request_actor, &order.target_actor]),
    );
    let channel_present = status == NativeChannelStatus::Present;
    if references.into_iter().any(|reference| {
        if channel_present {
            reference.status == NativeChannelStatus::NotSampled
        } else {
            reference.status != NativeChannelStatus::NotSampled
        }
    }) || (!event_queue.skip_registered
        && event_queue.skip_actor.status == NativeChannelStatus::Present)
    {
        return Err(NativeEpisodeShardError::new(
            "event-queue actor-reference availability is inconsistent",
        ));
    }
    if !matches!(
        status,
        NativeChannelStatus::Present | NativeChannelStatus::Unavailable
    ) || (!channel_present && (!event_queue.pending_orders.is_empty() || skip_registered))
    {
        return Err(NativeEpisodeShardError::new(
            "event-queue status and payload disagree",
        ));
    }
    Ok((
        status,
        (status == NativeChannelStatus::Present).then_some(event_queue),
    ))
}

pub(super) fn decode_attention_candidate(
    reader: &mut Reader<'_>,
) -> Result<NativeAttentionCandidateObservation, NativeEpisodeShardError> {
    let weight = reader.f32()?;
    let distance = reader.f32()?;
    let angle = reader.i16()?;
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero attention-candidate reserved field",
        ));
    }
    let attention_type = reader.u32()?;
    let actor = decode_event_actor_reference(reader)?;
    if distance < 0.0
        || attention_type >= 13
        || actor.status != NativeChannelStatus::Present
        || actor.actor.is_none()
    {
        return Err(NativeEpisodeShardError::new("invalid attention candidate"));
    }
    Ok(NativeAttentionCandidateObservation {
        actor,
        weight,
        distance,
        angle,
        attention_type,
    })
}

pub(super) fn decode_attention_candidates(
    reader: &mut Reader<'_>,
) -> Result<
    (
        NativeChannelStatus,
        Option<NativeAttentionCandidatesObservation>,
    ),
    NativeEpisodeShardError,
> {
    let status = decode_channel_status(reader)?;
    let attention_status = reader.u8()?;
    let lock_count = usize::from(reader.u8()?);
    let lock_offset = reader.u8()?;
    let action_count = usize::from(reader.u8()?);
    let action_offset = reader.u8()?;
    let check_count = usize::from(reader.u8()?);
    let check_offset = reader.u8()?;
    let player_attention_flags = reader.u32()?;
    let attention_block_timer = reader.i32()?;
    let count_offset_valid = |count: usize, offset: u8, capacity: usize| {
        count <= capacity
            && ((count == 0 && offset == 0) || (count != 0 && usize::from(offset) < count))
    };
    if !count_offset_valid(lock_count, lock_offset, 8)
        || !count_offset_valid(action_count, action_offset, 4)
        || !count_offset_valid(check_count, check_offset, 4)
    {
        return Err(NativeEpisodeShardError::new(
            "attention-candidate count or offset is invalid",
        ));
    }
    let mut decode_list = |count: usize| {
        (0..count)
            .map(|_| decode_attention_candidate(reader))
            .collect::<Result<Vec<_>, _>>()
    };
    let lock_candidates = decode_list(lock_count)?;
    let action_candidates = decode_list(action_count)?;
    let check_candidates = decode_list(check_count)?;
    let channel_present = status == NativeChannelStatus::Present;
    if !matches!(
        status,
        NativeChannelStatus::Present
            | NativeChannelStatus::Unavailable
            | NativeChannelStatus::Absent
    ) || (!channel_present
        && (attention_status != 0
            || player_attention_flags != 0
            || attention_block_timer != 0
            || !lock_candidates.is_empty()
            || !action_candidates.is_empty()
            || !check_candidates.is_empty()))
    {
        return Err(NativeEpisodeShardError::new(
            "attention-candidate status and payload disagree",
        ));
    }
    Ok((
        status,
        channel_present.then_some(NativeAttentionCandidatesObservation {
            player_attention_flags,
            attention_status,
            attention_block_timer,
            lock_offset,
            action_offset,
            check_offset,
            lock_candidates,
            action_candidates,
            check_candidates,
        }),
    ))
}

pub(super) fn decode_event_transition(
    reader: &mut Reader<'_>,
) -> Result<
    (
        NativeChannelStatus,
        Option<NativeEventTransitionObservation>,
    ),
    NativeEpisodeShardError,
> {
    let status = decode_channel_status(reader)?;
    let event_data_loaded = reader.bool()?;
    let current_status = decode_channel_status(reader)?;
    let goal_status = decode_channel_status(reader)?;
    let next_status = decode_channel_status(reader)?;
    let wipe = reader.i8()?;
    let wipe_speed = reader.u8()?;
    if reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero event-transition reserved byte",
        ));
    }
    let camera_play = reader.i32()?;
    let current_event_id = reader.i16()?;
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero event-transition reserved field",
        ));
    }
    let current_event_type = reader.i32()?;
    let current_event_room = reader.i32()?;
    let goal = reader.f32x3()?;
    let stage = reader.fixed_name()?;
    let next_room = reader.i8()?;
    let next_layer = reader.i8()?;
    let next_point = reader.i16()?;

    let current_empty =
        current_event_id == -1 && current_event_type == 0 && current_event_room == -1;
    let goal_empty = goal == [0.0; 3];
    let next_empty = stage.is_empty()
        && next_room == -1
        && next_layer == -1
        && next_point == -1
        && wipe == 0
        && wipe_speed == 0;
    if !matches!(
        status,
        NativeChannelStatus::Present | NativeChannelStatus::Unavailable
    ) || (status == NativeChannelStatus::Present
        && !matches!(
            (current_status, goal_status),
            (NativeChannelStatus::Present, NativeChannelStatus::Present)
                | (NativeChannelStatus::Absent, NativeChannelStatus::Absent)
        ))
        || (current_status != NativeChannelStatus::Present && !current_empty)
        || (goal_status != NativeChannelStatus::Present && !goal_empty)
        || !matches!(
            next_status,
            NativeChannelStatus::Present
                | NativeChannelStatus::Absent
                | NativeChannelStatus::NotSampled
        )
        || (next_status != NativeChannelStatus::Present && !next_empty)
        || (next_status == NativeChannelStatus::Present && stage.is_empty())
        || (status == NativeChannelStatus::Unavailable
            && (event_data_loaded
                || camera_play != 0
                || current_status != NativeChannelStatus::NotSampled
                || goal_status != NativeChannelStatus::NotSampled
                || next_status != NativeChannelStatus::NotSampled
                || !current_empty
                || !goal_empty
                || !next_empty))
    {
        return Err(NativeEpisodeShardError::new(
            "event-transition status and payload disagree",
        ));
    }

    let current_event =
        (current_status == NativeChannelStatus::Present).then_some(NativeCurrentEventObservation {
            event_id: current_event_id,
            event_type: current_event_type,
            room: current_event_room,
            goal,
        });
    let pending_stage =
        (next_status == NativeChannelStatus::Present).then_some(NativePendingStageObservation {
            stage,
            room: next_room,
            layer: next_layer,
            point: next_point,
            wipe,
            wipe_speed,
        });
    Ok((
        status,
        (status == NativeChannelStatus::Present).then_some(NativeEventTransitionObservation {
            event_data_loaded,
            camera_play,
            current_event,
            pending_stage,
        }),
    ))
}
