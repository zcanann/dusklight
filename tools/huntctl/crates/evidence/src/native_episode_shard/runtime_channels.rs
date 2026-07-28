//! Decode clock, room, warp, resource-load, and restart channels.

use super::*;

pub(super) fn decode_clock_domains(
    reader: &mut Reader<'_>,
) -> Result<(NativeChannelStatus, Option<NativeClockDomainObservation>), NativeEpisodeShardError> {
    let status = decode_channel_status(reader)?;
    let flags = reader.u8()?;
    let scene_pause_timer = reader.i8()?;
    let scene_next_pause_timer = reader.i8()?;
    let demo_status = decode_channel_status(reader)?;
    let timer_status = decode_channel_status(reader)?;
    if flags & !0x0f != 0 || reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "noncanonical clock-domain flags or reserved field",
        ));
    }
    let framework_frames = reader.u32()?;
    let gameplay_frames = reader.u32()?;
    let demo_mode = reader.i32()?;
    let demo_frame = reader.u32()?;
    let demo_frame_no_message = reader.u32()?;
    let demo_flags = reader.u32()?;
    let timer_mode = reader.i32()?;
    let timer_now_ms = reader.i32()?;
    let timer_limit_ms = reader.i32()?;

    let present = status == NativeChannelStatus::Present;
    let demo_present = demo_status == NativeChannelStatus::Present;
    let demo_empty =
        demo_mode == 0 && demo_frame == 0 && demo_frame_no_message == 0 && demo_flags == 0;
    let timer_present = timer_status == NativeChannelStatus::Present;
    let timer_empty = timer_mode == -1 && timer_now_ms == 0 && timer_limit_ms == 0;
    let outer_empty = flags == 0
        && scene_pause_timer == 0
        && scene_next_pause_timer == 0
        && framework_frames == 0
        && gameplay_frames == 0
        && demo_status == NativeChannelStatus::NotSampled
        && demo_empty
        && timer_status == NativeChannelStatus::NotSampled
        && timer_empty;
    if !matches!(
        status,
        NativeChannelStatus::Present | NativeChannelStatus::Unavailable
    ) || (present
        && (!matches!(
            demo_status,
            NativeChannelStatus::Present | NativeChannelStatus::Absent
        ) || !matches!(
            timer_status,
            NativeChannelStatus::Present
                | NativeChannelStatus::Absent
                | NativeChannelStatus::Unavailable
        )))
        || (!demo_present && !demo_empty)
        || (demo_present && (demo_mode == 0 || demo_frame_no_message > demo_frame))
        || (timer_present && timer_mode < 0)
        || (!timer_present && !timer_empty)
        || (!present && !outer_empty)
    {
        return Err(NativeEpisodeShardError::new(
            "clock-domain status and payload disagree",
        ));
    }

    Ok((
        status,
        present.then_some(NativeClockDomainObservation {
            framework_frames,
            gameplay_frames,
            global_pause: flags & (1 << 0) != 0,
            scene_paused: flags & (1 << 1) != 0,
            scene_pause_timer,
            scene_next_pause_timer,
            overlap_request_active: flags & (1 << 2) != 0,
            overlap_fadeout_peek: flags & (1 << 3) != 0,
            demo_status,
            demo_mode,
            demo_frame,
            demo_frame_no_message,
            demo_flags,
            timer_status,
            timer_mode,
            timer_now_ms,
            timer_limit_ms,
        }),
    ))
}

pub(super) fn decode_room_load(
    reader: &mut Reader<'_>,
) -> Result<(NativeChannelStatus, Option<NativeRoomLoadObservation>), NativeEpisodeShardError> {
    const ROOM_COUNT: usize = 64;
    const MEMORY_BLOCK_COUNT: i8 = 19;
    let status = decode_channel_status(reader)?;
    let flags = reader.u8()?;
    let room_read = reader.i8()?;
    let stay_room = reader.i8()?;
    let old_stay_room = reader.i8()?;
    let next_stay_room = reader.i8()?;
    if flags & !0x03 != 0 || reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "noncanonical room-load flags or reserved field",
        ));
    }

    let mut rooms = Vec::with_capacity(ROOM_COUNT);
    for room in 0..ROOM_COUNT {
        let status_flags = reader.u8()?;
        let room_flags = reader.u8()?;
        let zone_count = reader.i8()?;
        let zone = reader.i8()?;
        let memory_block = reader.i8()?;
        let region = reader.u8()?;
        let scene_status = decode_channel_status(reader)?;
        if room_flags & !0x03 != 0 || reader.u8()? != 0 {
            return Err(NativeEpisodeShardError::new(
                "noncanonical room-load entry flags or reserved byte",
            ));
        }
        let scene_phase = reader.i32()?;
        let scene_phase_active = room_flags & (1 << 1) != 0;
        let scene_present = scene_status == NativeChannelStatus::Present;
        if !matches!(
            scene_status,
            NativeChannelStatus::Present
                | NativeChannelStatus::Absent
                | NativeChannelStatus::Unavailable
        ) || zone < -1
            || !(-1..MEMORY_BLOCK_COUNT).contains(&memory_block)
            || (scene_present && (status_flags == 0 || !(0..=4).contains(&scene_phase)))
            || (!scene_present && (scene_phase != 0 || scene_phase_active))
        {
            return Err(NativeEpisodeShardError::new(
                "room-load entry status and payload disagree",
            ));
        }
        rooms.push(NativeRoomLoadEntryObservation {
            room: u8::try_from(room).expect("fixed room index fits u8"),
            status_flags,
            draw: room_flags & 1 != 0,
            zone_count,
            zone,
            memory_block,
            region,
            scene_status,
            scene_phase,
            scene_phase_active,
        });
    }

    let valid_room = |room: i8| (-1..64).contains(&room);
    let outer_empty = flags == 0
        && room_read == -1
        && stay_room == -1
        && old_stay_room == -1
        && next_stay_room == -1
        && rooms.iter().all(|room| {
            room.status_flags == 0
                && !room.draw
                && room.zone_count == 0
                && room.zone == -1
                && room.memory_block == -1
                && room.region == 0
                && room.scene_status == NativeChannelStatus::Absent
                && room.scene_phase == 0
                && !room.scene_phase_active
        });
    let present = status == NativeChannelStatus::Present;
    if !matches!(
        status,
        NativeChannelStatus::Present | NativeChannelStatus::Unavailable
    ) || (present
        && (!valid_room(room_read)
            || !valid_room(stay_room)
            || !valid_room(old_stay_room)
            || !valid_room(next_stay_room)))
        || (!present && !outer_empty)
    {
        return Err(NativeEpisodeShardError::new(
            "room-load status and payload disagree",
        ));
    }

    Ok((
        status,
        present.then_some(NativeRoomLoadObservation {
            room_read,
            stay_room,
            old_stay_room,
            next_stay_room,
            no_change_room: flags & 1 != 0,
            time_pass: flags & (1 << 1) != 0,
            rooms,
        }),
    ))
}

pub(super) fn decode_warp_session(
    reader: &mut Reader<'_>,
) -> Result<(NativeChannelStatus, Option<NativeWarpSessionObservation>), NativeEpisodeShardError> {
    let status = decode_channel_status(reader)?;
    let request_kind = reader.u8()?;
    let selection_status = decode_channel_status(reader)?;
    let return_status = decode_channel_status(reader)?;
    let target_status = decode_channel_status(reader)?;
    let selected_status = decode_channel_status(reader)?;
    let transport_match = reader.bool()?;
    if reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero warp-session reserved byte",
        ));
    }

    let selection_stage = reader.fixed_name()?;
    let selection_position = reader.f32x3()?;
    let selection_angle = reader.i16()?;
    let selection_room = reader.i8()?;
    let selection_parameter = reader.u8()?;
    let selection_player = reader.u8()?;
    if reader.u8()? != 0 || reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero warp-selection reserved field",
        ));
    }

    let return_stage = reader.fixed_name()?;
    let return_position = reader.f32x3()?;
    let return_angle = reader.i16()?;
    let return_room = reader.i8()?;
    let return_accept_stage = reader.i8()?;
    let target_point = reader.u8()?;
    let selected_point = reader.u8()?;
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero warp-point reserved field",
        ));
    }

    let present = status == NativeChannelStatus::Present;
    let selection_present = selection_status == NativeChannelStatus::Present;
    let selection_empty = selection_stage.is_empty()
        && selection_position == [0.0; 3]
        && selection_angle == 0
        && selection_room == -1
        && selection_parameter == 0
        && selection_player == 0;
    let return_present = return_status == NativeChannelStatus::Present;
    let return_empty = return_stage.is_empty()
        && return_position == [0.0; 3]
        && return_angle == 0
        && return_room == -1
        && return_accept_stage == -1;
    let target_present = target_status == NativeChannelStatus::Present;
    let selected_present = selected_status == NativeChannelStatus::Present;
    let outer_empty = request_kind == 0
        && selection_status == NativeChannelStatus::NotSampled
        && selection_empty
        && return_status == NativeChannelStatus::NotSampled
        && return_empty
        && target_status == NativeChannelStatus::NotSampled
        && target_point == 0
        && selected_status == NativeChannelStatus::NotSampled
        && selected_point == 0
        && !transport_match;
    if !matches!(
        status,
        NativeChannelStatus::Present | NativeChannelStatus::Unavailable
    ) || (present
        && (request_kind > 3
            || !matches!(
                selection_status,
                NativeChannelStatus::Present | NativeChannelStatus::Absent
            )
            || !matches!(
                return_status,
                NativeChannelStatus::Present | NativeChannelStatus::Absent
            )
            || !matches!(
                target_status,
                NativeChannelStatus::Present | NativeChannelStatus::Absent
            )
            || !matches!(
                selected_status,
                NativeChannelStatus::Present | NativeChannelStatus::Absent
            )))
        || (selection_present
            && (request_kind != 3
                || selection_stage.is_empty()
                || selection_stage.len() >= 8
                || !(0..64).contains(&selection_room)))
        || (!selection_present && !selection_empty)
        || (return_present
            && (return_stage.is_empty()
                || return_stage.len() >= 8
                || !(0..64).contains(&return_room)
                || return_accept_stage < 0))
        || (!return_present && !return_empty)
        || (!target_present && target_point != 0)
        || (!selected_present && selected_point != 0)
        || transport_match != (target_present && selected_present && target_point == selected_point)
        || (!present && !outer_empty)
    {
        return Err(NativeEpisodeShardError::new(
            "warp-session status and payload disagree",
        ));
    }

    Ok((
        status,
        present.then_some(NativeWarpSessionObservation {
            request_kind,
            selection: selection_present.then_some(NativeWarpSelectionObservation {
                stage: selection_stage,
                position: selection_position,
                angle: selection_angle,
                room: selection_room,
                parameter: selection_parameter,
                player: selection_player,
            }),
            return_mark: return_present.then_some(NativeWarpReturnMarkObservation {
                stage: return_stage,
                position: return_position,
                angle: return_angle,
                room: return_room,
                accept_stage: return_accept_stage,
            }),
            target_point: target_present.then_some(target_point),
            selected_point: selected_present.then_some(selected_point),
            transport_match,
        }),
    ))
}

pub(super) fn decode_resource_loads(
    reader: &mut Reader<'_>,
) -> Result<(NativeChannelStatus, Option<NativeResourceLoadObservation>), NativeEpisodeShardError> {
    const OBJECT_CAPACITY: u16 = 128;
    const STAGE_CAPACITY: u16 = 64;
    const MAXIMUM_ENTRIES: usize = OBJECT_CAPACITY as usize + STAGE_CAPACITY as usize;

    let status = decode_channel_status(reader)?;
    if reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero resource-load reserved byte",
        ));
    }
    let entry_count = reader.u16()?;
    let object_count = reader.u16()?;
    let stage_count = reader.u16()?;
    let object_capacity = reader.u16()?;
    let stage_capacity = reader.u16()?;
    let present = status == NativeChannelStatus::Present;
    if !matches!(
        status,
        NativeChannelStatus::Present | NativeChannelStatus::Unavailable
    ) || object_capacity != OBJECT_CAPACITY
        || stage_capacity != STAGE_CAPACITY
        || usize::from(entry_count) > MAXIMUM_ENTRIES
        || entry_count != object_count.saturating_add(stage_count)
        || object_count > object_capacity
        || stage_count > stage_capacity
        || (!present && entry_count != 0)
    {
        return Err(NativeEpisodeShardError::new(
            "resource-load status and counts disagree",
        ));
    }

    let mut entries = Vec::with_capacity(usize::from(entry_count));
    let mut previous_object_slot: Option<u8> = None;
    let mut previous_stage_slot: Option<u8> = None;
    for index in 0..entry_count {
        let kind = match reader.u8()? {
            0 => NativeResourceArchiveKind::Object,
            1 => NativeResourceArchiveKind::Stage,
            _ => {
                return Err(NativeEpisodeShardError::new(
                    "unknown resource archive kind",
                ));
            }
        };
        let slot = reader.u8()?;
        let outcome = match reader.u8()? {
            1 => NativeResourceLoadOutcome::Mounting,
            2 => NativeResourceLoadOutcome::Ready,
            3 => NativeResourceLoadOutcome::Failed,
            _ => {
                return Err(NativeEpisodeShardError::new(
                    "unknown resource-load outcome",
                ));
            }
        };
        let flags = reader.u8()?;
        let reference_count = reader.u16()?;
        if flags & !0x0f != 0 || reader.u16()? != 0 {
            return Err(NativeEpisodeShardError::new(
                "noncanonical resource-load flags or reserved field",
            ));
        }
        let archive_name = reader.fixed_string(12)?;
        let mount_command_present = flags & 1 != 0;
        let archive_present = flags & (1 << 1) != 0;
        let data_heap_present = flags & (1 << 2) != 0;
        let resource_table_present = flags & (1 << 3) != 0;
        let structural_outcome = if mount_command_present
            && !archive_present
            && !data_heap_present
            && !resource_table_present
        {
            Some(NativeResourceLoadOutcome::Mounting)
        } else if !mount_command_present && archive_present && resource_table_present {
            Some(NativeResourceLoadOutcome::Ready)
        } else if !mount_command_present
            && !resource_table_present
            && (!data_heap_present || archive_present)
        {
            Some(NativeResourceLoadOutcome::Failed)
        } else {
            None
        };
        let ordered = match kind {
            NativeResourceArchiveKind::Object => {
                index < object_count
                    && u16::from(slot) < object_capacity
                    && previous_object_slot.is_none_or(|previous| slot > previous)
            }
            NativeResourceArchiveKind::Stage => {
                index >= object_count
                    && u16::from(slot) < stage_capacity
                    && previous_stage_slot.is_none_or(|previous| slot > previous)
            }
        };
        if reference_count == 0
            || archive_name.is_empty()
            || !archive_name.bytes().all(|byte| byte.is_ascii_graphic())
            || structural_outcome != Some(outcome)
            || !ordered
        {
            return Err(NativeEpisodeShardError::new(
                "resource-load entry status and payload disagree",
            ));
        }
        match kind {
            NativeResourceArchiveKind::Object => previous_object_slot = Some(slot),
            NativeResourceArchiveKind::Stage => previous_stage_slot = Some(slot),
        }
        entries.push(NativeResourceLoadEntryObservation {
            kind,
            slot,
            outcome,
            mount_command_present,
            archive_present,
            data_heap_present,
            resource_table_present,
            reference_count,
            archive_name,
        });
    }

    Ok((
        status,
        present.then_some(NativeResourceLoadObservation {
            object_capacity,
            stage_capacity,
            object_count,
            stage_count,
            entries,
        }),
    ))
}

pub(super) fn decode_return_restart_write_trace(
    reader: &mut Reader<'_>,
) -> Result<
    (
        NativeChannelStatus,
        Option<NativeReturnRestartWriteTraceObservation>,
    ),
    NativeEpisodeShardError,
> {
    let status = decode_channel_status(reader)?;
    if reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero return/restart write-trace reserved byte",
        ));
    }
    let trace = NativeReturnRestartWriteTraceObservation {
        return_place_initialize_count: reader.u16()?,
        return_place_set_count: reader.u16()?,
        savmem_execute_count: reader.u16()?,
        savmem_eligible_execute_count: reader.u16()?,
        restart_place_set_count: reader.u16()?,
        restart_start_point_set_count: reader.u16()?,
        restart_room_parameter_set_count: reader.u16()?,
        restart_last_scene_info_set_count: reader.u16()?,
        return_place_value_change_count: reader.u16()?,
        restart_place_value_change_count: reader.u16()?,
        restart_start_point_value_change_count: reader.u16()?,
        restart_room_parameter_value_change_count: reader.u16()?,
        restart_last_scene_info_value_change_count: reader.u16()?,
    };
    let return_writes =
        u32::from(trace.return_place_initialize_count) + u32::from(trace.return_place_set_count);
    if status != NativeChannelStatus::Present
        || trace.savmem_eligible_execute_count > trace.savmem_execute_count
        || u32::from(trace.return_place_value_change_count) > return_writes
        || trace.restart_place_value_change_count > trace.restart_place_set_count
        || trace.restart_start_point_value_change_count > trace.restart_start_point_set_count
        || trace.restart_room_parameter_value_change_count > trace.restart_room_parameter_set_count
        || trace.restart_last_scene_info_value_change_count
            > trace.restart_last_scene_info_set_count
    {
        return Err(NativeEpisodeShardError::new(
            "inconsistent return/restart write trace",
        ));
    }
    Ok((status, Some(trace)))
}
