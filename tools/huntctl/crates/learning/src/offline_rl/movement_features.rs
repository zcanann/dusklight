//! Project trace records into versioned offline movement-state features.

use super::*;

pub(super) fn movement_features_for_view(
    record: &TraceRecord,
    first_state_frame: u64,
    end_tape_frame: u64,
    view: MovementView<'_>,
) -> Result<Vec<f32>, OfflineRlError> {
    match view {
        MovementView::V1 => movement_features(record, first_state_frame, end_tape_frame),
        MovementView::V2(spec) => {
            movement_features_v2(record, first_state_frame, end_tape_frame, spec)
        }
    }
}

pub(super) fn movement_features_v2(
    record: &TraceRecord,
    first_state_frame: u64,
    end_tape_frame: u64,
    spec: &ObservationSpec,
) -> Result<Vec<f32>, OfflineRlError> {
    const COLLISION_GROUND_CONTACT: u32 = 1 << 1;
    const COLLISION_GROUND_PLANE_VALID: u32 = 1 << 4;
    const COLLISION_TRAJECTORY_VALID: u32 = 1 << 15;

    let frame = record
        .tape_frame
        .ok_or(OfflineRlError::MissingTraceFrame(first_state_frame))?;
    let mut features = Vec::with_capacity(spec.feature_count() as usize);
    let mut stage = [0_u8; 8];
    for (destination, source) in stage.iter_mut().zip(record.stage_name.as_bytes()) {
        *destination = *source;
    }
    features.extend(stage.map(|byte| f32::from(byte) / 255.0));
    let target = &spec.objective.target;
    features.extend([
        f32::from(record.room),
        f32::from(record.layer),
        f32::from(record.point),
        bool_feature(record.stage_name == target.stage),
        bool_feature(location_matches(
            &record.stage_name,
            record.room,
            record.layer,
            record.point,
            target,
        )),
        bool_feature(record.next_stage_enabled),
        bool_feature(
            record.next_stage_enabled
                && location_matches(
                    &record.next_stage_name,
                    record.next_room,
                    record.next_layer,
                    record.next_point,
                    target,
                ),
        ),
        bool_feature(record.player_present()),
        bool_feature(record.player_is_link()),
        bool_feature(record.player_proc_id.is_some()),
        record.player_proc_id.map_or(0.0, f32::from),
    ]);
    features.extend(record.position.map(|value| value / 8192.0));
    features.extend(record.velocity.map(|value| value / 64.0));
    features.push(record.forward_speed / 64.0);
    let current = yaw_radians(record.current_angle_y);
    let shape = yaw_radians(record.shape_angle_y);
    let delta = yaw_radians(record.current_angle_y.wrapping_sub(record.shape_angle_y));
    features.extend([
        current.sin(),
        current.cos(),
        shape.sin(),
        shape.cos(),
        delta.sin(),
        delta.cos(),
        f32::from(record.buttons as u8) / 255.0,
        f32::from((record.buttons >> 8) as u8) / 255.0,
        f32::from(record.stick_x) / 127.0,
        f32::from(record.stick_y) / 127.0,
        f32::from(record.pad_error),
        bool_feature(record.event_running()),
        bool_feature(record.event_name_hash_present),
        if record.event_name_hash_present {
            f32::from(record.event_name_hash as u16) / 65535.0
        } else {
            0.0
        },
        if record.event_name_hash_present {
            f32::from((record.event_name_hash >> 16) as u16) / 65535.0
        } else {
            0.0
        },
        f32::from(record.event_id),
        f32::from(record.event_mode) / 255.0,
        f32::from(record.event_status) / 255.0,
        f32::from(record.event_map_tool_id) / 255.0,
    ]);

    if let Some(exit) = &record.scene_exit {
        let destination_matches = exit.destination.as_ref().is_some_and(|destination| {
            location_matches(
                &destination.stage_name,
                destination.room,
                destination.layer,
                destination.point,
                target,
            )
        });
        features.extend([
            1.0,
            bool_feature(destination_matches),
            exit.signed_distance_to_volume / 8192.0,
            exit.player_local_position[0] / 8192.0,
            exit.player_local_position[1] / 8192.0,
            exit.player_local_position[2] / 8192.0,
            exit.volume_extent[0] / 8192.0,
            exit.volume_extent[1] / 8192.0,
            exit.volume_extent[2] / 8192.0,
        ]);
    } else {
        features.extend([0.0; 9]);
    }

    let collision = record
        .player_background_collision
        .as_ref()
        .expect("v2 validation requires background collision");
    let ground_contact = collision.flags & COLLISION_GROUND_CONTACT != 0;
    let ground_plane_valid = collision.flags & COLLISION_GROUND_PLANE_VALID != 0;
    let trajectory_valid = collision.flags & COLLISION_TRAJECTORY_VALID != 0;
    features.push(bool_feature(ground_contact));
    features.push(if ground_contact {
        collision.ground_height / 8192.0
    } else {
        0.0
    });
    features.push(bool_feature(ground_plane_valid));
    if ground_plane_valid {
        features.extend([
            collision.ground_plane[0],
            collision.ground_plane[1],
            collision.ground_plane[2],
            collision.ground_plane[3] / 8192.0,
        ]);
    } else {
        features.extend([0.0; 4]);
    }
    features.push(bool_feature(trajectory_valid));
    if trajectory_valid {
        features.extend(
            collision
                .resolved_frame_displacement
                .map(|value| value / 64.0),
        );
    } else {
        features.extend([0.0; 3]);
    }

    let surfaces = record
        .player_collision_surfaces
        .as_ref()
        .expect("v2 validation requires cached collision surfaces");
    let ground = &surfaces.surfaces[0];
    let identity = ground.bg_index.is_some() && ground.poly_index.is_some();
    let backing = ground.backing_format.is_some();
    let destination = ground.destination.as_ref();
    let destination_matches = destination.is_some_and(|destination| {
        location_matches(
            &destination.stage_name,
            destination.room,
            destination.layer,
            destination.point,
            target,
        )
    });
    let kcl_height = ground.kcl_prism_height;
    let link_exit_present = surfaces.raw_link_exit != 0x003f;
    features.extend([
        bool_feature(identity),
        bool_feature(backing),
        bool_feature(destination.is_some()),
        bool_feature(destination_matches),
        ground.bg_index.map_or(0.0, f32::from),
        ground.poly_index.map_or(0.0, f32::from),
        ground.material_row.map_or(0.0, f32::from),
        ground.raw_exit_id.map_or(0.0, f32::from),
        bool_feature(kcl_height.is_some()),
        kcl_height.map_or(0.0, |height| height / 8192.0),
        bool_feature(link_exit_present),
        if link_exit_present {
            f32::from(surfaces.raw_link_exit)
        } else {
            0.0
        },
        bool_feature(surfaces.pending_match_mask != 0),
    ]);

    let rng = record.rng.as_ref().expect("v2 validation requires RNG");
    features.push(rng.primary.call_count as f32 / 1_048_576.0);
    features.extend(
        rng.primary
            .state
            .map(|value| value as f32 / 2_147_483_648.0),
    );
    features.push(rng.secondary.call_count as f32 / 1_048_576.0);
    features.extend(
        rng.secondary
            .state
            .map(|value| value as f32 / 2_147_483_648.0),
    );
    let camera = record
        .camera
        .as_ref()
        .expect("v2 validation requires camera");
    let camera_yaw = yaw_radians(camera.view_yaw);
    features.extend([
        camera_yaw.sin(),
        camera_yaw.cos(),
        camera.eye[0] / 8192.0,
        camera.eye[1] / 8192.0,
        camera.eye[2] / 8192.0,
    ]);
    let action = record
        .player_action
        .as_ref()
        .expect("v2 validation requires player action");
    let progress = record
        .goal_progress
        .as_ref()
        .expect("v2 validation requires goal progress");
    let progress_fraction = if progress.configured && progress.requested_count > 0 {
        f32::from(progress.hit_count) / f32::from(progress.requested_count)
    } else {
        0.0
    };
    features.extend([
        f32::from(action.procedure_id),
        action.mode_flags as f32 / u32::MAX as f32,
        f32::from(action.damage_wait_timer),
        f32::from(action.sword_at_up_time),
        f32::from(action.ice_damage_wait_timer),
        bool_feature(progress.configured),
        bool_feature(progress.reached),
        progress_fraction,
        (frame - first_state_frame) as f32 / 1024.0,
        end_tape_frame.saturating_sub(frame) as f32 / 1024.0,
    ]);

    debug_assert_eq!(features.len(), spec.feature_count() as usize);
    if let Some((index, _)) = features
        .iter()
        .enumerate()
        .find(|(_, value)| !value.is_finite())
    {
        return Err(OfflineRlError::NonFiniteFeature { frame, index });
    }
    Ok(features)
}

pub(super) fn location_matches(
    stage: &str,
    room: i8,
    layer: i8,
    point: i16,
    selector: &LocationSelector,
) -> bool {
    stage == selector.stage
        && room == selector.room
        && layer == selector.layer
        && point == selector.point
}

pub(super) fn movement_features(
    record: &TraceRecord,
    first_state_frame: u64,
    end_tape_frame: u64,
) -> Result<Vec<f32>, OfflineRlError> {
    let frame = record
        .tape_frame
        .ok_or(OfflineRlError::MissingTraceFrame(first_state_frame))?;
    let mut features = Vec::with_capacity(MOVEMENT_FEATURE_COUNT_V1 as usize);
    let mut stage = [0_u8; 8];
    for (destination, source) in stage.iter_mut().zip(record.stage_name.as_bytes()) {
        *destination = *source;
    }
    features.extend(stage.map(|byte| f32::from(byte) / 255.0));
    features.extend([
        f32::from(record.room),
        f32::from(record.layer),
        f32::from(record.point),
        bool_feature(record.player_present()),
        bool_feature(record.player_is_link()),
        bool_feature(record.event_running()),
        bool_feature(record.flags & (1 << 3) != 0),
        f32::from(record.player_actor_name),
        record.player_proc_id.map_or(-1.0, f32::from),
    ]);
    features.extend(record.position.map(|value| value / 8192.0));
    features.extend(record.velocity.map(|value| value / 64.0));
    features.push(record.forward_speed / 64.0);
    let current = yaw_radians(record.current_angle_y);
    let shape = yaw_radians(record.shape_angle_y);
    let delta = yaw_radians(record.current_angle_y.wrapping_sub(record.shape_angle_y));
    features.extend([
        current.sin(),
        current.cos(),
        shape.sin(),
        shape.cos(),
        delta.sin(),
        delta.cos(),
        f32::from(record.buttons as u8) / 255.0,
        f32::from((record.buttons >> 8) as u8) / 255.0,
        f32::from(record.stick_x) / 127.0,
        f32::from(record.stick_y) / 127.0,
        f32::from(record.pad_error),
        f32::from(record.event_id),
        f32::from(record.event_mode) / 255.0,
        f32::from(record.event_status) / 255.0,
        f32::from(record.event_map_tool_id) / 255.0,
        f32::from(record.event_name_hash as u16) / 65535.0,
        f32::from((record.event_name_hash >> 16) as u16) / 65535.0,
    ]);
    if let (Some(actor), Some(distance)) = (
        record.nearest_scene_exit_actor_name,
        record.nearest_scene_exit_distance,
    ) {
        features.push(1.0);
        features.push(f32::from(actor));
        features.extend(
            record
                .nearest_scene_exit_position
                .iter()
                .zip(record.position)
                .map(|(exit, player)| (exit - player) / 8192.0),
        );
        features.push(distance / 8192.0);
    } else {
        features.extend([0.0, -1.0, 0.0, 0.0, 0.0, -1.0 / 8192.0]);
    }
    features.push((frame - first_state_frame) as f32 / 1024.0);
    features.push(end_tape_frame.saturating_sub(frame) as f32 / 1024.0);

    debug_assert_eq!(features.len(), MOVEMENT_FEATURE_COUNT_V1 as usize);
    if let Some((index, _)) = features
        .iter()
        .enumerate()
        .find(|(_, value)| !value.is_finite())
    {
        return Err(OfflineRlError::NonFiniteFeature { frame, index });
    }
    Ok(features)
}

pub(super) fn yaw_radians(value: i16) -> f32 {
    f32::from(value) * PI / 32768.0
}

pub(super) fn bool_feature(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}
