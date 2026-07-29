use super::*;

#[test]
fn reports_outcomes_duplicates_sets_and_channel_coverage() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v4.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let report = inspect_native_episode_corpus(&[shard.clone(), shard]);
    assert_eq!(report.shard_count, 2);
    assert_eq!(report.episode_count, 4);
    assert_eq!(report.success_count, 2);
    assert_eq!(report.failure_count, 2);
    assert_eq!(report.transition_count, 4);
    assert_eq!(report.observation_count, 8);
    assert_eq!(report.player_trajectories.len(), 4);
    assert!(report.player_trajectories.iter().all(|trajectory| {
        trajectory.ticks_executed > 0
            && trajectory.planar_path_length >= trajectory.planar_displacement
            && (0.0..=1.0).contains(&trajectory.planar_straightness)
            && trajectory.commanded_stall_ticks <= trajectory.commanded_motion_ticks
            && trajectory.collision_correction_maximum <= trajectory.collision_correction_total
    }));
    assert_eq!(report.truncated_actor_observations, 0);
    assert_eq!(report.actor_set_sizes.minimum, 257);
    assert_eq!(report.actor_set_sizes.maximum, 257);
    assert_eq!(report.unique_actor_types, 1);
    assert_eq!(report.duplicate_trajectory_groups.len(), 2);
    assert!(
        report
            .duplicate_trajectory_groups
            .iter()
            .all(|group| group.copies == 2)
    );
    assert_eq!(report.determinism_conflicts.len(), 1);
    assert_eq!(report.determinism_conflicts[0].copies, 4);
    assert_eq!(report.determinism_conflicts[0].distinct_payloads, 2);
    assert_eq!(report.channel_coverage["camera"].present, 8);
    assert_eq!(report.missing_mask_counts["event_flags"], 0);
    assert_eq!(report.flag_mask_coverage["event_flags"].widths.minimum, 822);
    assert_eq!(report.rng_stream_set_sizes.minimum, 2);
    assert_eq!(report.validated_non_finite_values, 0);
    assert_eq!(report.validated_phase_discontinuities, 0);
}

#[test]
fn flags_identity_fields_that_separate_success_from_failure() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v4.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let mut success = shard.clone();
    success.episodes.retain(|episode| episode.success);
    success.metadata.checkpoint_identity = "11111111111111111111111111111111".into();
    let mut failure = shard;
    failure.episodes.retain(|episode| !episode.success);
    failure.metadata.checkpoint_identity = "22222222222222222222222222222222".into();

    let report = inspect_native_episode_corpus(&[success, failure]);
    assert!(
        report
            .identities
            .outcome_separating_candidates
            .iter()
            .any(|field| field == "checkpoint_identity")
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("leakage ablations"))
    );
    assert!(report.determinism_conflicts.is_empty());
}

#[test]
fn audits_v5_temporary_event_register_coverage() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v5.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let report = inspect_native_episode_corpus(&[shard]);
    assert_eq!(report.missing_mask_counts["temporary_event_bytes"], 0);
    let coverage = &report.flag_mask_coverage["temporary_event_bytes"];
    assert_eq!(coverage.present, report.observation_count);
    assert_eq!(coverage.widths.minimum, 256);
    assert_eq!(coverage.widths.maximum, 256);
    assert!(coverage.ever_set_indices.contains(&0));
    assert!(coverage.ever_set_indices.contains(&1));
    assert!(coverage.ever_set_indices.contains(&5));
}

#[test]
fn audits_v9_player_resource_and_inventory_coverage() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v9.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let report = inspect_native_episode_corpus(&[shard]);
    assert_eq!(report.schema, NATIVE_CORPUS_INSPECTION_SCHEMA_V19);
    assert_eq!(
        report.channel_coverage["player_resources"].present,
        report.observation_count
    );
    assert_eq!(report.missing_mask_counts["acquired_item_bits"], 0);
    assert_eq!(report.missing_mask_counts["collect_item_bits"], 0);
    assert_eq!(
        report.flag_mask_coverage["acquired_item_bits"]
            .widths
            .minimum,
        32
    );
    assert_eq!(
        report.flag_mask_coverage["collect_item_bits"]
            .widths
            .minimum,
        8
    );
}

#[test]
fn audits_v10_player_relationship_role_coverage() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v10.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let report = inspect_native_episode_corpus(&[shard]);
    assert_eq!(report.schema, NATIVE_CORPUS_INSPECTION_SCHEMA_V19);
    assert_eq!(
        report.channel_coverage["player_relationships"].present,
        report.observation_count
    );
    assert_eq!(
        report.player_relationship_role_presence["targeted_actor"],
        report.observation_count
    );
    assert_eq!(report.player_relationship_role_presence["ride_actor"], 0);
}

#[test]
fn audits_v11_player_collision_solver_coverage() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v11.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let report = inspect_native_episode_corpus(&[shard]);
    assert_eq!(
        report.channel_coverage["player_collision_solver"].present,
        report.observation_count
    );
    assert_eq!(
        report.missing_mask_counts["player_collision_solver_flags"],
        0
    );
    assert_eq!(
        report.flag_mask_coverage["player_collision_solver_flags"]
            .widths
            .minimum,
        4
    );
    assert_eq!(
        report.flag_mask_coverage["player_collision_solver_wall_flags"]
            .widths
            .minimum,
        12
    );
}

#[test]
fn audits_v18_event_queue_coverage() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v18.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let report = inspect_native_episode_corpus(&[shard]);
    assert_eq!(
        report.channel_coverage["event_queue"].present,
        report.observation_count
    );
}

#[test]
fn audits_v19_process_lifecycle_coverage() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v19.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let report = inspect_native_episode_corpus(&[shard]);
    assert_eq!(
        report.channel_coverage["process_lifecycle"].present,
        report.observation_count
    );
    assert_eq!(
        report
            .process_lifecycle_record_coverage
            .count_only_observations,
        report.observation_count
    );
    assert_eq!(
        report
            .process_lifecycle_record_coverage
            .detailed_observations,
        0
    );
}

#[test]
fn audits_v21_process_lifecycle_record_coverage() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v21.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let report = inspect_native_episode_corpus(&[shard]);
    let coverage = &report.process_lifecycle_record_coverage;
    assert_eq!(coverage.detailed_observations, report.observation_count);
    assert_eq!(coverage.count_only_observations, 0);
    assert_eq!(coverage.pending_create_sizes.minimum, 2);
    assert_eq!(coverage.pending_create_sizes.maximum, 2);
    assert_eq!(coverage.pending_delete_sizes.minimum, 3);
    assert_eq!(
        coverage.materialized_create_processes,
        report.observation_count
    );
    assert_eq!(
        coverage.unmaterialized_create_requests,
        report.observation_count
    );
    assert_eq!(coverage.doing_create_requests, report.observation_count);
    assert_eq!(
        coverage.pending_delete_records,
        report.observation_count * 3
    );
    assert_eq!(coverage.unique_process_kinds, 4);
}

#[test]
fn audits_v20_attention_candidate_coverage() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v20.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let report = inspect_native_episode_corpus(&[shard]);
    assert_eq!(
        report.channel_coverage["attention_candidates"].present,
        report.observation_count
    );
}

#[test]
fn audits_v22_event_transition_coverage() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v22.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let report = inspect_native_episode_corpus(&[shard]);
    assert_eq!(
        report.channel_coverage["event_transition"].present,
        report.observation_count
    );
}

#[test]
fn audits_v23_clock_domain_coverage() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v23.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let report = inspect_native_episode_corpus(&[shard]);
    assert_eq!(
        report.channel_coverage["clock_domains"].present,
        report.observation_count
    );
}

#[test]
fn audits_v24_room_load_coverage() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v24.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let report = inspect_native_episode_corpus(&[shard]);
    assert_eq!(
        report.channel_coverage["room_load"].present,
        report.observation_count
    );
}

#[test]
fn audits_v25_warp_session_coverage() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v25.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let report = inspect_native_episode_corpus(&[shard]);
    assert_eq!(
        report.channel_coverage["warp_session"].present,
        report.observation_count
    );
}

#[test]
fn audits_v26_resource_load_outcomes_and_capacity() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v26.dseps");
    let shard = NativeEpisodeShard::decode(bytes).unwrap();
    let report = inspect_native_episode_corpus(&[shard]);
    assert_eq!(
        report.channel_coverage["resource_loads"].present,
        report.observation_count
    );
    assert_eq!(report.resource_load_coverage.present_observations, 4);
    assert_eq!(report.resource_load_coverage.object_entries, 8);
    assert_eq!(report.resource_load_coverage.stage_entries, 4);
    assert_eq!(report.resource_load_coverage.maximum_object_occupancy, 2);
    assert_eq!(report.resource_load_coverage.maximum_stage_occupancy, 1);
    assert_eq!(report.resource_load_coverage.mounting_entries, 4);
    assert_eq!(report.resource_load_coverage.ready_entries, 4);
    assert_eq!(report.resource_load_coverage.failed_entries, 4);
    assert_eq!(report.resource_load_coverage.unique_archives, 3);
}

#[test]
fn audits_v27_door20_typed_changes_without_nominating_a_door() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v27.dseps");
    let mut shard = NativeEpisodeShard::decode(bytes).unwrap();
    shard.episodes.truncate(1);
    shard.episodes[0].steps.truncate(1);
    let step = &mut shard.episodes[0].steps[0];
    let door = step
        .post_simulation
        .actors
        .iter_mut()
        .find(|actor| actor.door20.is_some())
        .unwrap();
    let profile_name = door.profile_name;
    let door = door.door20.as_mut().unwrap();
    door.action = crate::native_episode_shard::NativeDoor20Action::Wait;
    door.locked = false;
    door.door_angle += 1;

    let report = inspect_native_episode_corpus(&[shard]);
    let profile = report
        .actor_temporal_coverage
        .profiles
        .iter()
        .find(|profile| profile.profile_name == profile_name)
        .unwrap();
    assert_eq!(profile.changed_fields["door20.action"], 1);
    assert_eq!(profile.changed_fields["door20.locked"], 1);
    assert_eq!(profile.changed_fields["door20.open_close"], 1);
    assert!(!profile.changed_fields.contains_key("door20.authored"));
}

#[test]
fn audits_actor_lifecycles_and_typed_changes_without_raw_values() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v14.dseps");
    let mut shard = NativeEpisodeShard::decode(bytes).unwrap();
    shard.episodes.truncate(1);
    shard.episodes[0].steps.truncate(1);
    let step = &mut shard.episodes[0].steps[0];
    let persistent = step.pre_input.actors[0].clone();

    let mut disappeared = persistent.clone();
    disappeared.runtime_generation += 10_000;
    step.pre_input.actors.push(disappeared);
    step.pre_input
        .actors
        .sort_by_key(|actor| actor.runtime_generation);

    let after_persistent = step
        .post_simulation
        .actors
        .iter_mut()
        .find(|actor| actor.runtime_generation == persistent.runtime_generation)
        .unwrap();
    after_persistent.position[0] += 1.0;
    after_persistent.velocity[2] += 2.0;

    let mut appeared = persistent.clone();
    appeared.runtime_generation += 20_000;
    appeared.profile_name += 1;
    appeared.actor_name += 1;
    step.post_simulation.actors.push(appeared.clone());
    step.post_simulation
        .actors
        .sort_by_key(|actor| actor.runtime_generation);

    let report = inspect_native_episode_corpus(&[shard]);
    let temporal = &report.actor_temporal_coverage;
    assert_eq!(temporal.boundary_count, 2);
    assert_eq!(temporal.compared_transition_count, 1);
    assert_eq!(temporal.in_context_appearances, 1);
    assert_eq!(temporal.in_context_disappearances, 1);
    assert_eq!(temporal.context_change_appearances, 0);
    assert_eq!(temporal.context_change_disappearances, 0);
    assert!(temporal.identity_conflicts.is_empty());

    let persistent_profile = temporal
        .profiles
        .iter()
        .find(|profile| profile.profile_name == persistent.profile_name)
        .unwrap();
    assert_eq!(persistent_profile.changed_fields["position"], 1);
    assert_eq!(persistent_profile.changed_fields["velocity"], 1);
    assert_eq!(persistent_profile.in_context_disappearances, 1);
    assert!(!persistent_profile.changed_fields.contains_key("health"));

    let appeared_profile = temporal
        .profiles
        .iter()
        .find(|profile| profile.profile_name == appeared.profile_name)
        .unwrap();
    assert_eq!(appeared_profile.in_context_appearances, 1);
    assert_eq!(appeared_profile.boundary_samples, 1);
}

#[test]
fn separates_context_teardown_from_in_context_actor_lifecycle() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v14.dseps");
    let mut shard = NativeEpisodeShard::decode(bytes).unwrap();
    shard.episodes.truncate(1);
    shard.episodes[0].steps.truncate(1);
    let step = &mut shard.episodes[0].steps[0];
    let removed = step.pre_input.actors[0].runtime_generation;
    step.post_simulation.room = step.pre_input.room.wrapping_add(1);
    step.post_simulation
        .actors
        .retain(|actor| actor.runtime_generation != removed);
    let mut appeared = step.pre_input.actors[0].clone();
    appeared.runtime_generation += 30_000;
    step.post_simulation.actors.push(appeared);
    step.post_simulation
        .actors
        .sort_by_key(|actor| actor.runtime_generation);

    let report = inspect_native_episode_corpus(&[shard]);
    let temporal = report.actor_temporal_coverage;
    assert_eq!(temporal.in_context_appearances, 0);
    assert_eq!(temporal.in_context_disappearances, 0);
    assert_eq!(temporal.context_change_appearances, 1);
    assert_eq!(temporal.context_change_disappearances, 1);
}

#[test]
fn flags_a_runtime_generation_that_reappears_after_omission() {
    let bytes =
        include_bytes!("../../../../../../tests/fixtures/automation/native_episode_v14.dseps");
    let mut shard = NativeEpisodeShard::decode(bytes).unwrap();
    shard.episodes.truncate(1);
    let second_step = shard.episodes[0].steps[0].clone();
    shard.episodes[0].steps.push(second_step);
    let generation = shard.episodes[0].steps[0].pre_input.actors[0].runtime_generation;
    let actor = shard.episodes[0].steps[0].pre_input.actors[0].clone();
    shard.episodes[0].steps[0]
        .post_simulation
        .actors
        .retain(|candidate| candidate.runtime_generation != generation);
    if !shard.episodes[0].steps[1]
        .post_simulation
        .actors
        .iter()
        .any(|candidate| candidate.runtime_generation == generation)
    {
        shard.episodes[0].steps[1]
            .post_simulation
            .actors
            .push(actor);
        shard.episodes[0].steps[1]
            .post_simulation
            .actors
            .sort_by_key(|candidate| candidate.runtime_generation);
    }

    let report = inspect_native_episode_corpus(&[shard]);
    assert_eq!(
        report
            .actor_temporal_coverage
            .runtime_generation_reappearances,
        1
    );
    assert!(report.warnings.iter().any(|warning| {
        warning.contains("actor runtime-generation identity is inconsistent within an episode")
    }));
}
