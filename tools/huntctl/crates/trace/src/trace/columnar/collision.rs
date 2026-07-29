use super::*;

pub(super) fn decode_player_background_collision_v1(
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

pub(super) fn decode_player_background_collision_v2(
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

pub(super) fn decode_player_collision_surfaces_v1(
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

pub(super) fn decode_collision_surface(
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

pub(super) fn validate_collision_surface_joins(record: &TraceRecord) -> Result<(), TraceError> {
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

pub(super) fn validate_identity_pair(
    bg: u16,
    poly: u16,
    present: bool,
    kind: &str,
) -> Result<(), TraceError> {
    if (bg != INVALID_U16_ID) != present || (poly != INVALID_U16_ID) != present {
        return Err(TraceError(format!(
            "invalid gameplay trace collision {kind} identity sentinel"
        )));
    }
    Ok(())
}

pub(super) fn validate_owner(owner: u32, present: bool, kind: &str) -> Result<(), TraceError> {
    if (owner != INVALID_U32_ID) != present {
        return Err(TraceError(format!(
            "invalid gameplay trace collision {kind} owner sentinel"
        )));
    }
    Ok(())
}

pub(super) fn validate_plane(plane: [f32; 4], present: bool, kind: &str) -> Result<(), TraceError> {
    if plane.iter().any(|value| !value.is_finite())
        || (!present && plane.iter().any(|value| *value != 0.0))
    {
        return Err(TraceError(format!(
            "invalid gameplay trace collision {kind} plane"
        )));
    }
    Ok(())
}
