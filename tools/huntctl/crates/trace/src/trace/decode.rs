use super::*;

pub fn decode_and_summarize(bytes: &[u8]) -> Result<TraceSummary, TraceError> {
    let decoded = decode(bytes)?;
    Ok(summarize(decoded))
}

pub fn decode(bytes: &[u8]) -> Result<DecodedTrace, TraceError> {
    if bytes.len() < 12 {
        return Err(TraceError("truncated gameplay trace header".into()));
    }
    if &bytes[..8] != MAGIC {
        return Err(TraceError("bad gameplay trace magic".into()));
    }
    match u16_at(bytes, 8) {
        1 => decode_v1(bytes),
        2 => decode_columnar(bytes, 2, V2_HEADER_SIZE, TapeBoot::Process),
        3 => decode_columnar(bytes, 3, V3_HEADER_SIZE, decode_v3_boot(bytes)?),
        4 => {
            let (boot, data_end) = decode_v4_boot(bytes)?;
            decode_columnar(&bytes[..data_end], 4, V4_HEADER_SIZE, boot)
        }
        5 => {
            let (boot, data_end) = decode_v5_boot(bytes)?;
            decode_columnar(&bytes[..data_end], 5, V5_HEADER_SIZE, boot)
        }
        version => Err(TraceError(format!(
            "unsupported gameplay trace version {version}"
        ))),
    }
}

fn decode_v1(bytes: &[u8]) -> Result<DecodedTrace, TraceError> {
    if bytes.len() < V1_HEADER_SIZE {
        return Err(TraceError("truncated gameplay trace v1 header".into()));
    }
    if usize::from(u16_at(bytes, 10)) != V1_RECORD_SIZE {
        return Err(TraceError(
            "unsupported gameplay trace v1 record size".into(),
        ));
    }
    let count = count_at(bytes, 20)?;
    let expected = checked_region_end(V1_HEADER_SIZE, count, V1_RECORD_SIZE)?;
    if bytes.len() != expected {
        return Err(TraceError(format!(
            "gameplay trace size mismatch: expected {expected}, got {}",
            bytes.len()
        )));
    }
    if u32_at(bytes, 28) > 1 || u32_at(bytes, 32) != 0 {
        return Err(TraceError(
            "noncanonical gameplay trace v1 flags or reserved header".into(),
        ));
    }
    validate_tick_rate(bytes)?;
    let records = bytes[V1_HEADER_SIZE..]
        .chunks_exact(V1_RECORD_SIZE)
        .map(decode_v1_record)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DecodedTrace {
        version: 1,
        boot: TapeBoot::Process,
        tick_rate_numerator: u32_at(bytes, 12),
        tick_rate_denominator: u32_at(bytes, 16),
        requested_channels: TraceChannel::Core.bit()
            | TraceChannel::Stage.bit()
            | TraceChannel::AppliedPads.bit()
            | TraceChannel::PlayerMotion.bit()
            | TraceChannel::Event.bit()
            | TraceChannel::SceneExit.bit(),
        capacity_exhausted: u32_at(bytes, 28) != 0,
        retention: None,
        channel_formats: BTreeMap::new(),
        records,
    })
}

fn decode_v1_record(bytes: &[u8]) -> Result<TraceRecord, TraceError> {
    if u16_at(bytes, 100) != 0 {
        return Err(TraceError(
            "nonzero gameplay trace v1 record reserved field".into(),
        ));
    }
    let raw_frame = u64_at(bytes, 8);
    let legacy_flags = u32_at(bytes, 28);
    let proc_id = u16_at(bytes, 70);
    let exit_actor = i16_at(bytes, 82);
    let exit_distance = f32_at(bytes, 96);
    let mut channel_status = BTreeMap::new();
    channel_status.insert(TraceChannel::Core, TraceChannelStatus::Present);
    channel_status.insert(TraceChannel::Stage, TraceChannelStatus::Present);
    channel_status.insert(TraceChannel::AppliedPads, TraceChannelStatus::Present);
    channel_status.insert(
        TraceChannel::PlayerMotion,
        if legacy_flags & LEGACY_PLAYER_PRESENT != 0 {
            TraceChannelStatus::Present
        } else {
            TraceChannelStatus::Absent
        },
    );
    channel_status.insert(TraceChannel::Event, TraceChannelStatus::Present);
    channel_status.insert(
        TraceChannel::SceneExit,
        if exit_actor != -1 {
            TraceChannelStatus::Present
        } else {
            TraceChannelStatus::Absent
        },
    );
    let simulation_tick = u64_at(bytes, 0);
    let boundary_index = simulation_tick
        .checked_add(1)
        .ok_or_else(|| TraceError("gameplay trace v1 simulation tick overflow".into()))?;
    Ok(TraceRecord {
        boundary_index,
        simulation_tick,
        tape_frame: (raw_frame != u64::MAX).then_some(raw_frame),
        observation_phase: TracePhase::PostSimulation,
        input_source: ((legacy_flags & LEGACY_TAPE_PLAYING != 0) as u8 * INPUT_TAPE)
            | ((legacy_flags & LEGACY_CONTROLLER_PLAYING != 0) as u8 * INPUT_CONTROLLER),
        channel_status,
        stage_name: decode_name(&bytes[16..24])?,
        room: bytes[24] as i8,
        layer: bytes[25] as i8,
        point: i16_at(bytes, 26),
        flags: legacy_flags,
        player_actor_name: i16_at(bytes, 32),
        current_angle: [0, i16_at(bytes, 34), 0],
        shape_angle: [0, i16_at(bytes, 36), 0],
        current_angle_y: i16_at(bytes, 34),
        shape_angle_y: i16_at(bytes, 36),
        buttons: u16_at(bytes, 38),
        stick_x: bytes[40] as i8,
        stick_y: bytes[41] as i8,
        position: [f32_at(bytes, 42), f32_at(bytes, 46), f32_at(bytes, 50)],
        velocity: [f32_at(bytes, 54), f32_at(bytes, 58), f32_at(bytes, 62)],
        forward_speed: f32_at(bytes, 66),
        player_proc_id: (proc_id != u16::MAX).then_some(proc_id),
        event_id: i16_at(bytes, 72),
        event_mode: bytes[74],
        event_status: bytes[75],
        event_map_tool_id: bytes[76],
        pad_error: bytes[77] as i8,
        event_name_hash: u32_at(bytes, 78),
        event_name_hash_present: true,
        nearest_scene_exit_actor_name: (exit_actor != -1).then_some(exit_actor),
        nearest_scene_exit_position: [f32_at(bytes, 84), f32_at(bytes, 88), f32_at(bytes, 92)],
        nearest_scene_exit_distance: (exit_distance >= 0.0).then_some(exit_distance),
        ..TraceRecord::default()
    })
}

fn decode_v3_boot(bytes: &[u8]) -> Result<TapeBoot, TraceError> {
    if bytes.len() < V3_HEADER_SIZE {
        return Err(TraceError("truncated gameplay trace v3 header".into()));
    }
    let extension = &bytes[V2_HEADER_SIZE..V3_HEADER_SIZE];
    match extension[0] {
        0 => {
            if extension.iter().any(|byte| *byte != 0) {
                return Err(TraceError(
                    "noncanonical process boot in gameplay trace v3".into(),
                ));
            }
            Ok(TapeBoot::Process)
        }
        1 => {
            let save_slot = extension[1];
            let stage_len = usize::from(extension[6]);
            if save_slot > 3
                || stage_len == 0
                || stage_len > 16
                || extension[7] != 0
                || extension[8 + stage_len..].iter().any(|byte| *byte != 0)
                || extension[8..8 + stage_len]
                    .iter()
                    .any(|byte| !(0x21..=0x7e).contains(byte) || *byte == b',')
            {
                return Err(TraceError(
                    "noncanonical stage boot in gameplay trace v3".into(),
                ));
            }
            let stage = String::from_utf8(extension[8..8 + stage_len].to_vec())
                .map_err(|_| TraceError("invalid stage name in gameplay trace v3".into()))?;
            Ok(TapeBoot::Stage {
                stage,
                room: extension[2] as i8,
                layer: extension[3] as i8,
                point: i16::from_le_bytes([extension[4], extension[5]]),
                save_slot: (save_slot != 0).then_some(save_slot),
                fixture: None,
            })
        }
        _ => Err(TraceError("unknown boot kind in gameplay trace v3".into())),
    }
}

fn decode_v4_boot(bytes: &[u8]) -> Result<(TapeBoot, usize), TraceError> {
    decode_v4_or_v5_boot(bytes, true)
}

fn decode_v5_boot(bytes: &[u8]) -> Result<(TapeBoot, usize), TraceError> {
    decode_v4_or_v5_boot(bytes, false)
}

fn decode_v4_or_v5_boot(
    bytes: &[u8],
    require_reserved_zero: bool,
) -> Result<(TapeBoot, usize), TraceError> {
    if bytes.len() < V4_HEADER_SIZE {
        return Err(TraceError("truncated gameplay trace v4/v5 header".into()));
    }
    let extension = &bytes[V2_HEADER_SIZE..88];
    let mut boot = match extension[0] {
        0 => {
            if extension.iter().any(|byte| *byte != 0) {
                return Err(TraceError(
                    "noncanonical process boot in gameplay trace v4".into(),
                ));
            }
            TapeBoot::Process
        }
        1 => {
            let save_slot = extension[1];
            let stage_len = usize::from(extension[6]);
            if save_slot > 3
                || stage_len == 0
                || stage_len > 16
                || extension[7] != 0
                || extension[8 + stage_len..].iter().any(|byte| *byte != 0)
                || extension[8..8 + stage_len]
                    .iter()
                    .any(|byte| !(0x21..=0x7e).contains(byte) || *byte == b',')
            {
                return Err(TraceError(
                    "noncanonical stage boot in gameplay trace v4".into(),
                ));
            }
            TapeBoot::Stage {
                stage: String::from_utf8(extension[8..8 + stage_len].to_vec())
                    .map_err(|_| TraceError("invalid stage name in gameplay trace v4".into()))?,
                room: extension[2] as i8,
                layer: extension[3] as i8,
                point: i16::from_le_bytes([extension[4], extension[5]]),
                save_slot: (save_slot != 0).then_some(save_slot),
                fixture: None,
            }
        }
        _ => return Err(TraceError("unknown boot kind in gameplay trace v4".into())),
    };
    if require_reserved_zero && bytes[100..V4_HEADER_SIZE].iter().any(|byte| *byte != 0) {
        return Err(TraceError(
            "nonzero gameplay trace v4 reserved field".into(),
        ));
    }
    let fixture_offset = usize_at_u64(bytes, 88)?;
    let fixture_size = usize::try_from(u32_at(bytes, 96))
        .map_err(|_| TraceError("gameplay trace fixture size overflow".into()))?;
    if fixture_size == 0 {
        if fixture_offset != 0 {
            return Err(TraceError(
                "gameplay trace v4 has an offset for an absent fixture".into(),
            ));
        }
        return Ok((boot, bytes.len()));
    }
    if fixture_offset < V4_HEADER_SIZE
        || fixture_offset
            .checked_add(fixture_size)
            .is_none_or(|end| end != bytes.len())
    {
        return Err(TraceError(
            "gameplay trace v4 fixture range is invalid".into(),
        ));
    }
    let fixture = ScenarioFixture::decode(&bytes[fixture_offset..])
        .map_err(|error| TraceError(format!("invalid gameplay trace fixture: {error}")))?;
    match &mut boot {
        TapeBoot::Stage {
            fixture: target, ..
        } => *target = Some(fixture),
        TapeBoot::Process => {
            return Err(TraceError(
                "process boot gameplay trace cannot carry a scenario fixture".into(),
            ));
        }
    }
    Ok((boot, fixture_offset))
}
