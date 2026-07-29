use super::*;

pub fn parse_stage_data(input: &[u8]) -> Result<ExtractedStageData, PlannerContractError> {
    let chunk_count = read_u32(input, 0, "orig.stage.chunk_count")? as usize;
    if chunk_count > MAX_STAGE_CHUNKS {
        return Err(PlannerContractError::new(
            "orig.stage.chunk_count",
            format!("exceeds bounded limit {MAX_STAGE_CHUNKS}"),
        ));
    }
    let header_bytes = chunk_count
        .checked_mul(12)
        .ok_or_else(|| PlannerContractError::new("orig.stage.headers", "size overflow"))?;
    require_range(input, 4, header_bytes, "orig.stage.headers")?;
    let records_floor = 4 + header_bytes;
    let mut chunks = Vec::with_capacity(chunk_count);
    let mut stage_information = None;
    let mut room_transforms = Vec::new();
    let mut file_lists = Vec::new();
    let mut room_read_table = Vec::new();
    let mut cameras = Vec::new();
    let mut camera_arrows = Vec::new();
    let mut paths = Vec::new();
    let mut path_points = Vec::new();
    let mut saw_path_table = false;
    let mut saw_path_point_table = false;
    let mut saw_room_read_table = false;
    let mut scene_transitions = Vec::new();
    let mut map_events = Vec::new();
    let mut demo_archive_banks = Vec::new();
    let mut actor_placements = Vec::new();
    let mut treasure_placements = Vec::new();
    let mut player_spawns = Vec::new();
    let mut recognized_ranges = Vec::new();
    let mut total_records = 0_usize;

    for chunk_index in 0..chunk_count {
        let header = 4 + chunk_index * 12;
        let tag_bytes = &input[header..header + 4];
        if !tag_bytes.iter().all(u8::is_ascii_graphic) {
            return Err(PlannerContractError::new(
                "orig.stage.chunk.tag",
                "must contain four printable ASCII bytes",
            ));
        }
        let tag = std::str::from_utf8(tag_bytes)
            .map_err(|_| PlannerContractError::new("orig.stage.chunk.tag", "must be UTF-8"))?
            .to_owned();
        let record_count = read_u32(input, header + 4, "orig.stage.chunk.record_count")?;
        let data_offset = read_u32(input, header + 8, "orig.stage.chunk.data_offset")?;
        let actor_layout = actor_record_layout(&tag);
        let record_size = actor_layout
            .map(|layout| layout.0)
            .or_else(|| recognized_stage_record_size(&tag));
        chunks.push(ExtractedStageChunk {
            tag: tag.clone(),
            record_count,
            data_offset,
            recognized_record_size: record_size.map(|size| size as u8),
        });
        if tag == "RTBL" {
            total_records = total_records
                .checked_add(record_count as usize)
                .ok_or_else(|| PlannerContractError::new("orig.stage.records", "count overflow"))?;
            if total_records > MAX_STAGE_RECORDS {
                return Err(PlannerContractError::new(
                    "orig.stage.records",
                    format!("exceeds bounded limit {MAX_STAGE_RECORDS}"),
                ));
            }
            if saw_room_read_table {
                return Err(PlannerContractError::new(
                    "orig.stage.rtbl",
                    "must contain one unique chunk",
                ));
            }
            saw_room_read_table = true;
            room_read_table = parse_room_read_table(
                input,
                data_offset as usize,
                record_count as usize,
                records_floor,
            )?;
            continue;
        }
        let Some(record_size) = record_size else {
            continue;
        };
        total_records = total_records
            .checked_add(record_count as usize)
            .ok_or_else(|| PlannerContractError::new("orig.stage.records", "count overflow"))?;
        if total_records > MAX_STAGE_RECORDS {
            return Err(PlannerContractError::new(
                "orig.stage.records",
                format!("exceeds bounded limit {MAX_STAGE_RECORDS}"),
            ));
        }
        let start = data_offset as usize;
        if start < records_floor {
            return Err(PlannerContractError::new(
                "orig.stage.chunk.data_offset",
                "overlaps the chunk header table",
            ));
        }
        let bytes = (record_count as usize)
            .checked_mul(record_size)
            .ok_or_else(|| PlannerContractError::new("orig.stage.records", "size overflow"))?;
        require_range(input, start, bytes, "orig.stage.records")?;
        recognized_ranges.push((start, start + bytes, tag.clone()));

        if tag == "STAG" {
            if record_count != 1 || stage_information.is_some() {
                return Err(PlannerContractError::new(
                    "orig.stage.stag",
                    "must contain exactly one unique record",
                ));
            }
            let record = &input[start..start + record_size];
            stage_information = Some(ExtractedStageInformation {
                message_group: record[0x28],
                raw_hex: hex_bytes(record),
            });
            continue;
        }

        if tag == "SCLS" {
            for exit_id in 0..record_count {
                let offset = start + exit_id as usize * record_size;
                let record = &input[offset..offset + record_size];
                let name_end = record[..8].iter().position(|byte| *byte == 0).unwrap_or(8);
                if name_end == 0 || !record[..name_end].iter().all(u8::is_ascii_graphic) {
                    return Err(PlannerContractError::new(
                        "orig.stage.scls.destination_stage",
                        "must contain a nonempty printable ASCII stage name",
                    ));
                }
                let destination_stage = std::str::from_utf8(&record[..name_end])
                    .map_err(|_| {
                        PlannerContractError::new(
                            "orig.stage.scls.destination_stage",
                            "must be UTF-8",
                        )
                    })?
                    .to_owned();
                let raw_layer = record[0x0b] & 0x0f;
                let raw_time = ((record[0x0a] >> 4) & 0x0f) | (record[0x0b] & 0x10);
                scene_transitions.push(ExtractedSceneTransition {
                    exit_id,
                    destination_stage,
                    destination_spawn: record[0x08],
                    destination_room: record[0x09] as i8,
                    scene_layer: (raw_layer < 15).then_some(raw_layer),
                    time_hour: (raw_time < 31).then_some(raw_time),
                    wipe: record[0x0c],
                    wipe_time: (record[0x0b] >> 5) & 7,
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "MULT" {
            for record_index in 0..record_count {
                let offset = start + record_index as usize * record_size;
                let record = &input[offset..offset + record_size];
                let translation_xz = [
                    read_f32(record, 0, "orig.stage.mult.translation_x")?,
                    read_f32(record, 4, "orig.stage.mult.translation_z")?,
                ];
                if !translation_xz
                    .iter()
                    .all(|coordinate| coordinate.is_finite())
                {
                    return Err(PlannerContractError::new(
                        "orig.stage.mult.translation_xz",
                        "must be finite",
                    ));
                }
                room_transforms.push(ExtractedRoomTransform {
                    record_index,
                    room: record[0x0a],
                    translation_xz,
                    angle_y: read_i16(record, 8, "orig.stage.mult.angle_y")?,
                    trailing_byte: record[0x0b],
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "FILI" {
            for record_index in 0..record_count {
                let offset = start + record_index as usize * record_size;
                let record = &input[offset..offset + record_size];
                let parameters = read_u32(record, 0, "orig.stage.fili.parameters")?;
                let sea_level = read_f32(record, 4, "orig.stage.fili.sea_level")?;
                let unknown_float_08 = read_f32(record, 8, "orig.stage.fili.unknown_float_08")?;
                let unknown_float_0c = read_f32(record, 12, "orig.stage.fili.unknown_float_0c")?;
                if ![sea_level, unknown_float_08, unknown_float_0c]
                    .iter()
                    .all(|value| value.is_finite())
                {
                    return Err(PlannerContractError::new(
                        "orig.stage.fili.floats",
                        "must be finite",
                    ));
                }
                let default_camera = record[0x1a];
                let bit_switch = record[0x1b];
                let message_id = read_u16(record, 0x1c, "orig.stage.fili.message_id")?;
                file_lists.push(ExtractedFileList {
                    record_index,
                    parameters,
                    sea_level,
                    unknown_float_08,
                    unknown_float_0c,
                    unknown_bytes_10_19_hex: hex_bytes(&record[0x10..0x1a]),
                    minimap_style: ((parameters >> 3) & 7) as u8,
                    enemy_appear_flag: parameters & 0x2000_0000 != 0,
                    global_wind_level: ((parameters >> 18) & 3) as u8,
                    global_wind_direction: ((parameters >> 15) & 7) as u8,
                    grass_light: ((parameters >> 7) & 0xff) as u8,
                    default_camera,
                    bit_switch,
                    message_id,
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "RCAM" {
            for record_index in 0..record_count {
                let offset = start + record_index as usize * record_size;
                let record = &input[offset..offset + record_size];
                let raw_type_index = read_u16(record, 0x16, "orig.stage.rcam.camera_type_index")?;
                cameras.push(ExtractedCamera {
                    record_index,
                    camera_type: parse_fixed_ascii(
                        &record[..0x10],
                        "orig.stage.rcam.camera_type",
                        false,
                    )?,
                    arrow_index: record[0x10],
                    field_of_view_y: record[0x11],
                    argument_0: record[0x12],
                    argument_1: record[0x13],
                    argument_2: read_u16(record, 0x14, "orig.stage.rcam.argument_2")?,
                    camera_type_index: (raw_type_index != u16::MAX).then_some(raw_type_index),
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "RARO" {
            for record_index in 0..record_count {
                let offset = start + record_index as usize * record_size;
                let record = &input[offset..offset + record_size];
                let position = [
                    read_f32(record, 0, "orig.stage.raro.position_x")?,
                    read_f32(record, 4, "orig.stage.raro.position_y")?,
                    read_f32(record, 8, "orig.stage.raro.position_z")?,
                ];
                if !position.iter().all(|coordinate| coordinate.is_finite()) {
                    return Err(PlannerContractError::new(
                        "orig.stage.raro.position",
                        "must be finite",
                    ));
                }
                camera_arrows.push(ExtractedCameraArrow {
                    record_index,
                    position,
                    angle: [
                        read_i16(record, 0x0c, "orig.stage.raro.angle_x")?,
                        read_i16(record, 0x0e, "orig.stage.raro.angle_y")?,
                        read_i16(record, 0x10, "orig.stage.raro.angle_z")?,
                    ],
                    trailing_i16: read_i16(record, 0x12, "orig.stage.raro.trailing_i16")?,
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "RPAT" {
            if saw_path_table {
                return Err(PlannerContractError::new(
                    "orig.stage.rpat",
                    "must contain one unique chunk",
                ));
            }
            saw_path_table = true;
            for record_index in 0..record_count {
                let offset = start + record_index as usize * record_size;
                let record = &input[offset..offset + record_size];
                let point_offset = read_u32(record, 8, "orig.stage.rpat.point_offset")?;
                if point_offset as usize % 0x10 != 0 {
                    return Err(PlannerContractError::new(
                        "orig.stage.rpat.point_offset",
                        "must align to an RPPN record",
                    ));
                }
                let next_raw = read_u16(record, 2, "orig.stage.rpat.next_path_index")?;
                paths.push(ExtractedPath {
                    record_index,
                    point_count: read_u16(record, 0, "orig.stage.rpat.point_count")?,
                    next_path_index: (next_raw != u16::MAX).then_some(next_raw),
                    path_argument: record[4],
                    closed: record[5] & 1 != 0,
                    closed_raw: record[5],
                    switch_no: (record[6] != u8::MAX).then_some(record[6]),
                    unknown_07: record[7],
                    point_offset,
                    first_point_index: point_offset / 0x10,
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "RPPN" {
            if saw_path_point_table {
                return Err(PlannerContractError::new(
                    "orig.stage.rppn",
                    "must contain one unique chunk",
                ));
            }
            saw_path_point_table = true;
            for record_index in 0..record_count {
                let offset = start + record_index as usize * record_size;
                let record = &input[offset..offset + record_size];
                let position = [
                    read_f32(record, 4, "orig.stage.rppn.position_x")?,
                    read_f32(record, 8, "orig.stage.rppn.position_y")?,
                    read_f32(record, 12, "orig.stage.rppn.position_z")?,
                ];
                if !position.iter().all(|coordinate| coordinate.is_finite()) {
                    return Err(PlannerContractError::new(
                        "orig.stage.rppn.position",
                        "must be finite",
                    ));
                }
                path_points.push(ExtractedPathPoint {
                    record_index,
                    arguments: [record[3], record[0], record[1], record[2]],
                    position,
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "REVT" {
            for record_index in 0..record_count {
                let offset = start + record_index as usize * record_size;
                let record = &input[offset..offset + record_size];
                let event_type = record[0];
                if event_type > 2 {
                    return Err(PlannerContractError::new(
                        "orig.stage.revt.event_type",
                        "is outside the source-audited 0..=2 dispatch",
                    ));
                }
                let event_name = if matches!(event_type, 1 | 2) {
                    Some(parse_fixed_ascii(
                        &record[0x0d..0x1a],
                        "orig.stage.revt.event_name",
                        false,
                    )?)
                } else {
                    None
                };
                map_events.push(ExtractedMapEvent {
                    record_index,
                    event_type,
                    map_tool_id: record[4],
                    priority: record[6],
                    normal_exit_id: {
                        let exit_id = if event_type == 0 {
                            record[0x17]
                        } else {
                            record[7]
                        };
                        (exit_id != u8::MAX).then_some(exit_id)
                    },
                    skip_exit_id: (record[9] != u8::MAX).then_some(record[9]),
                    event_name,
                    switch_no: (record[0x1b] != u8::MAX).then_some(record[0x1b]),
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "LBNK" {
            for layer in 0..record_count {
                let offset = start + layer as usize * record_size;
                let record = &input[offset..offset + record_size];
                let bank = (record[0] != u8::MAX).then_some(record[0]);
                let bank2 = (record[1] != u8::MAX).then_some(record[1]);
                if let Some(value) = bank
                    && (value >= 100 || bank2.is_none_or(|value| value >= 100))
                {
                    return Err(PlannerContractError::new(
                        "orig.stage.lbnk",
                        "configured demo archive bank coordinates must be below 100",
                    ));
                }
                demo_archive_banks.push(ExtractedDemoArchiveBank {
                    layer: layer.try_into().map_err(|_| {
                        PlannerContractError::new(
                            "orig.stage.lbnk.layer",
                            "must fit in one layer byte",
                        )
                    })?,
                    bank,
                    bank2,
                    archive_name: bank
                        .zip(bank2)
                        .map(|(bank, bank2)| format!("Demo{bank:02}_{bank2:02}")),
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        let Some((_, scaled, layer, placement_class)) = actor_layout else {
            unreachable!("all other recognized records are actor placements")
        };

        for record_index in 0..record_count {
            let offset = start + record_index as usize * record_size;
            let record = &input[offset..offset + record_size];
            let name_end = record[..8].iter().position(|byte| *byte == 0).unwrap_or(8);
            if !record[..name_end].iter().all(u8::is_ascii_graphic) {
                return Err(PlannerContractError::new(
                    "orig.stage.actor.name",
                    "must contain printable ASCII bytes",
                ));
            }
            let name = std::str::from_utf8(&record[..name_end])
                .map_err(|_| PlannerContractError::new("orig.stage.actor.name", "must be UTF-8"))?
                .to_owned();
            let position = [
                read_f32(record, 12, "orig.stage.actor.position_x")?,
                read_f32(record, 16, "orig.stage.actor.position_y")?,
                read_f32(record, 20, "orig.stage.actor.position_z")?,
            ];
            if !position.iter().all(|coordinate| coordinate.is_finite()) {
                return Err(PlannerContractError::new(
                    "orig.stage.actor.position",
                    "must be finite",
                ));
            }
            let placement = ExtractedActorPlacement {
                chunk_tag: tag.clone(),
                record_index,
                layer,
                name,
                parameters: read_u32(record, 8, "orig.stage.actor.parameters")?,
                position,
                angle: [
                    read_i16(record, 24, "orig.stage.actor.angle_x")?,
                    read_i16(record, 26, "orig.stage.actor.angle_y")?,
                    read_i16(record, 28, "orig.stage.actor.angle_z")?,
                ],
                set_id: read_u16(record, 30, "orig.stage.actor.set_id")?,
                scale_raw: scaled.then(|| [record[32], record[33], record[34]]),
                raw_hex: hex_bytes(record),
            };
            match placement_class {
                ExtractedPlacementClass::Actor => actor_placements.push(placement),
                ExtractedPlacementClass::Treasure => treasure_placements.push(placement),
                ExtractedPlacementClass::PlayerSpawn => player_spawns.push(placement),
            }
        }
    }
    recognized_ranges.sort_by_key(|range| range.0);
    for pair in recognized_ranges.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(PlannerContractError::new(
                "orig.stage.records",
                format!(
                    "recognized chunks {:?} and {:?} overlap",
                    pair[0].2, pair[1].2
                ),
            ));
        }
    }
    for path in &paths {
        let first = path.first_point_index as usize;
        let end = first
            .checked_add(usize::from(path.point_count))
            .ok_or_else(|| PlannerContractError::new("orig.stage.rpat", "point range overflow"))?;
        if !saw_path_point_table
            || end > path_points.len()
            || path
                .next_path_index
                .is_some_and(|next| usize::from(next) >= paths.len())
        {
            return Err(PlannerContractError::new(
                "orig.stage.rpat",
                "contains an out-of-range RPPN span or next-path index",
            ));
        }
    }
    Ok(ExtractedStageData {
        chunks,
        stage_information,
        room_transforms,
        file_lists,
        room_read_table,
        cameras,
        camera_arrows,
        paths,
        path_points,
        scene_transitions,
        map_events,
        demo_archive_banks,
        actor_placements,
        treasure_placements,
        player_spawns,
    })
}

/// Decode the engine's fixed-table `event_list.dat` format. This captures the
/// authored event/staff/cut/data graph; it does not infer actor callbacks or
/// JStudio `.stb` contents.
pub fn parse_event_list(input: &[u8]) -> Result<ExtractedEventList, PlannerContractError> {
    const HEADER_SIZE: usize = 0x40;
    const EVENT_SIZE: usize = 0xb0;
    const STAFF_SIZE: usize = 0x50;
    const CUT_SIZE: usize = 0x50;
    const DATA_SIZE: usize = 0x40;

    require_range(input, 0, HEADER_SIZE, "orig.event_list.header")?;
    let table = |offset: usize,
                 record_size: usize,
                 field: &'static str|
     -> Result<(usize, usize), PlannerContractError> {
        let start = read_u32(input, offset, field)? as usize;
        let count = read_i32(input, offset + 4, field)?;
        if count < 0 || count as usize > MAX_EVENT_RECORDS {
            return Err(PlannerContractError::new(
                field,
                format!("count must be between 0 and {MAX_EVENT_RECORDS}"),
            ));
        }
        let bytes = (count as usize)
            .checked_mul(record_size)
            .ok_or_else(|| PlannerContractError::new(field, "size overflow"))?;
        if start < HEADER_SIZE && bytes != 0 {
            return Err(PlannerContractError::new(field, "overlaps the header"));
        }
        require_range(input, start, bytes, field)?;
        Ok((start, count as usize))
    };

    let (event_top, event_count) = table(0x00, EVENT_SIZE, "orig.event_list.events")?;
    let (staff_top, staff_count) = table(0x08, STAFF_SIZE, "orig.event_list.staff")?;
    let (cut_top, cut_count) = table(0x10, CUT_SIZE, "orig.event_list.cuts")?;
    let (data_top, data_count) = table(0x18, DATA_SIZE, "orig.event_list.data")?;
    let (float_top, float_count) = table(0x20, 4, "orig.event_list.float_data")?;
    let (integer_top, integer_count) = table(0x28, 4, "orig.event_list.integer_data")?;
    let (string_top, string_count) = table(0x30, 1, "orig.event_list.string_data")?;

    let mut ranges = [
        (event_top, event_top + event_count * EVENT_SIZE, "events"),
        (staff_top, staff_top + staff_count * STAFF_SIZE, "staff"),
        (cut_top, cut_top + cut_count * CUT_SIZE, "cuts"),
        (data_top, data_top + data_count * DATA_SIZE, "data"),
        (float_top, float_top + float_count * 4, "float_data"),
        (integer_top, integer_top + integer_count * 4, "integer_data"),
        (string_top, string_top + string_count, "string_data"),
    ];
    ranges.sort_by_key(|range| range.0);
    let nonempty_ranges = ranges
        .iter()
        .filter(|range| range.0 != range.1)
        .collect::<Vec<_>>();
    for pair in nonempty_ranges.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(PlannerContractError::new(
                "orig.event_list.tables",
                format!("tables {} and {} overlap", pair[0].2, pair[1].2),
            ));
        }
    }

    let float_data_bits = (0..float_count)
        .map(|index| read_u32(input, float_top + index * 4, "orig.event_list.float_data"))
        .collect::<Result<Vec<_>, _>>()?;
    let integer_data = (0..integer_count)
        .map(|index| {
            read_i32(
                input,
                integer_top + index * 4,
                "orig.event_list.integer_data",
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let string_data = &input[string_top..string_top + string_count];

    let mut events = Vec::with_capacity(event_count);
    for index in 0..event_count {
        let offset = event_top + index * EVENT_SIZE;
        let record = &input[offset..offset + EVENT_SIZE];
        require_dense_index(record, 0x20, index, "orig.event_list.event.index")?;
        let staff_in_event = read_i32(record, 0x7c, "orig.event_list.event.staff_count")?;
        if !(0..=20).contains(&staff_in_event) {
            return Err(PlannerContractError::new(
                "orig.event_list.event.staff_count",
                "must be between 0 and 20",
            ));
        }
        let mut staff_indices = Vec::with_capacity(staff_in_event as usize);
        for ordinal in 0..staff_in_event as usize {
            let staff_index = read_i32(
                record,
                0x2c + ordinal * 4,
                "orig.event_list.event.staff_index",
            )?;
            if staff_index < 0 || staff_index as usize >= staff_count {
                return Err(PlannerContractError::new(
                    "orig.event_list.event.staff_index",
                    "references a staff record outside the table",
                ));
            }
            staff_indices.push(staff_index as u32);
        }
        events.push(ExtractedEvent {
            index: index as u32,
            name: parse_fixed_ascii(&record[..0x20], "orig.event_list.event.name", false)?,
            priority: read_i32(record, 0x28, "orig.event_list.event.priority")?,
            staff_indices,
            finish_flags: [
                read_i32(record, 0x88, "orig.event_list.event.start_flag")?,
                read_i32(record, 0x8c, "orig.event_list.event.start_flag")?,
                read_i32(record, 0x90, "orig.event_list.event.start_flag")?,
            ],
            raw_hex: hex_bytes(record),
        });
    }

    let mut staff = Vec::with_capacity(staff_count);
    for index in 0..staff_count {
        let offset = staff_top + index * STAFF_SIZE;
        let record = &input[offset..offset + STAFF_SIZE];
        require_dense_index(record, 0x24, index, "orig.event_list.staff.index")?;
        let start_cut = read_i32(record, 0x30, "orig.event_list.staff.start_cut")?;
        if start_cut < 0 || start_cut as usize >= cut_count {
            return Err(PlannerContractError::new(
                "orig.event_list.staff.start_cut",
                "references a cut outside the table",
            ));
        }
        staff.push(ExtractedEventStaff {
            index: index as u32,
            name: parse_fixed_ascii(&record[..8], "orig.event_list.staff.name", false)?,
            tag_id: read_i32(record, 0x20, "orig.event_list.staff.tag_id")?,
            flag_id: read_u32(record, 0x28, "orig.event_list.staff.flag_id")?,
            staff_type: read_i32(record, 0x2c, "orig.event_list.staff.type")?,
            start_cut_index: start_cut as u32,
            raw_hex: hex_bytes(record),
        });
    }

    let mut cuts = Vec::with_capacity(cut_count);
    for index in 0..cut_count {
        let offset = cut_top + index * CUT_SIZE;
        let record = &input[offset..offset + CUT_SIZE];
        require_dense_index(record, 0x24, index, "orig.event_list.cut.index")?;
        cuts.push(ExtractedEventCut {
            index: index as u32,
            name: parse_fixed_ascii(&record[..0x20], "orig.event_list.cut.name", false)?,
            tag_id: read_u32(record, 0x20, "orig.event_list.cut.tag_id")?,
            start_flags: [
                read_i32(record, 0x28, "orig.event_list.cut.start_flag")?,
                read_i32(record, 0x2c, "orig.event_list.cut.start_flag")?,
                read_i32(record, 0x30, "orig.event_list.cut.start_flag")?,
            ],
            flag_id: read_u32(record, 0x34, "orig.event_list.cut.flag_id")?,
            data_index: optional_table_index(
                read_i32(record, 0x38, "orig.event_list.cut.data_index")?,
                data_count,
                "orig.event_list.cut.data_index",
            )?,
            next_cut_index: optional_table_index(
                read_i32(record, 0x3c, "orig.event_list.cut.next_cut_index")?,
                cut_count,
                "orig.event_list.cut.next_cut_index",
            )?,
            raw_hex: hex_bytes(record),
        });
    }

    let mut data = Vec::with_capacity(data_count);
    for index in 0..data_count {
        let offset = data_top + index * DATA_SIZE;
        let record = &input[offset..offset + DATA_SIZE];
        require_dense_index(record, 0x20, index, "orig.event_list.data.index")?;
        let data_type = read_i32(record, 0x24, "orig.event_list.data.type")?;
        let value_index = read_i32(record, 0x28, "orig.event_list.data.value_index")?;
        let value_count = read_i32(record, 0x2c, "orig.event_list.data.value_count")?;
        if value_index < 0 || value_count <= 0 {
            return Err(PlannerContractError::new(
                "orig.event_list.data.value",
                "must have a nonnegative index and positive count",
            ));
        }
        let value_index = value_index as usize;
        let value_count = value_count as usize;
        let value = match data_type {
            0..=2 => {
                let values = slice_values(
                    &float_data_bits,
                    value_index,
                    value_count,
                    "orig.event_list.data.float_value",
                )?
                .to_vec();
                match data_type {
                    0 => ExtractedEventDataValue::FloatBits { values },
                    1 => ExtractedEventDataValue::VectorBits { values },
                    2 => ExtractedEventDataValue::UnknownFloatBits { values },
                    _ => unreachable!(),
                }
            }
            3 => ExtractedEventDataValue::Integers {
                values: slice_values(
                    &integer_data,
                    value_index,
                    value_count,
                    "orig.event_list.data.integer_value",
                )?
                .to_vec(),
            },
            4 => {
                let bytes = slice_values(
                    string_data,
                    value_index,
                    value_count,
                    "orig.event_list.data.string_value",
                )?;
                let end = bytes
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(bytes.len());
                let ascii = (bytes[..end].iter().all(u8::is_ascii_graphic)
                    && bytes[end..].iter().all(|byte| *byte == 0))
                .then(|| std::str::from_utf8(&bytes[..end]).ok().map(str::to_owned))
                .flatten();
                ExtractedEventDataValue::StringBytes {
                    raw_hex: hex_bytes(bytes),
                    ascii,
                }
            }
            _ => {
                return Err(PlannerContractError::new(
                    "orig.event_list.data.type",
                    "is outside the source-audited 0..=4 dispatch",
                ));
            }
        };
        data.push(ExtractedEventData {
            index: index as u32,
            name: parse_fixed_ascii(&record[..0x20], "orig.event_list.data.name", false)?,
            data_type,
            value_index: value_index as u32,
            value_count: value_count as u32,
            next_data_index: optional_table_index(
                read_i32(record, 0x30, "orig.event_list.data.next_data_index")?,
                data_count,
                "orig.event_list.data.next_data_index",
            )?,
            value,
            raw_hex: hex_bytes(record),
        });
    }

    Ok(ExtractedEventList {
        resource_size: input.len().try_into().map_err(|_| {
            PlannerContractError::new("orig.event_list", "resource size exceeds u32")
        })?,
        events,
        staff,
        cuts,
        data,
        float_data_bits,
        integer_data,
        string_data_hex: hex_bytes(string_data),
    })
}

fn require_dense_index(
    record: &[u8],
    offset: usize,
    expected: usize,
    field: &'static str,
) -> Result<(), PlannerContractError> {
    if read_u32(record, offset, field)? as usize != expected {
        return Err(PlannerContractError::new(
            field,
            "must equal the record's dense table index",
        ));
    }
    Ok(())
}

fn optional_table_index(
    value: i32,
    count: usize,
    field: &'static str,
) -> Result<Option<u32>, PlannerContractError> {
    if value == -1 {
        return Ok(None);
    }
    if value < 0 || value as usize >= count {
        return Err(PlannerContractError::new(
            field,
            "references a record outside its table",
        ));
    }
    Ok(Some(value as u32))
}

fn slice_values<'a, T>(
    values: &'a [T],
    start: usize,
    count: usize,
    field: &'static str,
) -> Result<&'a [T], PlannerContractError> {
    let end = start
        .checked_add(count)
        .ok_or_else(|| PlannerContractError::new(field, "range overflow"))?;
    values
        .get(start..end)
        .ok_or_else(|| PlannerContractError::new(field, "range exceeds its backing table"))
}

fn parse_room_read_table(
    input: &[u8],
    table_offset: usize,
    room_count: usize,
    records_floor: usize,
) -> Result<Vec<ExtractedRoomRead>, PlannerContractError> {
    if room_count > 64 {
        return Err(PlannerContractError::new(
            "orig.stage.rtbl.room_count",
            "exceeds the source-audited 64-room control table",
        ));
    }
    if table_offset < records_floor {
        return Err(PlannerContractError::new(
            "orig.stage.rtbl.table_offset",
            "overlaps the chunk header table",
        ));
    }
    let table_bytes = room_count
        .checked_mul(4)
        .ok_or_else(|| PlannerContractError::new("orig.stage.rtbl", "table size overflow"))?;
    require_range(input, table_offset, table_bytes, "orig.stage.rtbl.table")?;
    let mut rooms = Vec::with_capacity(room_count);
    for room_index in 0..room_count {
        let pointer_offset = table_offset + room_index * 4;
        let record_offset = read_u32(input, pointer_offset, "orig.stage.rtbl.record_offset")?;
        let record_start = record_offset as usize;
        if record_start < records_floor {
            return Err(PlannerContractError::new(
                "orig.stage.rtbl.record_offset",
                "must follow the chunk header table",
            ));
        }
        require_range(input, record_start, 8, "orig.stage.rtbl.record")?;
        let raw_header = &input[record_start..record_start + 8];
        let load_count = usize::from(raw_header[0]);
        let room_list_offset = read_u32(raw_header, 4, "orig.stage.rtbl.room_list_offset")?;
        let room_list_start = room_list_offset as usize;
        if room_list_start < records_floor && load_count != 0 {
            return Err(PlannerContractError::new(
                "orig.stage.rtbl.room_list_offset",
                "overlaps the chunk header table",
            ));
        }
        require_range(
            input,
            room_list_start,
            load_count,
            "orig.stage.rtbl.room_list",
        )?;
        let raw_room_list = &input[room_list_start..room_list_start + load_count];
        let load_rooms = raw_room_list
            .iter()
            .map(|raw| ExtractedLoadedRoom {
                room: raw & 0x3f,
                load_background: raw & 0x80 != 0,
                unknown_bit_6: raw & 0x40 != 0,
                raw: *raw,
            })
            .collect();
        rooms.push(ExtractedRoomRead {
            room_index: room_index as u32,
            record_offset,
            room_list_offset,
            reverb: raw_header[1] & 0x7f,
            reverb_raw: raw_header[1],
            time_pass: raw_header[2] & 3,
            vrbox_enabled: raw_header[2] & 8 != 0,
            flags_raw: raw_header[2],
            padding: raw_header[3],
            load_rooms,
            raw_header_hex: hex_bytes(raw_header),
            raw_room_list_hex: hex_bytes(raw_room_list),
        });
    }
    Ok(rooms)
}

fn recognized_stage_record_size(tag: &str) -> Option<usize> {
    match tag {
        "STAG" => Some(0x3c),
        "SCLS" => Some(0x0d),
        "REVT" => Some(0x1c),
        "LBNK" => Some(0x03),
        "MULT" => Some(0x0c),
        "FILI" => Some(0x20),
        "RCAM" => Some(0x18),
        "RARO" => Some(0x14),
        "RPAT" => Some(0x0c),
        "RPPN" => Some(0x10),
        _ => None,
    }
}

fn parse_fixed_ascii(
    bytes: &[u8],
    field: &'static str,
    allow_empty: bool,
) -> Result<String, PlannerContractError> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    if (!allow_empty && end == 0) || !bytes[..end].iter().all(u8::is_ascii_graphic) {
        return Err(PlannerContractError::new(
            field,
            "must contain printable ASCII before its first NUL",
        ));
    }
    std::str::from_utf8(&bytes[..end])
        .map(str::to_owned)
        .map_err(|_| PlannerContractError::new(field, "must be UTF-8"))
}

pub(super) fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Copy)]
enum ExtractedPlacementClass {
    Actor,
    Treasure,
    PlayerSpawn,
}

fn actor_record_layout(tag: &str) -> Option<(usize, bool, Option<u8>, ExtractedPlacementClass)> {
    if tag == "PLYR" {
        return Some((0x20, false, None, ExtractedPlacementClass::PlayerSpawn));
    }
    if tag == "TRES" {
        return Some((0x20, false, None, ExtractedPlacementClass::Treasure));
    }
    if matches!(tag, "ACTR" | "TGOB") {
        return Some((0x20, false, None, ExtractedPlacementClass::Actor));
    }
    if matches!(tag, "SCOB" | "TGSC" | "TGDR" | "Door") {
        return Some((0x24, true, None, ExtractedPlacementClass::Actor));
    }
    if tag.len() != 4 {
        return None;
    }
    let (prefix, scaled, placement_class) = match &tag[..3] {
        "ACT" => ("ACT", false, ExtractedPlacementClass::Actor),
        "TRE" => ("TRE", false, ExtractedPlacementClass::Treasure),
        "SCO" | "Doo" => (&tag[..3], true, ExtractedPlacementClass::Actor),
        _ => return None,
    };
    debug_assert_eq!(prefix, &tag[..3]);
    decode_layer(tag.as_bytes()[3]).map(|layer| {
        if scaled {
            (0x24, true, Some(layer), placement_class)
        } else {
            (0x20, false, Some(layer), placement_class)
        }
    })
}

fn decode_layer(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'e' => Some(byte - b'a' + 10),
        b'A'..=b'E' => Some(byte - b'A' + 10),
        _ => None,
    }
}
