use super::*;

pub(super) fn native_actor_feature_schema(
    spec: &NativeEncoderFeatureSpec,
) -> Result<Digest, TrainableSetError> {
    canonical_digest(
        b"dusklight.native-direct-actor-features/v10\0",
        &(
            spec,
            "signed-log-presence-unit-rms-reservoir/v1",
            HISTORY_RESERVOIR_SEED,
            selected_feature_names(
                native_base_feature_names(),
                &native_base_feature_families(),
                spec,
            ),
            selected_feature_names(
                native_actor_categorical_names(),
                &native_actor_categorical_families(),
                spec,
            ),
            selected_feature_names(
                native_actor_continuous_names(),
                &native_actor_continuous_families(),
                spec,
            ),
            selected_feature_names(
                native_actor_binary_names(),
                &native_actor_binary_families(),
                spec,
            ),
            native_history_feature_names(spec),
        ),
    )
}

pub(super) fn native_history_feature_names(spec: &NativeEncoderFeatureSpec) -> Vec<String> {
    if matches!(
        spec.history_encoding,
        NativeEncoderHistoryEncoding::None | NativeEncoderHistoryEncoding::TrainableGru
    ) {
        return Vec::new();
    }
    if spec.history_encoding == NativeEncoderHistoryEncoding::RecurrentReservoir {
        let mut names = vec![
            "history_recurrent_available".into(),
            "history_recurrent_fill".into(),
        ];
        names.extend(
            (0..spec.history_recurrent_width)
                .map(|index| format!("history_recurrent_hidden_{index}")),
        );
        return names;
    }
    let core_names = selected_feature_names(
        native_base_feature_names(),
        &native_base_feature_families(),
        spec,
    );
    let action_names = [
        "stick_x",
        "stick_y",
        "substick_x",
        "substick_y",
        "trigger_left",
        "trigger_right",
        "analog_a",
        "analog_b",
    ];
    let mut names = Vec::new();
    for slot in 0..spec.history_depth {
        names.push(format!("history_{slot}_present"));
        names.extend(
            action_names
                .iter()
                .map(|name| format!("history_{slot}_action_{name}")),
        );
        names.extend((0..16).map(|bit| format!("history_{slot}_action_button_{bit}")));
        names.extend(
            core_names
                .iter()
                .map(|name| format!("history_{slot}_state_{name}")),
        );
    }
    names
}

pub(super) fn native_base_feature_names() -> Vec<String> {
    let mut names = Vec::new();
    for prefix in [
        "player_position",
        "player_velocity",
        "player_current_angle_s16",
        "player_shape_angle_s16",
    ] {
        extend_vec3_feature_names(&mut names, prefix);
    }
    names.insert(6, "player_forward_speed".into());
    names.push("player_procedure".into());
    names.extend((0..32).map(|bit| format!("player_mode_flag_{bit}")));
    names.extend((0..8).map(|bit| format!("player_contact_bit_{bit}")));
    names.extend(
        [
            "event_running",
            "event_id",
            "event_mode",
            "event_status",
            "event_map_tool_id",
            "room",
            "layer",
            "point",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend(
        [
            "event_transition_data_loaded",
            "event_transition_camera_play",
            "event_transition_current_event_id",
            "event_transition_current_event_type",
            "event_transition_current_event_room",
            "event_transition_goal_x",
            "event_transition_goal_y",
            "event_transition_goal_z",
            "event_transition_pending_stage",
            "event_transition_pending_room",
            "event_transition_pending_layer",
            "event_transition_pending_point",
            "event_transition_pending_wipe",
            "event_transition_pending_wipe_speed",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend(
        [
            "clock_framework_frames",
            "clock_gameplay_frames",
            "clock_global_pause",
            "clock_scene_paused",
            "clock_scene_pause_timer",
            "clock_scene_next_pause_timer",
            "clock_overlap_request_active",
            "clock_overlap_fadeout_peek",
            "clock_demo_present",
            "clock_demo_mode",
            "clock_demo_frame",
            "clock_demo_frame_no_message",
            "clock_demo_flags",
            "clock_timer_present",
            "clock_timer_mode",
            "clock_timer_now_ms",
            "clock_timer_limit_ms",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend(
        [
            "warp_request_kind",
            "warp_selection_present",
            "warp_selection_position_x",
            "warp_selection_position_y",
            "warp_selection_position_z",
            "warp_selection_angle",
            "warp_selection_room",
            "warp_selection_parameter",
            "warp_selection_player",
            "warp_selection_stage_matches_current",
            "warp_return_mark_present",
            "warp_return_position_x",
            "warp_return_position_y",
            "warp_return_position_z",
            "warp_return_angle",
            "warp_return_room",
            "warp_return_accept_stage",
            "warp_return_stage_matches_current",
            "warp_target_point_present",
            "warp_target_point",
            "warp_selected_point_present",
            "warp_selected_point",
            "warp_transport_match",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend(
        [
            "previous_stick_x",
            "previous_stick_y",
            "previous_substick_x",
            "previous_substick_y",
            "previous_trigger_left",
            "previous_trigger_right",
            "previous_analog_a",
            "previous_analog_b",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend((0..16).map(|bit| format!("previous_button_bit_{bit}")));
    names.extend(
        [
            "camera_yaw_radians",
            "camera_view_yaw_s16",
            "camera_controlled_yaw_s16",
            "camera_bank_s16",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    extend_vec3_feature_names(&mut names, "camera_eye");
    extend_vec3_feature_names(&mut names, "camera_center");
    names.extend(
        [
            "player_ground_height",
            "player_roof_height",
            "player_water_height",
            "collision_correction_x",
            "collision_correction_z",
            "scene_exit_signed_distance",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    extend_vec3_feature_names(&mut names, "scene_exit_player_local_position");
    extend_vec3_feature_names(&mut names, "scene_exit_volume_extent");
    for stream in 0..2 {
        names.push(format!("rng_{stream}_id"));
        for state in 0..3 {
            names.push(format!("rng_{stream}_state_{state}"));
        }
        names.push(format!("rng_{stream}_call_count"));
    }
    names.extend(["rng_stream_count".into(), "rng_stream_overflow".into()]);
    names.extend(
        [
            "goal_requested_count",
            "goal_hit_count",
            "goal_stable_ticks",
            "goal_consecutive_ticks",
            "goal_sequence_steps",
            "goal_sequence_next_step",
            "goal_sequence_within_ticks",
            "goal_sequence_elapsed_ticks",
            "goal_reached",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend((0..32).map(|bit| format!("attention_player_flag_{bit}")));
    names.extend(
        [
            "attention_status",
            "attention_block_timer",
            "attention_lock_count",
            "attention_lock_offset",
            "attention_action_count",
            "attention_action_offset",
            "attention_check_count",
            "attention_check_offset",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    for prefix in ["temporal_player_position", "temporal_player_velocity"] {
        extend_vec3_feature_names(&mut names, prefix);
    }
    names.extend(
        [
            "temporal_player_forward_speed_delta",
            "temporal_camera_yaw_delta",
            "temporal_ground_height_delta",
            "temporal_roof_height_delta",
            "temporal_previous_state_available",
            "temporal_player_comparable",
            "temporal_procedure_changed",
            "temporal_mode_changed",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend((0..8).map(|bit| format!("temporal_contact_changed_bit_{bit}")));
    names.extend(
        [
            "temporal_event_running_changed",
            "temporal_event_id_changed",
            "temporal_context_changed",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names
}

pub(super) fn native_actor_categorical_names() -> Vec<String> {
    [
        "parameters",
        "status",
        "actor_name",
        "profile_name",
        "set_id",
        "home_room",
        "current_room",
        "group",
        "argument",
        "health",
        "actor_type",
        "process_subtype",
        "condition",
        "old_room",
        "pause_flag",
        "process_init_state",
        "process_create_phase",
        "cull_type",
        "demo_actor_id",
        "carry_type",
        "attention_flags",
        "attention_distance_0",
        "attention_distance_1",
        "attention_distance_2",
        "attention_distance_3",
        "attention_distance_4",
        "attention_distance_5",
        "attention_distance_6",
        "attention_distance_7",
        "attention_distance_8",
        "attention_auxiliary",
        "event_command",
        "event_condition",
        "event_id",
        "event_map_tool_id",
        "event_index",
        "return_save_room",
        "return_save_point",
        "return_switch_room",
        "return_required_event_set",
        "return_required_event_unset",
        "return_required_switch_set",
        "return_required_switch_unset",
        "enemy_flags",
        "enemy_throw_mode",
        "trigger_kind",
        "trigger_shape",
        "trigger_behavior",
        "door20_kind",
        "door20_model",
        "door20_front_option",
        "door20_back_option",
        "door20_front_room",
        "door20_back_room",
        "door20_exit_number",
        "door20_front_switch",
        "door20_back_switch",
        "door20_unlock_effect_switch",
        "door20_front_event",
        "door20_back_event",
        "door20_message_number",
        "door20_action",
        "door20_active_side",
        "door20_event_variant",
        "door20_key_type",
        "door20_enemy_clear_debounce",
        "door20_stopper_side",
        "door20_front_stopper_status",
        "door20_back_stopper_status",
        "attention_lock_type",
        "attention_lock_rank",
        "attention_action_type",
        "attention_action_rank",
        "attention_check_type",
        "attention_check_rank",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

pub(super) fn native_actor_continuous_names() -> Vec<String> {
    let mut names = Vec::new();
    for prefix in [
        "absolute_position",
        "absolute_home_position",
        "absolute_old_position",
        "absolute_velocity",
    ] {
        extend_vec3_feature_names(&mut names, prefix);
    }
    names.push("forward_speed".into());
    extend_vec3_feature_names(&mut names, "scale");
    names.extend(["gravity".into(), "max_fall_speed".into()]);
    extend_vec3_feature_names(&mut names, "absolute_eye_position");
    for prefix in [
        "home_angle_s16",
        "old_angle_s16",
        "current_angle_s16",
        "shape_angle_s16",
        "link_relative_position",
        "link_relative_home_position",
        "link_relative_velocity",
    ] {
        extend_vec3_feature_names(&mut names, prefix);
    }
    names.push("link_distance".into());
    extend_vec3_feature_names(&mut names, "parent_relative_position");
    extend_vec3_feature_names(&mut names, "parent_relative_velocity");
    extend_vec3_feature_names(&mut names, "attention_absolute_position");
    extend_vec3_feature_names(&mut names, "attention_link_relative_position");
    extend_vec3_feature_names(&mut names, "enemy_absolute_down_position");
    extend_vec3_feature_names(&mut names, "enemy_absolute_head_lock_position");
    extend_vec3_feature_names(&mut names, "trigger_absolute_center");
    extend_vec3_feature_names(&mut names, "trigger_half_extent");
    extend_vec3_feature_names(&mut names, "trigger_link_relative_center");
    names.extend([
        "trigger_yaw_relative_to_link_sin".into(),
        "trigger_yaw_relative_to_link_cos".into(),
    ]);
    names.push("door20_angle_s16".into());
    for prefix in ["attention_lock", "attention_action", "attention_check"] {
        names.extend([
            format!("{prefix}_weight"),
            format!("{prefix}_distance"),
            format!("{prefix}_angle_s16"),
        ]);
    }
    extend_vec3_feature_names(&mut names, "temporal_position_delta");
    extend_vec3_feature_names(&mut names, "temporal_velocity_delta");
    names.push("temporal_forward_speed_delta".into());
    extend_vec3_feature_names(&mut names, "temporal_current_angle_delta_s16");
    extend_vec3_feature_names(&mut names, "temporal_shape_angle_delta_s16");
    extend_vec3_feature_names(&mut names, "temporal_attention_position_delta");
    names
}

pub(super) fn native_actor_binary_names() -> Vec<String> {
    let mut names = [
        "base_state_available",
        "heap_present",
        "model_present",
        "joint_collision_present",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    names.extend((0..32).map(|bit| format!("status_bit_{bit}")));
    names.extend(
        [
            "attention_present",
            "event_participation_present",
            "return_place_writer_present",
            "enemy_base_present",
            "return_no_telop_clear",
            "return_event_set_satisfied",
            "return_event_unset_satisfied",
            "return_switch_set_satisfied",
            "return_switch_unset_satisfied",
            "return_eligible",
            "player_targeted_actor",
            "player_ride_actor",
            "player_held_item_actor",
            "player_grabbed_actor",
            "player_thrown_boomerang_actor",
            "player_copy_rod_actor",
            "player_hookshot_roof_wait_actor",
            "player_chain_grab_actor",
            "player_attention_hint_actor",
            "player_attention_catch_actor",
            "player_attention_look_actor",
            "trigger_volume_present",
            "trigger_enabled",
            "trigger_vertical_unbounded",
            "door20_present",
            "door20_message_door",
            "door20_front_switch_set",
            "door20_back_switch_set",
            "door20_unlock_effect_switch_set",
            "door20_locked",
            "door20_background_collision_released",
            "door20_unlock_effect_triggered",
            "door20_opening_active",
            "door20_closing_active",
            "attention_lock_candidate",
            "attention_action_candidate",
            "attention_check_candidate",
            "temporal_previous_actor_present",
            "temporal_base_state_changed",
            "temporal_actor_type_changed",
            "temporal_process_subtype_changed",
            "temporal_parameters_changed",
            "temporal_status_changed",
            "temporal_condition_changed",
            "temporal_home_room_changed",
            "temporal_old_room_changed",
            "temporal_current_room_changed",
            "temporal_group_changed",
            "temporal_argument_changed",
            "temporal_pause_flag_changed",
            "temporal_process_init_state_changed",
            "temporal_process_create_phase_changed",
            "temporal_cull_type_changed",
            "temporal_demo_actor_id_changed",
            "temporal_carry_type_changed",
            "temporal_health_changed",
            "temporal_heap_present_changed",
            "temporal_model_present_changed",
            "temporal_joint_collision_present_changed",
            "temporal_attention_presence_changed",
            "temporal_event_presence_changed",
            "temporal_enemy_presence_changed",
            "temporal_trigger_presence_changed",
            "temporal_door20_presence_changed",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names
}

pub(super) fn native_base_feature_families() -> Vec<NativeEncoderChannelFamily> {
    use NativeEncoderChannelFamily as Family;
    let mut families = Vec::new();
    extend_family(&mut families, Family::CorePlayerMotion, 13);
    extend_family(&mut families, Family::CoreActionPhase, 41);
    extend_family(&mut families, Family::CoreEventContext, 8);
    extend_family(&mut families, Family::CoreEventTransition, 14);
    extend_family(&mut families, Family::CoreClockDomains, 17);
    extend_family(&mut families, Family::CoreWarpSession, 23);
    extend_family(&mut families, Family::CorePreviousInput, 24);
    extend_family(&mut families, Family::CoreCameraCollisionWorld, 22);
    extend_family(&mut families, Family::CoreRng, 12);
    extend_family(&mut families, Family::CoreGoal, 9);
    extend_family(&mut families, Family::CoreAttentionCandidates, 40);
    extend_family(&mut families, Family::CoreTemporalDelta, 25);
    families
}

pub(super) fn native_actor_categorical_families() -> Vec<NativeEncoderChannelFamily> {
    use NativeEncoderChannelFamily as Family;
    let mut families = Vec::new();
    extend_family(&mut families, Family::ActorIdentity, 10);
    extend_family(&mut families, Family::ActorLifecyclePhysics, 10);
    extend_family(&mut families, Family::ActorAttention, 11);
    extend_family(&mut families, Family::ActorEventParticipation, 5);
    extend_family(&mut families, Family::ActorReturnWriter, 7);
    extend_family(&mut families, Family::ActorEnemyBase, 2);
    extend_family(&mut families, Family::ActorTriggerVolume, 3);
    extend_family(&mut families, Family::ActorDoor20, 21);
    extend_family(&mut families, Family::ActorAttentionCandidates, 6);
    families
}

pub(super) fn native_actor_continuous_families() -> Vec<NativeEncoderChannelFamily> {
    use NativeEncoderChannelFamily as Family;
    let mut families = Vec::new();
    extend_family(&mut families, Family::ActorMotion, 6);
    extend_family(&mut families, Family::ActorLifecyclePhysics, 3);
    extend_family(&mut families, Family::ActorMotion, 4);
    extend_family(&mut families, Family::ActorLifecyclePhysics, 14);
    extend_family(&mut families, Family::ActorMotion, 6);
    extend_family(&mut families, Family::ActorLinkRelative, 10);
    extend_family(&mut families, Family::ActorParentRelative, 6);
    extend_family(&mut families, Family::ActorAttention, 6);
    extend_family(&mut families, Family::ActorEnemyBase, 6);
    extend_family(&mut families, Family::ActorTriggerVolume, 11);
    extend_family(&mut families, Family::ActorDoor20, 1);
    extend_family(&mut families, Family::ActorAttentionCandidates, 9);
    extend_family(&mut families, Family::ActorTemporalDelta, 16);
    families
}

pub(super) fn native_actor_binary_families() -> Vec<NativeEncoderChannelFamily> {
    use NativeEncoderChannelFamily as Family;
    let mut families = Vec::new();
    extend_family(&mut families, Family::ActorLifecyclePhysics, 4);
    extend_family(&mut families, Family::ActorIdentity, 32);
    extend_family(&mut families, Family::ActorAttention, 1);
    extend_family(&mut families, Family::ActorEventParticipation, 1);
    extend_family(&mut families, Family::ActorReturnWriter, 1);
    extend_family(&mut families, Family::ActorEnemyBase, 1);
    extend_family(&mut families, Family::ActorReturnWriter, 6);
    extend_family(&mut families, Family::ActorPlayerRelationships, 11);
    extend_family(&mut families, Family::ActorTriggerVolume, 3);
    extend_family(&mut families, Family::ActorDoor20, 10);
    extend_family(&mut families, Family::ActorAttentionCandidates, 3);
    extend_family(&mut families, Family::ActorTemporalDelta, 27);
    families
}

pub(super) fn extend_family(
    families: &mut Vec<NativeEncoderChannelFamily>,
    family: NativeEncoderChannelFamily,
    count: usize,
) {
    families.extend(std::iter::repeat_n(family, count));
}

pub(super) fn selected_feature_names(
    names: Vec<String>,
    families: &[NativeEncoderChannelFamily],
    spec: &NativeEncoderFeatureSpec,
) -> Vec<String> {
    debug_assert_eq!(names.len(), families.len());
    names
        .into_iter()
        .zip(families)
        .filter_map(|(name, family)| spec.contains(*family).then_some(name))
        .collect()
}

pub(super) fn retain_feature_families<T>(
    values: &mut Vec<T>,
    present: &mut Vec<bool>,
    families: &[NativeEncoderChannelFamily],
    spec: &NativeEncoderFeatureSpec,
) {
    debug_assert_eq!(values.len(), present.len());
    debug_assert_eq!(values.len(), families.len());
    let retained = families
        .iter()
        .map(|family| spec.contains(*family))
        .collect::<Vec<_>>();
    *values = std::mem::take(values)
        .into_iter()
        .zip(&retained)
        .filter_map(|(value, retained)| retained.then_some(value))
        .collect();
    *present = std::mem::take(present)
        .into_iter()
        .zip(retained)
        .filter_map(|(value, retained)| retained.then_some(value))
        .collect();
}

pub(super) fn suppress_base_family(
    values: &mut [f32],
    present: &mut [bool],
    suppressed: NativeEncoderChannelFamily,
) {
    debug_assert_eq!(values.len(), present.len());
    debug_assert_eq!(values.len(), native_base_feature_families().len());
    for ((value, available), family) in values
        .iter_mut()
        .zip(present)
        .zip(native_base_feature_families())
    {
        if family == suppressed {
            *value = 0.0;
            *available = false;
        }
    }
}

pub(super) fn retain_node_feature_families(
    node: &mut TypedSetNode,
    spec: &NativeEncoderFeatureSpec,
) {
    retain_feature_families(
        &mut node.categorical,
        &mut node.categorical_present,
        &native_actor_categorical_families(),
        spec,
    );
    retain_feature_families(
        &mut node.continuous,
        &mut node.continuous_present,
        &native_actor_continuous_families(),
        spec,
    );
    retain_feature_families(
        &mut node.binary,
        &mut node.binary_present,
        &native_actor_binary_families(),
        spec,
    );
}

pub(super) fn extend_vec3_feature_names(names: &mut Vec<String>, prefix: &str) {
    names.extend(["x", "y", "z"].map(|axis| format!("{prefix}_{axis}")));
}
