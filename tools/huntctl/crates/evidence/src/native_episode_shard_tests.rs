use super::*;

const ORDON_PROGRAM_SHA256: &str =
    "b8cbfafaa025b883cecd2db4e4bef30696c801a591ce736d1281defd8af0c169";
const ORDON_DEFINITION_SHA256: &str =
    "631b025f41e16251e47f340fb0030fab07be15433204d2fdef8eb08915b11e57";

fn golden() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v2.dseps")
}

fn golden_v3() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v3.dseps")
}

fn golden_v4() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v4.dseps")
}

fn golden_v5() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v5.dseps")
}

fn golden_v6() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v6.dseps")
}

fn golden_v7() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v7.dseps")
}

fn golden_v8() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v8.dseps")
}

fn golden_v9() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v9.dseps")
}

fn golden_v10() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v10.dseps")
}

fn golden_v11() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v11.dseps")
}

fn golden_v12() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v12.dseps")
}

fn golden_v13() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v13.dseps")
}

fn golden_v14() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v14.dseps")
}

fn golden_v15() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v15.dseps")
}

fn golden_v16() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v16.dseps")
}

fn golden_v17() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v17.dseps")
}

fn golden_v18() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v18.dseps")
}

fn golden_v19() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v19.dseps")
}

fn golden_v20() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v20.dseps")
}

fn golden_v21() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v21.dseps")
}

fn golden_v22() -> &'static [u8] {
    include_bytes!("../../../../../tests/fixtures/automation/native_episode_v22.dseps")
}

#[test]
fn authored_objective_identity_binds_program_and_definition() {
    assert_eq!(
        authored_milestone_objective_identity(ORDON_PROGRAM_SHA256, ORDON_DEFINITION_SHA256)
            .unwrap(),
        "d0d98dc29bd4190312933ff7d10d9c11"
    );

    let mut changed_definition = ORDON_DEFINITION_SHA256.to_owned();
    changed_definition.replace_range(63..64, "8");
    assert_ne!(
        authored_milestone_objective_identity(ORDON_PROGRAM_SHA256, &changed_definition).unwrap(),
        "d0d98dc29bd4190312933ff7d10d9c11"
    );
    assert!(
        authored_milestone_objective_identity(
            &ORDON_PROGRAM_SHA256.to_uppercase(),
            ORDON_DEFINITION_SHA256
        )
        .is_err()
    );
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn mutate_first_episode(mutator: impl FnOnce(&mut [u8])) -> Vec<u8> {
    mutate_first_episode_in(golden(), mutator)
}

fn mutate_first_v3_episode(mutator: impl FnOnce(&mut [u8])) -> Vec<u8> {
    mutate_first_episode_in(golden_v3(), mutator)
}

fn mutate_first_v4_episode(mutator: impl FnOnce(&mut [u8])) -> Vec<u8> {
    mutate_first_episode_in(golden_v4(), mutator)
}

fn mutate_first_v7_episode(mutator: impl FnOnce(&mut [u8])) -> Vec<u8> {
    mutate_first_episode_in(golden_v7(), mutator)
}

fn mutate_first_v9_episode(mutator: impl FnOnce(&mut [u8])) -> Vec<u8> {
    mutate_first_episode_in(golden_v9(), mutator)
}

fn mutate_first_v13_episode(mutator: impl FnOnce(&mut [u8])) -> Vec<u8> {
    mutate_first_episode_in(golden_v13(), mutator)
}

fn mutate_first_v18_episode(mutator: impl FnOnce(&mut [u8])) -> Vec<u8> {
    mutate_first_episode_in(golden_v18(), mutator)
}

fn mutate_first_v19_episode(mutator: impl FnOnce(&mut [u8])) -> Vec<u8> {
    mutate_first_episode_in(golden_v19(), mutator)
}

fn mutate_first_v21_episode(mutator: impl FnOnce(&mut [u8])) -> Vec<u8> {
    mutate_first_episode_in(golden_v21(), mutator)
}

fn first_v21_process_records_offset(expanded: &[u8]) -> usize {
    const CREATE_RECORD_SIZE: usize = 32;
    const DELETE_RECORD_SIZE: usize = 28;
    let mut reader = Reader::new(expanded);
    reader.bytes(PAYLOAD_HEADER_SIZE).unwrap();
    let observation = decode_observation(&mut reader, OBSERVATION_VERSION_V21).unwrap();
    let lifecycle = observation.process_lifecycle.unwrap();
    reader.offset
        - lifecycle.pending_creates.len() * CREATE_RECORD_SIZE
        - lifecycle.pending_deletes.len() * DELETE_RECORD_SIZE
}

fn first_v18_event_queue_offset(expanded: &[u8]) -> usize {
    const ACTOR_IDENTITY_SIZE: usize = 25;
    const ACTOR_REFERENCE_SIZE: usize = 4 + ACTOR_IDENTITY_SIZE;
    const ORDER_HEADER_SIZE: usize = 12;
    const QUEUE_HEADER_SIZE: usize = 4;
    const PARTICIPANT_COUNT: usize = 7;
    let mut reader = Reader::new(expanded);
    reader.bytes(PAYLOAD_HEADER_SIZE).unwrap();
    let observation = decode_observation(&mut reader, OBSERVATION_VERSION_V18).unwrap();
    let order_count = observation
        .event_queue
        .as_ref()
        .unwrap()
        .pending_orders
        .len();
    let encoded_size = QUEUE_HEADER_SIZE
        + order_count * (ORDER_HEADER_SIZE + 2 * ACTOR_REFERENCE_SIZE)
        + PARTICIPANT_COUNT * ACTOR_REFERENCE_SIZE;
    reader.offset - encoded_size
}
fn mutate_first_episode_in(source: &[u8], mutator: impl FnOnce(&mut [u8])) -> Vec<u8> {
    let mut shard = source.to_vec();
    let payload_offset = read_u64(&shard, 56) as usize;
    let id_length = usize::from(read_u16(&shard, payload_offset + 20));
    let expanded_size = read_u64(&shard, payload_offset + 24) as usize;
    let old_compressed_size = read_u64(&shard, payload_offset + 32) as usize;
    let compressed_offset = payload_offset + BLOCK_HEADER_SIZE + id_length;
    let mut expanded = zstd::bulk::decompress(
        &shard[compressed_offset..compressed_offset + old_compressed_size],
        expanded_size,
    )
    .unwrap();
    mutator(&mut expanded);
    let compressed = zstd::bulk::compress(&expanded, 0).unwrap();
    let new_compressed_size = compressed.len();
    shard.splice(
        compressed_offset..compressed_offset + old_compressed_size,
        compressed,
    );
    write_u64(&mut shard, payload_offset + 32, new_compressed_size as u64);
    shard[payload_offset + 40..payload_offset + 56]
        .copy_from_slice(&xxhash_rust::xxh3::xxh3_128(&expanded).to_be_bytes());
    let delta = new_compressed_size as i64 - old_compressed_size as i64;
    write_u64(
        &mut shard,
        64,
        read_u64(source, 64).checked_add_signed(delta).unwrap(),
    );
    write_u64(
        &mut shard,
        80,
        read_u64(source, 80).checked_add_signed(delta).unwrap(),
    );
    shard
}

fn first_step_offsets(expanded: &[u8]) -> (usize, usize) {
    let mut reader = Reader::new(expanded);
    reader.bytes(8).unwrap();
    let observation_version = reader.u16().unwrap();
    reader.bytes(PAYLOAD_HEADER_SIZE - 10).unwrap();
    let pre_input = reader.offset;
    decode_observation(&mut reader, observation_version).unwrap();
    reader.bytes(24).unwrap();
    (pre_input, reader.offset)
}

#[test]
fn rejects_incomplete_header_before_allocating() {
    let mut bytes = vec![0; HEADER_SIZE];
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
    let error = NativeEpisodeShard::decode(&bytes).unwrap_err();
    assert!(error.to_string().contains("incomplete"));
}

#[test]
fn raw_pad_rejects_unknown_connection_flags() {
    let mut bytes = [0_u8; 12];
    bytes[10] = 2;
    assert!(decode_pad(&mut Reader::new(&bytes)).is_err());
}

#[test]
fn decodes_native_cpp_golden_shard_with_exact_phase_joins() {
    let shard = NativeEpisodeShard::decode(golden()).unwrap();
    assert_eq!(shard.source_frame, 440);
    assert_eq!(shard.maximum_ticks, 1);
    assert_eq!(shard.episodes.len(), 2);
    let episode = &shard.episodes[0];
    assert_eq!(episode.id, "failure-0");
    assert!(!episode.success);
    assert_eq!(episode.steps.len(), 1);
    let step = &episode.steps[0];
    assert_eq!(step.pre_input.phase, NativeObservationPhase::PreInput);
    assert_eq!(step.pre_input.terminal_reason, NativeTerminalReason::None);
    assert_eq!(
        step.post_simulation.phase,
        NativeObservationPhase::PostSimulation
    );
    assert_eq!(step.consumed_pad.buttons, 0x0100);
    assert_eq!(step.consumed_pad.stick_x, 100);
    assert_eq!(step.chosen_pad, step.consumed_pad);
    assert_eq!(
        step.post_simulation.terminal_reason,
        NativeTerminalReason::TickBudgetExhausted
    );
    assert_eq!(step.post_simulation.previous_input, step.consumed_pad);
    assert_eq!(step.pre_input.actors.len(), 1);
    assert_eq!(step.pre_input.actor_observed_count, 1);
    assert_eq!(step.pre_input.actors[0].parameters, 0x12345678);
    assert_eq!(step.pre_input.actors[0].velocity, [0.25, 0.0, 0.0]);
    assert_eq!(step.pre_input.event_flags.as_ref().unwrap()[3], 1);
    assert_eq!(
        step.pre_input.camera_status,
        NativeChannelStatus::NotSampled
    );
    assert!(step.pre_input.camera.is_none());
    assert!(step.pre_input.player_collision_surfaces.is_none());

    let success = &shard.episodes[1];
    assert_eq!(success.id, "success-0");
    assert!(success.success);
    assert_eq!(success.first_hit_tick, Some(0));
    assert_eq!(success.steps.len(), 1);
    assert_eq!(
        success.steps[0].post_simulation.terminal_reason,
        NativeTerminalReason::GoalReached
    );
    assert!(success.steps[0].post_simulation.goal.reached);
}

#[test]
fn decodes_v3_mechanics_and_collision_channels() {
    let shard = NativeEpisodeShard::decode(golden_v3()).unwrap();
    let observation = &shard.episodes[0].steps[0].pre_input;
    assert_eq!(observation.camera_status, NativeChannelStatus::Present);
    assert_eq!(observation.camera.as_ref().unwrap().view_yaw, 0x1200);
    assert_eq!(
        observation.player_action.as_ref().unwrap().procedure_id,
        0x42
    );
    assert_eq!(
        observation.scene_exit.as_ref().unwrap().destination_stage,
        "F_SP104"
    );
    assert!(observation.player_form_present);
    assert!(!observation.player_is_wolf);
    let background = observation.player_background_collision.as_ref().unwrap();
    assert_eq!(background.ground_identity, [1, 17, u32::MAX]);
    assert_eq!(background.ground_plane, [0.0, 1.0, 0.0, -2.0]);
    let surfaces = observation.player_collision_surfaces.as_ref().unwrap();
    assert_eq!(surfaces.identity_count, 1);
    assert_eq!(surfaces.surfaces[0].source_geometry_indices, [10, 11, 12]);
    assert_eq!(surfaces.surfaces[0].plane, Some([0.0, 1.0, 0.0, -2.0]));
}

#[test]
fn decodes_v4_complete_actor_contract() {
    let shard = NativeEpisodeShard::decode(golden_v4()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V4
    );
    assert_eq!(shard.episodes[0].steps[0].pre_input.actors.len(), 257);
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        assert_eq!(
            observation.actor_selection,
            NativeActorSelectionRule::Complete
        );
        assert!(!observation.actors_truncated);
        assert_eq!(
            observation.actor_observed_count as usize,
            observation.actors.len()
        );
    }
}

#[test]
fn decodes_v5_exact_temporary_event_register_bank() {
    let shard = NativeEpisodeShard::decode(golden_v5()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V5
    );
    assert_eq!(shard.metadata.shard_schema, NATIVE_EPISODE_SHARD_SCHEMA_V2);
    assert_eq!(shard.metadata.game_data_sha256, Some(Digest([0x11; 32])));
    assert_eq!(
        shard.metadata.card_fixture_identity.as_deref(),
        Some("card-fixture:xxh3-128:22222222222222222222222222222222")
    );
    assert_eq!(
        shard.metadata.actor_profile_catalog_identity.as_deref(),
        Some("actor-profile-catalog:xxh3-128:33333333333333333333333333333333")
    );
    assert_eq!(
        shard.metadata.world_context_sha256,
        Some(Digest([0x44; 32]))
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        let bytes = observation.temporary_event_bytes.as_ref().unwrap();
        assert_eq!(bytes.len(), 256);
        assert_eq!(bytes[0], 0x06);
        assert_eq!(bytes[1], 0xa5);
        assert_eq!(bytes[5], 0xc0);
    }
}

#[test]
fn decodes_v6_optional_actor_components() {
    let shard = NativeEpisodeShard::decode(golden_v6()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V6
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        let actor = &observation.actors[0];
        assert!(!actor.base_state_available);
        let attention = actor.attention.as_ref().unwrap();
        assert_eq!(attention.flags, 0x20000002);
        assert_eq!(attention.position, [11.0, 4.0, -7.0]);
        assert_eq!(attention.distance_indices, [1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(attention.auxiliary, -4);
        let event = actor.event_participation.as_ref().unwrap();
        assert_eq!(event.command, 1);
        assert_eq!(event.condition, 3);
        assert_eq!(event.event_id, 27);
        assert_eq!(event.map_tool_id, 8);
        assert_eq!(event.index, 2);
        let absent = observation.actors.last().unwrap();
        assert!(absent.attention.is_none());
        assert!(absent.event_participation.is_none());
    }
}

#[test]
fn decodes_v7_complete_actor_base_state() {
    let shard = NativeEpisodeShard::decode(golden_v7()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V7
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        let actor = &observation.actors[0];
        assert!(actor.base_state_available);
        assert_eq!(actor.actor_type, 5);
        assert_eq!(actor.process_subtype, 6);
        assert_eq!(actor.condition, 0x12);
        assert_eq!(actor.pause_flag, 4);
        assert_eq!(actor.process_init_state, -2);
        assert_eq!(actor.process_create_phase, 7);
        assert_eq!(actor.cull_type, 8);
        assert_eq!(actor.demo_actor_id, 9);
        assert_eq!(actor.carry_type, 10);
        assert_eq!(actor.old_room, 1);
        assert!(actor.heap_present);
        assert!(actor.model_present);
        assert!(actor.joint_collision_present);
        assert_eq!(actor.old_position, [12.0, 2.5, -8.5]);
        assert_eq!(actor.scale, [1.0, 2.0, 3.0]);
        assert_eq!(actor.gravity, -3.0);
        assert_eq!(actor.max_fall_speed, -20.0);
        assert_eq!(actor.eye_position, [12.5, 7.0, -8.0]);
        assert_eq!(actor.home_angle, [11, 12, 13]);
        assert_eq!(actor.old_angle, [14, 15, 16]);
    }
}

#[test]
fn rejects_noncanonical_v7_actor_base_state_header() {
    let shard = mutate_first_v7_episode(|expanded| {
        let header = [7, 0, 5, 0, 0, 0, 6, 0, 0, 0, 0x12, 0, 0, 0];
        let offset = expanded
            .windows(header.len())
            .position(|candidate| candidate == header)
            .expect("v7 actor base-state header");
        expanded[offset + 1] = 1;
    });
    assert!(
        NativeEpisodeShard::decode(&shard)
            .unwrap_err()
            .to_string()
            .contains("invalid actor base-state header")
    );
}

#[test]
fn decodes_v8_complete_dynamic_collision_set() {
    let shard = NativeEpisodeShard::decode(golden_v8()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V8
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        assert_eq!(
            observation.dynamic_colliders_status,
            NativeChannelStatus::Present
        );
        assert_eq!(observation.dynamic_colliders.len(), 1);
        let collider = &observation.dynamic_colliders[0];
        assert_eq!(collider.registration_index, 0);
        assert_eq!(collider.owner_runtime_generation, Some(7));
        assert_eq!(collider.attack_hit_owner_runtime_generation, Some(9));
        assert_eq!(collider.target_hit_owner_runtime_generation, None);
        assert!(collider.status_present);
        assert!(collider.shape_present);
        assert!(collider.attack_set && collider.target_set && collider.correction_set);
        assert!(collider.attack_hit && !collider.target_hit && !collider.correction_hit);
        assert_eq!(collider.shape, NativeDynamicColliderShape::Cylinder);
        assert_eq!(collider.attack_type, 0x20);
        assert_eq!(collider.target_type, 0xd8fbfdff);
        assert_eq!(collider.attack_source_parameters, 0x101);
        assert_eq!(collider.attack_result_parameters, 0x202);
        assert_eq!(collider.target_source_parameters, 0x303);
        assert_eq!(collider.target_result_parameters, 0x404);
        assert_eq!(collider.correction_source_parameters, 0x505);
        assert_eq!(collider.correction_result_parameters, 0x606);
        assert_eq!(collider.attack_power, 4);
        assert_eq!(collider.weight, 120);
        assert_eq!(collider.damage, 3);
        assert_eq!(collider.center, [12.5, 2.0, -8.0]);
        assert_eq!(collider.radius, 35.0);
        assert_eq!(collider.height, 80.0);
        assert_eq!(collider.aabb_min, [-22.5, 2.0, -43.0]);
        assert_eq!(collider.aabb_max, [47.5, 82.0, 27.0]);
        assert_eq!(collider.correction, [0.25, 0.0, -0.5]);
    }
}

#[test]
fn decodes_v9_typed_player_resources() {
    let shard = NativeEpisodeShard::decode(golden_v9()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V9
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        assert_eq!(
            observation.player_resources_status,
            NativeChannelStatus::Present
        );
        let resources = observation.player_resources.as_ref().unwrap();
        assert_eq!(resources.maximum_life, 20);
        assert_eq!(resources.life, 16);
        assert_eq!(resources.rupees, 123);
        assert_eq!(resources.rupee_capacity, 600);
        assert_eq!(resources.maximum_oil, 1200);
        assert_eq!(resources.oil, 875);
        assert_eq!(resources.maximum_magic, 32);
        assert_eq!(resources.magic, 17);
        assert_eq!(resources.world_time, 210.5);
        assert_eq!(resources.date, 3);
        assert_eq!(resources.arrows, 22);
        assert_eq!(resources.arrow_capacity, 30);
        assert_eq!(resources.inventory[1], 0x48);
        assert_eq!(resources.inventory[4], 0x43);
        assert_eq!(resources.selected_items, [1, 4, 0xff, 0xff]);
        assert_eq!(resources.bomb_counts, [12, 0, 0]);
        assert_eq!(resources.bomb_capacities, [30, 0, 0]);
        assert_eq!(resources.acquired_item_bits[8], 0x04);
        assert_eq!(resources.collect_item_bits[0], 0x03);
        assert!(resources.dungeon_map);
        assert!(!resources.dungeon_compass);
        assert!(resources.dungeon_boss_key);
        assert!(!resources.dungeon_warp);
    }
}

#[test]
fn decodes_v10_pointer_free_player_relationships() {
    let shard = NativeEpisodeShard::decode(golden_v10()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V10
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        assert_eq!(
            observation.player_relationships_status,
            NativeChannelStatus::Present
        );
        let relationships = observation.player_relationships.as_ref().unwrap();
        let target = relationships.targeted_actor.as_ref().unwrap();
        assert_eq!(target.runtime_generation, 7);
        assert_eq!(target.actor_name, 0x123);
        assert_eq!(target.set_id, 4);
        assert_eq!(target.home_position, Some([10.0, 2.0, -10.0]));
        assert!(relationships.ride_actor.is_none());
        assert!(relationships.held_item_actor.is_none());
        assert!(relationships.attention_look_actor.is_none());
        assert!(
            observation
                .actors
                .iter()
                .any(|actor| actor.runtime_generation == u64::from(target.runtime_generation))
        );
    }
}

#[test]
fn rejects_player_relationship_outside_complete_actor_population() {
    let shard = NativeEpisodeShard::decode(golden_v10()).unwrap();
    let observation = &shard.episodes[0].steps[0].pre_input;
    let mut relationships = observation.player_relationships.clone().unwrap();
    relationships
        .targeted_actor
        .as_mut()
        .unwrap()
        .runtime_generation = 999;
    assert!(
        validate_player_relationship_joins(&relationships, &observation.actors)
            .unwrap_err()
            .to_string()
            .contains("does not join the complete actor population")
    );
}

#[test]
fn decodes_v11_player_collision_solver_state() {
    let shard = NativeEpisodeShard::decode(golden_v11()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V11
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        assert_eq!(
            observation.player_collision_solver_status,
            NativeChannelStatus::Present
        );
        let solver = observation.player_collision_solver.as_ref().unwrap();
        assert_eq!(solver.flags, 0x2020);
        assert_eq!(solver.wall_table_size, 3);
        assert_eq!(solver.water_mode, 1);
        assert_eq!(solver.line_start, [-1.5, 2.0, 3.0]);
        assert_eq!(solver.wall_cylinder_radius, 35.0);
        assert_eq!(solver.ground_check_offset, 10.0);
        assert_eq!(solver.wall_circles[0].flags, 2);
        assert_eq!(solver.wall_circles[0].angle_y, 0x1200);
        assert_eq!(solver.wall_circles[0].realized_center, [-1.0, 37.0, 3.0]);
        assert_eq!(solver.wall_circles[0].realized_radius, 35.0);
    }
}

#[test]
fn decodes_v12_planner_runtime_channels_without_conflating_slots() {
    let shard = NativeEpisodeShard::decode(golden_v12()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V12
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        assert_eq!(
            observation.runtime_file_status,
            NativeChannelStatus::Present
        );
        assert_eq!(
            observation.player_relationships_status,
            NativeChannelStatus::Present
        );
        assert_eq!(
            observation.player_collision_solver_status,
            NativeChannelStatus::Present
        );
        let runtime = observation.runtime_file.as_ref().unwrap();
        assert_eq!(runtime.no_file_raw, 0);
        assert_eq!(runtime.data_num_raw, 1);
        assert_eq!(runtime.attached_physical_slot, Some(2));
        assert_eq!(runtime.physical_slots.map(|slot| slot.number), [1, 2, 3]);
        assert!(!runtime.physical_slots[0].attached_to_runtime);
        assert!(runtime.physical_slots[1].attached_to_runtime);
        assert_eq!(
            runtime.physical_slots[1].content_status,
            NativeChannelStatus::NotSampled
        );

        let return_place = observation.return_place.as_ref().unwrap();
        assert_eq!(return_place.stage, "F_SP103");
        assert_eq!(return_place.room, 0);
        assert_eq!(return_place.player_status, 2);
        let restart = observation.restart.as_ref().unwrap();
        assert_eq!(restart.room, 1);
        assert_eq!(restart.start_point, 3);
        assert_eq!(restart.position, [10.0, 20.0, 30.0]);
        assert_eq!(restart.room_param, 0x01020304);

        let handoff = observation.event_handoff.as_ref().unwrap();
        assert_eq!(handoff.pre_item_no, 0x48);
        assert_eq!(handoff.get_item_no, 0x43);
        assert_eq!(handoff.event_name.as_deref(), Some("DEFAULT_GETITEM"));
        assert!(handoff.item_partner.present);
        assert_eq!(handoff.item_partner.runtime_generation, 7);
        assert_eq!(
            handoff.message_flow_status,
            NativeChannelStatus::Unavailable
        );
        assert!(handoff.message_flow.is_none());
        assert_eq!(handoff.message_cut_status, NativeChannelStatus::NotSampled);
        assert_eq!(
            handoff.pending_cleanup_status,
            NativeChannelStatus::Unavailable
        );
        assert!(handoff.pending_cleanup_flags.is_none());
        assert_eq!(
            handoff.player_control,
            Some(NativePlayerControlObservation {
                mode_flags: 0x1234,
                do_status: 0x15,
            })
        );
        assert_eq!(handoff.no_telop, Some(true));
    }

    let legacy = NativeEpisodeShard::decode(golden_v9()).unwrap();
    let legacy_observation = &legacy.episodes[0].steps[0].pre_input;
    assert_eq!(
        legacy_observation.runtime_file_status,
        NativeChannelStatus::NotSampled
    );
    assert!(legacy_observation.runtime_file.is_none());
}

#[test]
fn decodes_v13_scoped_message_flow_without_inventing_a_cut() {
    let shard = NativeEpisodeShard::decode(golden_v13()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V13
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        let handoff = observation.event_handoff.as_ref().unwrap();
        assert_eq!(handoff.message_flow_status, NativeChannelStatus::Present);
        assert_eq!(
            handoff.message_flow,
            Some(NativeMessageFlowObservation {
                flow_id: 0x777,
                node_index: 0x12,
                cut_name_hash: 0,
            })
        );
        assert_eq!(handoff.message_cut_status, NativeChannelStatus::Unavailable);
    }
}

#[test]
fn decodes_v14_return_place_writer_configuration_and_guards() {
    let shard = NativeEpisodeShard::decode(golden_v14()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V14
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
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
}

#[test]
fn decodes_v15_typed_enemy_base_component() {
    let shard = NativeEpisodeShard::decode(golden_v15()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V15
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        assert!(observation.actors.iter().all(|actor| actor.group == 2));
        assert!(
            observation
                .actors
                .iter()
                .all(|actor| actor.enemy_base.is_some())
        );
        let enemy = observation.actors[0].enemy_base.as_ref().unwrap();
        assert_eq!(enemy.flags, 0x89);
        assert_eq!(enemy.throw_mode, 0x04);
        assert_eq!(enemy.down_position, [12.0, 3.5, -7.5]);
        assert_eq!(enemy.head_lock_position, [12.5, 7.0, -8.0]);
    }

    let legacy = NativeEpisodeShard::decode(golden_v14()).unwrap();
    assert!(
        legacy
            .episodes
            .iter()
            .all(|episode| episode.steps.iter().all(|step| {
                step.pre_input
                    .actors
                    .iter()
                    .chain(&step.post_simulation.actors)
                    .all(|actor| actor.enemy_base.is_none())
            }))
    );
}

#[test]
fn decodes_v16_global_message_session_without_an_npc_layout_cast() {
    let shard = NativeEpisodeShard::decode(golden_v16()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V16
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        assert_eq!(
            observation.message_session_status,
            NativeChannelStatus::Present
        );
        let message = observation.message_session.as_ref().unwrap();
        assert_eq!(message.procedure, 6);
        assert_eq!(message.message_id, 0x123456);
        assert_eq!(message.message_index, 17);
        assert_eq!(message.node_index, 9);
        assert_eq!(message.flow_id, 0x777);
        assert_eq!(message.selection_count, 3);
        assert_eq!(message.selection_cursor, 1);
        assert_eq!(message.selection_push, 2);
        assert_eq!(message.output_type, 4);
        assert!(message.talk_now);
        assert!(message.talk_message);
        assert!(message.send);
        assert!(!message.auto_message);
        assert!(!message.kill_pending);
        assert!(!message.camera_cancel);
        assert!(!message.send_control);
        assert_eq!(message.talk_actor.runtime_generation, 7);
        assert_eq!(message.talk_actor.actor_name, 0x123);
    }

    let legacy = NativeEpisodeShard::decode(golden_v15()).unwrap();
    assert!(
        legacy
            .episodes
            .iter()
            .all(|episode| episode.steps.iter().all(|step| {
                [&step.pre_input, &step.post_simulation]
                    .into_iter()
                    .all(|observation| {
                        observation.message_session_status == NativeChannelStatus::NotSampled
                            && observation.message_session.is_none()
                    })
            }))
    );
}

#[test]
fn decodes_v17_typed_trigger_volume_component() {
    let shard = NativeEpisodeShard::decode(golden_v17()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V17
    );
    for episode in &shard.episodes {
        for observation in episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
        {
            let trigger = observation.actors[1]
                .trigger_volume
                .as_ref()
                .expect("v17 trigger volume");
            assert_eq!(trigger.kind, NativeTriggerVolumeKind::SceneExit);
            assert_eq!(trigger.shape, NativeTriggerVolumeShape::Box);
            assert!(trigger.enabled);
            assert!(!trigger.vertical_unbounded);
            assert_eq!(trigger.behavior, 3);
            assert_eq!(trigger.center, [10.0, 20.0, -30.0]);
            assert_eq!(trigger.half_extent, [40.0, 50.0, 60.0]);
            assert_eq!(trigger.yaw, 0x1234);
        }
    }

    let mut legacy = NativeEpisodeShard::decode(golden_v16()).unwrap();
    for episode in &mut legacy.episodes {
        for step in &mut episode.steps {
            assert!(
                step.pre_input
                    .actors
                    .iter()
                    .all(|actor| actor.trigger_volume.is_none())
            );
            assert!(
                step.post_simulation
                    .actors
                    .iter()
                    .all(|actor| actor.trigger_volume.is_none())
            );
        }
    }
}

#[test]
fn decodes_v18_semantic_event_queue_and_pointer_free_participants() {
    let shard = NativeEpisodeShard::decode(golden_v18()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V18
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        assert_eq!(observation.event_queue_status, NativeChannelStatus::Present);
        let queue = observation.event_queue.as_ref().unwrap();
        assert_eq!(queue.pending_orders.len(), 2);
        assert_eq!(queue.pending_orders[0].event_type, 0);
        assert_eq!(queue.pending_orders[0].priority, 2);
        assert_eq!(queue.pending_orders[1].event_type, 2);
        assert_eq!(queue.pending_orders[1].priority, 5);
        assert_eq!(
            queue.pending_orders[0]
                .request_actor
                .actor
                .as_ref()
                .unwrap()
                .runtime_generation,
            7
        );
        assert_eq!(
            queue.pending_orders[0].target_actor.status,
            NativeChannelStatus::Absent
        );
        assert_eq!(
            queue.active_talk_actor.actor.as_ref().unwrap().actor_name,
            0x123
        );
        assert!(!queue.skip_registered);
        assert_eq!(queue.skip_actor.status, NativeChannelStatus::Absent);
    }

    let legacy = NativeEpisodeShard::decode(golden_v17()).unwrap();
    assert!(
        legacy
            .episodes
            .iter()
            .all(|episode| episode.steps.iter().all(|step| {
                [&step.pre_input, &step.post_simulation]
                    .into_iter()
                    .all(|observation| {
                        observation.event_queue_status == NativeChannelStatus::NotSampled
                            && observation.event_queue.is_none()
                    })
            }))
    );
}

#[test]
fn decodes_v19_process_lifecycle_pressure_with_legacy_missingness() {
    let shard = NativeEpisodeShard::decode(golden_v19()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V19
    );
    for episode in &shard.episodes {
        for observation in episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
        {
            assert_eq!(
                observation.process_lifecycle_status,
                NativeChannelStatus::Present
            );
            let lifecycle = observation
                .process_lifecycle
                .as_ref()
                .expect("v19 process lifecycle");
            assert_eq!(
                lifecycle.active_actor_count,
                observation.actors.len() as u32
            );
            assert_eq!(lifecycle.pending_create_count, 2);
            assert_eq!(lifecycle.pending_delete_count, 3);
            assert!(lifecycle.pending_creates.is_empty());
            assert!(lifecycle.pending_deletes.is_empty());
        }
    }

    let legacy = NativeEpisodeShard::decode(golden_v18()).unwrap();
    assert!(
        legacy
            .episodes
            .iter()
            .all(|episode| episode.steps.iter().all(|step| {
                [&step.pre_input, &step.post_simulation]
                    .into_iter()
                    .all(|observation| {
                        observation.process_lifecycle_status == NativeChannelStatus::NotSampled
                            && observation.process_lifecycle.is_none()
                    })
            }))
    );
}

#[test]
fn decodes_v20_pointer_free_attention_candidate_lists() {
    let shard = NativeEpisodeShard::decode(golden_v20()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V20
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        assert_eq!(
            observation.attention_candidates_status,
            NativeChannelStatus::Present
        );
        let attention = observation.attention_candidates.as_ref().unwrap();
        assert_eq!(attention.player_attention_flags, 0x1234);
        assert_eq!(attention.attention_status, 2);
        assert_eq!(attention.attention_block_timer, 3);
        assert_eq!(attention.lock_candidates.len(), 1);
        assert_eq!(attention.action_candidates.len(), 1);
        assert!(attention.check_candidates.is_empty());
        assert_eq!(attention.lock_candidates[0].weight, 0.25);
        assert_eq!(attention.lock_candidates[0].distance, 80.0);
        assert_eq!(attention.lock_candidates[0].angle, -0x100);
        assert_eq!(attention.lock_candidates[0].attention_type, 1);
        assert_eq!(
            attention.lock_candidates[0]
                .actor
                .actor
                .as_ref()
                .unwrap()
                .runtime_generation,
            7
        );
        assert_eq!(attention.action_candidates[0].attention_type, 6);
        let lifecycle = observation.process_lifecycle.as_ref().unwrap();
        assert_eq!(lifecycle.pending_create_count, 2);
        assert_eq!(lifecycle.pending_delete_count, 3);
        assert!(lifecycle.pending_creates.is_empty());
        assert!(lifecycle.pending_deletes.is_empty());
    }

    let legacy = NativeEpisodeShard::decode(golden_v19()).unwrap();
    assert!(legacy.episodes.iter().all(|episode| {
        episode.steps.iter().all(|step| {
            [&step.pre_input, &step.post_simulation]
                .into_iter()
                .all(|observation| {
                    observation.attention_candidates_status == NativeChannelStatus::NotSampled
                        && observation.attention_candidates.is_none()
                })
        })
    }));
}

#[test]
fn decodes_v22_generic_event_transition_state_with_legacy_missingness() {
    let shard = NativeEpisodeShard::decode(golden_v22()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V22
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        assert_eq!(
            observation.event_transition_status,
            NativeChannelStatus::Present
        );
        let transition = observation.event_transition.as_ref().unwrap();
        assert!(transition.event_data_loaded);
        assert_eq!(transition.camera_play, 2);
        assert_eq!(
            transition.current_event.as_ref().unwrap(),
            &NativeCurrentEventObservation {
                event_id: 0x123,
                event_type: 1,
                room: 0,
                goal: [10.0, 20.0, 30.0],
            }
        );
        assert_eq!(
            transition.pending_stage.as_ref().unwrap(),
            &NativePendingStageObservation {
                stage: "F_SP104".into(),
                room: 2,
                layer: 1,
                point: 3,
                wipe: 5,
                wipe_speed: 2,
            }
        );
    }

    let legacy = NativeEpisodeShard::decode(golden_v21()).unwrap();
    assert!(legacy.episodes.iter().all(|episode| {
        episode.steps.iter().all(|step| {
            [&step.pre_input, &step.post_simulation]
                .into_iter()
                .all(|observation| {
                    observation.event_transition_status == NativeChannelStatus::NotSampled
                        && observation.event_transition.is_none()
                })
        })
    }));
}

#[test]
fn rejects_nonsemantic_v18_event_queue_ordering_and_types() {
    let unknown_type = mutate_first_v18_episode(|expanded| {
        let queue = first_v18_event_queue_offset(expanded);
        expanded[queue + 4..queue + 6].copy_from_slice(&9_u16.to_le_bytes());
    });
    let error = NativeEpisodeShard::decode(&unknown_type)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown type or zero priority"), "{error}");

    let reversed_priority = mutate_first_v18_episode(|expanded| {
        let queue = first_v18_event_queue_offset(expanded);
        expanded[queue + 12..queue + 14].copy_from_slice(&6_u16.to_le_bytes());
    });
    let error = NativeEpisodeShard::decode(&reversed_priority)
        .unwrap_err()
        .to_string();
    assert!(error.contains("semantic priority order"), "{error}");
}

#[test]
fn decodes_v21_ordered_pending_process_records() {
    let shard = NativeEpisodeShard::decode(golden_v21()).unwrap();
    assert_eq!(
        shard.metadata.observation_schema,
        LEARNING_OBSERVATION_SCHEMA_V21
    );
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        let lifecycle = observation
            .process_lifecycle
            .as_ref()
            .expect("v21 process lifecycle");
        assert_eq!(lifecycle.pending_creates.len(), 2);
        assert_eq!(lifecycle.pending_deletes.len(), 3);
        assert_eq!(lifecycle.pending_creates[0].runtime_generation, 100);
        assert_eq!(
            lifecycle.pending_creates[0].process_status,
            NativeChannelStatus::Absent
        );
        assert!(lifecycle.pending_creates[0].process.is_none());
        assert_eq!(lifecycle.pending_creates[1].runtime_generation, 101);
        assert!(lifecycle.pending_creates[1].doing);
        assert_eq!(
            lifecycle.pending_creates[1]
                .process
                .as_ref()
                .expect("materialized process")
                .parameters,
            0x1020_3040
        );
        assert_eq!(lifecycle.pending_deletes[0].process.runtime_generation, 200);
        assert_eq!(lifecycle.pending_deletes[2].timer, 20);
    }
}

#[test]
fn rejects_v21_pending_create_presence_without_semantic_process() {
    let shard = mutate_first_v21_episode(|expanded| {
        let records = first_v21_process_records_offset(expanded);
        expanded[records + 5] = 1;
    });
    let error = NativeEpisodeShard::decode(&shard).unwrap_err().to_string();
    assert!(
        error.contains("pending-create process state is inconsistent"),
        "{error}"
    );
}

#[test]
fn rejects_v19_lifecycle_count_detached_from_complete_actor_set() {
    let shard = mutate_first_v19_episode(|expanded| {
        let lifecycle = [1, 0, 0, 0, 1, 1, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0];
        let offset = expanded
            .windows(lifecycle.len())
            .position(|candidate| candidate == lifecycle)
            .expect("v19 process lifecycle");
        expanded[offset + 4..offset + 8].copy_from_slice(&1_u32.to_le_bytes());
    });
    assert!(
        NativeEpisodeShard::decode(&shard)
            .unwrap_err()
            .to_string()
            .contains("process-lifecycle actor count disagrees")
    );
}

#[test]
fn rejects_v13_slot_attachment_claim_with_unavailable_backing() {
    let shard = mutate_first_v13_episode(|expanded| {
        let prefix = [1, 1, 0, 1, 2, 2, 0, 0, 0, 0, 1, 0, 2, 0, b'F'];
        let offset = expanded
            .windows(prefix.len())
            .position(|candidate| candidate == prefix)
            .expect("v13 player-resource/runtime boundary");
        expanded[offset + 1] = 3;
    });
    assert!(
        NativeEpisodeShard::decode(&shard)
            .unwrap_err()
            .to_string()
            .contains("unavailable planner backing has an attached slot")
    );
}

#[test]
fn rejects_player_resource_payload_when_channel_is_unavailable() {
    let shard = mutate_first_v9_episode(|expanded| {
        let prefix = [
            1, 0, 20, 0, 16, 0, 123, 0, 88, 2, 176, 4, 107, 3, 32, 17, 1, 0,
        ];
        let offset = expanded
            .windows(prefix.len())
            .position(|candidate| candidate == prefix)
            .expect("v9 player-resources header");
        expanded[offset] = 3;
    });
    assert!(
        NativeEpisodeShard::decode(&shard)
            .unwrap_err()
            .to_string()
            .contains("payload is present for an unavailable channel")
    );
}

#[test]
fn v4_rejects_an_explicitly_truncated_actor_subset() {
    let shard = mutate_first_v4_episode(|expanded| {
        let (pre_input, _) = first_step_offsets(expanded);
        expanded[pre_input + 1] = 1;
        expanded[pre_input + 6] |= 1 << 5;
        expanded[pre_input + 10..pre_input + 14].copy_from_slice(&258_u32.to_le_bytes());
    });
    assert!(
        NativeEpisodeShard::decode(&shard)
            .unwrap_err()
            .to_string()
            .contains("does not contain the complete actor set")
    );
}

#[test]
fn rejects_terminal_label_leakage_into_pre_input() {
    let shard = mutate_first_episode(|expanded| {
        let (pre_input, _) = first_step_offsets(expanded);
        expanded[pre_input + 2] = 1;
    });
    assert!(
        NativeEpisodeShard::decode(&shard)
            .unwrap_err()
            .to_string()
            .contains("action is not aligned")
    );
}

#[test]
fn rejects_noncanonical_v3_mechanics_header() {
    let shard = mutate_first_v3_episode(|expanded| {
        let (pre_input, _) = first_step_offsets(expanded);
        expanded[pre_input + 186] = 2;
    });
    assert!(
        NativeEpisodeShard::decode(&shard)
            .unwrap_err()
            .to_string()
            .contains("invalid mechanics observation header")
    );
}

#[test]
fn rejects_actor_completeness_that_masquerades_as_complete() {
    let shard = mutate_first_episode(|expanded| {
        let (pre_input, _) = first_step_offsets(expanded);
        expanded[pre_input + 10..pre_input + 14].copy_from_slice(&0_u32.to_le_bytes());
    });
    assert!(
        NativeEpisodeShard::decode(&shard)
            .unwrap_err()
            .to_string()
            .contains("inconsistent observation header")
    );
}

#[test]
fn rejects_post_phase_and_boundary_discontinuity() {
    let wrong_phase = mutate_first_episode(|expanded| {
        let (_, post_simulation) = first_step_offsets(expanded);
        expanded[post_simulation] = 1;
    });
    assert!(
        NativeEpisodeShard::decode(&wrong_phase)
            .unwrap_err()
            .to_string()
            .contains("action is not aligned")
    );

    let wrong_boundary = mutate_first_episode(|expanded| {
        let (_, post_simulation) = first_step_offsets(expanded);
        let boundary = read_u64(expanded, post_simulation + 18);
        write_u64(expanded, post_simulation + 18, boundary + 1);
    });
    assert!(
        NativeEpisodeShard::decode(&wrong_boundary)
            .unwrap_err()
            .to_string()
            .contains("action is not aligned")
    );
}

#[test]
fn rejects_episode_payload_corruption() {
    let mut shard = golden().to_vec();
    let payload_offset = read_u64(&shard, 56) as usize;
    let id_length = usize::from(read_u16(&shard, payload_offset + 20));
    let compressed_offset = payload_offset + BLOCK_HEADER_SIZE + id_length;
    shard[compressed_offset] ^= 0x40;
    assert!(NativeEpisodeShard::decode(&shard).is_err());
}

#[test]
fn decodes_requested_live_native_batch() {
    let Some(path) = std::env::var_os("DUSK_NATIVE_EPISODE_SHARD") else {
        return;
    };
    let shard = NativeEpisodeShard::read(path).expect("decode live native episode shard");
    assert!(!shard.episodes.is_empty());
    if let Some(expected) = std::env::var_os("DUSK_EXPECTED_GAME_DATA_SHA256") {
        let expected: Digest = expected
            .to_str()
            .expect("expected game-data SHA-256 is UTF-8")
            .parse()
            .expect("expected game-data SHA-256 is canonical");
        assert_eq!(
            shard.metadata.game_data_sha256,
            Some(expected),
            "live shard did not bind the authenticated game-data bytes"
        );
    }
    assert!(shard.episodes.iter().all(|episode| {
        episode.steps.len() == episode.ticks_executed as usize
            && episode.steps.iter().all(|step| {
                step.chosen_pad == step.consumed_pad
                    && (!matches!(
                        shard.metadata.observation_schema.as_str(),
                        LEARNING_OBSERVATION_SCHEMA_V5
                            | LEARNING_OBSERVATION_SCHEMA_V6
                            | LEARNING_OBSERVATION_SCHEMA_V7
                            | LEARNING_OBSERVATION_SCHEMA_V8
                            | LEARNING_OBSERVATION_SCHEMA_V9
                            | LEARNING_OBSERVATION_SCHEMA_V10
                            | LEARNING_OBSERVATION_SCHEMA_V11
                            | LEARNING_OBSERVATION_SCHEMA_V12
                            | LEARNING_OBSERVATION_SCHEMA_V13
                            | LEARNING_OBSERVATION_SCHEMA_V14
                            | LEARNING_OBSERVATION_SCHEMA_V15
                            | LEARNING_OBSERVATION_SCHEMA_V16
                            | LEARNING_OBSERVATION_SCHEMA_V17
                            | LEARNING_OBSERVATION_SCHEMA_V18
                            | LEARNING_OBSERVATION_SCHEMA_V19
                            | LEARNING_OBSERVATION_SCHEMA_V20
                            | LEARNING_OBSERVATION_SCHEMA_V21
                            | LEARNING_OBSERVATION_SCHEMA_V22
                    ) || [&step.pre_input, &step.post_simulation]
                        .iter()
                        .all(|observation| {
                            observation
                                .temporary_event_bytes
                                .as_ref()
                                .is_some_and(|bytes| bytes.len() == 256)
                        }))
            })
    }));
    let source_identity = shard.episodes[0].steps[0].pre_input.state_identity;
    assert!(
        shard
            .episodes
            .iter()
            .all(|episode| episode.steps[0].pre_input.state_identity == source_identity)
    );
    assert!(shard.episodes.iter().all(|episode| {
        episode.steps.last().is_some_and(|step| {
            step.post_simulation.terminal_reason
                == if episode.success {
                    NativeTerminalReason::GoalReached
                } else {
                    NativeTerminalReason::TickBudgetExhausted
                }
        })
    }));
    if matches!(
        shard.metadata.observation_schema.as_str(),
        LEARNING_OBSERVATION_SCHEMA_V3
            | LEARNING_OBSERVATION_SCHEMA_V4
            | LEARNING_OBSERVATION_SCHEMA_V5
            | LEARNING_OBSERVATION_SCHEMA_V6
            | LEARNING_OBSERVATION_SCHEMA_V7
            | LEARNING_OBSERVATION_SCHEMA_V8
            | LEARNING_OBSERVATION_SCHEMA_V9
            | LEARNING_OBSERVATION_SCHEMA_V10
            | LEARNING_OBSERVATION_SCHEMA_V11
            | LEARNING_OBSERVATION_SCHEMA_V12
            | LEARNING_OBSERVATION_SCHEMA_V13
            | LEARNING_OBSERVATION_SCHEMA_V14
            | LEARNING_OBSERVATION_SCHEMA_V15
            | LEARNING_OBSERVATION_SCHEMA_V16
            | LEARNING_OBSERVATION_SCHEMA_V17
            | LEARNING_OBSERVATION_SCHEMA_V18
            | LEARNING_OBSERVATION_SCHEMA_V19
            | LEARNING_OBSERVATION_SCHEMA_V20
            | LEARNING_OBSERVATION_SCHEMA_V21
            | LEARNING_OBSERVATION_SCHEMA_V22
    ) {
        let observations = shard.episodes.iter().flat_map(|episode| {
            episode
                .steps
                .iter()
                .flat_map(|step| [&step.pre_input, &step.post_simulation])
        });
        let observations: Vec<_> = observations.collect();
        assert!(observations.iter().all(|observation| {
            observation.camera_status == NativeChannelStatus::Present
                && observation.player_action_status == NativeChannelStatus::Present
                && observation.player_background_collision_status == NativeChannelStatus::Present
                && observation.player_collision_surfaces_status == NativeChannelStatus::Present
                && observation.scene_exit_status != NativeChannelStatus::NotSampled
                && observation.player_form_present
        }));
        assert!(observations.iter().any(|observation| {
            observation
                .player_collision_surfaces
                .as_ref()
                .is_some_and(|surfaces| {
                    surfaces
                        .surfaces
                        .iter()
                        .any(|surface| surface.plane.is_some())
                })
        }));
    }
}

#[test]
fn rejects_action_shift_and_terminal_label_leakage() {
    let bytes = include_bytes!("../../../../../tests/fixtures/automation/native_episode_v2.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let step = &shard.episodes[1].steps[0];

    let mut leaked_pre = step.pre_input.clone();
    leaked_pre.terminal_reason = NativeTerminalReason::GoalReached;
    assert!(
        validate_step(
            None,
            &leaked_pre,
            step.consumed_pad,
            &step.post_simulation,
            true,
            true,
        )
        .is_err()
    );

    let mut shifted_action = step.consumed_pad;
    shifted_action.buttons ^= 1;
    assert!(
        validate_step(
            None,
            &step.pre_input,
            shifted_action,
            &step.post_simulation,
            true,
            true,
        )
        .is_err()
    );

    let mut missing_terminal = step.post_simulation.clone();
    missing_terminal.terminal_reason = NativeTerminalReason::None;
    assert!(
        validate_step(
            None,
            &step.pre_input,
            step.consumed_pad,
            &missing_terminal,
            true,
            true,
        )
        .is_err()
    );
}
