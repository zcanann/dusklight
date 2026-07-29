use super::*;

pub(super) fn validate_extracted_placement(
    placement: &ExtractedActorPlacement,
    kind: PlacementKind,
) -> Result<(), PlannerContractError> {
    let scaled = kind == PlacementKind::ScaledActor;
    if (placement.scale_raw.is_some()) != scaled
        || matches!(kind, PlacementKind::Treasure | PlacementKind::PlayerSpawn)
            && placement.scale_raw.is_some()
    {
        return Err(PlannerContractError::new(
            "orig_world.placement.kind",
            "does not match the retained scaled-record fields",
        ));
    }
    let expected_size = if scaled { 36 } else { 32 };
    let raw = decode_hex_exact(
        &placement.raw_hex,
        expected_size,
        "orig_world.placement.raw_hex",
    )?;
    let name = fixed_name(&raw[..8], "orig_world.placement.name")?;
    let parameters = u32::from_be_bytes(raw[8..12].try_into().unwrap());
    let position = [
        f32::from_bits(u32::from_be_bytes(raw[12..16].try_into().unwrap())),
        f32::from_bits(u32::from_be_bytes(raw[16..20].try_into().unwrap())),
        f32::from_bits(u32::from_be_bytes(raw[20..24].try_into().unwrap())),
    ];
    let angle = [
        i16::from_be_bytes(raw[24..26].try_into().unwrap()),
        i16::from_be_bytes(raw[26..28].try_into().unwrap()),
        i16::from_be_bytes(raw[28..30].try_into().unwrap()),
    ];
    let set_id = u16::from_be_bytes(raw[30..32].try_into().unwrap());
    let scale = scaled.then(|| [raw[32], raw[33], raw[34]]);
    if name != placement.name
        || parameters != placement.parameters
        || position.map(f32::to_bits) != placement.position.map(f32::to_bits)
        || angle != placement.angle
        || set_id != placement.set_id
        || scale != placement.scale_raw
        || !position.iter().all(|value| value.is_finite())
        || placement.layer != layer_for_tag(&placement.chunk_tag)
    {
        return Err(PlannerContractError::new(
            "orig_world.placement",
            format!(
                "{} record {} decoded fields do not match the retained raw placement record (name={}, parameters={}, position={}, angle={}, set_id={}, scale={}, layer={})",
                placement.chunk_tag,
                placement.record_index,
                name == placement.name,
                parameters == placement.parameters,
                position.map(f32::to_bits) == placement.position.map(f32::to_bits),
                angle == placement.angle,
                set_id == placement.set_id,
                scale == placement.scale_raw,
                placement.layer == layer_for_tag(&placement.chunk_tag),
            ),
        ));
    }
    Ok(())
}

pub(super) fn validate_native_inventory(
    inventory: &WorldInventory,
) -> Result<(), PlannerContractError> {
    inventory.validate()?;
    if !inventory.collisions.is_empty() || !inventory.load_triggers.is_empty() {
        return Err(PlannerContractError::new(
            "orig_world.collision",
            "must remain empty while collision coverage is unavailable",
        ));
    }
    let mut source_by_digest = BTreeMap::new();
    let mut previous_scope = None;
    for source in &inventory.sources {
        let order = scope_order(source.scope);
        let expected_path = match source.scope {
            SourceScope {
                kind: SourceKind::Stage,
                room: None,
            } => "stage.dzs",
            SourceScope {
                kind: SourceKind::Room,
                room: Some(_),
            } => "room.dzr",
            _ => {
                return Err(PlannerContractError::new(
                    "orig_world.sources.scope",
                    "is not a valid stage or room scope",
                ));
            }
        };
        if previous_scope.is_some_and(|previous| previous >= order)
            || matches!(source.scope.room, Some(room) if room < 0)
            || source.archive_sha256 == Digest::ZERO
            || source.stage_data_sha256 == Digest::ZERO
            || source.stage_data_path != expected_path
            || source.kcl_path.is_some()
            || source.kcl_sha256.is_some()
            || source.plc_path.is_some()
            || source.plc_sha256.is_some()
            || source.addressable_prisms != 0
            || source_by_digest
                .insert(source.stage_data_sha256, source.scope)
                .is_some()
        {
            return Err(PlannerContractError::new(
                "orig_world.sources",
                "must be ordered, unique, content-addressed native DZS/DZR sources without collision claims",
            ));
        }
        previous_scope = Some(order);
    }
    if previous_scope.is_none() || scope_order(inventory.sources[0].scope) != (0, -1) {
        return Err(PlannerContractError::new(
            "orig_world.sources",
            "must begin with one stage source",
        ));
    }

    let mut seen_chunk_keys = BTreeSet::new();
    let chunk_keys = inventory
        .chunks
        .iter()
        .map(|chunk| {
            if source_by_digest.get(&chunk.source_sha256) != Some(&chunk.scope)
                || chunk.tag.len() != 4
                || chunk.record_count > 1_000_000
                || chunk.recognized_record_size != recognized_record_size(&chunk.tag)
                || !seen_chunk_keys.insert((chunk.source_sha256, chunk.tag.as_str()))
            {
                return Err(PlannerContractError::new(
                    "orig_world.chunks",
                    "contains an invalid source, tag, count, or record size",
                ));
            }
            Ok((chunk.source_sha256, chunk.tag.as_str(), chunk.record_count))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut record_ids = BTreeSet::new();
    for placement in inventory.placements.iter().chain(&inventory.player_spawns) {
        let Some((_, _, count)) = chunk_keys.iter().find(|(digest, tag, _)| {
            *digest == placement.source_sha256 && *tag == placement.chunk_tag
        }) else {
            return Err(PlannerContractError::new(
                "orig_world.placements",
                "references an absent chunk",
            ));
        };
        if placement_kind_for_tag(&placement.chunk_tag) != Some(placement.kind)
            || source_by_digest.get(&placement.source_sha256) != Some(&placement.scope)
            || placement.record_index >= *count
            || placement.stable_id
                != source_record_id(
                    placement.scope,
                    placement.source_sha256,
                    &placement.chunk_tag,
                    placement.record_index,
                )
            || !record_ids.insert(placement.stable_id.as_str())
        {
            return Err(PlannerContractError::new(
                "orig_world.placements",
                "contains an invalid, duplicate, or out-of-range source record",
            ));
        }
        let extracted = ExtractedActorPlacement {
            chunk_tag: placement.chunk_tag.clone(),
            record_index: placement.record_index as u32,
            layer: placement.layer,
            name: placement.name.clone(),
            parameters: placement.parameters,
            position: [
                placement.position.x,
                placement.position.y,
                placement.position.z,
            ],
            angle: placement.angle,
            set_id: placement.set_id,
            scale_raw: placement.scale_raw,
            raw_hex: placement.raw_hex.clone(),
        };
        validate_extracted_placement(&extracted, placement.kind)?;
    }
    if inventory
        .placements
        .iter()
        .any(|placement| placement.kind == PlacementKind::PlayerSpawn)
        || inventory
            .player_spawns
            .iter()
            .any(|placement| placement.kind != PlacementKind::PlayerSpawn)
    {
        return Err(PlannerContractError::new(
            "orig_world.placements",
            "must keep ordinary and player-spawn collections distinct",
        ));
    }
    for exit in &inventory.exits {
        let Some((_, _, count)) = chunk_keys
            .iter()
            .find(|(digest, tag, _)| *digest == exit.source_sha256 && *tag == "SCLS")
        else {
            return Err(PlannerContractError::new(
                "orig_world.exits",
                "references an absent SCLS chunk",
            ));
        };
        if source_by_digest.get(&exit.source_sha256) != Some(&exit.scope)
            || exit.record_index >= *count
            || exit.stable_id
                != source_record_id(exit.scope, exit.source_sha256, "SCLS", exit.record_index)
            || !record_ids.insert(exit.stable_id.as_str())
        {
            return Err(PlannerContractError::new(
                "orig_world.exits",
                "contains an invalid, duplicate, or out-of-range SCLS record",
            ));
        }
        let raw = decode_hex_exact(&exit.raw_hex, 13, "orig_world.exit.raw_hex")?;
        let raw_layer = raw[11] & 0x0f;
        let raw_hour = ((raw[10] >> 4) & 0x0f) | (raw[11] & 0x10);
        if fixed_name(&raw[..8], "orig_world.exit.destination_stage")? != exit.destination_stage
            || exit.destination_point != i16::from(raw[8])
            || exit.destination_room != raw[9] as i8
            || exit.destination_layer != if raw_layer < 15 { raw_layer as i8 } else { -1 }
            || exit.wipe != if raw[12] == 15 { 0 } else { raw[12] }
            || exit.wipe_time != (raw[11] >> 5) & 7
            || exit.time_hour != if raw_hour < 31 { raw_hour as i8 } else { -1 }
            || exit.raw_start != raw[8]
            || exit.raw_field_a != raw[10]
            || exit.raw_field_b != raw[11]
            || exit.raw_wipe != raw[12]
        {
            return Err(PlannerContractError::new(
                "orig_world.exits",
                "decoded fields do not match the retained raw SCLS record",
            ));
        }
    }
    for (digest, tag, count) in &chunk_keys {
        if let Some(kind) = placement_kind_for_tag(tag) {
            let records = inventory
                .placements
                .iter()
                .chain(&inventory.player_spawns)
                .filter(|placement| {
                    placement.source_sha256 == *digest
                        && placement.chunk_tag == *tag
                        && placement.kind == kind
                })
                .count();
            if records != *count {
                return Err(PlannerContractError::new(
                    "orig_world.placements",
                    "does not completely cover one recognized placement chunk",
                ));
            }
        } else if *tag == "SCLS"
            && inventory
                .exits
                .iter()
                .filter(|exit| exit.source_sha256 == *digest)
                .count()
                != *count
        {
            return Err(PlannerContractError::new(
                "orig_world.exits",
                "does not completely cover one SCLS chunk",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_stage_metadata(
    inventory: &WorldInventory,
    metadata: &NativeStageMetadata,
) -> Result<(), PlannerContractError> {
    metadata.validate_records()?;
    if metadata.stage != inventory.stage {
        return Err(PlannerContractError::new(
            "orig_world.stage_metadata.stage",
            "does not match its inventory stage",
        ));
    }
    let source_scopes = inventory
        .sources
        .iter()
        .map(|source| (source.stage_data_sha256, source.scope))
        .collect::<BTreeMap<_, _>>();
    let chunk_count = |digest: Digest, tag: &str| {
        inventory
            .chunks
            .iter()
            .find(|chunk| chunk.source_sha256 == digest && chunk.tag == tag)
            .map(|chunk| chunk.record_count)
            .unwrap_or(0)
    };

    for record in &metadata.room_transforms {
        let transform = &record.transform;
        if record.stage != inventory.stage
            || source_scopes.get(&record.source_sha256) != Some(&record.scope)
            || transform.record_index as usize >= chunk_count(record.source_sha256, "MULT")
        {
            return Err(PlannerContractError::new(
                "orig_world.room_transforms",
                "contains a MULT record outside its exact source chunk",
            ));
        }
    }

    for record in &metadata.file_lists {
        let file_list = &record.file_list;
        if record.stage != inventory.stage
            || source_scopes.get(&record.source_sha256) != Some(&record.scope)
            || file_list.record_index as usize >= chunk_count(record.source_sha256, "FILI")
        {
            return Err(PlannerContractError::new(
                "orig_world.file_lists",
                "contains a FILI record outside its exact source chunk",
            ));
        }
    }

    for record in &metadata.room_reads {
        if record.stage != inventory.stage
            || source_scopes.get(&record.source_sha256) != Some(&record.scope)
            || record.room_read.room_index as usize >= chunk_count(record.source_sha256, "RTBL")
        {
            return Err(PlannerContractError::new(
                "orig_world.room_reads",
                "contains an RTBL record outside its exact source chunk",
            ));
        }
    }

    for record in &metadata.cameras {
        let arrow_count = chunk_count(record.source_sha256, "RARO");
        if record.stage != inventory.stage
            || source_scopes.get(&record.source_sha256) != Some(&record.scope)
            || record.camera.record_index as usize >= chunk_count(record.source_sha256, "RCAM")
            || usize::from(record.camera.arrow_index) >= arrow_count
        {
            return Err(PlannerContractError::new(
                "orig_world.cameras",
                "contains an RCAM record outside its exact source chunk or referencing a missing RARO record",
            ));
        }
    }

    for record in &metadata.camera_arrows {
        if record.stage != inventory.stage
            || source_scopes.get(&record.source_sha256) != Some(&record.scope)
            || record.arrow.record_index as usize >= chunk_count(record.source_sha256, "RARO")
        {
            return Err(PlannerContractError::new(
                "orig_world.camera_arrows",
                "contains a RARO record outside its exact source chunk",
            ));
        }
    }

    for record in &metadata.paths {
        let path_count = chunk_count(record.source_sha256, "RPAT");
        let point_count = chunk_count(record.source_sha256, "RPPN");
        let first = record.path.first_point_index as usize;
        let end = first
            .checked_add(usize::from(record.path.point_count))
            .ok_or_else(|| PlannerContractError::new("orig_world.paths", "point range overflow"))?;
        if record.stage != inventory.stage
            || source_scopes.get(&record.source_sha256) != Some(&record.scope)
            || record.path.record_index as usize >= path_count
            || end > point_count
            || record
                .path
                .next_path_index
                .is_some_and(|next| usize::from(next) >= path_count)
        {
            return Err(PlannerContractError::new(
                "orig_world.paths",
                "contains an RPAT record outside its source chunk or referencing an absent path/point",
            ));
        }
    }

    for record in &metadata.path_points {
        if record.stage != inventory.stage
            || source_scopes.get(&record.source_sha256) != Some(&record.scope)
            || record.point.record_index as usize >= chunk_count(record.source_sha256, "RPPN")
        {
            return Err(PlannerContractError::new(
                "orig_world.path_points",
                "contains an RPPN record outside its exact source chunk",
            ));
        }
    }

    let expected_transforms = inventory
        .chunks
        .iter()
        .filter(|chunk| chunk.tag == "MULT")
        .map(|chunk| chunk.record_count)
        .sum::<usize>();
    let expected_file_lists = inventory
        .chunks
        .iter()
        .filter(|chunk| chunk.tag == "FILI")
        .map(|chunk| chunk.record_count)
        .sum::<usize>();
    let expected_room_reads = inventory
        .chunks
        .iter()
        .filter(|chunk| chunk.tag == "RTBL")
        .map(|chunk| chunk.record_count)
        .sum::<usize>();
    let expected_cameras = inventory
        .chunks
        .iter()
        .filter(|chunk| chunk.tag == "RCAM")
        .map(|chunk| chunk.record_count)
        .sum::<usize>();
    let expected_camera_arrows = inventory
        .chunks
        .iter()
        .filter(|chunk| chunk.tag == "RARO")
        .map(|chunk| chunk.record_count)
        .sum::<usize>();
    let expected_paths = inventory
        .chunks
        .iter()
        .filter(|chunk| chunk.tag == "RPAT")
        .map(|chunk| chunk.record_count)
        .sum::<usize>();
    let expected_path_points = inventory
        .chunks
        .iter()
        .filter(|chunk| chunk.tag == "RPPN")
        .map(|chunk| chunk.record_count)
        .sum::<usize>();
    if metadata.room_transforms.len() != expected_transforms
        || metadata.file_lists.len() != expected_file_lists
        || metadata.room_reads.len() != expected_room_reads
        || metadata.cameras.len() != expected_cameras
        || metadata.camera_arrows.len() != expected_camera_arrows
        || metadata.paths.len() != expected_paths
        || metadata.path_points.len() != expected_path_points
    {
        return Err(PlannerContractError::new(
            "orig_world.stage_metadata",
            "does not completely cover its recognized MULT, FILI, RTBL, RCAM, RARO, RPAT, and RPPN chunks",
        ));
    }
    Ok(())
}

pub(super) fn validate_room_transform_raw(
    transform: &ExtractedRoomTransform,
) -> Result<(), PlannerContractError> {
    let raw = decode_hex_exact(&transform.raw_hex, 12, "orig_world.mult.raw_hex")?;
    let translation_x = f32::from_bits(u32::from_be_bytes(raw[0..4].try_into().unwrap()));
    let translation_z = f32::from_bits(u32::from_be_bytes(raw[4..8].try_into().unwrap()));
    if !translation_x.is_finite()
        || !translation_z.is_finite()
        || translation_x.to_bits() != transform.translation_xz[0].to_bits()
        || translation_z.to_bits() != transform.translation_xz[1].to_bits()
        || i16::from_be_bytes(raw[8..10].try_into().unwrap()) != transform.angle_y
        || raw[10] != transform.room
        || raw[11] != transform.trailing_byte
    {
        return Err(PlannerContractError::new(
            "orig_world.room_transforms",
            "decoded fields do not match the retained raw MULT record",
        ));
    }
    Ok(())
}

pub(super) fn validate_file_list_raw(
    file_list: &ExtractedFileList,
) -> Result<(), PlannerContractError> {
    let raw = decode_hex_exact(&file_list.raw_hex, 32, "orig_world.fili.raw_hex")?;
    let parameters = u32::from_be_bytes(raw[0..4].try_into().unwrap());
    let sea_level = f32::from_bits(u32::from_be_bytes(raw[4..8].try_into().unwrap()));
    let unknown_08 = f32::from_bits(u32::from_be_bytes(raw[8..12].try_into().unwrap()));
    let unknown_0c = f32::from_bits(u32::from_be_bytes(raw[12..16].try_into().unwrap()));
    let message_id = u16::from_be_bytes(raw[0x1c..0x1e].try_into().unwrap());
    if !sea_level.is_finite()
        || !unknown_08.is_finite()
        || !unknown_0c.is_finite()
        || parameters != file_list.parameters
        || sea_level.to_bits() != file_list.sea_level.to_bits()
        || unknown_08.to_bits() != file_list.unknown_float_08.to_bits()
        || unknown_0c.to_bits() != file_list.unknown_float_0c.to_bits()
        || hex_bytes(&raw[0x10..0x1a]) != file_list.unknown_bytes_10_19_hex
        || ((parameters >> 3) & 7) as u8 != file_list.minimap_style
        || (parameters & 0x2000_0000 != 0) != file_list.enemy_appear_flag
        || ((parameters >> 18) & 3) as u8 != file_list.global_wind_level
        || ((parameters >> 15) & 7) as u8 != file_list.global_wind_direction
        || ((parameters >> 7) & 0xff) as u8 != file_list.grass_light
        || raw[0x1a] != file_list.default_camera
        || raw[0x1b] != file_list.bit_switch
        || message_id != file_list.message_id
    {
        return Err(PlannerContractError::new(
            "orig_world.file_lists",
            "decoded fields do not match the retained raw FILI record",
        ));
    }
    Ok(())
}

pub(super) fn validate_room_read_raw(
    room_read: &ExtractedRoomRead,
) -> Result<(), PlannerContractError> {
    let header = decode_hex_exact(
        &room_read.raw_header_hex,
        8,
        "orig_world.rtbl.raw_header_hex",
    )?;
    let room_list = decode_hex_exact(
        &room_read.raw_room_list_hex,
        room_read.load_rooms.len(),
        "orig_world.rtbl.raw_room_list_hex",
    )?;
    if room_read.record_offset == 0
        || (room_read.room_list_offset == 0 && !room_list.is_empty())
        || usize::from(header[0]) != room_read.load_rooms.len()
        || header[1] != room_read.reverb_raw
        || header[1] & 0x7f != room_read.reverb
        || header[2] != room_read.flags_raw
        || header[2] & 3 != room_read.time_pass
        || (header[2] & 8 != 0) != room_read.vrbox_enabled
        || header[3] != room_read.padding
        || u32::from_be_bytes(header[4..8].try_into().unwrap()) != room_read.room_list_offset
    {
        return Err(PlannerContractError::new(
            "orig_world.room_reads",
            "decoded fields do not match the retained raw RTBL header",
        ));
    }
    for (decoded, raw) in room_read.load_rooms.iter().zip(room_list) {
        if decoded.raw != raw
            || decoded.room != raw & 0x3f
            || decoded.load_background != (raw & 0x80 != 0)
            || decoded.unknown_bit_6 != (raw & 0x40 != 0)
        {
            return Err(PlannerContractError::new(
                "orig_world.room_reads.load_rooms",
                "decoded fields do not match the retained raw room-load byte",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_camera_raw(camera: &ExtractedCamera) -> Result<(), PlannerContractError> {
    let raw = decode_hex_exact(&camera.raw_hex, 0x18, "orig_world.rcam.raw_hex")?;
    let type_end = raw[..0x10]
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(0x10);
    let raw_type_index = u16::from_be_bytes(raw[0x16..0x18].try_into().unwrap());
    if type_end == 0
        || !raw[..type_end].iter().all(u8::is_ascii_graphic)
        || &raw[..type_end] != camera.camera_type.as_bytes()
        || raw[0x10] != camera.arrow_index
        || raw[0x11] != camera.field_of_view_y
        || raw[0x12] != camera.argument_0
        || raw[0x13] != camera.argument_1
        || u16::from_be_bytes(raw[0x14..0x16].try_into().unwrap()) != camera.argument_2
        || (raw_type_index != u16::MAX).then_some(raw_type_index) != camera.camera_type_index
    {
        return Err(PlannerContractError::new(
            "orig_world.cameras",
            "decoded fields do not match the retained raw RCAM record",
        ));
    }
    Ok(())
}

pub(super) fn validate_camera_arrow_raw(
    arrow: &ExtractedCameraArrow,
) -> Result<(), PlannerContractError> {
    let raw = decode_hex_exact(&arrow.raw_hex, 0x14, "orig_world.raro.raw_hex")?;
    let position = [
        f32::from_bits(u32::from_be_bytes(raw[0..4].try_into().unwrap())),
        f32::from_bits(u32::from_be_bytes(raw[4..8].try_into().unwrap())),
        f32::from_bits(u32::from_be_bytes(raw[8..12].try_into().unwrap())),
    ];
    let angle = [
        i16::from_be_bytes(raw[0x0c..0x0e].try_into().unwrap()),
        i16::from_be_bytes(raw[0x0e..0x10].try_into().unwrap()),
        i16::from_be_bytes(raw[0x10..0x12].try_into().unwrap()),
    ];
    if !position.iter().all(|coordinate| coordinate.is_finite())
        || position
            .iter()
            .zip(arrow.position)
            .any(|(raw, decoded)| raw.to_bits() != decoded.to_bits())
        || angle != arrow.angle
        || i16::from_be_bytes(raw[0x12..0x14].try_into().unwrap()) != arrow.trailing_i16
    {
        return Err(PlannerContractError::new(
            "orig_world.camera_arrows",
            "decoded fields do not match the retained raw RARO record",
        ));
    }
    Ok(())
}

pub(super) fn validate_path_raw(path: &ExtractedPath) -> Result<(), PlannerContractError> {
    let raw = decode_hex_exact(&path.raw_hex, 0x0c, "orig_world.rpat.raw_hex")?;
    let next_raw = u16::from_be_bytes(raw[2..4].try_into().unwrap());
    let point_offset = u32::from_be_bytes(raw[8..12].try_into().unwrap());
    if u16::from_be_bytes(raw[0..2].try_into().unwrap()) != path.point_count
        || (next_raw != u16::MAX).then_some(next_raw) != path.next_path_index
        || raw[4] != path.path_argument
        || (raw[5] & 1 != 0) != path.closed
        || raw[5] != path.closed_raw
        || (raw[6] != u8::MAX).then_some(raw[6]) != path.switch_no
        || raw[7] != path.unknown_07
        || point_offset != path.point_offset
        || point_offset % 0x10 != 0
        || point_offset / 0x10 != path.first_point_index
    {
        return Err(PlannerContractError::new(
            "orig_world.paths",
            "decoded fields do not match the retained raw RPAT record",
        ));
    }
    Ok(())
}

pub(super) fn validate_path_point_raw(
    point: &ExtractedPathPoint,
) -> Result<(), PlannerContractError> {
    let raw = decode_hex_exact(&point.raw_hex, 0x10, "orig_world.rppn.raw_hex")?;
    let position = [
        f32::from_bits(u32::from_be_bytes(raw[4..8].try_into().unwrap())),
        f32::from_bits(u32::from_be_bytes(raw[8..12].try_into().unwrap())),
        f32::from_bits(u32::from_be_bytes(raw[12..16].try_into().unwrap())),
    ];
    if [raw[3], raw[0], raw[1], raw[2]] != point.arguments
        || !position.iter().all(|coordinate| coordinate.is_finite())
        || position
            .iter()
            .zip(point.position)
            .any(|(raw, decoded)| raw.to_bits() != decoded.to_bits())
    {
        return Err(PlannerContractError::new(
            "orig_world.path_points",
            "decoded fields do not match the retained raw RPPN record",
        ));
    }
    Ok(())
}

pub(super) fn source_record_id(
    scope: SourceScope,
    digest: Digest,
    tag: &str,
    record_index: usize,
) -> String {
    let prefix = match scope.kind {
        SourceKind::Stage => "dzs",
        SourceKind::Room => "dzr",
    };
    format!("{prefix}-sha256:{digest}/chunk/{tag}/record/{record_index}")
}

pub(super) fn layer_for_tag(tag: &str) -> Option<u8> {
    if matches!(
        tag,
        "ACTR" | "TGOB" | "SCOB" | "TGSC" | "TGDR" | "Door" | "TRES" | "PLYR"
    ) || tag.len() != 4
        || !matches!(&tag[..3], "ACT" | "TRE" | "SCO" | "Doo")
    {
        return None;
    }
    match tag.as_bytes()[3] {
        b'0'..=b'9' => Some(tag.as_bytes()[3] - b'0'),
        b'a'..=b'e' => Some(tag.as_bytes()[3] - b'a' + 10),
        b'A'..=b'E' => Some(tag.as_bytes()[3] - b'A' + 10),
        _ => None,
    }
}

pub(super) fn placement_kind_for_tag(tag: &str) -> Option<PlacementKind> {
    if tag == "PLYR" {
        return Some(PlacementKind::PlayerSpawn);
    }
    if tag == "TRES" || layered_tag(tag, "TRE") {
        return Some(PlacementKind::Treasure);
    }
    if matches!(tag, "ACTR" | "TGOB") || layered_tag(tag, "ACT") {
        return Some(PlacementKind::Actor);
    }
    if matches!(tag, "SCOB" | "TGSC" | "TGDR" | "Door")
        || layered_tag(tag, "SCO")
        || layered_tag(tag, "Doo")
    {
        return Some(PlacementKind::ScaledActor);
    }
    None
}

pub(super) fn recognized_record_size(tag: &str) -> Option<usize> {
    if let Some(kind) = placement_kind_for_tag(tag) {
        return Some(if kind == PlacementKind::ScaledActor {
            36
        } else {
            32
        });
    }
    match tag {
        "STAG" => Some(60),
        "SCLS" => Some(13),
        "REVT" => Some(28),
        "LBNK" => Some(3),
        "MULT" => Some(12),
        "FILI" => Some(32),
        "RCAM" => Some(24),
        "RARO" => Some(20),
        "RPAT" => Some(12),
        "RPPN" => Some(16),
        _ => None,
    }
}

pub(super) fn layered_tag(tag: &str, prefix: &str) -> bool {
    tag.len() == 4 && tag.starts_with(prefix) && layer_for_tag(tag).is_some()
}

pub(super) fn fixed_name(
    bytes: &[u8],
    field: &'static str,
) -> Result<String, PlannerContractError> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    if end == 0 || !bytes[..end].iter().all(u8::is_ascii_graphic) {
        return Err(PlannerContractError::new(
            field,
            "must contain printable ASCII before its first NUL",
        ));
    }
    std::str::from_utf8(&bytes[..end])
        .map(str::to_owned)
        .map_err(|_| PlannerContractError::new(field, "must be UTF-8"))
}

pub(super) fn decode_hex_exact(
    value: &str,
    bytes: usize,
    field: &'static str,
) -> Result<Vec<u8>, PlannerContractError> {
    if value.len() != bytes * 2 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PlannerContractError::new(
            field,
            format!("must contain exactly {bytes} lowercase hex bytes"),
        ));
    }
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(PlannerContractError::new(field, "must use lowercase hex"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("hex is ASCII");
            u8::from_str_radix(pair, 16)
                .map_err(|_| PlannerContractError::new(field, "contains invalid hex"))
        })
        .collect()
}

pub(super) fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn validate_stage_name(stage: &str) -> Result<(), PlannerContractError> {
    if stage.is_empty()
        || stage.len() > 8
        || !stage
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(PlannerContractError::new(
            "orig_world.stage",
            "must contain 1-8 ASCII letters, digits, or underscores",
        ));
    }
    Ok(())
}
