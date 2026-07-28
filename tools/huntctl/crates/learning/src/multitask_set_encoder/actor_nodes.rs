use super::*;

pub(super) fn native_actor_nodes(
    observation: &NativeLearningObservation,
    previous: Option<&NativeLearningObservation>,
) -> Vec<TypedSetNode> {
    let actors_by_generation = observation
        .actors
        .iter()
        .map(|actor| (actor.runtime_generation, actor))
        .collect::<BTreeMap<_, _>>();
    let previous_comparable = previous.is_some_and(|previous| {
        observation.stage == previous.stage
            && observation.room == previous.room
            && observation.layer == previous.layer
    });
    let previous_actors_by_generation = previous
        .filter(|_| previous_comparable)
        .map(|observation| {
            observation
                .actors
                .iter()
                .map(|actor| (actor.runtime_generation, actor))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    observation
        .actors
        .iter()
        .map(|actor| {
            native_actor_node(
                observation,
                actor,
                &actors_by_generation,
                previous_actors_by_generation
                    .get(&actor.runtime_generation)
                    .copied(),
                previous_comparable,
            )
        })
        .collect()
}

pub(super) fn attention_candidate_for(
    candidates: &[NativeAttentionCandidateObservation],
    runtime_generation: u64,
) -> Option<(usize, &NativeAttentionCandidateObservation)> {
    candidates.iter().enumerate().find(|(_, candidate)| {
        candidate.actor.actor.as_ref().is_some_and(|identity| {
            identity.present && u64::from(identity.runtime_generation) == runtime_generation
        })
    })
}

pub(super) fn native_actor_node(
    observation: &NativeLearningObservation,
    actor: &NativeActorObservation,
    actors_by_generation: &BTreeMap<u64, &NativeActorObservation>,
    previous_actor: Option<&NativeActorObservation>,
    previous_observation_available: bool,
) -> TypedSetNode {
    let attention_available =
        observation.attention_candidates_status == NativeChannelStatus::Present;
    let attention = observation.attention_candidates.as_ref();
    let lock_candidate = attention.and_then(|value| {
        attention_candidate_for(&value.lock_candidates, actor.runtime_generation)
    });
    let action_candidate = attention.and_then(|value| {
        attention_candidate_for(&value.action_candidates, actor.runtime_generation)
    });
    let check_candidate = attention.and_then(|value| {
        attention_candidate_for(&value.check_candidates, actor.runtime_generation)
    });

    let mut categorical = Vec::new();
    let mut categorical_present = Vec::new();
    let mut category = |value: i64, available: bool| {
        categorical.push(if available { value } else { 0 });
        categorical_present.push(available);
    };
    for value in [
        i64::from(actor.parameters),
        i64::from(actor.status),
        i64::from(actor.actor_name),
        i64::from(actor.profile_name),
        i64::from(actor.set_id),
        i64::from(actor.home_room),
        i64::from(actor.current_room),
        i64::from(actor.group),
        i64::from(actor.argument),
        i64::from(actor.health),
    ] {
        category(value, true);
    }
    for value in [
        i64::from(actor.actor_type),
        i64::from(actor.process_subtype),
        i64::from(actor.condition),
        i64::from(actor.old_room),
        i64::from(actor.pause_flag),
        i64::from(actor.process_init_state),
        i64::from(actor.process_create_phase),
        i64::from(actor.cull_type),
        i64::from(actor.demo_actor_id),
        i64::from(actor.carry_type),
    ] {
        category(value, actor.base_state_available);
    }
    if let Some(attention) = &actor.attention {
        category(i64::from(attention.flags), true);
        for value in attention.distance_indices {
            category(i64::from(value), true);
        }
        category(i64::from(attention.auxiliary), true);
    } else {
        for _ in 0..11 {
            category(0, false);
        }
    }
    if let Some(event) = &actor.event_participation {
        for value in [
            i64::from(event.command),
            i64::from(event.condition),
            i64::from(event.event_id),
            i64::from(event.map_tool_id),
            i64::from(event.index),
        ] {
            category(value, true);
        }
    } else {
        for _ in 0..5 {
            category(0, false);
        }
    }
    if let Some(writer) = &actor.return_place_writer {
        for value in [
            i64::from(writer.save_room),
            i64::from(writer.save_point),
            i64::from(writer.switch_room),
            i64::from(writer.required_event_set),
            i64::from(writer.required_event_unset),
            i64::from(writer.required_switch_set),
            i64::from(writer.required_switch_unset),
        ] {
            category(value, true);
        }
    } else {
        for _ in 0..7 {
            category(0, false);
        }
    }
    if let Some(enemy) = &actor.enemy_base {
        category(i64::from(enemy.flags), true);
        category(i64::from(enemy.throw_mode), true);
    } else {
        category(0, false);
        category(0, false);
    }
    if let Some(trigger) = &actor.trigger_volume {
        use dusklight_evidence::native_episode_shard::{
            NativeTriggerVolumeKind as Kind, NativeTriggerVolumeShape as Shape,
        };
        category(
            match trigger.kind {
                Kind::SceneExit => 0,
                Kind::SceneExitCylinder => 1,
                Kind::EventArea => 2,
                Kind::ScriptedEvent => 3,
                Kind::MappedEvent => 4,
            },
            true,
        );
        category(
            match trigger.shape {
                Shape::Box => 0,
                Shape::EllipticCylinder => 1,
            },
            true,
        );
        category(i64::from(trigger.behavior), true);
    } else {
        for _ in 0..3 {
            category(0, false);
        }
    }
    if let Some(door) = &actor.door20 {
        for value in [
            door.kind,
            door.door_model,
            door.front_option,
            door.back_option,
            door.front_room,
            door.back_room,
            door.exit_number,
        ] {
            category(i64::from(value), true);
        }
        for switch in [
            door.front_switch,
            door.back_switch,
            door.unlock_effect_switch,
        ] {
            category(i64::from(switch), switch != u8::MAX);
        }
        for value in [
            i64::from(door.front_event),
            i64::from(door.back_event),
            i64::from(door.message_number),
            door.action as u8 as i64,
            door.active_side as u8 as i64,
            i64::from(door.event_variant),
            i64::from(door.key_type),
            i64::from(door.enemy_clear_debounce),
            door.stopper_side as u8 as i64,
            door.front_stopper_status as u8 as i64,
            door.back_stopper_status as u8 as i64,
        ] {
            category(value, true);
        }
    } else {
        for _ in 0..21 {
            category(0, false);
        }
    }
    for candidate in [lock_candidate, action_candidate, check_candidate] {
        category(
            candidate.map_or(0, |(_, value)| i64::from(value.attention_type)),
            candidate.is_some(),
        );
        category(
            candidate.map_or(0, |(rank, _)| rank as i64),
            candidate.is_some(),
        );
    }

    let mut continuous = Vec::new();
    let mut continuous_present = Vec::new();
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.position,
        true,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.home_position,
        true,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.old_position,
        actor.base_state_available,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.velocity,
        true,
    );
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        actor.forward_speed,
        true,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.scale,
        actor.base_state_available,
    );
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        actor.gravity,
        actor.base_state_available,
    );
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        actor.max_fall_speed,
        actor.base_state_available,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.eye_position,
        actor.base_state_available,
    );
    for (index, angles) in [
        actor.home_angle,
        actor.old_angle,
        actor.current_angle,
        actor.shape_angle,
    ]
    .into_iter()
    .enumerate()
    {
        let available = actor.base_state_available || index >= 2;
        push_continuous3(
            &mut continuous,
            &mut continuous_present,
            angles.map(f32::from),
            available,
        );
    }
    let player_available = observation.player_present;
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        subtract3(actor.position, observation.player_position),
        player_available,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        subtract3(actor.home_position, observation.player_position),
        player_available,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        subtract3(actor.velocity, observation.player_velocity),
        player_available,
    );
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        length3(subtract3(actor.position, observation.player_position)),
        player_available,
    );
    let parent = actors_by_generation.get(&u64::from(actor.parent_runtime_generation));
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        parent.map_or([0.0; 3], |parent| {
            subtract3(actor.position, parent.position)
        }),
        parent.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        parent.map_or([0.0; 3], |parent| {
            subtract3(actor.velocity, parent.velocity)
        }),
        parent.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor
            .attention
            .as_ref()
            .map_or([0.0; 3], |value| value.position),
        actor.attention.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.attention.as_ref().map_or([0.0; 3], |value| {
            subtract3(value.position, observation.player_position)
        }),
        actor.attention.is_some() && player_available,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor
            .enemy_base
            .as_ref()
            .map_or([0.0; 3], |value| value.down_position),
        actor.enemy_base.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor
            .enemy_base
            .as_ref()
            .map_or([0.0; 3], |value| value.head_lock_position),
        actor.enemy_base.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor
            .trigger_volume
            .as_ref()
            .map_or([0.0; 3], |value| value.center),
        actor.trigger_volume.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor
            .trigger_volume
            .as_ref()
            .map_or([0.0; 3], |value| value.half_extent),
        actor.trigger_volume.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.trigger_volume.as_ref().map_or([0.0; 3], |value| {
            direction_yaw3(
                subtract3(value.center, observation.player_position),
                observation.player_shape_angle[1],
            )
        }),
        actor.trigger_volume.is_some() && player_available,
    );
    let trigger_yaw = actor
        .trigger_volume
        .as_ref()
        .map(|value| angle_pair(value.yaw.wrapping_sub(observation.player_shape_angle[1])));
    for component in trigger_yaw.unwrap_or([0.0; 2]) {
        push_continuous(
            &mut continuous,
            &mut continuous_present,
            component,
            trigger_yaw.is_some() && player_available,
        );
    }
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        actor
            .door20
            .as_ref()
            .map_or(0.0, |door| f32::from(door.door_angle)),
        actor.door20.is_some(),
    );
    for candidate in [lock_candidate, action_candidate, check_candidate] {
        push_continuous(
            &mut continuous,
            &mut continuous_present,
            candidate.map_or(0.0, |(_, value)| value.weight),
            candidate.is_some(),
        );
        push_continuous(
            &mut continuous,
            &mut continuous_present,
            candidate.map_or(0.0, |(_, value)| value.distance),
            candidate.is_some(),
        );
        push_continuous(
            &mut continuous,
            &mut continuous_present,
            candidate.map_or(0.0, |(_, value)| f32::from(value.angle)),
            candidate.is_some(),
        );
    }
    let actor_comparable = previous_actor.is_some();
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        previous_actor.map_or([0.0; 3], |previous| {
            subtract3(actor.position, previous.position)
        }),
        actor_comparable,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        previous_actor.map_or([0.0; 3], |previous| {
            subtract3(actor.velocity, previous.velocity)
        }),
        actor_comparable,
    );
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        previous_actor.map_or(0.0, |previous| actor.forward_speed - previous.forward_speed),
        actor_comparable,
    );
    for (current, previous) in [
        (
            actor.current_angle,
            previous_actor.map(|actor| actor.current_angle),
        ),
        (
            actor.shape_angle,
            previous_actor.map(|actor| actor.shape_angle),
        ),
    ] {
        push_continuous3(
            &mut continuous,
            &mut continuous_present,
            previous.map_or([0.0; 3], |previous| {
                std::array::from_fn(|index| f32::from(current[index].wrapping_sub(previous[index])))
            }),
            previous.is_some(),
        );
    }
    let attention_pair = actor
        .attention
        .as_ref()
        .zip(previous_actor.and_then(|previous| previous.attention.as_ref()));
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        attention_pair.map_or([0.0; 3], |(current, previous)| {
            subtract3(current.position, previous.position)
        }),
        attention_pair.is_some(),
    );

    let mut binary = Vec::new();
    let mut binary_present = Vec::new();
    let mut boolean = |value: bool, available: bool| {
        binary.push(value && available);
        binary_present.push(available);
    };
    boolean(actor.base_state_available, true);
    boolean(actor.heap_present, actor.base_state_available);
    boolean(actor.model_present, actor.base_state_available);
    boolean(actor.joint_collision_present, actor.base_state_available);
    for bit in 0..32 {
        boolean(actor.status & (1_u32 << bit) != 0, true);
    }
    boolean(actor.attention.is_some(), true);
    boolean(actor.event_participation.is_some(), true);
    boolean(actor.return_place_writer.is_some(), true);
    boolean(actor.enemy_base.is_some(), true);
    if let Some(writer) = &actor.return_place_writer {
        for value in [
            writer.no_telop_clear,
            writer.event_set_satisfied,
            writer.event_unset_satisfied,
            writer.switch_set_satisfied,
            writer.switch_unset_satisfied,
            writer.eligible,
        ] {
            boolean(value, true);
        }
    } else {
        for _ in 0..6 {
            boolean(false, false);
        }
    }
    let relationships_available =
        observation.player_relationships_status == NativeChannelStatus::Present;
    let relationships = observation.player_relationships.as_ref();
    let related = |identity: Option<
        &dusklight_evidence::native_episode_shard::NativeActorIdentity,
    >| {
        identity.is_some_and(|identity| {
            identity.present && u64::from(identity.runtime_generation) == actor.runtime_generation
        })
    };
    for value in [
        relationships.and_then(|value| value.targeted_actor.as_ref()),
        relationships.and_then(|value| value.ride_actor.as_ref()),
        relationships.and_then(|value| value.held_item_actor.as_ref()),
        relationships.and_then(|value| value.grabbed_actor.as_ref()),
        relationships.and_then(|value| value.thrown_boomerang_actor.as_ref()),
        relationships.and_then(|value| value.copy_rod_actor.as_ref()),
        relationships.and_then(|value| value.hookshot_roof_wait_actor.as_ref()),
        relationships.and_then(|value| value.chain_grab_actor.as_ref()),
        relationships.and_then(|value| value.attention_hint_actor.as_ref()),
        relationships.and_then(|value| value.attention_catch_actor.as_ref()),
        relationships.and_then(|value| value.attention_look_actor.as_ref()),
    ] {
        boolean(related(value), relationships_available);
    }
    boolean(actor.trigger_volume.is_some(), true);
    boolean(
        actor
            .trigger_volume
            .as_ref()
            .is_some_and(|value| value.enabled),
        actor.trigger_volume.is_some(),
    );
    boolean(
        actor
            .trigger_volume
            .as_ref()
            .is_some_and(|value| value.vertical_unbounded),
        actor.trigger_volume.is_some(),
    );
    boolean(actor.door20.is_some(), true);
    if let Some(door) = &actor.door20 {
        boolean(door.message_door, true);
        for (switch, set) in [
            (door.front_switch, door.front_switch_set),
            (door.back_switch, door.back_switch_set),
            (door.unlock_effect_switch, door.unlock_effect_switch_set),
        ] {
            boolean(set, switch != u8::MAX);
        }
        for value in [
            door.locked,
            door.background_collision_released,
            door.unlock_effect_triggered,
            door.opening_active,
            door.closing_active,
        ] {
            boolean(value, true);
        }
    } else {
        for _ in 0..9 {
            boolean(false, false);
        }
    }
    for candidate in [lock_candidate, action_candidate, check_candidate] {
        boolean(candidate.is_some(), attention_available);
    }
    boolean(previous_actor.is_some(), previous_observation_available);
    let previous_available = previous_actor.is_some();
    let changed = previous_actor.map(|previous| {
        [
            actor.base_state_available != previous.base_state_available,
            actor.actor_type != previous.actor_type,
            actor.process_subtype != previous.process_subtype,
            actor.parameters != previous.parameters,
            actor.status != previous.status,
            actor.condition != previous.condition,
            actor.home_room != previous.home_room,
            actor.old_room != previous.old_room,
            actor.current_room != previous.current_room,
            actor.group != previous.group,
            actor.argument != previous.argument,
            actor.pause_flag != previous.pause_flag,
            actor.process_init_state != previous.process_init_state,
            actor.process_create_phase != previous.process_create_phase,
            actor.cull_type != previous.cull_type,
            actor.demo_actor_id != previous.demo_actor_id,
            actor.carry_type != previous.carry_type,
            actor.health != previous.health,
            actor.heap_present != previous.heap_present,
            actor.model_present != previous.model_present,
            actor.joint_collision_present != previous.joint_collision_present,
            actor.attention.is_some() != previous.attention.is_some(),
            actor.event_participation.is_some() != previous.event_participation.is_some(),
            actor.enemy_base.is_some() != previous.enemy_base.is_some(),
            actor.trigger_volume.is_some() != previous.trigger_volume.is_some(),
            actor.door20.is_some() != previous.door20.is_some(),
        ]
    });
    for value in changed.unwrap_or([false; 26]) {
        boolean(value, previous_available);
    }
    debug_assert_eq!(categorical.len(), native_actor_categorical_names().len());
    debug_assert_eq!(continuous.len(), native_actor_continuous_names().len());
    debug_assert_eq!(binary.len(), native_actor_binary_names().len());
    TypedSetNode {
        stable_id: actor.runtime_generation,
        categorical,
        categorical_present,
        continuous,
        continuous_present,
        binary,
        binary_present,
    }
}
