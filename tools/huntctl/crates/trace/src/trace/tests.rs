use super::*;

#[test]
fn rejects_bad_magic() {
    let mut bytes = vec![0; V1_HEADER_SIZE];
    bytes[..8].copy_from_slice(b"NOTTRACE");
    assert!(decode(&bytes).unwrap_err().to_string().contains("magic"));
}

#[test]
fn v1_keeps_explicit_post_simulation_alignment() {
    let mut bytes = vec![0; V1_HEADER_SIZE + V1_RECORD_SIZE];
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&1_u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&(V1_RECORD_SIZE as u16).to_le_bytes());
    bytes[12..16].copy_from_slice(&30_u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&1_u32.to_le_bytes());
    bytes[20..28].copy_from_slice(&1_u64.to_le_bytes());
    let record = &mut bytes[V1_HEADER_SIZE..];
    record[0..8].copy_from_slice(&41_u64.to_le_bytes());
    record[8..16].copy_from_slice(&9_u64.to_le_bytes());
    record[16..23].copy_from_slice(b"F_SP103");
    record[70..72].copy_from_slice(&u16::MAX.to_le_bytes());
    record[82..84].copy_from_slice(&(-1_i16).to_le_bytes());
    record[96..100].copy_from_slice(&(-1.0_f32).to_bits().to_le_bytes());
    let decoded = decode(&bytes).unwrap();
    assert_eq!(decoded.records[0].boundary_index, 42);
    assert_eq!(
        decoded.records[0].observation_phase,
        TracePhase::PostSimulation
    );
    assert_eq!(decoded.records[0].tape_frame, Some(9));
    assert!(decoded.channel_formats.is_empty());
}

#[test]
fn v2_decodes_scene_exit_v1_and_retains_wire_format() {
    let mut payload = vec![0; 24];
    payload[0..4].copy_from_slice(&7_u32.to_le_bytes());
    payload[4..6].copy_from_slice(&(-13_i16).to_le_bytes());
    write_f32(&mut payload, 8, 10.0);
    write_f32(&mut payload, 12, 20.0);
    write_f32(&mut payload, 16, 30.0);
    write_f32(&mut payload, 20, 40.0);
    let decoded = build_v2_trace(vec![(
        TraceChannel::SceneExit,
        1,
        TraceChannelStatus::Present,
        payload,
    )]);
    let decoded = decode(&decoded).unwrap();
    assert_eq!(
        decoded.channel_formats[&TraceChannel::SceneExit],
        TraceChannelWireFormat {
            version: 1,
            stride: 24
        }
    );
    let record = &decoded.records[0];
    assert_eq!(record.nearest_scene_exit_session_process_id, Some(7));
    assert_eq!(record.nearest_scene_exit_actor_name, Some(-13));
    assert_eq!(record.nearest_scene_exit_position, [10.0, 20.0, 30.0]);
    assert_eq!(record.nearest_scene_exit_distance, Some(40.0));
    assert!(record.scene_exit.is_none());
}

#[test]
fn v2_decodes_scene_exit_v2_destination_and_geometry() {
    let payload = scene_exit_v2_payload();
    let decoded = decode(&build_v2_trace(vec![(
        TraceChannel::SceneExit,
        2,
        TraceChannelStatus::Present,
        payload,
    )]))
    .unwrap();
    assert_eq!(
        decoded.channel_formats[&TraceChannel::SceneExit],
        TraceChannelWireFormat {
            version: 2,
            stride: 88
        }
    );
    let exit = decoded.records[0].scene_exit.as_ref().unwrap();
    assert_eq!(exit.session_process_id, 0x1234);
    assert_eq!(exit.kind, TraceSceneExitKind::OrientedBox);
    assert_eq!(exit.observed_count, 2);
    assert!(!exit.observed_count_saturated);
    assert_eq!(exit.signed_distance_to_volume, -0.25);
    assert_eq!(exit.home_position, [100.0, 200.0, 300.0]);
    let destination = exit.destination.as_ref().unwrap();
    assert_eq!(destination.stage_name, "F_SP103");
    assert_eq!(destination.room, 1);
    assert_eq!(destination.layer, -1);
    assert_eq!(destination.point, 4);
    assert_eq!(destination.wipe, 17);
    assert_eq!(destination.wipe_time, 3);
    assert_eq!(destination.time_hour, -1);
    assert_eq!(decoded.records[0].nearest_scene_exit_distance, None);
}

#[test]
fn v2_scene_exit_latch_preserves_raw_ff_direction() {
    let mut payload = scene_exit_v2_payload();
    let flags = u32_at(&payload, 8) | SCENE_EXIT_PLAYER_LATCHED;
    payload[8..12].copy_from_slice(&flags.to_le_bytes());
    payload[27] = u8::MAX;
    payload[28..30].copy_from_slice(&7_u16.to_le_bytes());
    payload[8..12].copy_from_slice(&(flags & !SCENE_EXIT_DESTINATION_VALID).to_le_bytes());
    payload[72..80].fill(0);
    payload[80] = u8::MAX;
    payload[81] = u8::MAX;
    payload[82..84].copy_from_slice(&(-1_i16).to_le_bytes());
    payload[84..87].fill(u8::MAX);

    let decoded = decode(&build_v2_trace(vec![(
        TraceChannel::SceneExit,
        2,
        TraceChannelStatus::Present,
        payload,
    )]))
    .unwrap();
    let exit = decoded.records[0].scene_exit.as_ref().unwrap();
    assert_eq!(exit.link_exit_id, Some(7));
    assert_eq!(exit.link_exit_direction, Some(u8::MAX));
    assert!(exit.destination.is_none());
}

#[test]
fn v2_scene_exit_preserves_saturated_observed_count() {
    let mut payload = scene_exit_v2_payload();
    let flags = u32_at(&payload, 8) | SCENE_EXIT_OBSERVED_COUNT_SATURATED;
    payload[8..12].copy_from_slice(&flags.to_le_bytes());
    payload[25] = u8::MAX;
    let decoded = decode(&build_v2_trace(vec![(
        TraceChannel::SceneExit,
        2,
        TraceChannelStatus::Present,
        payload,
    )]))
    .unwrap();
    let exit = decoded.records[0].scene_exit.as_ref().unwrap();
    assert_eq!(exit.observed_count, u8::MAX);
    assert!(exit.observed_count_saturated);
}

#[test]
fn v2_decodes_player_background_collision_v1() {
    let decoded = decode(&build_v2_trace(vec![(
        TraceChannel::PlayerBackgroundCollision,
        1,
        TraceChannelStatus::Present,
        background_collision_v1_payload(),
    )]))
    .unwrap();
    assert_eq!(
        decoded.channel_formats[&TraceChannel::PlayerBackgroundCollision],
        TraceChannelWireFormat {
            version: 1,
            stride: 128
        }
    );
    let collision = decoded.records[0]
        .player_background_collision
        .as_ref()
        .unwrap();
    assert_eq!(collision.flags, COLLISION_TRAJECTORY_VALID);
    assert_eq!(collision.ground_height, -1.0e9);
    assert_eq!(collision.roof_height, 1.0e9);
    assert_eq!(collision.old_position, [1.0, 2.0, 3.0]);
    assert_eq!(collision.resolved_frame_displacement, [4.0, 5.0, 6.0]);
    assert_eq!(collision.final_position, [5.0, 7.0, 9.0]);
    assert!(collision.walls.iter().all(|wall| wall.bg_index.is_none()));
}

#[test]
fn v2_decodes_collision_surfaces_and_pending_ground_destination() {
    let decoded = decode(&build_v2_trace(vec![
        (
            TraceChannel::Stage,
            1,
            TraceChannelStatus::Present,
            stage_payload(true),
        ),
        (
            TraceChannel::PlayerCollisionSurfaces,
            1,
            TraceChannelStatus::Present,
            collision_surfaces_pending_ground_payload(),
        ),
    ]))
    .unwrap();
    assert_eq!(
        decoded.channel_formats[&TraceChannel::PlayerCollisionSurfaces],
        TraceChannelWireFormat {
            version: 1,
            stride: 496
        }
    );
    let set = decoded.records[0]
        .player_collision_surfaces
        .as_ref()
        .unwrap();
    assert_eq!(set.link_room, Some(1));
    assert_eq!(set.identity_count, 1);
    assert_eq!(set.backing_count, 1);
    assert_eq!(set.destination_count, 1);
    assert_eq!(set.pending_match_mask, 1);
    let ground = &set.surfaces[0];
    assert_eq!(ground.kind, TraceCollisionSurfaceKind::Ground);
    assert_eq!(
        ground.backing_format,
        Some(TraceCollisionBackingFormat::Kcl)
    );
    assert_eq!(ground.bg_index, Some(7));
    assert_eq!(ground.poly_index, Some(2217));
    assert_eq!(ground.material_row, Some(19));
    assert_eq!(ground.raw_exit_id, Some(1));
    assert_eq!(ground.source_geometry_indices, vec![2, 3, 5, 7, 11]);
    assert_eq!(ground.kcl_prism_height, Some(42.5));
    assert_eq!(ground.destination.as_ref().unwrap().stage_name, "F_SP104");
    assert!(
        set.surfaces[1..]
            .iter()
            .all(|surface| surface.bg_index.is_none())
    );
}

#[test]
fn v2_cross_checks_collision_surface_cache_identity() {
    let channels = vec![
        (
            TraceChannel::Stage,
            1,
            TraceChannelStatus::Present,
            stage_payload(true),
        ),
        (
            TraceChannel::PlayerBackgroundCollision,
            1,
            TraceChannelStatus::Present,
            background_collision_with_ground(7, 2217),
        ),
        (
            TraceChannel::PlayerCollisionSurfaces,
            1,
            TraceChannelStatus::Present,
            collision_surfaces_pending_ground_payload(),
        ),
    ];
    decode(&build_v2_trace(channels.clone())).unwrap();

    let mut mismatched = channels;
    mismatched[1].3[18..20].copy_from_slice(&841_u16.to_le_bytes());
    let error = decode(&build_v2_trace(mismatched)).unwrap_err();
    assert!(error.to_string().contains("identity or owner disagrees"));
}

#[test]
fn v2_rejects_collision_surface_wire_corruption() {
    let build = |surface_payload| {
        build_v2_trace(vec![
            (
                TraceChannel::Stage,
                1,
                TraceChannelStatus::Present,
                stage_payload(true),
            ),
            (
                TraceChannel::PlayerCollisionSurfaces,
                1,
                TraceChannelStatus::Present,
                surface_payload,
            ),
        ])
    };

    let mut payload = collision_surfaces_pending_ground_payload();
    payload[16 + 40] = 2;
    assert!(
        decode(&build(payload))
            .unwrap_err()
            .to_string()
            .contains("raw exit disagrees")
    );

    let mut payload = collision_surfaces_pending_ground_payload();
    payload[10] = 0;
    assert!(
        decode(&build(payload))
            .unwrap_err()
            .to_string()
            .contains("pending-match mask")
    );

    let mut payload = collision_surfaces_pending_ground_payload();
    payload[16 + 76] = 1;
    assert!(
        decode(&build(payload))
            .unwrap_err()
            .to_string()
            .contains("reserved")
    );

    let mut payload = collision_surfaces_pending_ground_payload();
    write_f32(&mut payload, 16 + 64, f32::NAN);
    assert!(
        decode(&build(payload))
            .unwrap_err()
            .to_string()
            .contains("prism height")
    );

    let error = decode(&build_v2_trace(vec![(
        TraceChannel::PlayerCollisionSurfaces,
        1,
        TraceChannelStatus::Present,
        empty_collision_surfaces_payload(),
    )]))
    .unwrap_err();
    assert!(error.to_string().contains("require the Stage channel"));
}

#[test]
fn v2_rejects_known_channel_version_stride_and_status_mismatches() {
    let wrong_version = build_v2_trace(vec![(
        TraceChannel::SceneExit,
        3,
        TraceChannelStatus::Present,
        vec![0; 88],
    )]);
    assert!(
        decode(&wrong_version)
            .unwrap_err()
            .to_string()
            .contains("scene_exit version 3")
    );

    let wrong_stride = build_v2_trace(vec![(
        TraceChannel::SceneExit,
        2,
        TraceChannelStatus::Present,
        vec![0; 72],
    )]);
    assert!(
        decode(&wrong_stride)
            .unwrap_err()
            .to_string()
            .contains("expected 88")
    );

    let truncated = build_v2_trace(vec![(
        TraceChannel::SceneExit,
        2,
        TraceChannelStatus::Truncated,
        scene_exit_v2_payload(),
    )]);
    assert!(
        decode(&truncated)
            .unwrap_err()
            .to_string()
            .contains("status Truncated")
    );

    let collision_truncated = build_v2_trace(vec![(
        TraceChannel::PlayerBackgroundCollision,
        1,
        TraceChannelStatus::Truncated,
        background_collision_v1_payload(),
    )]);
    assert!(
        decode(&collision_truncated)
            .unwrap_err()
            .to_string()
            .contains("status Truncated")
    );

    let collision_not_sampled = build_v2_trace(vec![(
        TraceChannel::PlayerBackgroundCollision,
        1,
        TraceChannelStatus::NotSampled,
        background_collision_v1_payload(),
    )]);
    assert!(
        decode(&collision_not_sampled)
            .unwrap_err()
            .to_string()
            .contains("status NotSampled")
    );

    let collision_surfaces_truncated = build_v2_trace(vec![
        (
            TraceChannel::Stage,
            1,
            TraceChannelStatus::Present,
            stage_payload(false),
        ),
        (
            TraceChannel::PlayerCollisionSurfaces,
            1,
            TraceChannelStatus::Truncated,
            empty_collision_surfaces_payload(),
        ),
    ]);
    assert!(
        decode(&collision_surfaces_truncated)
            .unwrap_err()
            .to_string()
            .contains("status Truncated")
    );
}

#[test]
fn v2_rejects_scene_exit_and_collision_corruption() {
    let mut scene = scene_exit_v2_payload();
    scene[87] = 1;
    let error = decode(&build_v2_trace(vec![(
        TraceChannel::SceneExit,
        2,
        TraceChannelStatus::Present,
        scene,
    )]))
    .unwrap_err();
    assert!(error.to_string().contains("reserved"));

    let mut scene = scene_exit_v2_payload();
    let flags = u32_at(&scene, 8) | SCENE_EXIT_OBSERVED_COUNT_SATURATED;
    scene[8..12].copy_from_slice(&flags.to_le_bytes());
    let error = decode(&build_v2_trace(vec![(
        TraceChannel::SceneExit,
        2,
        TraceChannelStatus::Present,
        scene,
    )]))
    .unwrap_err();
    assert!(error.to_string().contains("candidate count"));

    let mut scene = scene_exit_v2_payload();
    scene[20] = 4;
    let error = decode(&build_v2_trace(vec![(
        TraceChannel::SceneExit,
        2,
        TraceChannelStatus::Present,
        scene,
    )]))
    .unwrap_err();
    assert!(error.to_string().contains("raw parameters"));

    let mut scene = scene_exit_v2_payload();
    let flags = u32_at(&scene, 8) | SCENE_EXIT_CHANGE_OK;
    scene[8..12].copy_from_slice(&flags.to_le_bytes());
    let error = decode(&build_v2_trace(vec![(
        TraceChannel::SceneExit,
        2,
        TraceChannelStatus::Present,
        scene,
    )]))
    .unwrap_err();
    assert!(error.to_string().contains("change-ok"));

    let mut scene = scene_exit_v2_payload();
    let flags = u32_at(&scene, 8) | SCENE_EXIT_PLAYER_LATCHED;
    scene[8..12].copy_from_slice(&flags.to_le_bytes());
    scene[24] = 2;
    scene[28..30].copy_from_slice(&7_u16.to_le_bytes());
    let error = decode(&build_v2_trace(vec![(
        TraceChannel::SceneExit,
        2,
        TraceChannelStatus::Present,
        scene,
    )]))
    .unwrap_err();
    assert!(error.to_string().contains("radial"));

    let mut collision = background_collision_v1_payload();
    collision[0..4].copy_from_slice(&(1_u32 << 31).to_le_bytes());
    let error = decode(&build_v2_trace(vec![(
        TraceChannel::PlayerBackgroundCollision,
        1,
        TraceChannelStatus::Present,
        collision,
    )]))
    .unwrap_err();
    assert!(error.to_string().contains("unknown"));

    let mut collision = background_collision_v1_payload();
    collision[0..4].copy_from_slice(
        &(COLLISION_TRAJECTORY_VALID | COLLISION_GROUND_IDENTITY_PRESENT).to_le_bytes(),
    );
    let error = decode(&build_v2_trace(vec![(
        TraceChannel::PlayerBackgroundCollision,
        1,
        TraceChannelStatus::Present,
        collision,
    )]))
    .unwrap_err();
    assert!(error.to_string().contains("identity sentinel"));

    let mut collision = background_collision_v1_payload();
    write_f32(&mut collision, 116, 6.0);
    let error = decode(&build_v2_trace(vec![(
        TraceChannel::PlayerBackgroundCollision,
        1,
        TraceChannelStatus::Present,
        collision,
    )]))
    .unwrap_err();
    assert!(error.to_string().contains("trajectory"));

    let mut collision = background_collision_v1_payload();
    collision[0..4].copy_from_slice(
        &(COLLISION_TRAJECTORY_VALID | COLLISION_WALL_PROBE_ENABLED | COLLISION_WALL_CONTACT)
            .to_le_bytes(),
    );
    let error = decode(&build_v2_trace(vec![(
        TraceChannel::PlayerBackgroundCollision,
        1,
        TraceChannelStatus::Present,
        collision,
    )]))
    .unwrap_err();
    assert!(error.to_string().contains("aggregate wall contact"));

    let mut collision = background_collision_v1_payload();
    collision[0..4].copy_from_slice(
        &(COLLISION_TRAJECTORY_VALID | COLLISION_WATER_PROBE_ENABLED | COLLISION_WATER_IN)
            .to_le_bytes(),
    );
    let error = decode(&build_v2_trace(vec![(
        TraceChannel::PlayerBackgroundCollision,
        1,
        TraceChannelStatus::Present,
        collision,
    )]))
    .unwrap_err();
    assert!(error.to_string().contains("contradictory"));

    let mut collision = background_collision_v1_payload();
    collision[0..4].copy_from_slice(
        &(COLLISION_TRAJECTORY_VALID | COLLISION_GROUND_PLANE_VALID).to_le_bytes(),
    );
    let error = decode(&build_v2_trace(vec![(
        TraceChannel::PlayerBackgroundCollision,
        1,
        TraceChannelStatus::Present,
        collision,
    )]))
    .unwrap_err();
    assert!(error.to_string().contains("contradictory"));
}

#[test]
fn v2_decodes_goal_progress_and_bounded_selected_actors() {
    let mut goal = vec![0; 32];
    goal[0..4].copy_from_slice(
        &(GOAL_CONFIGURED | GOAL_REACHED | GOAL_AUTHORED | GOAL_FIRST_HIT_TICK_PRESENT)
            .to_le_bytes(),
    );
    goal[4..8].copy_from_slice(&0x1234_5678_u32.to_le_bytes());
    goal[8..10].copy_from_slice(&3_u16.to_le_bytes());
    goal[10..12].copy_from_slice(&2_u16.to_le_bytes());
    goal[12..14].copy_from_slice(&3_u16.to_le_bytes());
    goal[14..16].copy_from_slice(&3_u16.to_le_bytes());
    goal[16] = 2;
    goal[17] = 2;
    goal[18..20].copy_from_slice(&30_u16.to_le_bytes());
    goal[20..22].copy_from_slice(&7_u16.to_le_bytes());
    goal[24..32].copy_from_slice(&12_u64.to_le_bytes());

    let mut actors = vec![0; 656];
    actors[0..2].copy_from_slice(&1_u16.to_le_bytes());
    actors[2..4].copy_from_slice(&(SELECTED_ACTOR_CAPACITY as u16).to_le_bytes());
    actors[4..8].copy_from_slice(&SELECTED_ACTORS_TRUNCATED.to_le_bytes());
    actors[8..12].copy_from_slice(&2_u32.to_le_bytes());
    for index in 0..SELECTED_ACTOR_CAPACITY {
        let offset = 16 + index * 40;
        actors[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        actors[offset + 4..offset + 6].copy_from_slice(&(-1_i16).to_le_bytes());
        actors[offset + 6..offset + 8].copy_from_slice(&u16::MAX.to_le_bytes());
        actors[offset + 8] = -1_i8 as u8;
        actors[offset + 9] = -1_i8 as u8;
    }
    actors[16..20].copy_from_slice(&10_u32.to_le_bytes());
    actors[20..22].copy_from_slice(&77_i16.to_le_bytes());
    actors[22..24].copy_from_slice(&4_u16.to_le_bytes());
    actors[24] = 1;
    actors[25] = 2;
    actors[26..28].copy_from_slice(&9_i16.to_le_bytes());
    actors[28..32].copy_from_slice(&0x1234_u32.to_le_bytes());
    write_f32(&mut actors, 32, 1.0);
    write_f32(&mut actors, 36, 2.0);
    write_f32(&mut actors, 40, 3.0);
    actors[44..46].copy_from_slice(&4_i16.to_le_bytes());
    actors[50..52].copy_from_slice(&7_i16.to_le_bytes());

    let bytes = build_v2_trace(vec![
        (
            TraceChannel::GoalProgress,
            1,
            TraceChannelStatus::Present,
            goal.clone(),
        ),
        (
            TraceChannel::SelectedActors,
            1,
            TraceChannelStatus::Present,
            actors.clone(),
        ),
    ]);
    let decoded = decode(&bytes).unwrap();
    let record = &decoded.records[0];
    let decoded_goal = record.goal_progress.as_ref().unwrap();
    assert!(decoded_goal.reached && decoded_goal.authored);
    assert_eq!(decoded_goal.goal_name_hash, Some(0x1234_5678));
    assert_eq!(decoded_goal.first_hit_tick, Some(12));
    let decoded_actors = record.selected_actors.as_ref().unwrap();
    assert!(decoded_actors.truncated);
    assert_eq!(decoded_actors.observed_count, 2);
    assert_eq!(decoded_actors.actors[0].session_process_id, 10);
    assert_eq!(decoded_actors.actors[0].position, [1.0, 2.0, 3.0]);

    goal[24..32].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(
        decode(&build_v2_trace(vec![(
            TraceChannel::GoalProgress,
            1,
            TraceChannelStatus::Present,
            goal,
        )]))
        .unwrap_err()
        .to_string()
        .contains("goal-progress")
    );
    actors[56..60].copy_from_slice(&0_u32.to_le_bytes());
    assert!(
        decode(&build_v2_trace(vec![(
            TraceChannel::SelectedActors,
            1,
            TraceChannelStatus::Present,
            actors,
        )]))
        .unwrap_err()
        .to_string()
        .contains("unused")
    );
}

#[test]
fn v2_decodes_portable_player_interaction_identities() {
    let mut action = vec![0; 136];
    action[104..108].copy_from_slice(&(TALK_PARTNER_PRESENT | GRABBED_ACTOR_PRESENT).to_le_bytes());
    action[108] = 0x15;
    action[112..116].copy_from_slice(&11_u32.to_le_bytes());
    action[116..118].copy_from_slice(&42_i16.to_le_bytes());
    action[118..120].copy_from_slice(&7_u16.to_le_bytes());
    action[120] = 1;
    action[121] = 2;
    action[124..128].copy_from_slice(&12_u32.to_le_bytes());
    action[128..130].copy_from_slice(&43_i16.to_le_bytes());
    action[130..132].copy_from_slice(&8_u16.to_le_bytes());
    action[132] = 3;
    action[133] = 4;

    let decoded = decode(&build_v2_trace(vec![(
        TraceChannel::PlayerAction,
        2,
        TraceChannelStatus::Present,
        action.clone(),
    )]))
    .unwrap();
    assert_eq!(
        decoded.channel_formats[&TraceChannel::PlayerAction],
        TraceChannelWireFormat {
            version: 2,
            stride: 136,
        }
    );
    let player_action = decoded.records[0].player_action.as_ref().unwrap();
    assert_eq!(player_action.do_status, 0x15);
    assert_eq!(player_action.talk_partner.as_ref().unwrap().actor_name, 42);
    assert_eq!(player_action.talk_partner.as_ref().unwrap().set_id, 7);
    assert_eq!(
        player_action.talk_partner.as_ref().unwrap().home_position,
        None
    );
    assert_eq!(player_action.grabbed_actor.as_ref().unwrap().actor_name, 43);
    assert_eq!(player_action.grabbed_actor.as_ref().unwrap().home_room, 3);

    action[104..108].copy_from_slice(&GRABBED_ACTOR_PRESENT.to_le_bytes());
    let error = decode(&build_v2_trace(vec![(
        TraceChannel::PlayerAction,
        2,
        TraceChannelStatus::Present,
        action,
    )]))
    .unwrap_err();
    assert!(error.to_string().contains("noncanonical absent"));
}

#[test]
fn v3_decodes_actor_home_positions_and_rejects_noncanonical_values() {
    let mut action = vec![0; 160];
    action[104..108].copy_from_slice(&TALK_PARTNER_PRESENT.to_le_bytes());
    action[112..116].copy_from_slice(&11_u32.to_le_bytes());
    action[116..118].copy_from_slice(&42_i16.to_le_bytes());
    action[118..120].copy_from_slice(&7_u16.to_le_bytes());
    action[120] = 1;
    action[121] = 2;
    for (index, value) in [10.0_f32, 20.0, 30.0].into_iter().enumerate() {
        action[124 + index * 4..128 + index * 4].copy_from_slice(&value.to_le_bytes());
    }
    action[136..140].copy_from_slice(&u32::MAX.to_le_bytes());
    action[140..142].copy_from_slice(&(-1_i16).to_le_bytes());
    action[142..144].copy_from_slice(&u16::MAX.to_le_bytes());
    action[144] = u8::MAX;
    action[145] = u8::MAX;

    let decoded = decode(&build_v2_trace(vec![(
        TraceChannel::PlayerAction,
        3,
        TraceChannelStatus::Present,
        action.clone(),
    )]))
    .unwrap();
    assert_eq!(
        decoded.records[0]
            .player_action
            .as_ref()
            .unwrap()
            .talk_partner
            .as_ref()
            .unwrap()
            .home_position,
        Some([10.0, 20.0, 30.0])
    );

    action[124..128].copy_from_slice(&(-0.0_f32).to_le_bytes());
    assert!(
        decode(&build_v2_trace(vec![(
            TraceChannel::PlayerAction,
            3,
            TraceChannelStatus::Present,
            action,
        )]))
        .unwrap_err()
        .to_string()
        .contains("home position")
    );
}

#[test]
fn rejects_unknown_required_v2_channel() {
    let mut bytes = minimal_v2_header(2, TraceChannel::Core.bit());
    bytes.resize(V2_HEADER_SIZE + 2 * V2_DIRECTORY_ENTRY_SIZE, 0);
    write_empty_descriptor(&mut bytes[V2_HEADER_SIZE..], 0, 32, true);
    write_empty_descriptor(
        &mut bytes[V2_HEADER_SIZE + V2_DIRECTORY_ENTRY_SIZE..],
        15,
        1,
        true,
    );
    assert!(decode(&bytes).unwrap_err().to_string().contains("required"));
}

#[test]
fn rejects_trace_record_count_above_global_bound_before_allocation() {
    let mut bytes = minimal_v2_header(0, TraceChannel::Core.bit());
    bytes[20..28].copy_from_slice(&((MAX_TRACE_RECORDS as u64) + 1).to_le_bytes());
    assert!(
        decode(&bytes)
            .unwrap_err()
            .to_string()
            .contains("record count exceeds")
    );
}

#[test]
fn v3_authenticates_stage_boot_origin() {
    let mut bytes = build_v2_trace(Vec::new());
    let channel_count = usize::from(u16_at(&bytes, 32));
    bytes.splice(V2_HEADER_SIZE..V2_HEADER_SIZE, [0_u8; 64]);
    bytes[8..10].copy_from_slice(&3_u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&(V3_HEADER_SIZE as u16).to_le_bytes());
    bytes[36..44].copy_from_slice(&(V3_HEADER_SIZE as u64).to_le_bytes());
    let data_offset = usize_at_u64(&bytes, 44).unwrap() + 64;
    bytes[44..52].copy_from_slice(&(data_offset as u64).to_le_bytes());
    for index in 0..channel_count {
        let descriptor = V3_HEADER_SIZE + index * V2_DIRECTORY_ENTRY_SIZE;
        for offset in [16, 32] {
            let value = usize_at_u64(&bytes, descriptor + offset).unwrap() + 64;
            bytes[descriptor + offset..descriptor + offset + 8]
                .copy_from_slice(&(value as u64).to_le_bytes());
        }
    }
    bytes[64] = 1;
    bytes[65] = 2;
    bytes[66] = 1;
    bytes[67] = 3;
    bytes[68..70].copy_from_slice(&1_i16.to_le_bytes());
    bytes[70] = 7;
    bytes[72..79].copy_from_slice(b"F_SP103");

    let decoded = decode(&bytes).unwrap();
    assert_eq!(decoded.version, 3);
    assert_eq!(
        decoded.boot,
        TapeBoot::Stage {
            stage: "F_SP103".into(),
            room: 1,
            point: 1,
            layer: 3,
            save_slot: Some(2),
            fixture: None,
        }
    );

    bytes[79] = 1;
    assert!(
        decode(&bytes)
            .unwrap_err()
            .to_string()
            .contains("noncanonical stage boot")
    );
}

#[test]
fn v4_authenticates_embedded_scenario_fixture() {
    use crate::scenario_fixture::{HealthFixture, PlayerForm, SCENARIO_FIXTURE_SCHEMA};

    let mut bytes = build_v2_trace(Vec::new());
    let channel_count = usize::from(u16_at(&bytes, 32));
    bytes.splice(V2_HEADER_SIZE..V2_HEADER_SIZE, [0_u8; 64]);
    bytes[8..10].copy_from_slice(&4_u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&(V4_HEADER_SIZE as u16).to_le_bytes());
    bytes[36..44].copy_from_slice(&(V4_HEADER_SIZE as u64).to_le_bytes());
    let data_offset = usize_at_u64(&bytes, 44).unwrap() + 64;
    bytes[44..52].copy_from_slice(&(data_offset as u64).to_le_bytes());
    for index in 0..channel_count {
        let descriptor = V4_HEADER_SIZE + index * V2_DIRECTORY_ENTRY_SIZE;
        for offset in [16, 32] {
            let value = usize_at_u64(&bytes, descriptor + offset).unwrap() + 64;
            bytes[descriptor + offset..descriptor + offset + 8]
                .copy_from_slice(&(value as u64).to_le_bytes());
        }
    }
    bytes[64] = 1;
    bytes[66] = 1;
    bytes[67] = 3;
    bytes[68..70].copy_from_slice(&1_i16.to_le_bytes());
    bytes[70] = 7;
    bytes[72..79].copy_from_slice(b"F_SP103");
    let fixture = ScenarioFixture {
        schema: SCENARIO_FIXTURE_SCHEMA.into(),
        name: "low-health wolf".into(),
        form: Some(PlayerForm::Wolf),
        health: Some(HealthFixture {
            current: 4,
            maximum: 20,
        }),
        rng: Vec::new(),
        video_mode: None,
        inventory: Vec::new(),
        equipment: Vec::new(),
        flags: Vec::new(),
        settings: Vec::new(),
    };
    let encoded = fixture.encode().unwrap();
    let fixture_offset = bytes.len();
    bytes[88..96].copy_from_slice(&(fixture_offset as u64).to_le_bytes());
    bytes[96..100].copy_from_slice(&(encoded.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&encoded);

    let decoded = decode(&bytes).unwrap();
    assert_eq!(decoded.version, 4);
    assert!(matches!(
        decoded.boot,
        TapeBoot::Stage {
            fixture: Some(value),
            ..
        } if value == fixture
    ));

    bytes[fixture_offset + 20] = 1;
    assert!(decode(&bytes).is_err());
}

#[test]
fn v5_authenticates_trigger_retention_metadata() {
    let mut bytes = build_v2_trace(Vec::new());
    let channel_count = usize::from(u16_at(&bytes, 32));
    bytes.splice(V2_HEADER_SIZE..V2_HEADER_SIZE, [0_u8; 64]);
    bytes[8..10].copy_from_slice(&5_u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&(V5_HEADER_SIZE as u16).to_le_bytes());
    bytes[28..32].copy_from_slice(&(FILE_COMPLETE | FILE_TRIGGER_RETENTION).to_le_bytes());
    bytes[36..44].copy_from_slice(&(V5_HEADER_SIZE as u64).to_le_bytes());
    let data_offset = usize_at_u64(&bytes, 44).unwrap() + 64;
    bytes[44..52].copy_from_slice(&(data_offset as u64).to_le_bytes());
    for index in 0..channel_count {
        let descriptor = V5_HEADER_SIZE + index * V2_DIRECTORY_ENTRY_SIZE;
        for offset in [16, 32] {
            let value = usize_at_u64(&bytes, descriptor + offset).unwrap() + 64;
            bytes[descriptor + offset..descriptor + offset + 8]
                .copy_from_slice(&(value as u64).to_le_bytes());
        }
    }
    bytes[100..104].copy_from_slice(&KNOWN_RETENTION_TRIGGERS.to_le_bytes());
    bytes[104..108].copy_from_slice(&RETENTION_PREDICATE_HIT.to_le_bytes());
    bytes[108..112].copy_from_slice(&2_u32.to_le_bytes());
    bytes[112..116].copy_from_slice(&1_u32.to_le_bytes());
    bytes[116..120].copy_from_slice(&1_u32.to_le_bytes());
    bytes[120..128].copy_from_slice(&10_u64.to_le_bytes());

    let decoded = decode(&bytes).unwrap();
    assert_eq!(decoded.version, 5);
    assert_eq!(
        decoded.retention,
        Some(TraceRetention {
            configured_triggers: KNOWN_RETENTION_TRIGGERS,
            observed_triggers: RETENTION_PREDICATE_HIT,
            pre_trigger_ticks: 2,
            post_trigger_ticks: 1,
            trigger_count: 1,
            observed_sample_count: 10,
        })
    );

    bytes[104..108].copy_from_slice(&(1_u32 << 12).to_le_bytes());
    assert!(
        decode(&bytes)
            .unwrap_err()
            .to_string()
            .contains("retention")
    );
}

fn write_empty_descriptor(bytes: &mut [u8], id: u16, stride: u32, required: bool) {
    bytes[0..2].copy_from_slice(&id.to_le_bytes());
    bytes[2..4].copy_from_slice(&1_u16.to_le_bytes());
    let flags = CHANNEL_DENSE | if required { CHANNEL_REQUIRED } else { 0 };
    bytes[4..8].copy_from_slice(&flags.to_le_bytes());
    bytes[8..12].copy_from_slice(&stride.to_le_bytes());
    bytes[12..16].copy_from_slice(&1_u32.to_le_bytes());
    let data_offset = V2_HEADER_SIZE + 2 * V2_DIRECTORY_ENTRY_SIZE;
    bytes[16..24].copy_from_slice(&(data_offset as u64).to_le_bytes());
    bytes[32..40].copy_from_slice(&(data_offset as u64).to_le_bytes());
}

fn minimal_v2_header(channel_count: u16, requested: u64) -> Vec<u8> {
    let mut bytes = vec![0; V2_HEADER_SIZE];
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&2_u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&(V2_HEADER_SIZE as u16).to_le_bytes());
    bytes[12..16].copy_from_slice(&30_u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&1_u32.to_le_bytes());
    bytes[28..32].copy_from_slice(&FILE_COMPLETE.to_le_bytes());
    bytes[32..34].copy_from_slice(&channel_count.to_le_bytes());
    bytes[34..36].copy_from_slice(&(V2_DIRECTORY_ENTRY_SIZE as u16).to_le_bytes());
    bytes[36..44].copy_from_slice(&(V2_HEADER_SIZE as u64).to_le_bytes());
    let data_offset = V2_HEADER_SIZE + usize::from(channel_count) * V2_DIRECTORY_ENTRY_SIZE;
    bytes[44..52].copy_from_slice(&(data_offset as u64).to_le_bytes());
    bytes[52..60].copy_from_slice(&requested.to_le_bytes());
    bytes
}

fn build_v2_trace(
    extra_channels: Vec<(TraceChannel, u16, TraceChannelStatus, Vec<u8>)>,
) -> Vec<u8> {
    let mut core = vec![0; 32];
    core[0..8].copy_from_slice(&1_u64.to_le_bytes());
    core[8..16].copy_from_slice(&0_u64.to_le_bytes());
    core[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
    core[24..28].copy_from_slice(&CORE_SIMULATION_TICK_VALID.to_le_bytes());
    core[28] = 2;
    core[29] = 1;
    let mut channels = vec![(TraceChannel::Core, 1, TraceChannelStatus::Present, core)];
    channels.extend(extra_channels);
    let requested = channels
        .iter()
        .fold(0_u64, |mask, (channel, _, _, _)| mask | channel.bit());
    let mut bytes = minimal_v2_header(channels.len() as u16, requested);
    bytes[20..28].copy_from_slice(&1_u64.to_le_bytes());
    bytes.resize(V2_HEADER_SIZE + channels.len() * V2_DIRECTORY_ENTRY_SIZE, 0);
    for (index, (channel, version, status, payload)) in channels.into_iter().enumerate() {
        let descriptor = V2_HEADER_SIZE + index * V2_DIRECTORY_ENTRY_SIZE;
        let status_offset = bytes.len();
        bytes.push(match status {
            TraceChannelStatus::NotSampled => 0,
            TraceChannelStatus::Present => 1,
            TraceChannelStatus::Absent => 2,
            TraceChannelStatus::Unavailable => 3,
            TraceChannelStatus::Truncated => 4,
        });
        let payload_offset = bytes.len();
        bytes.extend_from_slice(&payload);
        bytes[descriptor..descriptor + 2].copy_from_slice(&(channel as u16).to_le_bytes());
        bytes[descriptor + 2..descriptor + 4].copy_from_slice(&version.to_le_bytes());
        let flags = CHANNEL_DENSE
            | if channel == TraceChannel::Core {
                CHANNEL_REQUIRED
            } else {
                0
            };
        bytes[descriptor + 4..descriptor + 8].copy_from_slice(&flags.to_le_bytes());
        bytes[descriptor + 8..descriptor + 12]
            .copy_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes[descriptor + 12..descriptor + 16].copy_from_slice(&1_u32.to_le_bytes());
        bytes[descriptor + 16..descriptor + 24]
            .copy_from_slice(&(status_offset as u64).to_le_bytes());
        bytes[descriptor + 24..descriptor + 32].copy_from_slice(&1_u64.to_le_bytes());
        bytes[descriptor + 32..descriptor + 40]
            .copy_from_slice(&(payload_offset as u64).to_le_bytes());
        bytes[descriptor + 40..descriptor + 48]
            .copy_from_slice(&(payload.len() as u64).to_le_bytes());
    }
    bytes
}

fn scene_exit_v2_payload() -> Vec<u8> {
    let mut payload = vec![0; 88];
    payload[0..4].copy_from_slice(&0x1234_u32.to_le_bytes());
    payload[4..8].copy_from_slice(&0x0604_0503_u32.to_le_bytes());
    payload[8..12].copy_from_slice(
        &(SCENE_EXIT_VOLUME_VALID | SCENE_EXIT_PLAYER_INSIDE | SCENE_EXIT_DESTINATION_VALID)
            .to_le_bytes(),
    );
    write_f32(&mut payload, 12, -0.25);
    payload[16..18].copy_from_slice(&(-42_i16).to_le_bytes());
    payload[18..20].copy_from_slice(&9_u16.to_le_bytes());
    payload[20] = 3;
    payload[21] = 4;
    payload[22] = 5;
    payload[23] = 6;
    payload[24] = 1;
    payload[25] = 2;
    payload[26] = 1;
    payload[27] = u8::MAX;
    payload[28..30].copy_from_slice(&u16::MAX.to_le_bytes());
    payload[30..32].copy_from_slice(&0x123_i16.to_le_bytes());
    payload[32] = u8::MAX;
    for (offset, value) in [1.0, 2.0, 3.0, 10.0, 11.0, 12.0, 100.0, 200.0, 300.0]
        .into_iter()
        .enumerate()
    {
        write_f32(&mut payload, 36 + offset * 4, value);
    }
    payload[72..79].copy_from_slice(b"F_SP103");
    payload[80] = 1;
    payload[81] = -1_i8 as u8;
    payload[82..84].copy_from_slice(&4_i16.to_le_bytes());
    payload[84] = 17;
    payload[85] = 3;
    payload[86] = u8::MAX;
    payload
}

fn background_collision_v1_payload() -> Vec<u8> {
    let mut payload = vec![0; 128];
    payload[0..4].copy_from_slice(&COLLISION_TRAJECTORY_VALID.to_le_bytes());
    write_f32(&mut payload, 4, -1.0e9);
    write_f32(&mut payload, 8, 1.0e9);
    write_f32(&mut payload, 12, -1.0e9);
    for offset in [16, 18, 40, 42, 48, 50, 56, 58, 68, 70, 80, 82] {
        payload[offset..offset + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    }
    for offset in [20, 44, 52, 60, 72, 84] {
        payload[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    }
    for (index, value) in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 5.0, 7.0, 9.0]
        .into_iter()
        .enumerate()
    {
        write_f32(&mut payload, 92 + index * 4, value);
    }
    payload
}

fn stage_payload(next_pending: bool) -> Vec<u8> {
    let mut payload = vec![0; 32];
    payload[0..7].copy_from_slice(b"F_SP103");
    payload[8] = 1;
    payload[9] = u8::MAX;
    payload[10..12].copy_from_slice(&1_i16.to_le_bytes());
    if next_pending {
        payload[12..19].copy_from_slice(b"F_SP104");
        payload[20] = 1;
        payload[21] = u8::MAX;
        payload[22..24].copy_from_slice(&0_i16.to_le_bytes());
        payload[24..28].copy_from_slice(&1_u32.to_le_bytes());
    } else {
        payload[20] = u8::MAX;
        payload[21] = u8::MAX;
        payload[22..24].copy_from_slice(&(-1_i16).to_le_bytes());
    }
    payload
}

fn empty_collision_surface(payload: &mut [u8], index: usize) {
    let offset = 16 + index * 80;
    payload[offset + 4] = match index {
        0 => 1,
        1 => 2,
        2 => 3,
        3..=5 => 4,
        _ => unreachable!(),
    };
    payload[offset + 5] = index.saturating_sub(3) as u8;
    for field in [8, 10, 16, 18] {
        payload[offset + field..offset + field + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    }
    payload[offset + 12..offset + 16].copy_from_slice(&u32::MAX.to_le_bytes());
    payload[offset + 40] = u8::MAX;
    payload[offset + 41] = INVALID_I8 as u8;
    payload[offset + 42] = INVALID_I8 as u8;
    payload[offset + 43] = INVALID_I8 as u8;
    payload[offset + 44] = INVALID_I8 as u8;
    payload[offset + 45] = u8::MAX;
    payload[offset + 46] = u8::MAX;
    payload[offset + 47] = INVALID_I8 as u8;
    payload[offset + 48..offset + 50].copy_from_slice(&INVALID_I16.to_le_bytes());
    for geometry in 0..6 {
        let field = offset + 52 + geometry * 2;
        payload[field..field + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    }
}

fn empty_collision_surfaces_payload() -> Vec<u8> {
    let mut payload = vec![0; 496];
    payload[0..4].copy_from_slice(&COLLISION_SURFACE_SET_ROOM_VALID.to_le_bytes());
    payload[4] = 1;
    payload[8..10].copy_from_slice(&0x003f_u16.to_le_bytes());
    for index in 0..6 {
        empty_collision_surface(&mut payload, index);
    }
    payload
}

fn collision_surfaces_pending_ground_payload() -> Vec<u8> {
    let mut payload = empty_collision_surfaces_payload();
    payload[0..4].copy_from_slice(
        &(COLLISION_SURFACE_SET_ROOM_VALID | COLLISION_SURFACE_SET_NEXT_STAGE_PENDING)
            .to_le_bytes(),
    );
    payload[5] = 1;
    payload[6] = 1;
    payload[7] = 1;
    payload[10] = 1;
    let offset = 16;
    let flags = COLLISION_SURFACE_IDENTITY_PRESENT
        | COLLISION_SURFACE_BACKING_PRESENT
        | COLLISION_SURFACE_CODES_PRESENT
        | COLLISION_SURFACE_MATERIAL_PRESENT
        | COLLISION_SURFACE_SOURCE_ROOM_PRESENT
        | COLLISION_SURFACE_SOURCE_ROOM_EXACT
        | COLLISION_SURFACE_SCLS_SOURCE_PRESENT
        | COLLISION_SURFACE_DESTINATION_PRESENT
        | COLLISION_SURFACE_PENDING_MATCH
        | COLLISION_SURFACE_GEOMETRY_PRESENT
        | COLLISION_SURFACE_KCL_HEIGHT_PRESENT;
    payload[offset..offset + 4].copy_from_slice(&flags.to_le_bytes());
    payload[offset + 6] = 2;
    payload[offset + 7] = 0x1f;
    payload[offset + 8..offset + 10].copy_from_slice(&7_u16.to_le_bytes());
    payload[offset + 10..offset + 12].copy_from_slice(&2217_u16.to_le_bytes());
    payload[offset + 16..offset + 18].copy_from_slice(&19_u16.to_le_bytes());
    payload[offset + 20..offset + 24].copy_from_slice(&1_u32.to_le_bytes());
    payload[offset + 24..offset + 28].copy_from_slice(&0x1234_u32.to_le_bytes());
    payload[offset + 28..offset + 32].copy_from_slice(&0x5678_u32.to_le_bytes());
    payload[offset + 32..offset + 36].copy_from_slice(&0x9abc_u32.to_le_bytes());
    payload[offset + 36..offset + 40].copy_from_slice(&0xdef0_u32.to_le_bytes());
    payload[offset + 40] = 1;
    payload[offset + 41] = 1;
    payload[offset + 42] = 1;
    payload[offset + 43] = 1;
    payload[offset + 44] = u8::MAX;
    payload[offset + 45] = 0;
    payload[offset + 46] = 3;
    payload[offset + 47] = u8::MAX;
    payload[offset + 48..offset + 50].copy_from_slice(&0_i16.to_le_bytes());
    payload[offset + 50] = 5;
    for (geometry, value) in [2_u16, 3, 5, 7, 11].into_iter().enumerate() {
        let field = offset + 52 + geometry * 2;
        payload[field..field + 2].copy_from_slice(&value.to_le_bytes());
    }
    write_f32(&mut payload, offset + 64, 42.5);
    payload[offset + 68..offset + 75].copy_from_slice(b"F_SP104");
    payload
}

fn background_collision_with_ground(bg: u16, poly: u16) -> Vec<u8> {
    let mut payload = background_collision_v1_payload();
    let flags = u32_at(&payload, 0)
        | COLLISION_GROUND_PROBE_VALID
        | COLLISION_GROUND_CONTACT
        | COLLISION_GROUND_IDENTITY_PRESENT;
    payload[0..4].copy_from_slice(&flags.to_le_bytes());
    write_f32(&mut payload, 4, 0.0);
    payload[16..18].copy_from_slice(&bg.to_le_bytes());
    payload[18..20].copy_from_slice(&poly.to_le_bytes());
    payload
}

fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_bits().to_le_bytes());
}
