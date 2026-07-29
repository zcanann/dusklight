use super::*;

pub(super) fn record_changed_field(
    profile: &mut ActorTemporalProfileAccumulator,
    name: &'static str,
    changed: bool,
) {
    if changed {
        *profile.changed_fields.entry(name.into()).or_default() += 1;
    }
}

pub(super) fn float_changed(left: f32, right: f32) -> bool {
    left.to_bits() != right.to_bits()
}

pub(super) fn float_array_changed<const N: usize>(left: [f32; N], right: [f32; N]) -> bool {
    left.iter()
        .zip(right)
        .any(|(left, right)| float_changed(*left, right))
}

pub(super) fn record_persistent_actor_changes(
    profile: &mut ActorTemporalProfileAccumulator,
    before: &NativeActorObservation,
    after: &NativeActorObservation,
) {
    record_changed_field(profile, "actor_name", before.actor_name != after.actor_name);
    record_changed_field(
        profile,
        "profile_name",
        before.profile_name != after.profile_name,
    );
    record_changed_field(profile, "set_id", before.set_id != after.set_id);
    record_changed_field(profile, "home_room", before.home_room != after.home_room);
    record_changed_field(
        profile,
        "current_room",
        before.current_room != after.current_room,
    );
    record_changed_field(profile, "health", before.health != after.health);
    record_changed_field(profile, "status", before.status != after.status);
    record_changed_field(
        profile,
        "position",
        float_array_changed(before.position, after.position),
    );
    record_changed_field(
        profile,
        "current_angle",
        before.current_angle != after.current_angle,
    );
    record_changed_field(
        profile,
        "shape_angle",
        before.shape_angle != after.shape_angle,
    );
    record_changed_field(
        profile,
        "base_state_available",
        before.base_state_available != after.base_state_available,
    );

    if before.base_state_available && after.base_state_available {
        record_changed_field(profile, "actor_type", before.actor_type != after.actor_type);
        record_changed_field(
            profile,
            "process_subtype",
            before.process_subtype != after.process_subtype,
        );
        record_changed_field(
            profile,
            "parent_runtime_generation",
            before.parent_runtime_generation != after.parent_runtime_generation,
        );
        record_changed_field(profile, "parameters", before.parameters != after.parameters);
        record_changed_field(profile, "condition", before.condition != after.condition);
        record_changed_field(profile, "old_room", before.old_room != after.old_room);
        record_changed_field(profile, "group", before.group != after.group);
        record_changed_field(profile, "argument", before.argument != after.argument);
        record_changed_field(profile, "pause_flag", before.pause_flag != after.pause_flag);
        record_changed_field(
            profile,
            "process_init_state",
            before.process_init_state != after.process_init_state,
        );
        record_changed_field(
            profile,
            "process_create_phase",
            before.process_create_phase != after.process_create_phase,
        );
        record_changed_field(profile, "cull_type", before.cull_type != after.cull_type);
        record_changed_field(
            profile,
            "demo_actor_id",
            before.demo_actor_id != after.demo_actor_id,
        );
        record_changed_field(profile, "carry_type", before.carry_type != after.carry_type);
        record_changed_field(
            profile,
            "heap_present",
            before.heap_present != after.heap_present,
        );
        record_changed_field(
            profile,
            "model_present",
            before.model_present != after.model_present,
        );
        record_changed_field(
            profile,
            "joint_collision_present",
            before.joint_collision_present != after.joint_collision_present,
        );
        record_changed_field(
            profile,
            "home_position",
            float_array_changed(before.home_position, after.home_position),
        );
        record_changed_field(
            profile,
            "old_position",
            float_array_changed(before.old_position, after.old_position),
        );
        record_changed_field(
            profile,
            "velocity",
            float_array_changed(before.velocity, after.velocity),
        );
        record_changed_field(
            profile,
            "forward_speed",
            float_changed(before.forward_speed, after.forward_speed),
        );
        record_changed_field(
            profile,
            "scale",
            float_array_changed(before.scale, after.scale),
        );
        record_changed_field(
            profile,
            "gravity",
            float_changed(before.gravity, after.gravity),
        );
        record_changed_field(
            profile,
            "max_fall_speed",
            float_changed(before.max_fall_speed, after.max_fall_speed),
        );
        record_changed_field(
            profile,
            "eye_position",
            float_array_changed(before.eye_position, after.eye_position),
        );
        record_changed_field(profile, "home_angle", before.home_angle != after.home_angle);
        record_changed_field(profile, "old_angle", before.old_angle != after.old_angle);
    }

    record_changed_field(
        profile,
        "attention.present",
        before.attention.is_some() != after.attention.is_some(),
    );
    if let (Some(before), Some(after)) = (&before.attention, &after.attention) {
        record_changed_field(profile, "attention.flags", before.flags != after.flags);
        record_changed_field(
            profile,
            "attention.position",
            float_array_changed(before.position, after.position),
        );
        record_changed_field(
            profile,
            "attention.distance_indices",
            before.distance_indices != after.distance_indices,
        );
        record_changed_field(
            profile,
            "attention.auxiliary",
            before.auxiliary != after.auxiliary,
        );
    }
    record_changed_field(
        profile,
        "event_participation.present",
        before.event_participation.is_some() != after.event_participation.is_some(),
    );
    if let (Some(before), Some(after)) = (&before.event_participation, &after.event_participation) {
        record_changed_field(
            profile,
            "event_participation.command",
            before.command != after.command,
        );
        record_changed_field(
            profile,
            "event_participation.condition",
            before.condition != after.condition,
        );
        record_changed_field(
            profile,
            "event_participation.event_id",
            before.event_id != after.event_id,
        );
        record_changed_field(
            profile,
            "event_participation.map_tool_id",
            before.map_tool_id != after.map_tool_id,
        );
        record_changed_field(
            profile,
            "event_participation.index",
            before.index != after.index,
        );
    }
    record_changed_field(
        profile,
        "return_place_writer",
        before.return_place_writer != after.return_place_writer,
    );
    record_changed_field(
        profile,
        "enemy_base.present",
        before.enemy_base.is_some() != after.enemy_base.is_some(),
    );
    if let (Some(before), Some(after)) = (&before.enemy_base, &after.enemy_base) {
        record_changed_field(profile, "enemy_base.flags", before.flags != after.flags);
        record_changed_field(
            profile,
            "enemy_base.throw_mode",
            before.throw_mode != after.throw_mode,
        );
        record_changed_field(
            profile,
            "enemy_base.down_position",
            float_array_changed(before.down_position, after.down_position),
        );
        record_changed_field(
            profile,
            "enemy_base.head_lock_position",
            float_array_changed(before.head_lock_position, after.head_lock_position),
        );
    }
    record_changed_field(
        profile,
        "trigger_volume.present",
        before.trigger_volume.is_some() != after.trigger_volume.is_some(),
    );
    if let (Some(before), Some(after)) = (&before.trigger_volume, &after.trigger_volume) {
        record_changed_field(profile, "trigger_volume.kind", before.kind != after.kind);
        record_changed_field(profile, "trigger_volume.shape", before.shape != after.shape);
        record_changed_field(
            profile,
            "trigger_volume.enabled",
            before.enabled != after.enabled,
        );
        record_changed_field(
            profile,
            "trigger_volume.vertical_unbounded",
            before.vertical_unbounded != after.vertical_unbounded,
        );
        record_changed_field(
            profile,
            "trigger_volume.behavior",
            before.behavior != after.behavior,
        );
        record_changed_field(
            profile,
            "trigger_volume.center",
            float_array_changed(before.center, after.center),
        );
        record_changed_field(
            profile,
            "trigger_volume.half_extent",
            float_array_changed(before.half_extent, after.half_extent),
        );
        record_changed_field(profile, "trigger_volume.yaw", before.yaw != after.yaw);
    }
    record_changed_field(
        profile,
        "door20.present",
        before.door20.is_some() != after.door20.is_some(),
    );
    if let (Some(before), Some(after)) = (&before.door20, &after.door20) {
        record_changed_field(
            profile,
            "door20.authored",
            before.kind != after.kind
                || before.door_model != after.door_model
                || before.front_option != after.front_option
                || before.back_option != after.back_option
                || before.front_room != after.front_room
                || before.back_room != after.back_room
                || before.exit_number != after.exit_number
                || before.message_door != after.message_door
                || before.front_switch != after.front_switch
                || before.back_switch != after.back_switch
                || before.unlock_effect_switch != after.unlock_effect_switch
                || before.front_event != after.front_event
                || before.back_event != after.back_event
                || before.message_number != after.message_number,
        );
        record_changed_field(profile, "door20.action", before.action != after.action);
        record_changed_field(
            profile,
            "door20.active_side",
            before.active_side != after.active_side,
        );
        record_changed_field(
            profile,
            "door20.event_variant",
            before.event_variant != after.event_variant,
        );
        record_changed_field(
            profile,
            "door20.switch_values",
            before.front_switch_set != after.front_switch_set
                || before.back_switch_set != after.back_switch_set
                || before.unlock_effect_switch_set != after.unlock_effect_switch_set,
        );
        record_changed_field(profile, "door20.locked", before.locked != after.locked);
        record_changed_field(
            profile,
            "door20.background_collision_released",
            before.background_collision_released != after.background_collision_released,
        );
        record_changed_field(
            profile,
            "door20.unlock_effect_triggered",
            before.unlock_effect_triggered != after.unlock_effect_triggered,
        );
        record_changed_field(
            profile,
            "door20.enemy_clear_debounce",
            before.enemy_clear_debounce != after.enemy_clear_debounce,
        );
        record_changed_field(
            profile,
            "door20.open_close",
            before.opening_active != after.opening_active
                || before.closing_active != after.closing_active
                || before.door_angle != after.door_angle,
        );
        record_changed_field(
            profile,
            "door20.stoppers",
            before.stopper_side != after.stopper_side
                || before.front_stopper_status != after.front_stopper_status
                || before.back_stopper_status != after.back_stopper_status,
        );
    }
}

pub(super) fn record_actor_temporal_episode(
    accumulator: &mut ActorTemporalAccumulator,
    episode: &NativeEpisode,
) {
    let mut boundaries = Vec::with_capacity(episode.steps.len() + 1);
    boundaries.push(&episode.steps[0].pre_input);
    boundaries.extend(episode.steps.iter().map(|step| &step.post_simulation));
    accumulator.boundary_count += boundaries.len() as u64;

    let mut episode_lifetimes = BTreeSet::new();
    let mut seen_generations = BTreeSet::new();
    let mut previous_generations = BTreeSet::new();
    for (boundary_index, observation) in boundaries.iter().enumerate() {
        accumulator.actor_boundary_samples += observation.actors.len() as u64;
        let current_generations = observation
            .actors
            .iter()
            .map(|actor| actor.runtime_generation)
            .collect::<BTreeSet<_>>();
        if boundary_index != 0 {
            accumulator.runtime_generation_reappearances += current_generations
                .iter()
                .filter(|generation| {
                    !previous_generations.contains(*generation)
                        && seen_generations.contains(*generation)
                })
                .count() as u64;
        }
        seen_generations.extend(current_generations.iter().copied());
        previous_generations = current_generations;
        for actor in &observation.actors {
            episode_lifetimes.insert((actor.profile_name, actor.runtime_generation));
            let profile = accumulator.profiles.entry(actor.profile_name).or_default();
            profile.actor_names.insert(actor.actor_name);
            profile.stages.insert(observation.stage.clone());
            profile.boundary_samples += 1;
        }
    }
    accumulator.episode_local_lifetimes += episode_lifetimes.len() as u64;
    for (profile_name, _) in episode_lifetimes {
        accumulator
            .profiles
            .entry(profile_name)
            .or_default()
            .episode_local_lifetimes += 1;
    }

    for pair in boundaries.windows(2) {
        let before = pair[0];
        let after = pair[1];
        accumulator.compared_transition_count += 1;
        let same_context =
            before.stage == after.stage && before.room == after.room && before.layer == after.layer;
        let before_by_id = before
            .actors
            .iter()
            .map(|actor| (actor.runtime_generation, actor))
            .collect::<BTreeMap<_, _>>();
        let after_by_id = after
            .actors
            .iter()
            .map(|actor| (actor.runtime_generation, actor))
            .collect::<BTreeMap<_, _>>();

        for (runtime_generation, actor) in &after_by_id {
            if let Some(previous) = before_by_id.get(runtime_generation) {
                accumulator.persistent_transition_pairs += 1;
                let profile = accumulator
                    .profiles
                    .entry(previous.profile_name)
                    .or_default();
                profile.persistent_transition_pairs += 1;
                record_persistent_actor_changes(profile, previous, actor);
                if previous.profile_name != actor.profile_name
                    || previous.actor_name != actor.actor_name
                {
                    *accumulator
                        .identity_conflicts
                        .entry((
                            previous.profile_name,
                            actor.profile_name,
                            previous.actor_name,
                            actor.actor_name,
                        ))
                        .or_default() += 1;
                }
            } else {
                let profile = accumulator.profiles.entry(actor.profile_name).or_default();
                if same_context {
                    accumulator.in_context_appearances += 1;
                    profile.in_context_appearances += 1;
                } else {
                    accumulator.context_change_appearances += 1;
                    profile.context_change_appearances += 1;
                }
            }
        }
        for (runtime_generation, actor) in &before_by_id {
            if !after_by_id.contains_key(runtime_generation) {
                let profile = accumulator.profiles.entry(actor.profile_name).or_default();
                if same_context {
                    accumulator.in_context_disappearances += 1;
                    profile.in_context_disappearances += 1;
                } else {
                    accumulator.context_change_disappearances += 1;
                    profile.context_change_disappearances += 1;
                }
            }
        }
    }
}

impl IdentityValues {
    pub(super) fn insert(&mut self, value: impl Into<String>, success: bool) {
        *self.values.entry(value.into()).or_default() |= if success { 2 } else { 1 };
    }

    pub(super) fn separates_outcomes(&self, both_outcomes_present: bool) -> bool {
        both_outcomes_present
            && self.values.len() > 1
            && self
                .values
                .values()
                .all(|outcomes| matches!(outcomes, 1 | 2))
    }
}

pub(super) fn record_status(coverage: &mut ChannelCoverage, status: NativeChannelStatus) {
    match status {
        NativeChannelStatus::Present => coverage.present += 1,
        NativeChannelStatus::Absent => coverage.absent += 1,
        NativeChannelStatus::Unavailable => coverage.unavailable += 1,
        NativeChannelStatus::NotSampled => coverage.not_sampled += 1,
    }
}

pub(super) fn encode_pad(pad: NativeRawPad, output: &mut Vec<u8>) {
    output.extend_from_slice(&pad.buttons.to_le_bytes());
    output.extend_from_slice(&[
        pad.stick_x as u8,
        pad.stick_y as u8,
        pad.substick_x as u8,
        pad.substick_y as u8,
        pad.trigger_left,
        pad.trigger_right,
        pad.analog_a,
        pad.analog_b,
        u8::from(pad.connected),
        pad.error as u8,
    ]);
}

pub(super) fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
}

pub(super) fn replay_key(
    shard: &NativeEpisodeShard,
    source_state: [u8; 16],
    trajectory: &[u8],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.native-corpus-determinism-key/v1\0");
    for value in [
        shard.metadata.build_revision.as_bytes(),
        shard.metadata.aurora_revision.as_bytes(),
        shard.metadata.feature_digest.as_bytes(),
        shard.metadata.fidelity_profile.as_bytes(),
        shard
            .metadata
            .game_data_sha256
            .map(|digest| digest.to_string())
            .unwrap_or_default()
            .as_bytes(),
        shard
            .metadata
            .card_fixture_identity
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
        shard
            .metadata
            .actor_profile_catalog_identity
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
        shard
            .metadata
            .world_context_sha256
            .map(|digest| digest.to_string())
            .unwrap_or_default()
            .as_bytes(),
        shard.metadata.checkpoint_identity.as_bytes(),
    ] {
        hash_field(&mut hasher, value);
    }
    hash_field(&mut hasher, &source_state);
    hash_field(&mut hasher, trajectory);
    format!("{:x}", hasher.finalize())
}

pub(super) fn pad_key(pad: NativeRawPad) -> String {
    let mut bytes = Vec::with_capacity(12);
    encode_pad(pad, &mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn float_bits(value: f32) -> String {
    format!("0x{:08x}", value.to_bits())
}
