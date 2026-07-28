use super::*;
use dusklight_evidence::native_episode_shard::authored_milestone_objective_identity;
use dusklight_objectives::milestone_dsl::compile_source;
use dusklight_world::actor_profile_catalog::{ACTOR_PROFILE_CATALOG_SCHEMA, ActorProfileEntry};

fn catalog_for(shard: &NativeEpisodeShard) -> ActorProfileCatalog {
    let mut names = shard
        .episodes
        .iter()
        .flat_map(|episode| &episode.steps)
        .flat_map(|step| [&step.pre_input, &step.post_simulation])
        .flat_map(|observation| &observation.actors)
        .map(|actor| actor.profile_name)
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();
    let mut catalog = ActorProfileCatalog {
        schema: ACTOR_PROFILE_CATALOG_SCHEMA.into(),
        identity: String::new(),
        profiles: names
            .into_iter()
            .enumerate()
            .map(|(slot, profile_name)| ActorProfileEntry {
                slot: slot as u32,
                present: true,
                layer_id: Some(u32::MAX - 2),
                list_id: Some(7),
                list_priority: Some(u16::MAX - 2),
                profile_name: Some(profile_name),
                process_size: Some(512),
                auxiliary_size: Some(0),
                parameters: Some(0),
                is_leaf: Some(true),
                draw_priority: Some(slot as i16),
                is_actor: Some(true),
                status: Some(0),
                group: Some(0),
                cull_type: Some(0),
            })
            .collect(),
    };
    catalog.identity = catalog.computed_identity().unwrap();
    catalog
}

fn shard() -> NativeEpisodeShard {
    let mut shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v6.dseps"
    ))
    .unwrap();
    shard.episodes.truncate(1);
    shard.episodes[0].steps.truncate(1);
    shard
}

fn shard_v7() -> NativeEpisodeShard {
    let mut shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v7.dseps"
    ))
    .unwrap();
    shard.episodes.truncate(1);
    shard.episodes[0].steps.truncate(1);
    shard
}

fn shard_v10() -> NativeEpisodeShard {
    let mut shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v10.dseps"
    ))
    .unwrap();
    shard.episodes.truncate(1);
    shard.episodes[0].steps.truncate(1);
    shard
}

fn shard_v14() -> NativeEpisodeShard {
    let mut shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v14.dseps"
    ))
    .unwrap();
    shard.episodes.truncate(1);
    shard.episodes[0].steps.truncate(1);
    shard
}

fn shard_v15() -> NativeEpisodeShard {
    let mut shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v15.dseps"
    ))
    .unwrap();
    shard.episodes.truncate(1);
    shard.episodes[0].steps.truncate(1);
    shard
}

fn shard_v17() -> NativeEpisodeShard {
    NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v17.dseps"
    ))
    .unwrap()
}

fn shard_v27() -> NativeEpisodeShard {
    NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v27.dseps"
    ))
    .unwrap()
}

#[test]
fn builds_complete_absolute_link_camera_and_parent_relations() {
    let mut shard = shard();
    let catalog = catalog_for(&shard);
    shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
    let view = NativeEpisodeActorView::build(&shard, &catalog).unwrap();
    assert_eq!(view.observations.len(), 2);
    for observation in &view.observations {
        assert!(view.goal_graph.is_none());
        assert!(observation.goal_anchors.is_empty());
        assert_eq!(
            observation.actors.len(),
            shard.episodes[0].steps[0].pre_input.actors.len()
        );
        assert!(observation.player_present);
        assert!(observation.camera_frame_present);
        assert!(observation.actors.iter().all(|actor| {
            actor.link_relative_position.is_some()
                && actor.camera_relative_position.is_some()
                && !actor.profile_slots.is_empty()
                && actor.goal_relative_positions.is_empty()
                && actor.base_state.is_none()
        }));
        let attention = observation.actors[0].attention.as_ref().unwrap();
        assert_eq!(attention.flags, 0x20000002);
        assert!(attention.link_relative_position.is_some());
        assert!(attention.camera_relative_position.is_some());
        assert_eq!(
            observation.actors[0]
                .event_participation
                .as_ref()
                .unwrap()
                .event_id,
            27
        );
    }
    let bytes = view.canonical_bytes().unwrap();
    assert_eq!(
        NativeEpisodeActorView::decode_canonical(&bytes).unwrap(),
        view
    );
}

#[test]
fn exposes_v7_universal_base_state_without_fabricating_legacy_values() {
    let mut v7_shard = shard_v7();
    let catalog = catalog_for(&v7_shard);
    v7_shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
    let view = NativeEpisodeActorView::build(&v7_shard, &catalog).unwrap();
    for observation in &view.observations {
        let state = observation.actors[0]
            .base_state
            .as_ref()
            .expect("v7 actor base state");
        assert_eq!(state.actor_type, 5);
        assert_eq!(state.process_subtype, 6);
        assert_eq!(state.condition, 0x12);
        assert_eq!(state.old_room, 1);
        assert_eq!(state.pause_flag, 4);
        assert_eq!(state.process_init_state, -2);
        assert_eq!(state.process_create_phase, 7);
        assert_eq!(state.cull_type, 8);
        assert_eq!(state.demo_actor_id, 9);
        assert_eq!(state.carry_type, 10);
        assert!(state.heap_present);
        assert!(state.model_present);
        assert!(state.joint_collision_present);
        assert_eq!(state.absolute_old_position, [12.0, 2.5, -8.5]);
        assert_eq!(state.scale, [1.0, 2.0, 3.0]);
        assert_eq!(state.gravity, -3.0);
        assert_eq!(state.max_fall_speed, -20.0);
        assert_eq!(state.absolute_eye_position, [12.5, 7.0, -8.0]);
        assert_eq!(state.home_angle, [11, 12, 13]);
        assert_eq!(state.old_angle, [14, 15, 16]);
    }

    let mut legacy = shard();
    let legacy_catalog = catalog_for(&legacy);
    legacy.metadata.actor_profile_catalog_identity = Some(legacy_catalog.identity.clone());
    let legacy_view = NativeEpisodeActorView::build(&legacy, &legacy_catalog).unwrap();
    assert!(legacy_view.observations.iter().all(|observation| {
        observation.player_relationships_status == ActorViewChannelStatus::NotSampled
            && observation.player_relationships.is_empty()
            && observation
                .actors
                .iter()
                .all(|actor| actor.base_state.is_none())
    }));
}

#[test]
fn exposes_v15_typed_enemy_state_without_fabricating_legacy_values() {
    let mut v15_shard = shard_v15();
    let catalog = catalog_for(&v15_shard);
    v15_shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
    let view = NativeEpisodeActorView::build(&v15_shard, &catalog).unwrap();
    for observation in &view.observations {
        assert!(observation.actors.iter().all(|actor| actor.group == 2));
        let enemy = observation.actors[0]
            .enemy_base
            .as_ref()
            .expect("v15 enemy base state");
        assert_eq!(enemy.flags, 0x89);
        assert_eq!(enemy.throw_mode, 0x04);
        assert_eq!(enemy.absolute_down_position, [12.0, 3.5, -7.5]);
        assert_eq!(enemy.absolute_head_lock_position, [12.5, 7.0, -8.0]);
    }

    let mut legacy = shard_v10();
    let catalog = catalog_for(&legacy);
    legacy.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
    let view = NativeEpisodeActorView::build(&legacy, &catalog).unwrap();
    assert!(view.observations.iter().all(|observation| {
        observation
            .actors
            .iter()
            .all(|actor| actor.enemy_base.is_none())
    }));
}

#[test]
fn exposes_v14_return_place_writer_without_fabricating_legacy_values() {
    let mut v14_shard = shard_v14();
    let catalog = catalog_for(&v14_shard);
    v14_shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
    let view = NativeEpisodeActorView::build(&v14_shard, &catalog).unwrap();
    for observation in &view.observations {
        let writers = observation
            .actors
            .iter()
            .filter_map(|actor| actor.return_place_writer.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(writers.len(), 1);
        let writer = writers[0];
        assert_eq!(writer.save_room, 3);
        assert_eq!(writer.save_point, 2);
        assert_eq!(writer.switch_room, 0);
        assert_eq!(writer.required_event_set, 0x10);
        assert_eq!(writer.required_event_unset, u16::MAX);
        assert_eq!(writer.required_switch_set, 8);
        assert_eq!(writer.required_switch_unset, u8::MAX);
        assert!(!writer.no_telop_clear);
        assert!(writer.event_set_satisfied);
        assert!(writer.event_unset_satisfied);
        assert!(writer.switch_set_satisfied);
        assert!(writer.switch_unset_satisfied);
        assert!(!writer.eligible);
    }

    let mut tampered = view.clone();
    tampered.observations[0]
        .actors
        .iter_mut()
        .find_map(|actor| actor.return_place_writer.as_mut())
        .unwrap()
        .eligible = true;
    tampered.view_sha256 = tampered.compute_identity().unwrap();
    assert!(tampered.validate().is_err());

    let mut legacy = shard_v10();
    let catalog = catalog_for(&legacy);
    legacy.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
    let view = NativeEpisodeActorView::build(&legacy, &catalog).unwrap();
    assert!(view.observations.iter().all(|observation| {
        observation
            .actors
            .iter()
            .all(|actor| actor.return_place_writer.is_none())
    }));
}

#[test]
fn exposes_v17_trigger_geometry_with_relative_frames_and_legacy_masks() {
    let mut shard = shard_v17();
    let catalog = catalog_for(&shard);
    shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
    let view = NativeEpisodeActorView::build(&shard, &catalog).unwrap();
    for observation in &view.observations {
        let trigger = observation.actors[1]
            .trigger_volume
            .as_ref()
            .expect("v17 trigger volume");
        assert_eq!(trigger.kind, NativeActorTriggerVolumeKind::SceneExit);
        assert_eq!(trigger.shape, NativeActorTriggerVolumeShape::Box);
        assert_eq!(trigger.absolute_center, [10.0, 20.0, -30.0]);
        assert!(trigger.link_relative_center.is_some());
        assert!(trigger.camera_relative_center.is_some());
        assert!(trigger.yaw_relative_to_link.is_some());
        assert!(trigger.yaw_relative_to_camera.is_some());
    }

    let mut legacy = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v16.dseps"
    ))
    .unwrap();
    let legacy_catalog = catalog_for(&legacy);
    legacy.metadata.actor_profile_catalog_identity = Some(legacy_catalog.identity.clone());
    let legacy_view = NativeEpisodeActorView::build(&legacy, &legacy_catalog).unwrap();
    assert!(
        legacy_view
            .observations
            .iter()
            .flat_map(|observation| &observation.actors)
            .all(|actor| actor.trigger_volume.is_none())
    );
}

#[test]
fn exposes_v27_profile_bound_door20_state_and_rejects_authored_drift() {
    let mut shard = shard_v27();
    let catalog = catalog_for(&shard);
    shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
    let view = NativeEpisodeActorView::build(&shard, &catalog).unwrap();
    for observation in &view.observations {
        let door = observation
            .actors
            .iter()
            .find_map(|actor| actor.door20.as_ref())
            .expect("v27 DOOR20 state");
        assert_eq!(door.kind, 9);
        assert_eq!(door.door_model, 3);
        assert_eq!(door.front_option, 2);
        assert_eq!(door.back_option, 1);
        assert_eq!(door.front_room, 4);
        assert_eq!(door.back_room, 5);
        assert_eq!(door.exit_number, 6);
        assert!(door.message_door);
        assert_eq!(
            door.front_switch,
            Some(NativeActorDoor20SwitchState {
                id: 0x11,
                set: true,
            })
        );
        assert_eq!(
            door.back_switch,
            Some(NativeActorDoor20SwitchState {
                id: 0x22,
                set: false,
            })
        );
        assert_eq!(
            door.unlock_effect_switch,
            Some(NativeActorDoor20SwitchState {
                id: 0x33,
                set: true,
            })
        );
        assert_eq!(door.action, NativeActorDoor20Action::Demo);
        assert_eq!(door.active_side, NativeActorDoor20Side::Back);
        assert_eq!(door.event_variant, 13);
        assert!(door.locked);
        assert_eq!(door.key_type, 1);
        assert_eq!(door.enemy_clear_debounce, 42);
        assert!(door.opening_active);
        assert!(!door.closing_active);
        assert_eq!(door.door_angle, -1234);
        assert_eq!(door.stopper_side, NativeActorDoor20Side::Back);
        assert_eq!(
            door.front_stopper_status,
            NativeActorDoor20StopperStatus::RoomUnavailable
        );
        assert_eq!(
            door.back_stopper_status,
            NativeActorDoor20StopperStatus::Closed
        );
    }

    let mut tampered = view.clone();
    tampered.observations[0]
        .actors
        .iter_mut()
        .find_map(|actor| actor.door20.as_mut())
        .unwrap()
        .front_room = 5;
    tampered.view_sha256 = tampered.compute_identity().unwrap();
    assert!(tampered.validate().is_err());

    let mut schema_tampered = view.clone();
    schema_tampered.observation_schema =
        dusklight_evidence::native_episode_shard::LEARNING_OBSERVATION_SCHEMA_V26.into();
    schema_tampered.view_sha256 = schema_tampered.compute_identity().unwrap();
    assert!(schema_tampered.validate().is_err());

    let mut legacy = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v26.dseps"
    ))
    .unwrap();
    let legacy_catalog = catalog_for(&legacy);
    legacy.metadata.actor_profile_catalog_identity = Some(legacy_catalog.identity.clone());
    let legacy_view = NativeEpisodeActorView::build(&legacy, &legacy_catalog).unwrap();
    assert!(
        legacy_view
            .observations
            .iter()
            .flat_map(|observation| &observation.actors)
            .all(|actor| actor.door20.is_none())
    );
}

#[test]
fn materializes_v10_player_relationships_as_typed_actor_edges() {
    let mut shard = shard_v10();
    let catalog = catalog_for(&shard);
    shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
    let view = NativeEpisodeActorView::build(&shard, &catalog).unwrap();
    for observation in &view.observations {
        assert_eq!(
            observation.player_relationships_status,
            ActorViewChannelStatus::Present
        );
        assert_eq!(
            observation.player_relationships,
            [NativePlayerRelationshipEdge {
                role: NativePlayerRelationshipRole::TargetedActor,
                actor_runtime_generation: 7,
            }]
        );
        assert!(
            observation
                .actors
                .iter()
                .any(|actor| actor.runtime_generation == 7)
        );
    }

    let mut tampered = view.clone();
    tampered.observations[0].player_relationships[0].actor_runtime_generation = 999;
    tampered.view_sha256 = tampered.compute_identity().unwrap();
    assert!(tampered.validate().is_err());
}

#[test]
fn binds_exact_compiled_goal_and_derives_only_real_spatial_anchors() {
    const SOURCE: &str = r#"milestones 1.8
milestone spatial_goal {
  phase post_sim
  when player.in_aabb(10.0, 0.0, -10.0, 14.0, 6.0, -6.0) &&
       actor.placed.exists("F_SP103", 0, 4, 291) &&
       player.plane_signed_distance(1.0, 2.0, 3.0, 1.0, 0.0, 0.0) >= 0.0
}
"#;
    let compiled = compile_source(SOURCE).unwrap();
    let definition = &compiled.definitions[0];
    let mut shard = shard();
    shard.metadata.objective = definition.name.clone();
    shard.metadata.objective_identity = authored_milestone_objective_identity(
        &Digest(compiled.program_sha256).to_string(),
        &Digest(definition.sha256).to_string(),
    )
    .unwrap();
    for observation in shard.episodes.iter_mut().flat_map(|episode| {
        episode
            .steps
            .iter_mut()
            .flat_map(|step| [&mut step.pre_input, &mut step.post_simulation])
    }) {
        for actor in observation.actors.iter_mut().skip(1) {
            actor.set_id = 5;
        }
    }
    let catalog = catalog_for(&shard);
    shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
    let view =
        NativeEpisodeActorView::build_for_goal(&shard, &catalog, &compiled.bytes, "spatial_goal")
            .unwrap();
    let graph = view.goal_graph.as_ref().unwrap();
    assert_eq!(graph.program_sha256, Digest(compiled.program_sha256));
    assert_eq!(graph.definition_sha256, Digest(definition.sha256));
    assert_eq!(graph.spatial_anchors().len(), 3);
    for observation in &view.observations {
        assert_eq!(observation.goal_anchors.len(), 3);
        assert_eq!(observation.goal_anchors[0].node_index, 0);
        assert_eq!(observation.goal_anchors[0].sequence_step, 0);
        assert_eq!(
            observation.goal_anchors[0].status,
            NativeGoalAnchorStatus::Static
        );
        assert_eq!(
            observation.goal_anchors[0].absolute_position,
            Some([12.0, 3.0, -8.0])
        );
        assert_eq!(
            observation.goal_anchors[1].status,
            NativeGoalAnchorStatus::ResolvedActor
        );
        assert_eq!(
            observation.goal_anchors[1].actor_runtime_generation,
            Some(1)
        );
        assert_eq!(
            observation.actors[0].goal_relative_positions,
            [
                Some([0.5, 0.0, 0.0]),
                Some([0.0, 0.0, 0.0]),
                Some([11.5, 1.0, -11.0])
            ]
        );
    }

    let mut wrong_identity = shard.clone();
    wrong_identity.metadata.objective_identity = "00000000000000000000000000000000".into();
    assert!(
        NativeEpisodeActorView::build_for_goal(
            &wrong_identity,
            &catalog,
            &compiled.bytes,
            "spatial_goal"
        )
        .is_err()
    );
}

#[test]
fn goal_actor_resolution_preserves_ambiguous_and_stage_missingness() {
    const SOURCE: &str = r#"milestones 1.8
milestone actor_goal {
  phase post_sim
  when actor.placed.exists("F_SP103", 0, 4, 291)
}
"#;
    let compiled = compile_source(SOURCE).unwrap();
    let definition = &compiled.definitions[0];
    let mut shard = shard();
    shard.metadata.objective = definition.name.clone();
    shard.metadata.objective_identity = authored_milestone_objective_identity(
        &Digest(compiled.program_sha256).to_string(),
        &Digest(definition.sha256).to_string(),
    )
    .unwrap();
    let catalog = catalog_for(&shard);
    shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());

    let ambiguous =
        NativeEpisodeActorView::build_for_goal(&shard, &catalog, &compiled.bytes, "actor_goal")
            .unwrap();
    assert!(ambiguous.observations.iter().all(|observation| {
        observation.goal_anchors[0].status == NativeGoalAnchorStatus::ActorAmbiguous
            && observation.goal_anchors[0].absolute_position.is_none()
            && observation
                .actors
                .iter()
                .all(|actor| actor.goal_relative_positions == [None])
    }));

    for observation in shard.episodes.iter_mut().flat_map(|episode| {
        episode
            .steps
            .iter_mut()
            .flat_map(|step| [&mut step.pre_input, &mut step.post_simulation])
    }) {
        observation.stage = "F_SP104".into();
    }
    let wrong_stage =
        NativeEpisodeActorView::build_for_goal(&shard, &catalog, &compiled.bytes, "actor_goal")
            .unwrap();
    assert!(wrong_stage.observations.iter().all(|observation| {
        observation.goal_anchors[0].status == NativeGoalAnchorStatus::StageMismatch
            && observation.goal_anchors[0].absolute_position.is_none()
    }));
}

#[test]
fn rejects_the_wrong_catalog_and_noncanonical_or_tampered_views() {
    let mut shard = shard();
    let catalog = catalog_for(&shard);
    shard.metadata.actor_profile_catalog_identity =
        Some("actor-profile-catalog:xxh3-128:00000000000000000000000000000001".into());
    assert!(NativeEpisodeActorView::build(&shard, &catalog).is_err());

    shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
    let view = NativeEpisodeActorView::build(&shard, &catalog).unwrap();
    let mut bytes = view.canonical_bytes().unwrap();
    bytes.push(b'\n');
    assert!(NativeEpisodeActorView::decode_canonical(&bytes).is_err());
    let mut tampered = view;
    tampered.observations[0].actors[0].absolute_position[0] += 1.0;
    assert!(tampered.validate().is_err());
}
