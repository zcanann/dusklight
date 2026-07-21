use super::*;

pub(super) fn decode_columnar(
    bytes: &[u8],
    version: u16,
    header_size: usize,
    boot: TapeBoot,
) -> Result<DecodedTrace, TraceError> {
    if bytes.len() < header_size {
        return Err(TraceError(format!(
            "truncated gameplay trace v{version} header"
        )));
    }
    if usize::from(u16_at(bytes, 10)) != header_size {
        return Err(TraceError(format!(
            "unsupported gameplay trace v{version} header size"
        )));
    }
    validate_tick_rate(bytes)?;
    let count = count_at(bytes, 20)?;
    let file_flags = u32_at(bytes, 28);
    let known_file_flags = if version >= 5 {
        FILE_COMPLETE | FILE_CAPACITY_EXHAUSTED | FILE_TRIGGER_RETENTION
    } else {
        FILE_COMPLETE | FILE_CAPACITY_EXHAUSTED
    };
    if file_flags & FILE_COMPLETE == 0 || file_flags & !known_file_flags != 0 {
        return Err(TraceError(
            "incomplete or noncanonical gameplay trace v2 flags".into(),
        ));
    }
    let retention = if version >= 5 {
        let configured_triggers = u32_at(bytes, 100);
        let observed_triggers = u32_at(bytes, 104);
        let pre_trigger_ticks = u32_at(bytes, 108);
        let post_trigger_ticks = u32_at(bytes, 112);
        let trigger_count = u32_at(bytes, 116);
        let observed_sample_count = u64_at(bytes, 120);
        let enabled = file_flags & FILE_TRIGGER_RETENTION != 0;
        if configured_triggers & !KNOWN_RETENTION_TRIGGERS != 0
            || observed_triggers & !configured_triggers != 0
            || ((trigger_count == 0) != (observed_triggers == 0))
            || observed_sample_count < count as u64
            || (enabled != (configured_triggers != 0))
            || (!enabled
                && (observed_triggers != 0
                    || pre_trigger_ticks != 0
                    || post_trigger_ticks != 0
                    || trigger_count != 0
                    || observed_sample_count != count as u64))
        {
            return Err(TraceError(
                "inconsistent gameplay trace v5 retention metadata".into(),
            ));
        }
        enabled.then_some(TraceRetention {
            configured_triggers,
            observed_triggers,
            pre_trigger_ticks,
            post_trigger_ticks,
            trigger_count,
            observed_sample_count,
        })
    } else {
        None
    };
    let channel_count = usize::from(u16_at(bytes, 32));
    if usize::from(u16_at(bytes, 34)) != V2_DIRECTORY_ENTRY_SIZE
        || usize_at_u64(bytes, 36)? != header_size
    {
        return Err(TraceError(
            "unsupported gameplay trace v2 directory layout".into(),
        ));
    }
    let directory_end = checked_region_end(header_size, channel_count, V2_DIRECTORY_ENTRY_SIZE)?;
    if usize_at_u64(bytes, 44)? != directory_end || directory_end > bytes.len() {
        return Err(TraceError("invalid gameplay trace v2 data offset".into()));
    }
    let requested_channels = u64_at(bytes, 52);
    if requested_channels & TraceChannel::Core.bit() == 0
        || requested_channels & !KNOWN_CHANNELS != 0
        || u32_at(bytes, 60) != 0
    {
        return Err(TraceError(
            "invalid gameplay trace v2 channel mask or reserved field".into(),
        ));
    }

    let mut descriptors = BTreeMap::<u16, ChannelDescriptor>::new();
    let mut regions = vec![(0_usize, directory_end)];
    for index in 0..channel_count {
        let offset = header_size + index * V2_DIRECTORY_ENTRY_SIZE;
        let id = u16_at(bytes, offset);
        let version = u16_at(bytes, offset + 2);
        let flags = u32_at(bytes, offset + 4);
        let stride = usize::try_from(u32_at(bytes, offset + 8))
            .map_err(|_| TraceError("gameplay trace stride overflow".into()))?;
        let status_stride = u32_at(bytes, offset + 12);
        let status_offset = usize_at_u64(bytes, offset + 16)?;
        let status_length = usize_at_u64(bytes, offset + 24)?;
        let payload_offset = usize_at_u64(bytes, offset + 32)?;
        let payload_length = usize_at_u64(bytes, offset + 40)?;
        let metadata_offset = usize_at_u64(bytes, offset + 48)?;
        let metadata_length = usize_at_u64(bytes, offset + 56)?;
        if flags & !(CHANNEL_REQUIRED | CHANNEL_DENSE) != 0
            || flags & CHANNEL_DENSE == 0
            || status_stride != 1
            || status_length != count
            || metadata_offset != 0
            || metadata_length != 0
        {
            return Err(TraceError(format!(
                "noncanonical gameplay trace channel {id}"
            )));
        }
        let expected_payload = count
            .checked_mul(stride)
            .ok_or_else(|| TraceError("gameplay trace payload size overflow".into()))?;
        if payload_length != expected_payload {
            return Err(TraceError(format!(
                "gameplay trace channel {id} length mismatch"
            )));
        }
        let status_end = status_offset
            .checked_add(status_length)
            .ok_or_else(|| TraceError("gameplay trace status range overflow".into()))?;
        let payload_end = payload_offset
            .checked_add(payload_length)
            .ok_or_else(|| TraceError("gameplay trace payload range overflow".into()))?;
        if status_offset < directory_end
            || payload_offset < directory_end
            || status_end > bytes.len()
            || payload_end > bytes.len()
        {
            return Err(TraceError(format!(
                "gameplay trace channel {id} is out of bounds"
            )));
        }
        regions.push((status_offset, status_end));
        regions.push((payload_offset, payload_end));
        let channel = TraceChannel::from_id(id);
        if let Some(channel) = channel {
            let definition = channel_definition(channel, version).ok_or_else(|| {
                TraceError(format!(
                    "unsupported gameplay trace channel {} version {version}",
                    channel.name()
                ))
            })?;
            if stride != definition.stride {
                return Err(TraceError(format!(
                    "unsupported gameplay trace channel {} version {version} stride {stride}; expected {}",
                    channel.name(),
                    definition.stride
                )));
            }
            if requested_channels & channel.bit() == 0 {
                return Err(TraceError(format!(
                    "unrequested known gameplay trace channel {} is present",
                    channel.name()
                )));
            }
        } else if flags & CHANNEL_REQUIRED != 0 {
            return Err(TraceError(format!(
                "unknown required gameplay trace channel {id}"
            )));
        }
        if descriptors
            .insert(
                id,
                ChannelDescriptor {
                    channel,
                    version,
                    flags,
                    stride,
                    status_offset,
                    status_length,
                    payload_offset,
                    payload_length,
                },
            )
            .is_some()
        {
            return Err(TraceError(format!("duplicate gameplay trace channel {id}")));
        }
    }
    regions.sort_unstable();
    if regions.windows(2).any(|pair| pair[0].1 != pair[1].0) {
        return Err(TraceError(
            "overlapping or unreferenced gameplay trace v2 regions".into(),
        ));
    }
    if descriptors
        .get(&(TraceChannel::Core as u16))
        .is_none_or(|descriptor| descriptor.flags & CHANNEL_REQUIRED == 0)
    {
        return Err(TraceError(
            "missing required gameplay trace core channel".into(),
        ));
    }
    if descriptors.contains_key(&(TraceChannel::PlayerCollisionSurfaces as u16))
        && !descriptors.contains_key(&(TraceChannel::Stage as u16))
    {
        return Err(TraceError(
            "player collision surfaces require the Stage channel".into(),
        ));
    }
    for channel in TraceChannel::ALL {
        if requested_channels & channel.bit() != 0 && !descriptors.contains_key(&(channel as u16)) {
            return Err(TraceError(format!(
                "requested channel {} is missing",
                channel.name()
            )));
        }
    }
    if regions.last().is_some_and(|region| region.1 != bytes.len()) {
        return Err(TraceError(
            "trailing or unreferenced gameplay trace v2 data".into(),
        ));
    }

    let mut records = vec![TraceRecord::default(); count];
    let mut channel_formats = BTreeMap::new();
    for descriptor in descriptors.values() {
        let Some(channel) = descriptor.channel else {
            continue;
        };
        channel_formats.insert(
            channel,
            TraceChannelWireFormat {
                version: descriptor.version,
                stride: descriptor.stride,
            },
        );
        debug_assert_eq!(descriptor.status_length, count);
        debug_assert_eq!(descriptor.payload_length, count * descriptor.stride);
        for (index, record) in records.iter_mut().enumerate() {
            let status = TraceChannelStatus::try_from(bytes[descriptor.status_offset + index])?;
            if channel == TraceChannel::Core && status != TraceChannelStatus::Present {
                return Err(TraceError("gameplay trace core is not present".into()));
            }
            validate_channel_status(channel, descriptor.version, status)?;
            record.channel_status.insert(channel, status);
            if status == TraceChannelStatus::Present || status == TraceChannelStatus::Truncated {
                let start = descriptor.payload_offset + index * descriptor.stride;
                decode_v2_channel(
                    channel,
                    descriptor.version,
                    &bytes[start..start + descriptor.stride],
                    record,
                )?;
            }
        }
    }
    for record in &records {
        if record.observation_phase != TracePhase::PostSimulation
            || record.simulation_tick.checked_add(1) != Some(record.boundary_index)
        {
            return Err(TraceError(
                "contradictory gameplay trace v2 boundary".into(),
            ));
        }
        validate_collision_surface_joins(record)?;
    }
    Ok(DecodedTrace {
        version,
        boot,
        tick_rate_numerator: u32_at(bytes, 12),
        tick_rate_denominator: u32_at(bytes, 16),
        requested_channels,
        capacity_exhausted: file_flags & FILE_CAPACITY_EXHAUSTED != 0,
        retention,
        channel_formats,
        records,
    })
}

fn validate_channel_status(
    channel: TraceChannel,
    version: u16,
    status: TraceChannelStatus,
) -> Result<(), TraceError> {
    let valid = status != TraceChannelStatus::NotSampled
        && match (channel, version) {
            (TraceChannel::SceneExit, 1) => matches!(
                status,
                TraceChannelStatus::Present | TraceChannelStatus::Absent
            ),
            (TraceChannel::SceneExit, 2) => matches!(
                status,
                TraceChannelStatus::Present
                    | TraceChannelStatus::Absent
                    | TraceChannelStatus::Unavailable
            ),
            (TraceChannel::PlayerBackgroundCollision, 1 | 2) => matches!(
                status,
                TraceChannelStatus::Present
                    | TraceChannelStatus::Absent
                    | TraceChannelStatus::Unavailable
            ),
            (TraceChannel::PlayerCollisionSurfaces, 1) => matches!(
                status,
                TraceChannelStatus::Present
                    | TraceChannelStatus::Absent
                    | TraceChannelStatus::Unavailable
            ),
            (TraceChannel::GoalProgress | TraceChannel::SelectedActors, 1) => {
                status == TraceChannelStatus::Present
            }
            _ => true,
        };
    if !valid {
        return Err(TraceError(format!(
            "invalid gameplay trace channel status {status:?} for {} version {version}",
            channel.name()
        )));
    }
    Ok(())
}

fn decode_v2_channel(
    channel: TraceChannel,
    version: u16,
    bytes: &[u8],
    record: &mut TraceRecord,
) -> Result<(), TraceError> {
    match channel {
        TraceChannel::Core => {
            if bytes[31] != 0 {
                return Err(TraceError(
                    "nonzero gameplay trace core reserved byte".into(),
                ));
            }
            let flags = u32_at(bytes, 24);
            if flags & CORE_SIMULATION_TICK_VALID == 0 || flags & !3 != 0 || bytes[29] != 1 {
                return Err(TraceError(
                    "invalid gameplay trace core flags or boundary".into(),
                ));
            }
            record.boundary_index = u64_at(bytes, 0);
            record.simulation_tick = u64_at(bytes, 8);
            record.tape_frame = (flags & CORE_TAPE_FRAME_VALID != 0).then(|| u64_at(bytes, 16));
            record.observation_phase = TracePhase::try_from(bytes[28])?;
            record.input_source = bytes[30];
            let wire_tape_frame = u64_at(bytes, 16);
            if record.input_source & !KNOWN_INPUT_SOURCES != 0
                || record.input_source.count_ones() > 1
                || (record.tape_frame.is_some() && wire_tape_frame == u64::MAX)
                || (record.tape_frame.is_none() && wire_tape_frame != u64::MAX)
            {
                return Err(TraceError(
                    "noncanonical gameplay trace core input or tape-frame state".into(),
                ));
            }
            if record.input_source & INPUT_TAPE != 0 {
                record.flags |= LEGACY_TAPE_PLAYING;
            }
            if record.input_source & INPUT_CONTROLLER != 0 {
                record.flags |= LEGACY_CONTROLLER_PLAYING;
            }
        }
        TraceChannel::Stage => {
            if u32_at(bytes, 28) != 0 {
                return Err(TraceError(
                    "nonzero gameplay trace stage reserved field".into(),
                ));
            }
            record.stage_name = decode_name(&bytes[0..8])?;
            record.room = bytes[8] as i8;
            record.layer = bytes[9] as i8;
            record.point = i16_at(bytes, 10);
            record.next_stage_name = decode_name(&bytes[12..20])?;
            record.next_room = bytes[20] as i8;
            record.next_layer = bytes[21] as i8;
            record.next_point = i16_at(bytes, 22);
            let flags = u32_at(bytes, 24);
            if flags & !1 != 0 {
                return Err(TraceError("unknown gameplay trace stage flags".into()));
            }
            record.next_stage_enabled = flags & 1 != 0;
        }
        TraceChannel::AppliedPads => {
            if u16_at(bytes, 2) != 0 || bytes[0] & !0x0f != 0 || bytes[1] & !0x0f != 0 {
                return Err(TraceError(
                    "invalid gameplay trace applied-pad header".into(),
                ));
            }
            let pads = [
                decode_pad(&bytes[4..16])?,
                decode_pad(&bytes[16..28])?,
                decode_pad(&bytes[28..40])?,
                decode_pad(&bytes[40..52])?,
            ];
            let connected_ports = pads.iter().enumerate().fold(0_u8, |mask, (port, pad)| {
                mask | if pad.connected { 1 << port } else { 0 }
            });
            if connected_ports != bytes[0] {
                return Err(TraceError(
                    "gameplay trace applied-pad validity disagrees with pad flags".into(),
                ));
            }
            record.buttons = pads[0].buttons;
            record.stick_x = pads[0].stick_x;
            record.stick_y = pads[0].stick_y;
            record.pad_error = pads[0].error;
            record.applied_pads = Some(TraceAppliedPads {
                valid_ports: bytes[0],
                owned_ports: bytes[1],
                pads,
            });
        }
        TraceChannel::PlayerMotion => {
            record.player_session_process_id = Some(u32_at(bytes, 0));
            record.player_actor_name = i16_at(bytes, 4);
            let procedure = u16_at(bytes, 6);
            record.player_proc_id = (procedure != u16::MAX).then_some(procedure);
            record.current_angle = [i16_at(bytes, 8), i16_at(bytes, 10), i16_at(bytes, 12)];
            record.shape_angle = [i16_at(bytes, 14), i16_at(bytes, 16), i16_at(bytes, 18)];
            record.current_angle_y = record.current_angle[1];
            record.shape_angle_y = record.shape_angle[1];
            record.position = [f32_at(bytes, 20), f32_at(bytes, 24), f32_at(bytes, 28)];
            record.velocity = [f32_at(bytes, 32), f32_at(bytes, 36), f32_at(bytes, 40)];
            record.forward_speed = f32_at(bytes, 44);
            let flags = u32_at(bytes, 48);
            if flags & !PLAYER_IS_LINK != 0 {
                return Err(TraceError("unknown gameplay trace player flags".into()));
            }
            record.flags |= LEGACY_PLAYER_PRESENT;
            if flags & PLAYER_IS_LINK != 0 {
                record.flags |= LEGACY_PLAYER_IS_LINK;
            }
        }
        TraceChannel::Event => {
            if bytes[9] != 0 || u16_at(bytes, 10) != 0 {
                return Err(TraceError(
                    "nonzero gameplay trace event reserved field".into(),
                ));
            }
            let flags = u32_at(bytes, 0);
            if flags & !(EVENT_RUNNING | EVENT_NAME_HASH_PRESENT) != 0 {
                return Err(TraceError("unknown gameplay trace event flags".into()));
            }
            if flags & EVENT_RUNNING != 0 {
                record.flags |= LEGACY_EVENT_RUNNING;
            }
            record.event_id = i16_at(bytes, 4);
            record.event_mode = bytes[6];
            record.event_status = bytes[7];
            record.event_map_tool_id = bytes[8];
            record.event_name_hash = u32_at(bytes, 12);
            record.event_name_hash_present = flags & EVENT_NAME_HASH_PRESENT != 0;
        }
        TraceChannel::SceneExit => match version {
            1 => decode_scene_exit_v1(bytes, record)?,
            2 => decode_scene_exit_v2(bytes, record)?,
            _ => unreachable!("channel version was validated"),
        },
        TraceChannel::Rng => {
            let primary = decode_rng_stream(&bytes[8..36])?;
            let secondary = decode_rng_stream(&bytes[36..64])?;
            let stream_count = u32_at(bytes, 4);
            if u32_at(bytes, 0) != 1
                || stream_count != 2
                || primary.id != 0
                || secondary.id != 1
                || primary.algorithm_version != 1
                || secondary.algorithm_version != 1
            {
                return Err(TraceError(
                    "invalid gameplay trace RNG stream identity".into(),
                ));
            }
            record.rng = Some(TraceRngSnapshot {
                version: u32_at(bytes, 0),
                stream_count,
                primary,
                secondary,
            });
        }
        TraceChannel::Camera => {
            if u16_at(bytes, 6) != 0 {
                return Err(TraceError(
                    "nonzero gameplay trace camera reserved field".into(),
                ));
            }
            record.camera = Some(TraceCamera {
                view_yaw: i16_at(bytes, 0),
                controlled_yaw: i16_at(bytes, 2),
                bank: i16_at(bytes, 4),
                eye: [f32_at(bytes, 8), f32_at(bytes, 12), f32_at(bytes, 16)],
                center: [f32_at(bytes, 20), f32_at(bytes, 24), f32_at(bytes, 28)],
                up: [f32_at(bytes, 32), f32_at(bytes, 36), f32_at(bytes, 40)],
                fovy: f32_at(bytes, 44),
            });
        }
        TraceChannel::PlayerAction => {
            if u16_at(bytes, 2) != 0 || bytes[27..32].iter().any(|value| *value != 0) {
                return Err(TraceError(
                    "nonzero gameplay trace player-action reserved field".into(),
                ));
            }
            let lane = |offset| TraceAnimationLane {
                resource_id: u16_at(bytes, offset),
                frame: f32_at(bytes, offset + 4),
                rate: f32_at(bytes, offset + 8),
            };
            for offset in (32..104).step_by(12) {
                if u16_at(bytes, offset + 2) != 0 {
                    return Err(TraceError("nonzero animation-lane reserved field".into()));
                }
            }
            let decode_identity = |offset: usize| -> Result<TraceActorIdentity, TraceError> {
                if u16_at(bytes, offset + 10) != 0 {
                    return Err(TraceError(
                        "nonzero gameplay trace actor-identity reserved field".into(),
                    ));
                }
                Ok(TraceActorIdentity {
                    session_process_id: u32_at(bytes, offset),
                    actor_name: i16_at(bytes, offset + 4),
                    set_id: u16_at(bytes, offset + 6),
                    home_room: bytes[offset + 8] as i8,
                    current_room: bytes[offset + 9] as i8,
                    home_position: (version >= 3).then(|| {
                        [
                            f32_at(bytes, offset + 12),
                            f32_at(bytes, offset + 16),
                            f32_at(bytes, offset + 20),
                        ]
                    }),
                })
            };
            let (do_status, talk_partner, grabbed_actor) = if version >= 2 {
                let flags = u32_at(bytes, 104);
                if flags & !PLAYER_ACTION_KNOWN_FLAGS != 0
                    || bytes[109..112].iter().any(|value| *value != 0)
                {
                    return Err(TraceError(
                        "invalid gameplay trace player-action interaction metadata".into(),
                    ));
                }
                let identity_stride = if version >= 3 { 24 } else { 12 };
                let talk = decode_identity(112)?;
                let grabbed = decode_identity(112 + identity_stride)?;
                let absent_is_canonical = |identity: &TraceActorIdentity| {
                    identity.session_process_id == u32::MAX
                        && identity.actor_name == -1
                        && identity.set_id == u16::MAX
                        && identity.home_room == -1
                        && identity.current_room == -1
                        && identity
                            .home_position
                            .is_none_or(|position| position == [0.0; 3])
                };
                if flags & TALK_PARTNER_PRESENT == 0 && !absent_is_canonical(&talk)
                    || flags & GRABBED_ACTOR_PRESENT == 0 && !absent_is_canonical(&grabbed)
                {
                    return Err(TraceError(
                        "noncanonical absent gameplay trace actor identity".into(),
                    ));
                }
                for identity in [&talk, &grabbed] {
                    if identity.home_position.is_some_and(|position| {
                        position.iter().any(|value| {
                            !value.is_finite() || (value.to_bits() == (-0.0_f32).to_bits())
                        })
                    }) {
                        return Err(TraceError(
                            "noncanonical gameplay trace actor home position".into(),
                        ));
                    }
                }
                (
                    bytes[108],
                    (flags & TALK_PARTNER_PRESENT != 0).then_some(talk),
                    (flags & GRABBED_ACTOR_PRESENT != 0).then_some(grabbed),
                )
            } else {
                (0, None, None)
            };
            record.player_action = Some(TracePlayerAction {
                procedure_id: u16_at(bytes, 0),
                mode_flags: u32_at(bytes, 4),
                procedure_context_raw: std::array::from_fn(|index| i16_at(bytes, 8 + index * 2)),
                damage_wait_timer: i16_at(bytes, 20),
                sword_at_up_time: u16_at(bytes, 22),
                ice_damage_wait_timer: i16_at(bytes, 24),
                sword_change_wait_timer: bytes[26],
                under_animations: [lane(32), lane(44), lane(56)],
                upper_animations: [lane(68), lane(80), lane(92)],
                do_status,
                talk_partner,
                grabbed_actor,
            });
        }
        TraceChannel::PlayerBackgroundCollision => match version {
            1 => decode_player_background_collision_v1(bytes, record)?,
            2 => decode_player_background_collision_v2(bytes, record)?,
            _ => unreachable!("channel version was validated"),
        },
        TraceChannel::PlayerCollisionSurfaces => {
            decode_player_collision_surfaces_v1(bytes, record)?
        }
        TraceChannel::GoalProgress => {
            let flags = u32_at(bytes, 0);
            let first_hit_tick = u64_at(bytes, 24);
            let configured = flags & GOAL_CONFIGURED != 0;
            let reached = flags & GOAL_REACHED != 0;
            let authored = flags & GOAL_AUTHORED != 0;
            let first_hit_present = flags & GOAL_FIRST_HIT_TICK_PRESENT != 0;
            let requested_count = u16_at(bytes, 8);
            let hit_count = u16_at(bytes, 10);
            let stable_ticks = u16_at(bytes, 12);
            let consecutive_ticks = u16_at(bytes, 14);
            let sequence_steps = bytes[16];
            let sequence_next_step = bytes[17];
            let sequence_within_ticks = u16_at(bytes, 18);
            let sequence_elapsed_ticks = u16_at(bytes, 20);
            if flags & !GOAL_KNOWN_FLAGS != 0
                || u16_at(bytes, 22) != 0
                || reached != first_hit_present
                || reached && !configured
                || authored && !configured
                || first_hit_present == (first_hit_tick == u64::MAX)
                || hit_count > requested_count
                || consecutive_ticks > stable_ticks
                || sequence_next_step > sequence_steps
                || (!configured
                    && (u32_at(bytes, 4) != 0
                        || stable_ticks != 0
                        || consecutive_ticks != 0
                        || sequence_steps != 0
                        || sequence_next_step != 0
                        || sequence_within_ticks != 0
                        || sequence_elapsed_ticks != 0))
            {
                return Err(TraceError(
                    "inconsistent gameplay trace goal-progress payload".into(),
                ));
            }
            record.goal_progress = Some(TraceGoalProgress {
                configured,
                reached,
                authored,
                goal_name_hash: configured.then(|| u32_at(bytes, 4)),
                requested_count,
                hit_count,
                stable_ticks,
                consecutive_ticks,
                sequence_steps,
                sequence_next_step,
                sequence_within_ticks,
                sequence_elapsed_ticks,
                first_hit_tick: first_hit_present.then_some(first_hit_tick),
            });
        }
        TraceChannel::SelectedActors => {
            let count = usize::from(u16_at(bytes, 0));
            let capacity = usize::from(u16_at(bytes, 2));
            let flags = u32_at(bytes, 4);
            let observed_count = u32_at(bytes, 8);
            let truncated = flags & SELECTED_ACTORS_TRUNCATED != 0;
            if capacity != SELECTED_ACTOR_CAPACITY
                || count > capacity
                || flags & !SELECTED_ACTORS_TRUNCATED != 0
                || u32_at(bytes, 12) != 0
                || observed_count < count as u32
                || truncated != (observed_count > count as u32)
            {
                return Err(TraceError(
                    "inconsistent gameplay trace selected-actor header".into(),
                ));
            }
            let mut actors = Vec::with_capacity(count);
            for index in 0..SELECTED_ACTOR_CAPACITY {
                let offset = 16 + index * 40;
                if index < count {
                    let actor = decode_selected_actor(&bytes[offset..offset + 40])?;
                    if actors.last().is_some_and(|previous: &TraceSelectedActor| {
                        previous.session_process_id >= actor.session_process_id
                    }) {
                        return Err(TraceError(
                            "gameplay trace selected actors are not strictly ordered".into(),
                        ));
                    }
                    actors.push(actor);
                } else if !unused_selected_actor_is_canonical(&bytes[offset..offset + 40]) {
                    return Err(TraceError(
                        "noncanonical unused gameplay trace selected-actor slot".into(),
                    ));
                }
            }
            record.selected_actors = Some(TraceSelectedActors {
                observed_count,
                truncated,
                actors,
            });
        }
    }
    Ok(())
}

fn decode_selected_actor(bytes: &[u8]) -> Result<TraceSelectedActor, TraceError> {
    let actor = TraceSelectedActor {
        session_process_id: u32_at(bytes, 0),
        actor_name: i16_at(bytes, 4),
        set_id: u16_at(bytes, 6),
        home_room: bytes[8] as i8,
        current_room: bytes[9] as i8,
        health: i16_at(bytes, 10),
        status: u32_at(bytes, 12),
        position: [f32_at(bytes, 16), f32_at(bytes, 20), f32_at(bytes, 24)],
        current_angle: [i16_at(bytes, 28), i16_at(bytes, 30), i16_at(bytes, 32)],
        shape_angle: [i16_at(bytes, 34), i16_at(bytes, 36), i16_at(bytes, 38)],
    };
    if actor.session_process_id == u32::MAX || actor.position.iter().any(|value| !value.is_finite())
    {
        return Err(TraceError(
            "invalid retained gameplay trace selected actor".into(),
        ));
    }
    Ok(actor)
}

fn unused_selected_actor_is_canonical(bytes: &[u8]) -> bool {
    u32_at(bytes, 0) == u32::MAX
        && i16_at(bytes, 4) == -1
        && u16_at(bytes, 6) == u16::MAX
        && bytes[8] as i8 == -1
        && bytes[9] as i8 == -1
        && i16_at(bytes, 10) == 0
        && u32_at(bytes, 12) == 0
        && bytes[16..40].iter().all(|byte| *byte == 0)
}

fn decode_scene_exit_v1(bytes: &[u8], record: &mut TraceRecord) -> Result<(), TraceError> {
    if u16_at(bytes, 6) != 0 {
        return Err(TraceError(
            "nonzero gameplay trace scene-exit v1 reserved field".into(),
        ));
    }
    let values = [
        f32_at(bytes, 8),
        f32_at(bytes, 12),
        f32_at(bytes, 16),
        f32_at(bytes, 20),
    ];
    if values.iter().any(|value| !value.is_finite()) {
        return Err(TraceError(
            "nonfinite gameplay trace scene-exit v1 value".into(),
        ));
    }
    record.nearest_scene_exit_session_process_id = Some(u32_at(bytes, 0));
    record.nearest_scene_exit_actor_name = Some(i16_at(bytes, 4));
    record.nearest_scene_exit_position = values[..3].try_into().expect("fixed slice");
    record.nearest_scene_exit_distance = Some(values[3]);
    Ok(())
}

fn decode_scene_exit_v2(bytes: &[u8], record: &mut TraceRecord) -> Result<(), TraceError> {
    let flags = u32_at(bytes, 8);
    if flags & !SCENE_EXIT_KNOWN_FLAGS != 0 || flags & SCENE_EXIT_VOLUME_VALID == 0 {
        return Err(TraceError(
            "invalid gameplay trace scene-exit v2 flags".into(),
        ));
    }
    if bytes[33..36].iter().any(|value| *value != 0) || bytes[87] != 0 {
        return Err(TraceError(
            "nonzero gameplay trace scene-exit v2 reserved field".into(),
        ));
    }
    let kind = match bytes[24] {
        1 => TraceSceneExitKind::OrientedBox,
        2 => TraceSceneExitKind::RadialXz,
        value => {
            return Err(TraceError(format!(
                "invalid gameplay trace scene-exit kind {value}"
            )));
        }
    };
    let observed_count = bytes[25];
    if observed_count == 0
        || flags & SCENE_EXIT_OBSERVED_COUNT_SATURATED != 0 && observed_count != u8::MAX
    {
        return Err(TraceError(
            "gameplay trace scene-exit has an invalid observed candidate count".into(),
        ));
    }
    let latched = flags & SCENE_EXIT_PLAYER_LATCHED != 0;
    let link_exit_direction = latched.then_some(bytes[27]);
    let raw_link_exit_id = u16_at(bytes, 28);
    let link_exit_id = (raw_link_exit_id != u16::MAX).then_some(raw_link_exit_id);
    if link_exit_id.is_some() != latched || (!latched && bytes[27] != u8::MAX) {
        return Err(TraceError(
            "inconsistent gameplay trace scene-exit Link exit sentinels".into(),
        ));
    }
    let raw_actor_action = bytes[32];
    let actor_action = (raw_actor_action != u8::MAX).then_some(raw_actor_action);
    if kind == TraceSceneExitKind::OrientedBox && actor_action.is_some() {
        return Err(TraceError(
            "box gameplay trace scene-exit has radial actor action".into(),
        ));
    }
    if kind == TraceSceneExitKind::RadialXz && flags & SCENE_EXIT_CHANGE_OK != 0 {
        return Err(TraceError(
            "radial gameplay trace scene-exit has box-only change-ok state".into(),
        ));
    }
    if kind == TraceSceneExitKind::RadialXz && latched {
        return Err(TraceError(
            "radial gameplay trace scene-exit cannot be Link's regular exit latch".into(),
        ));
    }
    if flags & SCENE_EXIT_CHANGE_OK != 0 && !latched {
        return Err(TraceError(
            "gameplay trace scene-exit change-ok state lacks Link latch".into(),
        ));
    }
    if kind == TraceSceneExitKind::RadialXz
        && (bytes[21] != u8::MAX
            || bytes[22] != u8::MAX
            || bytes[23] != u8::MAX
            || actor_action.is_none_or(|action| action > 3))
    {
        return Err(TraceError(
            "invalid gameplay trace radial scene-exit parameters".into(),
        ));
    }
    let raw_parameters = u32_at(bytes, 4);
    if bytes[20] != raw_parameters as u8
        || kind == TraceSceneExitKind::OrientedBox
            && (bytes[22] != (raw_parameters >> 8) as u8
                || bytes[21] != (raw_parameters >> 16) as u8
                || bytes[23] != (raw_parameters >> 24) as u8)
    {
        return Err(TraceError(
            "gameplay trace scene-exit fields disagree with raw parameters".into(),
        ));
    }
    if flags & SCENE_EXIT_CHANGE_STARTED != 0 && flags & SCENE_EXIT_PLAYER_LATCHED == 0 {
        return Err(TraceError(
            "gameplay trace scene-change start lacks selected exit latch".into(),
        ));
    }
    let player_local_position = [f32_at(bytes, 36), f32_at(bytes, 40), f32_at(bytes, 44)];
    let volume_extent = [f32_at(bytes, 48), f32_at(bytes, 52), f32_at(bytes, 56)];
    let home_position = [f32_at(bytes, 60), f32_at(bytes, 64), f32_at(bytes, 68)];
    if std::iter::once(f32_at(bytes, 12))
        .chain(player_local_position)
        .chain(volume_extent)
        .chain(home_position)
        .any(|value| !value.is_finite())
    {
        return Err(TraceError(
            "nonfinite gameplay trace scene-exit v2 geometry".into(),
        ));
    }
    let canonical_extent = match kind {
        TraceSceneExitKind::OrientedBox => volume_extent.iter().all(|value| *value >= 0.0),
        TraceSceneExitKind::RadialXz => {
            volume_extent[0] >= 0.0
                && volume_extent[1] == 0.0
                && volume_extent[2] == volume_extent[0]
        }
    };
    if !canonical_extent {
        return Err(TraceError(
            "invalid gameplay trace scene-exit volume extent".into(),
        ));
    }
    let geometrically_inside = match kind {
        TraceSceneExitKind::OrientedBox => f32_at(bytes, 12) <= 0.0,
        TraceSceneExitKind::RadialXz => f32_at(bytes, 12) < 0.0,
    };
    if geometrically_inside != (flags & SCENE_EXIT_PLAYER_INSIDE != 0) {
        return Err(TraceError(
            "gameplay trace scene-exit inside flag disagrees with signed distance".into(),
        ));
    }
    let destination_present = flags & SCENE_EXIT_DESTINATION_VALID != 0;
    let destination_name = decode_name(&bytes[72..80])?;
    let destination_wipe = bytes[84];
    let destination_wipe_time = bytes[85];
    let destination_time_hour = bytes[86] as i8;
    if destination_present {
        if destination_name.is_empty()
            || !(-1..=63).contains(&(bytes[80] as i8))
            || destination_wipe_time > 7
            || !(-1..=30).contains(&destination_time_hour)
            || !(bytes[81] as i8 == -1 || (0..=14).contains(&(bytes[81] as i8)))
            || i16_at(bytes, 82) < 0
        {
            return Err(TraceError(
                "invalid gameplay trace scene-exit destination".into(),
            ));
        }
    } else if !destination_name.is_empty()
        || bytes[80] as i8 != -1
        || bytes[81] as i8 != -1
        || i16_at(bytes, 82) != -1
        || destination_wipe != u8::MAX
        || destination_wipe_time != u8::MAX
        || destination_time_hour != -1
    {
        return Err(TraceError(
            "gameplay trace scene-exit destination sentinels disagree with flags".into(),
        ));
    }
    let destination = destination_present.then(|| TraceSceneExitDestination {
        stage_name: destination_name,
        room: bytes[80] as i8,
        layer: bytes[81] as i8,
        point: i16_at(bytes, 82),
        wipe: destination_wipe,
        wipe_time: destination_wipe_time,
        time_hour: destination_time_hour,
    });
    let scene_exit = TraceSceneExit {
        session_process_id: u32_at(bytes, 0),
        raw_parameters,
        flags,
        signed_distance_to_volume: f32_at(bytes, 12),
        actor_name: i16_at(bytes, 16),
        set_id: u16_at(bytes, 18),
        exit_id: bytes[20],
        path_id: bytes[21],
        argument_1: bytes[22],
        switch_no: bytes[23],
        kind,
        observed_count,
        observed_count_saturated: flags & SCENE_EXIT_OBSERVED_COUNT_SATURATED != 0,
        home_room: bytes[26] as i8,
        link_exit_direction,
        link_exit_id,
        shape_yaw: i16_at(bytes, 30),
        actor_action,
        player_local_position,
        volume_extent,
        home_position,
        destination,
    };
    record.nearest_scene_exit_session_process_id = Some(scene_exit.session_process_id);
    record.nearest_scene_exit_actor_name = Some(scene_exit.actor_name);
    // Preserve the old actor-origin projection for callers that only display it.
    // movement-state/v1 rejects SceneExit v2 before featurization because its
    // old Euclidean-distance slot has no equivalent in this signed-volume wire.
    record.nearest_scene_exit_position = scene_exit.home_position;
    record.scene_exit = Some(scene_exit);
    Ok(())
}

fn decode_player_background_collision_v1(
    bytes: &[u8],
    record: &mut TraceRecord,
) -> Result<(), TraceError> {
    let flags = u32_at(bytes, 0);
    if flags & !COLLISION_KNOWN_FLAGS != 0 {
        return Err(TraceError(
            "unknown gameplay trace player-background-collision flags".into(),
        ));
    }
    if flags & (COLLISION_GROUND_CONTACT | COLLISION_GROUND_LANDING) != 0
        && flags & COLLISION_GROUND_PROBE_VALID == 0
        || flags & COLLISION_ROOF_CONTACT != 0 && flags & COLLISION_ROOF_PROBE_VALID == 0
        || flags & COLLISION_WALL_CONTACT != 0 && flags & COLLISION_WALL_PROBE_ENABLED == 0
        || flags & COLLISION_WATER_SURFACE_FOUND != 0 && flags & COLLISION_WATER_PROBE_ENABLED == 0
        || flags & COLLISION_WATER_IN != 0 && flags & COLLISION_WATER_SURFACE_FOUND == 0
        || flags & COLLISION_WATER_OWNER_PRESENT != 0 && flags & COLLISION_WATER_SURFACE_FOUND == 0
        || flags & COLLISION_GROUND_PLANE_VALID != 0
            && flags & (COLLISION_GROUND_PROBE_VALID | COLLISION_GROUND_CONTACT)
                != (COLLISION_GROUND_PROBE_VALID | COLLISION_GROUND_CONTACT)
    {
        return Err(TraceError(
            "contradictory gameplay trace player-background-collision flags".into(),
        ));
    }

    let ground_bg = u16_at(bytes, 16);
    let ground_poly = u16_at(bytes, 18);
    let ground_owner = u32_at(bytes, 20);
    let ground_identity = flags & COLLISION_GROUND_IDENTITY_PRESENT != 0;
    validate_identity_pair(ground_bg, ground_poly, ground_identity, "ground")?;
    validate_owner(
        ground_owner,
        flags & COLLISION_GROUND_OWNER_PRESENT != 0,
        "ground",
    )?;
    if ground_identity && flags & COLLISION_GROUND_PROBE_VALID == 0
        || flags & COLLISION_GROUND_OWNER_PRESENT != 0 && !ground_identity
    {
        return Err(TraceError(
            "gameplay trace collision ground identity disagrees with flags".into(),
        ));
    }
    let ground_plane = [
        f32_at(bytes, 24),
        f32_at(bytes, 28),
        f32_at(bytes, 32),
        f32_at(bytes, 36),
    ];
    validate_plane(
        ground_plane,
        flags & COLLISION_GROUND_PLANE_VALID != 0,
        "ground",
    )?;

    let roof_bg = u16_at(bytes, 40);
    let roof_poly = u16_at(bytes, 42);
    let roof_owner = u32_at(bytes, 44);
    let roof_identity = flags & COLLISION_ROOF_IDENTITY_PRESENT != 0;
    validate_identity_pair(roof_bg, roof_poly, roof_identity, "roof")?;
    validate_owner(
        roof_owner,
        flags & COLLISION_ROOF_OWNER_PRESENT != 0,
        "roof",
    )?;
    if roof_identity && flags & COLLISION_ROOF_PROBE_VALID == 0
        || flags & COLLISION_ROOF_OWNER_PRESENT != 0 && !roof_identity
    {
        return Err(TraceError(
            "gameplay trace collision roof identity disagrees with flags".into(),
        ));
    }
    let water_bg = u16_at(bytes, 48);
    let water_poly = u16_at(bytes, 50);
    let water_owner = u32_at(bytes, 52);
    let water_identity = flags & COLLISION_WATER_IDENTITY_PRESENT != 0;
    validate_identity_pair(water_bg, water_poly, water_identity, "water")?;
    validate_owner(
        water_owner,
        flags & COLLISION_WATER_OWNER_PRESENT != 0,
        "water",
    )?;
    if water_identity && flags & COLLISION_WATER_SURFACE_FOUND == 0
        || flags & COLLISION_WATER_OWNER_PRESENT != 0 && !water_identity
    {
        return Err(TraceError(
            "gameplay trace collision water identity disagrees with flags".into(),
        ));
    }

    let walls: [TraceCollisionWall; 3] = (0..3)
        .map(|index| {
            let offset = 56 + index * 12;
            let wall_flags = u16_at(bytes, offset + 10);
            if wall_flags & !COLLISION_WALL_KNOWN_FLAGS != 0 {
                return Err(TraceError(format!(
                    "unknown gameplay trace collision wall {index} flags"
                )));
            }
            let bg = u16_at(bytes, offset);
            let poly = u16_at(bytes, offset + 2);
            let owner = u32_at(bytes, offset + 4);
            let identity = wall_flags & COLLISION_WALL_IDENTITY_PRESENT != 0;
            validate_identity_pair(bg, poly, identity, "wall")?;
            validate_owner(
                owner,
                wall_flags & COLLISION_WALL_OWNER_PRESENT != 0,
                "wall",
            )?;
            if identity && wall_flags & COLLISION_WALL_HIT == 0
                || wall_flags & COLLISION_WALL_OWNER_PRESENT != 0 && !identity
                || wall_flags & COLLISION_WALL_HIT == 0 && i16_at(bytes, offset + 8) != 0
            {
                return Err(TraceError(
                    "gameplay trace collision wall identity or angle disagrees with flags".into(),
                ));
            }
            Ok(TraceCollisionWall {
                identity_present: identity,
                bg_index: (bg != INVALID_U16_ID).then_some(bg),
                poly_index: (poly != INVALID_U16_ID).then_some(poly),
                owner_session_process_id: (owner != INVALID_U32_ID).then_some(owner),
                angle_y: i16_at(bytes, offset + 8),
                flags: wall_flags,
            })
        })
        .collect::<Result<Vec<_>, TraceError>>()?
        .try_into()
        .expect("three collision wall slots");
    let any_wall_hit = walls
        .iter()
        .any(|wall| wall.flags & COLLISION_WALL_HIT != 0);
    if any_wall_hit != (flags & COLLISION_WALL_CONTACT != 0) {
        return Err(TraceError(
            "gameplay trace aggregate wall contact disagrees with wall hits".into(),
        ));
    }
    let heights = [f32_at(bytes, 4), f32_at(bytes, 8), f32_at(bytes, 12)];
    let old_position = [f32_at(bytes, 92), f32_at(bytes, 96), f32_at(bytes, 100)];
    let resolved_frame_displacement = [f32_at(bytes, 104), f32_at(bytes, 108), f32_at(bytes, 112)];
    let final_position = [f32_at(bytes, 116), f32_at(bytes, 120), f32_at(bytes, 124)];
    if heights
        .iter()
        .chain(&old_position)
        .chain(&resolved_frame_displacement)
        .chain(&final_position)
        .any(|value| !value.is_finite())
        || (flags & COLLISION_GROUND_PROBE_VALID == 0 && heights[0] != -1.0e9)
        || (flags & COLLISION_GROUND_PROBE_VALID != 0 && heights[0] == -1.0e9)
        || (flags & COLLISION_ROOF_PROBE_VALID == 0 && heights[1] != 1.0e9)
        || (flags & COLLISION_ROOF_PROBE_VALID != 0 && heights[1] == 1.0e9)
        || (flags & COLLISION_WATER_SURFACE_FOUND == 0 && heights[2] != -1.0e9)
        || (flags & COLLISION_WATER_SURFACE_FOUND != 0 && heights[2] == -1.0e9)
        || (flags & COLLISION_TRAJECTORY_VALID == 0
            && old_position
                .iter()
                .chain(&resolved_frame_displacement)
                .chain(&final_position)
                .any(|value| *value != 0.0))
    {
        return Err(TraceError(
            "invalid gameplay trace player-background-collision height sentinel".into(),
        ));
    }
    if flags & COLLISION_TRAJECTORY_VALID != 0
        && (0..3).any(|axis| {
            let reconstructed = old_position[axis] + resolved_frame_displacement[axis];
            let tolerance = 1.0e-4 * final_position[axis].abs().max(1.0);
            (reconstructed - final_position[axis]).abs() > tolerance
        })
    {
        return Err(TraceError(
            "gameplay trace collision trajectory does not reconstruct final position".into(),
        ));
    }
    record.player_background_collision = Some(TracePlayerBackgroundCollision {
        flags,
        ground_height: heights[0],
        roof_height: heights[1],
        water_height: heights[2],
        ground_bg_index: (ground_bg != INVALID_U16_ID).then_some(ground_bg),
        ground_poly_index: (ground_poly != INVALID_U16_ID).then_some(ground_poly),
        ground_owner_session_process_id: (ground_owner != INVALID_U32_ID).then_some(ground_owner),
        ground_plane,
        ground_identity_present: ground_identity,
        roof_bg_index: (roof_bg != INVALID_U16_ID).then_some(roof_bg),
        roof_poly_index: (roof_poly != INVALID_U16_ID).then_some(roof_poly),
        roof_owner_session_process_id: (roof_owner != INVALID_U32_ID).then_some(roof_owner),
        roof_identity_present: roof_identity,
        water_bg_index: (water_bg != INVALID_U16_ID).then_some(water_bg),
        water_poly_index: (water_poly != INVALID_U16_ID).then_some(water_poly),
        water_owner_session_process_id: (water_owner != INVALID_U32_ID).then_some(water_owner),
        water_identity_present: water_identity,
        walls,
        old_position,
        resolved_frame_displacement,
        final_position,
        solver: None,
    });
    Ok(())
}

fn decode_player_background_collision_v2(
    bytes: &[u8],
    record: &mut TraceRecord,
) -> Result<(), TraceError> {
    decode_player_background_collision_v1(&bytes[..128], record)?;
    let flags = u32_at(bytes, 128);
    if flags & !0x00f1_fffe != 0 || bytes[137] != 0 || u16_at(bytes, 138) != 0 {
        return Err(TraceError(
            "invalid gameplay trace collision-solver header".into(),
        ));
    }
    let line_start = [f32_at(bytes, 140), f32_at(bytes, 144), f32_at(bytes, 148)];
    let line_end = [f32_at(bytes, 152), f32_at(bytes, 156), f32_at(bytes, 160)];
    let wall_cylinder_center = [f32_at(bytes, 164), f32_at(bytes, 168), f32_at(bytes, 172)];
    let wall_cylinder_radius = f32_at(bytes, 176);
    let wall_cylinder_height = f32_at(bytes, 180);
    let ground_check_offset = f32_at(bytes, 184);
    let roof_correction_height = f32_at(bytes, 188);
    let water_check_offset = f32_at(bytes, 192);
    let walls: [TraceCollisionSolverWall; 3] = (0..3)
        .map(|index| {
            let offset = 196 + index * 40;
            let wall_flags = u32_at(bytes, offset);
            if wall_flags & !0x6 != 0 || u16_at(bytes, offset + 6) != 0 {
                return Err(TraceError(format!(
                    "invalid gameplay trace collision-solver wall {index} header"
                )));
            }
            Ok(TraceCollisionSolverWall {
                flags: wall_flags,
                angle_y: i16_at(bytes, offset + 4),
                wall_radius_squared: f32_at(bytes, offset + 8),
                wall_height: f32_at(bytes, offset + 12),
                wall_radius: f32_at(bytes, offset + 16),
                direct_wall_height: f32_at(bytes, offset + 20),
                realized_center: [
                    f32_at(bytes, offset + 24),
                    f32_at(bytes, offset + 28),
                    f32_at(bytes, offset + 32),
                ],
                realized_radius: f32_at(bytes, offset + 36),
            })
        })
        .collect::<Result<Vec<_>, TraceError>>()?
        .try_into()
        .expect("three collision-solver wall slots");
    if line_start
        .iter()
        .chain(&line_end)
        .chain(&wall_cylinder_center)
        .chain([
            &wall_cylinder_radius,
            &wall_cylinder_height,
            &ground_check_offset,
            &roof_correction_height,
            &water_check_offset,
        ])
        .chain(walls.iter().flat_map(|wall| {
            [
                &wall.wall_radius_squared,
                &wall.wall_height,
                &wall.wall_radius,
                &wall.direct_wall_height,
                &wall.realized_center[0],
                &wall.realized_center[1],
                &wall.realized_center[2],
                &wall.realized_radius,
            ]
        }))
        .any(|value| !value.is_finite())
    {
        return Err(TraceError(
            "nonfinite gameplay trace collision-solver geometry".into(),
        ));
    }
    record
        .player_background_collision
        .as_mut()
        .expect("v1 collision prefix decoded")
        .solver = Some(TracePlayerCollisionSolver {
        flags,
        wall_table_size: u32_at(bytes, 132) as i32,
        water_mode: bytes[136],
        line_start,
        line_end,
        wall_cylinder_center,
        wall_cylinder_radius,
        wall_cylinder_height,
        ground_check_offset,
        roof_correction_height,
        water_check_offset,
        walls,
    });
    Ok(())
}

fn decode_player_collision_surfaces_v1(
    bytes: &[u8],
    record: &mut TraceRecord,
) -> Result<(), TraceError> {
    let flags = u32_at(bytes, 0);
    if flags & !COLLISION_SURFACE_SET_KNOWN_FLAGS != 0
        || bytes[10] & !0x3f != 0
        || bytes[11..16].iter().any(|value| *value != 0)
    {
        return Err(TraceError(
            "invalid gameplay trace collision-surface set header".into(),
        ));
    }
    let room_valid = flags & COLLISION_SURFACE_SET_ROOM_VALID != 0;
    let raw_room = bytes[4] as i8;
    if room_valid != (raw_room != INVALID_I8) || room_valid && !(-1..=63).contains(&raw_room) {
        return Err(TraceError(
            "invalid gameplay trace collision-surface Link room".into(),
        ));
    }
    let raw_link_exit = u16_at(bytes, 8);
    if (flags & COLLISION_SURFACE_SET_EXPLICIT_LINK_EXIT != 0) != (raw_link_exit != 0x003f) {
        return Err(TraceError(
            "collision-surface explicit Link exit flag disagrees with raw field".into(),
        ));
    }

    let surfaces: [TraceCollisionSurface; 6] = (0..6)
        .map(|index| {
            decode_collision_surface(&bytes[16 + index * 80..16 + (index + 1) * 80], index)
        })
        .collect::<Result<Vec<_>, TraceError>>()?
        .try_into()
        .expect("six collision surface slots");
    let identity_count = surfaces
        .iter()
        .filter(|surface| surface.flags & COLLISION_SURFACE_IDENTITY_PRESENT != 0)
        .count() as u8;
    let backing_count = surfaces
        .iter()
        .filter(|surface| surface.flags & COLLISION_SURFACE_BACKING_PRESENT != 0)
        .count() as u8;
    let destination_count = surfaces
        .iter()
        .filter(|surface| surface.flags & COLLISION_SURFACE_DESTINATION_PRESENT != 0)
        .count() as u8;
    let pending_match_mask = surfaces
        .iter()
        .enumerate()
        .fold(0_u8, |mask, (index, surface)| {
            mask | (((surface.flags & COLLISION_SURFACE_PENDING_MATCH != 0) as u8) << index)
        });
    if bytes[5] != identity_count
        || bytes[6] != backing_count
        || bytes[7] != destination_count
        || bytes[10] != pending_match_mask
    {
        return Err(TraceError(
            "collision-surface set counts or pending-match mask disagree with slots".into(),
        ));
    }
    if flags & COLLISION_SURFACE_SET_EXPLICIT_LINK_EXIT != 0
        && surfaces[0].flags & COLLISION_SURFACE_PENDING_MATCH != 0
    {
        return Err(TraceError(
            "explicit Link exit cannot attribute the pending transition to ground collision".into(),
        ));
    }
    if surfaces
        .iter()
        .filter_map(|surface| surface.scls_source_room)
        .any(|room| !room_valid || room != raw_room)
    {
        return Err(TraceError(
            "collision-surface SCLS source disagrees with Link room".into(),
        ));
    }

    record.player_collision_surfaces = Some(TracePlayerCollisionSurfaces {
        flags,
        link_room: room_valid.then_some(raw_room),
        identity_count,
        backing_count,
        destination_count,
        raw_link_exit,
        pending_match_mask,
        surfaces,
    });
    Ok(())
}

fn decode_collision_surface(
    bytes: &[u8],
    expected_index: usize,
) -> Result<TraceCollisionSurface, TraceError> {
    let flags = u32_at(bytes, 0);
    if flags & !COLLISION_SURFACE_KNOWN_FLAGS != 0
        || bytes[51] != 0
        || bytes[76..80].iter().any(|value| *value != 0)
    {
        return Err(TraceError(format!(
            "invalid gameplay trace collision surface {expected_index} flags or reserved bytes"
        )));
    }
    let (expected_kind, expected_slot) = match expected_index {
        0 => (TraceCollisionSurfaceKind::Ground, 0),
        1 => (TraceCollisionSurfaceKind::Roof, 0),
        2 => (TraceCollisionSurfaceKind::Water, 0),
        3..=5 => (TraceCollisionSurfaceKind::Wall, (expected_index - 3) as u8),
        _ => unreachable!("bounded collision surface slot"),
    };
    let kind = match bytes[4] {
        1 => TraceCollisionSurfaceKind::Ground,
        2 => TraceCollisionSurfaceKind::Roof,
        3 => TraceCollisionSurfaceKind::Water,
        4 => TraceCollisionSurfaceKind::Wall,
        value => {
            return Err(TraceError(format!(
                "invalid gameplay trace collision surface kind {value}"
            )));
        }
    };
    if kind != expected_kind || bytes[5] != expected_slot {
        return Err(TraceError(format!(
            "collision surface {expected_index} has a noncanonical kind or wall slot"
        )));
    }

    let has = |flag| flags & flag != 0;
    let identity = has(COLLISION_SURFACE_IDENTITY_PRESENT);
    let owner_present = has(COLLISION_SURFACE_OWNER_PRESENT);
    let backing_present = has(COLLISION_SURFACE_BACKING_PRESENT);
    let codes_present = has(COLLISION_SURFACE_CODES_PRESENT);
    let material_present = has(COLLISION_SURFACE_MATERIAL_PRESENT);
    let group_present = has(COLLISION_SURFACE_GROUP_PRESENT);
    let source_room_present = has(COLLISION_SURFACE_SOURCE_ROOM_PRESENT);
    let source_room_exact = has(COLLISION_SURFACE_SOURCE_ROOM_EXACT);
    let scls_source_present = has(COLLISION_SURFACE_SCLS_SOURCE_PRESENT);
    let destination_present = has(COLLISION_SURFACE_DESTINATION_PRESENT);
    let pending_match = has(COLLISION_SURFACE_PENDING_MATCH);
    let geometry_present = has(COLLISION_SURFACE_GEOMETRY_PRESENT);
    let kcl_height_present = has(COLLISION_SURFACE_KCL_HEIGHT_PRESENT);
    if (flags & !COLLISION_SURFACE_IDENTITY_PRESENT) != 0 && !identity
        || source_room_exact && !source_room_present
        || pending_match && (!scls_source_present || !destination_present)
        || (scls_source_present || destination_present || pending_match)
            && kind != TraceCollisionSurfaceKind::Ground
    {
        return Err(TraceError(format!(
            "collision surface {expected_index} has incoherent presence or provenance flags"
        )));
    }

    let bg = u16_at(bytes, 8);
    let poly = u16_at(bytes, 10);
    validate_identity_pair(bg, poly, identity, "surface")?;
    let owner = u32_at(bytes, 12);
    validate_owner(owner, owner_present, "surface")?;
    let material = u16_at(bytes, 16);
    let group = u16_at(bytes, 18);
    if (material != INVALID_U16_ID) != material_present
        || (group != INVALID_U16_ID) != group_present
    {
        return Err(TraceError(format!(
            "collision surface {expected_index} row sentinels disagree with flags"
        )));
    }

    let backing_format = match bytes[6] {
        0 if !backing_present => None,
        1 if backing_present => Some(TraceCollisionBackingFormat::Dzb),
        2 if backing_present => Some(TraceCollisionBackingFormat::Kcl),
        value => {
            return Err(TraceError(format!(
                "collision surface {expected_index} has invalid backing format {value}"
            )));
        }
    };
    let raw_code_word_mask = bytes[7];
    if raw_code_word_mask & !0x1f != 0
        || codes_present != (raw_code_word_mask != 0)
        || codes_present && (!backing_present || raw_code_word_mask & 1 == 0)
    {
        return Err(TraceError(format!(
            "collision surface {expected_index} has invalid raw-code presence"
        )));
    }
    let raw_code_words = std::array::from_fn(|word| u32_at(bytes, 20 + word * 4));
    if raw_code_words.iter().enumerate().any(|(word, value)| {
        let present = raw_code_word_mask & (1 << word) != 0;
        !present && *value != 0
    }) {
        return Err(TraceError(format!(
            "collision surface {expected_index} has data in an absent raw-code word"
        )));
    }
    match backing_format {
        None => {
            if codes_present
                || material_present
                || group_present
                || geometry_present
                || kcl_height_present
            {
                return Err(TraceError(format!(
                    "collision surface {expected_index} has backing fields without backing"
                )));
            }
        }
        Some(TraceCollisionBackingFormat::Dzb) => {
            if kcl_height_present {
                return Err(TraceError(format!(
                    "collision surface {expected_index} has inconsistent DZB backing"
                )));
            }
        }
        Some(TraceCollisionBackingFormat::Kcl) => {
            if group_present {
                return Err(TraceError(format!(
                    "collision surface {expected_index} has inconsistent KCL backing"
                )));
            }
        }
    }

    let raw_exit = bytes[40];
    if codes_present {
        if raw_exit != (raw_code_words[0] & 0x3f) as u8 {
            return Err(TraceError(format!(
                "collision surface {expected_index} raw exit disagrees with collision code"
            )));
        }
    } else if raw_exit != u8::MAX {
        return Err(TraceError(format!(
            "collision surface {expected_index} has a raw exit without collision codes"
        )));
    }

    let raw_source_room = bytes[41] as i8;
    let raw_scls_room = bytes[42] as i8;
    if source_room_present != (raw_source_room != INVALID_I8)
        || source_room_present && !(-1..=63).contains(&raw_source_room)
        || scls_source_present != (raw_scls_room != INVALID_I8)
        || scls_source_present && !(-1..=63).contains(&raw_scls_room)
    {
        return Err(TraceError(format!(
            "collision surface {expected_index} has invalid room sentinels"
        )));
    }

    let destination_name = decode_name(&bytes[68..76])?;
    let destination_room = bytes[43] as i8;
    let destination_layer = bytes[44] as i8;
    let destination_wipe = bytes[45];
    let destination_wipe_time = bytes[46];
    let destination_time_hour = bytes[47] as i8;
    let destination_point = i16_at(bytes, 48);
    if destination_present {
        if !scls_source_present
            || !codes_present
            || raw_exit == 0x3f
            || raw_exit == u8::MAX
            || destination_name.is_empty()
            || !(-1..=63).contains(&destination_room)
            || !(destination_layer == -1 || (0..=14).contains(&destination_layer))
            || destination_point < 0
            || destination_wipe_time > 7
            || !(-1..=30).contains(&destination_time_hour)
        {
            return Err(TraceError(format!(
                "collision surface {expected_index} has an invalid destination"
            )));
        }
    } else if !destination_name.is_empty()
        || destination_room != INVALID_I8
        || destination_layer != INVALID_I8
        || destination_wipe != u8::MAX
        || destination_wipe_time != u8::MAX
        || destination_time_hour != INVALID_I8
        || destination_point != INVALID_I16
    {
        return Err(TraceError(format!(
            "collision surface {expected_index} destination sentinels disagree with flags"
        )));
    }

    let geometry_count = usize::from(bytes[50]);
    let geometry_indices: [u16; 6] = std::array::from_fn(|index| u16_at(bytes, 52 + index * 2));
    if geometry_present != (geometry_count != 0)
        || geometry_count > 6
        || geometry_indices
            .iter()
            .enumerate()
            .any(|(index, value)| (*value != INVALID_U16_ID) != (index < geometry_count))
    {
        return Err(TraceError(format!(
            "collision surface {expected_index} has invalid source geometry"
        )));
    }

    let kcl_prism_height = f32_at(bytes, 64);
    if !kcl_prism_height.is_finite()
        || kcl_height_present && backing_format != Some(TraceCollisionBackingFormat::Kcl)
        || !kcl_height_present && kcl_prism_height != 0.0
    {
        return Err(TraceError(format!(
            "collision surface {expected_index} has invalid KCL prism height"
        )));
    }

    Ok(TraceCollisionSurface {
        flags,
        kind,
        wall_slot: bytes[5],
        backing_format,
        raw_code_word_mask,
        bg_index: identity.then_some(bg),
        poly_index: identity.then_some(poly),
        owner_session_process_id: owner_present.then_some(owner),
        material_row: material_present.then_some(material),
        group_row: group_present.then_some(group),
        raw_code_words,
        raw_exit_id: codes_present.then_some(raw_exit),
        source_room: source_room_present.then_some(raw_source_room),
        source_room_exact,
        scls_source_room: scls_source_present.then_some(raw_scls_room),
        destination: destination_present.then_some(TraceCollisionSurfaceDestination {
            stage_name: destination_name,
            room: destination_room,
            layer: destination_layer,
            point: destination_point,
            wipe: destination_wipe,
            wipe_time: destination_wipe_time,
            time_hour: destination_time_hour,
        }),
        source_geometry_indices: geometry_indices[..geometry_count].to_vec(),
        kcl_prism_height: kcl_height_present.then_some(kcl_prism_height),
    })
}

fn validate_collision_surface_joins(record: &TraceRecord) -> Result<(), TraceError> {
    let Some(surfaces) = &record.player_collision_surfaces else {
        return Ok(());
    };
    let stage_present =
        record.channel_status.get(&TraceChannel::Stage) == Some(&TraceChannelStatus::Present);
    if !stage_present {
        return Err(TraceError(
            "player collision surfaces require present Stage observations".into(),
        ));
    }
    let pending = surfaces.flags & COLLISION_SURFACE_SET_NEXT_STAGE_PENDING != 0;
    if pending != record.next_stage_enabled {
        return Err(TraceError(
            "collision-surface pending-stage flag disagrees with Stage channel".into(),
        ));
    }
    for (index, surface) in surfaces.surfaces.iter().enumerate() {
        let matches_stage = pending
            && surface.destination.as_ref().is_some_and(|destination| {
                destination.stage_name == record.next_stage_name
                    && destination.room == record.next_room
                    && destination.layer == record.next_layer
                    && destination.point == record.next_point
            });
        if matches_stage != (surface.flags & COLLISION_SURFACE_PENDING_MATCH != 0) {
            return Err(TraceError(format!(
                "collision surface {index} pending-stage match disagrees with Stage channel"
            )));
        }
    }

    let Some(collision) = &record.player_background_collision else {
        return Ok(());
    };
    let wall_identity = |index: usize| {
        let wall = &collision.walls[index];
        (
            wall.bg_index,
            wall.poly_index,
            wall.owner_session_process_id,
        )
    };
    let expected: [(Option<u16>, Option<u16>, Option<u32>); 6] = [
        (
            collision.ground_bg_index,
            collision.ground_poly_index,
            collision.ground_owner_session_process_id,
        ),
        (
            collision.roof_bg_index,
            collision.roof_poly_index,
            collision.roof_owner_session_process_id,
        ),
        (
            collision.water_bg_index,
            collision.water_poly_index,
            collision.water_owner_session_process_id,
        ),
        wall_identity(0),
        wall_identity(1),
        wall_identity(2),
    ];
    for (index, (surface, expected)) in surfaces.surfaces.iter().zip(expected).enumerate() {
        let actual = (
            surface.bg_index,
            surface.poly_index,
            surface.owner_session_process_id,
        );
        if actual != expected {
            return Err(TraceError(format!(
                "collision surface {index} identity or owner disagrees with background collision"
            )));
        }
    }
    Ok(())
}

fn validate_identity_pair(bg: u16, poly: u16, present: bool, kind: &str) -> Result<(), TraceError> {
    if (bg != INVALID_U16_ID) != present || (poly != INVALID_U16_ID) != present {
        return Err(TraceError(format!(
            "invalid gameplay trace collision {kind} identity sentinel"
        )));
    }
    Ok(())
}

fn validate_owner(owner: u32, present: bool, kind: &str) -> Result<(), TraceError> {
    if (owner != INVALID_U32_ID) != present {
        return Err(TraceError(format!(
            "invalid gameplay trace collision {kind} owner sentinel"
        )));
    }
    Ok(())
}

fn validate_plane(plane: [f32; 4], present: bool, kind: &str) -> Result<(), TraceError> {
    if plane.iter().any(|value| !value.is_finite())
        || (!present && plane.iter().any(|value| *value != 0.0))
    {
        return Err(TraceError(format!(
            "invalid gameplay trace collision {kind} plane"
        )));
    }
    Ok(())
}

fn decode_rng_stream(bytes: &[u8]) -> Result<TraceRngStream, TraceError> {
    if bytes[1..4].iter().any(|value| *value != 0) {
        return Err(TraceError(
            "nonzero gameplay trace RNG reserved field".into(),
        ));
    }
    Ok(TraceRngStream {
        id: bytes[0],
        algorithm_version: u32_at(bytes, 4),
        state: [i32_at(bytes, 8), i32_at(bytes, 12), i32_at(bytes, 16)],
        call_count: u64_at(bytes, 20),
    })
}

fn decode_pad(bytes: &[u8]) -> Result<RawPadState, TraceError> {
    if bytes[10] & !1 != 0 {
        return Err(TraceError("unknown gameplay trace pad flags".into()));
    }
    Ok(RawPadState {
        buttons: u16_at(bytes, 0),
        stick_x: bytes[2] as i8,
        stick_y: bytes[3] as i8,
        substick_x: bytes[4] as i8,
        substick_y: bytes[5] as i8,
        trigger_left: bytes[6],
        trigger_right: bytes[7],
        analog_a: bytes[8],
        analog_b: bytes[9],
        connected: bytes[10] & 1 != 0,
        error: bytes[11] as i8,
    })
}
