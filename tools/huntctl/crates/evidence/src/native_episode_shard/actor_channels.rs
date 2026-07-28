//! Decode camera, action, scene-exit, and collision observations.

use super::*;

pub(super) fn decode_camera(
    reader: &mut Reader<'_>,
) -> Result<NativeCameraObservation, NativeEpisodeShardError> {
    let view_yaw = reader.i16()?;
    let controlled_yaw = reader.i16()?;
    let bank = reader.i16()?;
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero camera reserved field",
        ));
    }
    Ok(NativeCameraObservation {
        view_yaw,
        controlled_yaw,
        bank,
        eye: reader.f32x3()?,
        center: reader.f32x3()?,
        up: reader.f32x3()?,
        fovy: reader.f32()?,
    })
}

pub(super) fn decode_animation_lane(
    reader: &mut Reader<'_>,
) -> Result<NativeAnimationLane, NativeEpisodeShardError> {
    let resource_id = reader.u16()?;
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero animation-lane reserved field",
        ));
    }
    Ok(NativeAnimationLane {
        resource_id,
        frame: reader.f32()?,
        rate: reader.f32()?,
    })
}

pub(super) fn decode_trace_actor_identity(
    reader: &mut Reader<'_>,
) -> Result<NativeTraceActorIdentity, NativeEpisodeShardError> {
    let runtime_generation = reader.u32()?;
    let actor_name = reader.i16()?;
    let set_id = reader.u16()?;
    let home_room = reader.i8()?;
    let current_room = reader.i8()?;
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero trace actor-identity reserved field",
        ));
    }
    Ok(NativeTraceActorIdentity {
        runtime_generation,
        actor_name,
        set_id,
        home_room,
        current_room,
        home_position: reader.f32x3()?,
    })
}

pub(super) fn trace_actor_identity_is_absent(actor: &NativeTraceActorIdentity) -> bool {
    actor.runtime_generation == u32::MAX
        && actor.actor_name == -1
        && actor.set_id == u16::MAX
        && actor.home_room == -1
        && actor.current_room == -1
        && actor.home_position == [0.0; 3]
}

pub(super) fn decode_player_action(
    reader: &mut Reader<'_>,
) -> Result<NativePlayerActionObservation, NativeEpisodeShardError> {
    let procedure_id = reader.u16()?;
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero player-action reserved field",
        ));
    }
    let mode_flags = reader.u32()?;
    let procedure_context_raw = [
        reader.i16()?,
        reader.i16()?,
        reader.i16()?,
        reader.i16()?,
        reader.i16()?,
        reader.i16()?,
    ];
    let damage_wait_timer = reader.i16()?;
    let sword_at_up_time = reader.u16()?;
    let ice_damage_wait_timer = reader.i16()?;
    let sword_change_wait_timer = reader.u8()?;
    if reader.bytes(5)?.iter().any(|byte| *byte != 0) {
        return Err(NativeEpisodeShardError::new(
            "nonzero player-action padding",
        ));
    }
    let mut under_animations = Vec::with_capacity(3);
    let mut upper_animations = Vec::with_capacity(3);
    for _ in 0..3 {
        under_animations.push(decode_animation_lane(reader)?);
    }
    for _ in 0..3 {
        upper_animations.push(decode_animation_lane(reader)?);
    }
    let flags = reader.u32()?;
    let do_status = reader.u8()?;
    if flags & !0x3 != 0 || reader.u8()? != 0 || reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "invalid player-action flags or reserved fields",
        ));
    }
    let talk_partner = decode_trace_actor_identity(reader)?;
    let grabbed_actor = decode_trace_actor_identity(reader)?;
    if (flags & 1 != 0) == trace_actor_identity_is_absent(&talk_partner)
        || (flags & 2 != 0) == trace_actor_identity_is_absent(&grabbed_actor)
    {
        return Err(NativeEpisodeShardError::new(
            "noncanonical player-action actor identity",
        ));
    }
    Ok(NativePlayerActionObservation {
        procedure_id,
        mode_flags,
        procedure_context_raw,
        damage_wait_timer,
        sword_at_up_time,
        ice_damage_wait_timer,
        sword_change_wait_timer,
        under_animations: under_animations.try_into().expect("three animation lanes"),
        upper_animations: upper_animations.try_into().expect("three animation lanes"),
        flags,
        do_status,
        talk_partner,
        grabbed_actor,
    })
}

pub(super) fn decode_scene_exit(
    reader: &mut Reader<'_>,
    _status: NativeChannelStatus,
) -> Result<NativeSceneExitObservation, NativeEpisodeShardError> {
    let exit = NativeSceneExitObservation {
        runtime_generation: reader.u32()?,
        raw_parameters: reader.u32()?,
        flags: reader.u32()?,
        signed_distance_to_volume: reader.f32()?,
        actor_name: reader.i16()?,
        set_id: reader.u16()?,
        exit_id: reader.u8()?,
        path_id: reader.u8()?,
        argument1: reader.u8()?,
        switch_no: reader.u8()?,
        kind: reader.u8()?,
        observed_count: reader.u8()?,
        home_room: reader.i8()?,
        link_exit_direction: reader.u8()?,
        link_exit_id: reader.u16()?,
        shape_yaw: reader.i16()?,
        actor_action: reader.u8()?,
        player_local_position: {
            if reader.u8()? != 0 || reader.u16()? != 0 {
                return Err(NativeEpisodeShardError::new(
                    "nonzero scene-exit reserved fields",
                ));
            }
            reader.f32x3()?
        },
        volume_extent: reader.f32x3()?,
        home_position: reader.f32x3()?,
        destination_stage: reader.fixed_name()?,
        destination_room: reader.i8()?,
        destination_layer: reader.i8()?,
        destination_point: reader.i16()?,
        destination_wipe: reader.u8()?,
        destination_wipe_time: reader.u8()?,
        destination_time_hour: reader.i8()?,
    };
    if reader.u8()? != 0 || exit.flags & !0x7f != 0 {
        return Err(NativeEpisodeShardError::new(
            "invalid scene-exit flags or reserved fields",
        ));
    }
    Ok(exit)
}

pub(super) fn decode_background_collision(
    reader: &mut Reader<'_>,
) -> Result<NativePlayerBackgroundCollision, NativeEpisodeShardError> {
    let flags = reader.u32()?;
    if flags & !0x0007_ffff != 0 {
        return Err(NativeEpisodeShardError::new(
            "unknown background collision flags",
        ));
    }
    let ground_height = reader.f32()?;
    let roof_height = reader.f32()?;
    let water_height = reader.f32()?;
    let ground_identity = [
        u32::from(reader.u16()?),
        u32::from(reader.u16()?),
        reader.u32()?,
    ];
    let ground_plane = reader.f32x4()?;
    let roof_identity = [
        u32::from(reader.u16()?),
        u32::from(reader.u16()?),
        reader.u32()?,
    ];
    let water_identity = [
        u32::from(reader.u16()?),
        u32::from(reader.u16()?),
        reader.u32()?,
    ];
    let mut walls = Vec::with_capacity(3);
    for _ in 0..3 {
        let wall = NativeCollisionWallObservation {
            bg_index: reader.u16()?,
            poly_index: reader.u16()?,
            owner_runtime_generation: reader.u32()?,
            angle_y: reader.i16()?,
            flags: reader.u16()?,
        };
        if wall.flags & !0x0007 != 0 {
            return Err(NativeEpisodeShardError::new("unknown collision wall flags"));
        }
        walls.push(wall);
    }
    let collision = NativePlayerBackgroundCollision {
        flags,
        ground_height,
        roof_height,
        water_height,
        ground_identity,
        ground_plane,
        roof_identity,
        water_identity,
        walls: walls.try_into().expect("three collision walls"),
        old_position: reader.f32x3()?,
        resolved_frame_displacement: reader.f32x3()?,
        final_position: reader.f32x3()?,
    };
    validate_background_collision(&collision)?;
    Ok(collision)
}

pub(super) fn identity_is_coherent(
    identity: [u32; 3],
    identity_present: bool,
    owner_present: bool,
) -> bool {
    let bg_present = identity[0] != u32::from(u16::MAX);
    let polygon_present = identity[1] != u32::from(u16::MAX);
    let actual_owner_present = identity[2] != u32::MAX;
    bg_present == polygon_present
        && bg_present == identity_present
        && actual_owner_present == owner_present
        && (!actual_owner_present || identity_present)
}

pub(super) fn validate_background_collision(
    collision: &NativePlayerBackgroundCollision,
) -> Result<(), NativeEpisodeShardError> {
    let has = |flag| collision.flags & flag != 0;
    let ground_valid = has(1 << 0);
    let ground_identity = has(1 << 16);
    let ground_owner = has(1 << 5);
    let roof_valid = has(1 << 7);
    let roof_identity = has(1 << 17);
    let roof_owner = has(1 << 9);
    let water_enabled = has(1 << 10);
    let water_found = has(1 << 11);
    let water_identity = has(1 << 18);
    let water_owner = has(1 << 13);
    if ground_valid
        != (collision.ground_height != -1_000_000_000.0 && collision.ground_height.is_finite())
        || (!ground_valid && (has(1 << 1) || has(1 << 2)))
        || !identity_is_coherent(collision.ground_identity, ground_identity, ground_owner)
        || (!ground_valid && collision.ground_identity[0] != u32::from(u16::MAX))
        || (has(1 << 4) && (!ground_valid || !has(1 << 1)))
        || (has(1 << 4) != (collision.ground_plane != [0.0; 4]))
        || roof_valid
            != (collision.roof_height != 1_000_000_000.0 && collision.roof_height.is_finite())
        || (has(1 << 8) && !roof_valid)
        || !identity_is_coherent(collision.roof_identity, roof_identity, roof_owner)
        || (!roof_valid && collision.roof_identity[0] != u32::from(u16::MAX))
        || water_found
            != (collision.water_height != -1_000_000_000.0 && collision.water_height.is_finite())
        || (water_found && !water_enabled)
        || (has(1 << 12) && !water_found)
        || !identity_is_coherent(collision.water_identity, water_identity, water_owner)
        || (!water_found && collision.water_identity[0] != u32::from(u16::MAX))
    {
        return Err(NativeEpisodeShardError::new(
            "inconsistent background collision payload",
        ));
    }

    let mut any_wall_hit = false;
    for wall in &collision.walls {
        let hit = wall.flags & 1 != 0;
        let owner = wall.flags & (1 << 1) != 0;
        let identity = wall.flags & (1 << 2) != 0;
        any_wall_hit |= hit;
        if !identity_is_coherent(
            [
                u32::from(wall.bg_index),
                u32::from(wall.poly_index),
                wall.owner_runtime_generation,
            ],
            identity,
            owner,
        ) || (!hit
            && (wall.bg_index != u16::MAX
                || wall.poly_index != u16::MAX
                || wall.owner_runtime_generation != u32::MAX
                || wall.angle_y != 0
                || wall.flags != 0))
        {
            return Err(NativeEpisodeShardError::new(
                "inconsistent background collision wall",
            ));
        }
    }
    if any_wall_hit != has(1 << 6) || (any_wall_hit && !has(1 << 14)) {
        return Err(NativeEpisodeShardError::new(
            "inconsistent background collision wall aggregate",
        ));
    }

    let trajectory_valid = has(1 << 15);
    if !trajectory_valid
        && (collision.old_position != [0.0; 3]
            || collision.resolved_frame_displacement != [0.0; 3]
            || collision.final_position != [0.0; 3])
    {
        return Err(NativeEpisodeShardError::new(
            "inconsistent background collision trajectory",
        ));
    }
    if trajectory_valid {
        for axis in 0..3 {
            let reconstructed =
                collision.old_position[axis] + collision.resolved_frame_displacement[axis];
            let tolerance = 0.0001 * collision.final_position[axis].abs().max(1.0);
            if (reconstructed - collision.final_position[axis]).abs() > tolerance {
                return Err(NativeEpisodeShardError::new(
                    "incoherent background collision trajectory",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn decode_collision_surfaces(
    reader: &mut Reader<'_>,
    plane_mask: u8,
    status: NativeChannelStatus,
    next_stage: Option<&str>,
    next_room: i8,
    next_layer: i8,
    next_point: i16,
) -> Result<NativePlayerCollisionSurfaces, NativeEpisodeShardError> {
    let flags = reader.u32()?;
    let current_room = reader.i8()?;
    let identity_count = reader.u8()?;
    let backing_code_count = reader.u8()?;
    let destination_count = reader.u8()?;
    let raw_link_exit = reader.u16()?;
    let pending_stage_match_mask = reader.u8()?;
    let room_valid = flags & 1 != 0;
    let explicit_exit = flags & (1 << 1) != 0;
    let pending = flags & (1 << 2) != 0;
    if flags & !0x0007 != 0
        || pending_stage_match_mask & !0x3f != 0
        || reader.u8()? != 0
        || plane_mask & !0x3f != 0
        || (room_valid && !(-1..64).contains(&current_room))
        || (!room_valid && current_room != i8::MIN)
        || explicit_exit != (raw_link_exit != 0x003f)
        || (status == NativeChannelStatus::Present && pending != next_stage.is_some())
    {
        return Err(NativeEpisodeShardError::new(
            "invalid collision surface-set header",
        ));
    }
    let expected_kinds = [(1_u8, 0_u8), (2, 0), (3, 0), (4, 0), (4, 1), (4, 2)];
    let mut surfaces = Vec::with_capacity(6);
    for (index, (expected_kind, expected_wall_slot)) in expected_kinds.into_iter().enumerate() {
        let surface_flags = reader.u32()?;
        let kind = reader.u8()?;
        let wall_slot = reader.u8()?;
        let backing_format = reader.u8()?;
        let raw_code_presence_mask = reader.u8()?;
        if surface_flags & !0x0000_1fff != 0
            || kind != expected_kind
            || wall_slot != expected_wall_slot
            || backing_format > 2
            || raw_code_presence_mask & !0x1f != 0
        {
            return Err(NativeEpisodeShardError::new(
                "invalid collision surface identity",
            ));
        }
        let bg_index = reader.u16()?;
        let poly_index = reader.u16()?;
        let owner_runtime_generation = reader.u32()?;
        let material_index = reader.u16()?;
        let group_index = reader.u16()?;
        let raw_codes = [
            reader.u32()?,
            reader.u32()?,
            reader.u32()?,
            reader.u32()?,
            reader.u32()?,
        ];
        let raw_exit_id = reader.u8()?;
        let source_room = reader.i8()?;
        let scls_source_room = reader.i8()?;
        let destination_room = reader.i8()?;
        let destination_layer = reader.i8()?;
        let destination_wipe = reader.u8()?;
        let destination_wipe_time = reader.u8()?;
        let destination_time_hour = reader.i8()?;
        let destination_point = reader.i16()?;
        let geometry_count = usize::from(reader.u8()?);
        if geometry_count > 6 || reader.u8()? != 0 {
            return Err(NativeEpisodeShardError::new(
                "invalid collision surface geometry count",
            ));
        }
        let mut geometry_indices = [0_u16; 6];
        for geometry_index in &mut geometry_indices {
            *geometry_index = reader.u16()?;
        }
        let kcl_prism_height = reader.f32()?;
        let destination_stage = reader.fixed_name()?;
        let plane_values = reader.f32x4()?;
        let plane_present = plane_mask & (1 << index) != 0;
        let identity = surface_flags & 1 != 0;
        let owner = surface_flags & (1 << 1) != 0;
        let backing = surface_flags & (1 << 2) != 0;
        let codes = surface_flags & (1 << 3) != 0;
        let material = surface_flags & (1 << 4) != 0;
        let group = surface_flags & (1 << 5) != 0;
        let source_room_present = surface_flags & (1 << 6) != 0;
        let source_room_exact = surface_flags & (1 << 7) != 0;
        let scls_source = surface_flags & (1 << 8) != 0;
        let destination = surface_flags & (1 << 9) != 0;
        let destination_match = surface_flags & (1 << 10) != 0;
        let geometry = surface_flags & (1 << 11) != 0;
        let kcl_height = surface_flags & (1 << 12) != 0;
        let identity_tuple = [
            u32::from(bg_index),
            u32::from(poly_index),
            owner_runtime_generation,
        ];
        let destination_name_valid = !destination_stage.is_empty()
            && destination_stage
                .as_bytes()
                .iter()
                .all(|byte| (0x20..=0x7e).contains(byte));
        let destination_fields_valid = destination_name_valid
            && (-1..64).contains(&destination_room)
            && (-1..15).contains(&destination_layer)
            && destination_point >= 0
            && destination_wipe_time <= 7
            && (-1..31).contains(&destination_time_hour);
        let destination_fields_absent = destination_stage.is_empty()
            && destination_room == i8::MIN
            && destination_layer == i8::MIN
            && destination_point == i16::MIN
            && destination_wipe == u8::MAX
            && destination_wipe_time == u8::MAX
            && destination_time_hour == i8::MIN;
        let tuple_matches_pending = destination
            && pending
            && next_stage == Some(destination_stage.as_str())
            && destination_room == next_room
            && destination_layer == next_layer
            && destination_point == next_point;
        if !identity_is_coherent(identity_tuple, identity, owner)
            || (owner && !identity)
            || backing != (backing_format != 0)
            || (backing && !identity)
            || codes != (raw_code_presence_mask != 0)
            || (codes && (!backing || raw_code_presence_mask & 1 == 0))
            || material != (material_index != u16::MAX)
            || (material && !backing)
            || group != (group_index != u16::MAX)
            || (group && (!backing || backing_format != 1))
            || (source_room_present && (!identity || !(-1..64).contains(&source_room)))
            || (!source_room_present && source_room != i8::MIN)
            || (source_room_exact && !source_room_present)
            || (scls_source
                && (index != 0
                    || !identity
                    || !room_valid
                    || scls_source_room != current_room
                    || !(-1..64).contains(&scls_source_room)))
            || (!scls_source && scls_source_room != i8::MIN)
            || (destination
                && (index != 0 || !scls_source || !codes || matches!(raw_exit_id, 0x3f | 0xff)))
            || (destination_match && (!destination || !pending))
            || geometry != (geometry_count != 0)
            || (geometry && !backing)
            || (kcl_height && (!backing || backing_format != 2))
            || (!kcl_height && kcl_prism_height != 0.0)
            || raw_codes
                .iter()
                .enumerate()
                .any(|(word, code)| raw_code_presence_mask & (1 << word) == 0 && *code != 0)
            || geometry_indices
                .iter()
                .enumerate()
                .any(|(slot, value)| (geometry && slot < geometry_count) == (*value == u16::MAX))
            || (destination && !destination_fields_valid)
            || (!destination && !destination_fields_absent)
            || destination_match != tuple_matches_pending
        {
            return Err(NativeEpisodeShardError::new(
                "inconsistent collision surface payload",
            ));
        }
        if (plane_present && !identity) || (!plane_present && plane_values != [0.0; 4]) {
            return Err(NativeEpisodeShardError::new(
                "collision plane does not match realized surface identity",
            ));
        }
        surfaces.push(NativeCollisionSurfaceObservation {
            flags: surface_flags,
            kind,
            wall_slot,
            backing_format,
            raw_code_presence_mask,
            bg_index,
            poly_index,
            owner_runtime_generation,
            material_index,
            group_index,
            raw_codes,
            raw_exit_id,
            source_room,
            scls_source_room,
            destination_room,
            destination_layer,
            destination_wipe,
            destination_wipe_time,
            destination_time_hour,
            destination_point,
            source_geometry_indices: geometry_indices[..geometry_count].to_vec(),
            kcl_prism_height,
            destination_stage,
            plane: plane_present.then_some(plane_values),
        });
    }
    let observed_identity_count = surfaces
        .iter()
        .filter(|surface| surface.flags & 1 != 0)
        .count();
    let observed_backing_count = surfaces
        .iter()
        .filter(|surface| surface.flags & (1 << 2) != 0)
        .count();
    let observed_destination_count = surfaces
        .iter()
        .filter(|surface| surface.flags & (1 << 9) != 0)
        .count();
    let observed_match_mask = surfaces
        .iter()
        .enumerate()
        .fold(0_u8, |mask, (index, surface)| {
            mask | (((surface.flags & (1 << 10) != 0) as u8) << index)
        });
    if usize::from(identity_count) != observed_identity_count
        || usize::from(backing_code_count) != observed_backing_count
        || usize::from(destination_count) != observed_destination_count
        || pending_stage_match_mask != observed_match_mask
    {
        return Err(NativeEpisodeShardError::new(
            "collision surface counts disagree with entries",
        ));
    }
    Ok(NativePlayerCollisionSurfaces {
        flags,
        current_room,
        identity_count,
        backing_code_count,
        destination_count,
        raw_link_exit,
        pending_stage_match_mask,
        surfaces,
    })
}

pub(super) fn collision_channels_agree(
    background: &NativePlayerBackgroundCollision,
    surfaces: &NativePlayerCollisionSurfaces,
) -> bool {
    let agrees = |surface: &NativeCollisionSurfaceObservation,
                  identity: [u32; 3],
                  identity_present: bool,
                  owner_present: bool| {
        (surface.flags & 1 != 0) == identity_present
            && (surface.flags & (1 << 1) != 0) == owner_present
            && (!identity_present
                || (u32::from(surface.bg_index) == identity[0]
                    && u32::from(surface.poly_index) == identity[1]))
            && (!owner_present || surface.owner_runtime_generation == identity[2])
    };
    agrees(
        &surfaces.surfaces[0],
        background.ground_identity,
        background.flags & (1 << 16) != 0,
        background.flags & (1 << 5) != 0,
    ) && agrees(
        &surfaces.surfaces[1],
        background.roof_identity,
        background.flags & (1 << 17) != 0,
        background.flags & (1 << 9) != 0,
    ) && agrees(
        &surfaces.surfaces[2],
        background.water_identity,
        background.flags & (1 << 18) != 0,
        background.flags & (1 << 13) != 0,
    ) && background.walls.iter().enumerate().all(|(index, wall)| {
        agrees(
            &surfaces.surfaces[index + 3],
            [
                u32::from(wall.bg_index),
                u32::from(wall.poly_index),
                wall.owner_runtime_generation,
            ],
            wall.flags & (1 << 2) != 0,
            wall.flags & (1 << 1) != 0,
        )
    })
}
