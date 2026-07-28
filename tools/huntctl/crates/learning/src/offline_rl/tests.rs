use super::*;
use crate::observation_view::movement_state_v2_spec;
use crate::trace::{
    TraceAnimationLane, TraceAppliedPads, TraceCamera, TraceChannelWireFormat,
    TraceCollisionSurface, TraceCollisionSurfaceKind, TraceGoalProgress, TracePlayerAction,
    TracePlayerBackgroundCollision, TracePlayerCollisionSurfaces, TraceRngSnapshot, TraceRngStream,
};

fn record(frame: u64, x: f32) -> TraceRecord {
    let channel_status = [
        (TraceChannel::Core, TraceChannelStatus::Present),
        (TraceChannel::Stage, TraceChannelStatus::Present),
        (TraceChannel::AppliedPads, TraceChannelStatus::Present),
        (TraceChannel::PlayerMotion, TraceChannelStatus::Present),
        (TraceChannel::Event, TraceChannelStatus::Present),
        (TraceChannel::SceneExit, TraceChannelStatus::Absent),
    ]
    .into_iter()
    .collect();
    TraceRecord {
        boundary_index: 101 + frame,
        simulation_tick: 100 + frame,
        tape_frame: Some(frame),
        input_source: 1,
        channel_status,
        stage_name: "F_SP103".into(),
        room: 1,
        layer: 3,
        point: 1,
        flags: (1 << 0) | (1 << 1) | (1 << 3),
        player_actor_name: 253,
        current_angle_y: 0,
        shape_angle_y: 0,
        buttons: 0,
        stick_x: 0,
        stick_y: 0,
        position: [x, 800.0, -2300.0],
        velocity: [x, 0.0, 0.0],
        forward_speed: x,
        player_proc_id: Some(4),
        event_id: -1,
        event_mode: 0,
        event_status: 0,
        event_map_tool_id: 0xff,
        pad_error: 0,
        event_name_hash: 0,
        event_name_hash_present: true,
        nearest_scene_exit_actor_name: None,
        nearest_scene_exit_position: [0.0; 3],
        nearest_scene_exit_distance: None,
        ..TraceRecord::default()
    }
}

fn frame(stick_x: i8, stick_y: i8, buttons: u16) -> InputFrame {
    let mut frame = InputFrame {
        owned_ports: 1,
        ..InputFrame::default()
    };
    frame.pads[0] = RawPadState {
        stick_x,
        stick_y,
        buttons,
        ..RawPadState::default()
    };
    frame
}

fn fixture() -> (DecodedTrace, InputTape) {
    let mut first_applied = record(1, 20.0);
    first_applied.stick_y = 72;
    let mut second_applied = record(2, 30.0);
    second_applied.buttons = BUTTON_B;
    second_applied.stick_x = 72;
    (
        DecodedTrace {
            version: 1,
            boot: crate::tape::TapeBoot::Process,
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            requested_channels: 0,
            capacity_exhausted: false,
            retention: None,
            channel_formats: BTreeMap::new(),
            records: vec![record(0, 10.0), first_applied, second_applied],
        },
        InputTape {
            frames: vec![frame(0, 0, 0), frame(0, 127, 0), frame(127, 0, BUTTON_B)],
            ..InputTape::default()
        },
    )
}

fn empty_surface(kind: TraceCollisionSurfaceKind, wall_slot: u8) -> TraceCollisionSurface {
    TraceCollisionSurface {
        flags: 0,
        kind,
        wall_slot,
        backing_format: None,
        raw_code_word_mask: 0,
        bg_index: None,
        poly_index: None,
        owner_session_process_id: None,
        material_row: None,
        group_row: None,
        raw_code_words: [0; 5],
        raw_exit_id: None,
        source_room: None,
        source_room_exact: false,
        scls_source_room: None,
        destination: None,
        source_geometry_indices: Vec::new(),
        kcl_prism_height: None,
    }
}

fn fixture_v2() -> (DecodedTrace, InputTape) {
    let (mut trace, tape) = fixture();
    trace.version = 5;
    trace.requested_channels = TraceChannel::ALL
        .into_iter()
        .fold(0, |mask, channel| mask | channel.bit());
    trace.channel_formats = [
        (TraceChannel::Core, 1, 32),
        (TraceChannel::Stage, 1, 32),
        (TraceChannel::AppliedPads, 1, 52),
        (TraceChannel::PlayerMotion, 1, 52),
        (TraceChannel::Event, 1, 16),
        (TraceChannel::SceneExit, 2, 88),
        (TraceChannel::Rng, 1, 64),
        (TraceChannel::Camera, 1, 48),
        (TraceChannel::PlayerAction, 3, 160),
        (TraceChannel::PlayerBackgroundCollision, 1, 128),
        (TraceChannel::PlayerCollisionSurfaces, 1, 496),
        (TraceChannel::GoalProgress, 1, 32),
    ]
    .into_iter()
    .map(|(channel, version, stride)| (channel, TraceChannelWireFormat { version, stride }))
    .collect();
    for record in &mut trace.records {
        for channel in [
            TraceChannel::Rng,
            TraceChannel::Camera,
            TraceChannel::PlayerAction,
            TraceChannel::PlayerBackgroundCollision,
            TraceChannel::PlayerCollisionSurfaces,
            TraceChannel::GoalProgress,
        ] {
            record
                .channel_status
                .insert(channel, TraceChannelStatus::Present);
        }
        record.event_name_hash = 0;
        record.event_name_hash_present = false;
        record.applied_pads = Some(TraceAppliedPads {
            valid_ports: 1,
            owned_ports: 1,
            pads: [
                RawPadState {
                    buttons: record.buttons,
                    stick_x: record.stick_x,
                    stick_y: record.stick_y,
                    ..RawPadState::default()
                },
                RawPadState::default(),
                RawPadState::default(),
                RawPadState::default(),
            ],
        });
        record.rng = Some(TraceRngSnapshot {
            version: 1,
            stream_count: 2,
            primary: TraceRngStream {
                id: 1,
                algorithm_version: 1,
                state: [1, 2, 3],
                call_count: record.simulation_tick,
            },
            secondary: TraceRngStream {
                id: 2,
                algorithm_version: 1,
                state: [4, 5, 6],
                call_count: record.simulation_tick + 1,
            },
        });
        record.camera = Some(TraceCamera {
            view_yaw: 0,
            controlled_yaw: 0,
            bank: 0,
            eye: [0.0, 1000.0, -2000.0],
            center: [0.0; 3],
            up: [0.0, 1.0, 0.0],
            fovy: 45.0,
        });
        record.player_action = Some(TracePlayerAction {
            procedure_id: record.player_proc_id.unwrap(),
            mode_flags: 0,
            procedure_context_raw: [0; 6],
            damage_wait_timer: 0,
            sword_at_up_time: 0,
            ice_damage_wait_timer: 0,
            sword_change_wait_timer: 0,
            under_animations: std::array::from_fn(|_| TraceAnimationLane {
                resource_id: 0xffff,
                frame: 0.0,
                rate: 0.0,
            }),
            upper_animations: std::array::from_fn(|_| TraceAnimationLane {
                resource_id: 0xffff,
                frame: 0.0,
                rate: 0.0,
            }),
            do_status: 0,
            talk_partner: None,
            grabbed_actor: None,
        });
        record.player_background_collision = Some(TracePlayerBackgroundCollision {
            flags: 1 << 15,
            ground_height: -1.0e9,
            roof_height: 1.0e9,
            water_height: -1.0e9,
            ground_bg_index: None,
            ground_poly_index: None,
            ground_owner_session_process_id: None,
            ground_plane: [0.0; 4],
            ground_identity_present: false,
            roof_bg_index: None,
            roof_poly_index: None,
            roof_owner_session_process_id: None,
            roof_identity_present: false,
            water_bg_index: None,
            water_poly_index: None,
            water_owner_session_process_id: None,
            water_identity_present: false,
            walls: std::array::from_fn(|_| crate::trace::TraceCollisionWall {
                identity_present: false,
                bg_index: None,
                poly_index: None,
                owner_session_process_id: None,
                angle_y: 0,
                flags: 0,
            }),
            old_position: record.position,
            resolved_frame_displacement: [1.0, 0.0, 0.0],
            final_position: record.position,
            solver: None,
        });
        record.player_collision_surfaces = Some(TracePlayerCollisionSurfaces {
            flags: 1,
            link_room: Some(1),
            identity_count: 0,
            backing_count: 0,
            destination_count: 0,
            raw_link_exit: 0x3f,
            pending_match_mask: 0,
            surfaces: [
                empty_surface(TraceCollisionSurfaceKind::Ground, 0),
                empty_surface(TraceCollisionSurfaceKind::Roof, 0),
                empty_surface(TraceCollisionSurfaceKind::Water, 0),
                empty_surface(TraceCollisionSurfaceKind::Wall, 0),
                empty_surface(TraceCollisionSurfaceKind::Wall, 1),
                empty_surface(TraceCollisionSurfaceKind::Wall, 2),
            ],
        });
        record.goal_progress = Some(TraceGoalProgress {
            configured: true,
            reached: false,
            authored: true,
            goal_name_hash: Some(0x1234_5678),
            requested_count: 3,
            hit_count: u16::try_from(record.tape_frame.unwrap_or_default())
                .unwrap_or(u16::MAX)
                .min(2),
            stable_ticks: 0,
            consecutive_ticks: 0,
            sequence_steps: 0,
            sequence_next_step: 0,
            sequence_within_ticks: 0,
            sequence_elapsed_ticks: 0,
            first_hit_tick: None,
        });
    }
    (trace, tape)
}

fn config(start_tape_frame: u64, end_tape_frame: u64) -> ExploratoryExtractConfig {
    ExploratoryExtractConfig {
        episode_digest: Digest([0x55; 32]),
        start_tape_frame,
        end_tape_frame,
        start_reference: None,
        terminal_reference: None,
        end_is_terminal: true,
    }
}

#[test]
fn aligns_action_with_prior_and_post_tick_records() {
    let (trace, tape) = fixture();
    let corpus = extract_exploratory(&trace, &tape, config(1, 2)).unwrap();
    assert_eq!(corpus.transitions.len(), 2);
    assert_eq!(corpus.transitions[0].state[17], 10.0 / 8192.0);
    assert_eq!(corpus.transitions[0].next_state[17], 20.0 / 8192.0);
    assert_eq!(corpus.transitions[0].action.action_id, 1);
    assert_eq!(corpus.transitions[1].state[17], 20.0 / 8192.0);
    assert_eq!(corpus.transitions[1].next_state[17], 30.0 / 8192.0);
    assert_eq!(corpus.transitions[1].action.action_id, 39);
    assert!(!corpus.transitions[0].terminal);
    assert!(corpus.transitions[1].terminal);
}

#[test]
fn movement_state_v1_rejects_scene_exit_v2_semantics() {
    let (mut trace, tape) = fixture();
    trace.version = 2;
    trace.channel_formats.insert(
        TraceChannel::SceneExit,
        crate::trace::TraceChannelWireFormat {
            version: 2,
            stride: 88,
        },
    );
    let error = extract_exploratory(&trace, &tape, config(1, 2)).unwrap_err();
    assert!(matches!(
        error,
        OfflineRlError::UnsupportedObservationChannelFormat {
            channel: "scene_exit",
            expected_version: 1,
            expected_stride: 24,
            actual_version: Some(2),
            actual_stride: Some(88),
        }
    ));
}

#[test]
fn movement_state_v1_rejects_collision_surface_semantics() {
    let (mut trace, tape) = fixture();
    trace.version = 2;
    trace.channel_formats.insert(
        TraceChannel::PlayerCollisionSurfaces,
        crate::trace::TraceChannelWireFormat {
            version: 1,
            stride: 496,
        },
    );
    let error = extract_exploratory(&trace, &tape, config(1, 2)).unwrap_err();
    assert!(matches!(
        error,
        OfflineRlError::UnsupportedObservationChannel {
            channel: "player_collision_surfaces"
        }
    ));
}

#[test]
fn movement_state_v2_authenticates_spec_and_masks_semantic_absence() {
    let (trace, tape) = fixture_v2();
    let corpus = extract_exploratory_v2(&trace, &tape, config(1, 2)).unwrap();
    let spec = movement_state_v2_spec();
    assert_eq!(corpus.feature_schema, spec.digest().unwrap());
    assert_eq!(corpus.feature_count, spec.feature_count());
    assert_eq!(corpus.transitions.len(), 2);
    let state = &corpus.transitions[0].state;
    assert_eq!(state[38], 0.0, "event hash presence mask");
    assert_eq!(&state[39..41], &[0.0, 0.0]);
    assert_eq!(state[45], 0.0, "scene-exit presence mask");
    assert_eq!(&state[46..54], &[0.0; 8]);
    assert_eq!(
        corpus.transitions[0].reward,
        -1.0 + GOAL_PROGRESS_STEP_REWARD_V2
    );
    assert_eq!(
        corpus.transitions[1].reward,
        -1.0 + GOAL_PROGRESS_STEP_REWARD_V2
    );
    assert!(state.iter().all(|value| value.is_finite()));
}

#[test]
fn movement_state_v2_rejects_regressing_authenticated_progress() {
    let (mut trace, tape) = fixture_v2();
    trace.records[1].goal_progress.as_mut().unwrap().hit_count = 2;
    trace.records[2].goal_progress.as_mut().unwrap().hit_count = 1;
    let error = extract_exploratory_v2(&trace, &tape, config(2, 2)).unwrap_err();
    assert!(matches!(
        error,
        OfflineRlError::InvalidGoalProgress { frame: 2, .. }
    ));
}

#[test]
fn movement_state_v2_distinguishes_absent_from_unavailable() {
    let (mut trace, tape) = fixture_v2();
    trace.records[0]
        .channel_status
        .insert(TraceChannel::SceneExit, TraceChannelStatus::Unavailable);
    let error = extract_exploratory_v2(&trace, &tape, config(1, 1)).unwrap_err();
    assert!(matches!(
        error,
        OfflineRlError::MissingObservationChannel {
            frame: 0,
            channel: "scene_exit",
            status: Some(TraceChannelStatus::Unavailable),
        }
    ));
}

#[test]
fn movement_state_v2_rejects_channel_format_drift() {
    let (mut trace, tape) = fixture_v2();
    trace
        .channel_formats
        .get_mut(&TraceChannel::PlayerCollisionSurfaces)
        .unwrap()
        .stride = 495;
    let error = extract_exploratory_v2(&trace, &tape, config(1, 1)).unwrap_err();
    assert!(matches!(
        error,
        OfflineRlError::UnsupportedObservationChannelFormat {
            channel: "player_collision_surfaces",
            expected_version: 1,
            expected_stride: 496,
            actual_version: Some(1),
            actual_stride: Some(495),
        }
    ));
}

#[test]
fn preserves_supplied_endpoint_references() {
    let (trace, tape) = fixture();
    let start = StateReference {
        kind: StateReferenceKind::Snapshot,
        digest: Digest([0x11; 32]),
    };
    let end = StateReference {
        kind: StateReferenceKind::Boundary,
        digest: Digest([0x22; 32]),
    };
    let corpus = extract_exploratory(
        &trace,
        &tape,
        ExploratoryExtractConfig {
            episode_digest: Digest([0x55; 32]),
            start_reference: Some(start),
            terminal_reference: Some(end),
            ..config(1, 2)
        },
    )
    .unwrap();
    assert_eq!(corpus.transitions[0].source, start);
    assert_eq!(corpus.transitions[1].next, end);
}

#[test]
fn crop_terminal_is_explicit_and_references_are_episode_scoped() {
    let (trace, tape) = fixture();
    let mut first_config = config(1, 1);
    first_config.end_is_terminal = false;
    let first = extract_exploratory(&trace, &tape, first_config).unwrap();
    assert!(!first.transitions[0].terminal);

    let mut second_config = first_config;
    second_config.episode_digest = Digest([0x66; 32]);
    let second = extract_exploratory(&trace, &tape, second_config).unwrap();
    assert_ne!(first.transitions[0].source, second.transitions[0].source);

    let mut missing = first_config;
    missing.episode_digest = Digest::ZERO;
    assert!(matches!(
        extract_exploratory(&trace, &tape, missing),
        Err(OfflineRlError::MissingEpisodeDigest)
    ));
}

#[test]
fn held_b_remains_a_valid_controller_state() {
    let (mut trace, tape) = fixture();
    trace.records[1].buttons = BUTTON_B;
    let corpus = extract_exploratory(&trace, &tape, config(2, 2)).unwrap();
    assert_eq!(corpus.transitions[0].action.action_id, 39);
}

#[test]
fn rejects_wrong_tape_even_when_both_actions_are_catalogued() {
    let (trace, mut tape) = fixture();
    tape.frames[1] = frame(127, 0, 0);
    assert!(matches!(
        extract_exploratory(&trace, &tape, config(1, 1)),
        Err(OfflineRlError::AppliedInputMismatch {
            frame: 1,
            expected_stick_x: 72,
            expected_stick_y: 0,
            actual_stick_x: 0,
            actual_stick_y: 72,
            ..
        })
    ));
}

#[test]
fn rejects_post_tick_button_mismatch() {
    let (mut trace, tape) = fixture();
    trace.records[2].buttons = 0;
    assert!(matches!(
        extract_exploratory(&trace, &tape, config(2, 2)),
        Err(OfflineRlError::AppliedInputMismatch {
            frame: 2,
            expected_buttons: BUTTON_B,
            actual_buttons: 0,
            ..
        })
    ));
}

#[test]
fn categorical_feature_indices_are_unique_and_in_range() {
    let mut indices = MOVEMENT_CATEGORICAL_FEATURES_V1.to_vec();
    assert!(
        indices
            .iter()
            .all(|index| *index < MOVEMENT_FEATURE_COUNT_V1 as usize)
    );
    indices.sort_unstable();
    indices.dedup();
    assert_eq!(indices.len(), MOVEMENT_CATEGORICAL_FEATURES_V1.len());
}

#[test]
fn quantizes_arbitrary_stick_and_a_b_combinations_without_losing_raw_parameters() {
    let (mut trace, mut tape) = fixture();
    tape.frames[1].pads[0].stick_x = 70;
    tape.frames[1].pads[0].stick_y = 127;
    tape.frames[1].pads[0].buttons = BUTTON_A | BUTTON_B;
    let clamped = pad_clamp_main_stick(70, 127);
    trace.records[1].stick_x = clamped.0;
    trace.records[1].stick_y = clamped.1;
    trace.records[1].buttons = BUTTON_A | BUTTON_B;
    let corpus = extract_exploratory(&trace, &tape, config(1, 1)).unwrap();
    let action = &corpus.transitions[0].action;
    assert_eq!(action.action_id, 3 * 17 + 2);
    assert_eq!(action.parameters, [70, 127, (BUTTON_A | BUTTON_B) as i16]);
}

#[test]
fn rejects_buttons_outside_the_movement_catalog() {
    let (trace, mut tape) = fixture();
    tape.frames[1].pads[0].buttons = 0x1000;
    assert!(matches!(
        extract_exploratory(&trace, &tape, config(1, 1)),
        Err(OfflineRlError::UnsupportedAction { frame: 1, .. })
    ));
}

#[test]
fn every_v2_action_has_a_canonical_pad_in_its_own_class() {
    for action_id in 0..68 {
        let pad = canonical_movement_pad_v2(action_id).unwrap();
        let frame = InputFrame {
            owned_ports: 1,
            pads: [
                pad,
                RawPadState {
                    connected: false,
                    error: -1,
                    ..RawPadState::default()
                },
                RawPadState {
                    connected: false,
                    error: -1,
                    ..RawPadState::default()
                },
                RawPadState {
                    connected: false,
                    error: -1,
                    ..RawPadState::default()
                },
            ],
            ..InputFrame::default()
        };
        assert_eq!(
            classify_action(&frame, 1, MovementActionSchema::V2)
                .unwrap()
                .action_id,
            action_id
        );
    }
    assert!(canonical_movement_pad_v2(68).is_none());
}

#[test]
fn v3_is_an_append_only_l_targeting_expansion() {
    assert_ne!(
        movement_action_schema_digest_v2(),
        movement_action_schema_digest_v3()
    );
    for action_id in 0..MOVEMENT_ACTION_COUNT_V2 {
        let legacy = canonical_movement_pad_v2(action_id).unwrap();
        assert_eq!(canonical_movement_pad_v3(action_id), Some(legacy));
        assert_eq!(movement_action_id_v2(legacy), Some(action_id));
        assert_eq!(movement_action_id_v3(legacy), Some(action_id));
    }
    for action_id in MOVEMENT_ACTION_COUNT_V2..MOVEMENT_ACTION_COUNT_V3 {
        let pad = canonical_movement_pad_v3(action_id).unwrap();
        assert_ne!(pad.buttons & BUTTON_L, 0);
        assert_eq!(movement_action_id_v3(pad), Some(action_id));
        assert_eq!(movement_action_id_v2(pad), None);
    }
    assert!(canonical_movement_pad_v3(MOVEMENT_ACTION_COUNT_V3).is_none());
}

#[test]
fn v3_classification_accepts_targeting_without_changing_v2_decoding() {
    let mut frame = fixture().1.frames[1].clone();
    frame.pads[0].stick_y = -127;
    frame.pads[0].buttons = BUTTON_A | BUTTON_L;

    let legacy_error = classify_action(&frame, 1, MovementActionSchema::V2).unwrap_err();
    assert!(
        matches!(
            legacy_error,
            OfflineRlError::UnsupportedAction { frame: 1, .. }
        ),
        "unexpected legacy classification error: {legacy_error:?}"
    );
    let action = classify_action(&frame, 1, MovementActionSchema::V3).unwrap();
    assert_eq!(action.action_id, 5 * 17 + 9);
    assert_eq!(action.macro_kind, 3);
}

#[test]
fn rejects_reactive_frame_and_trace_gap() {
    let (trace, mut tape) = fixture();
    tape.frames[1].wait_condition = WaitCondition::NameEntryActive;
    tape.frames[1].wait_timeout_ticks = 10;
    let config = config(1, 1);
    assert!(matches!(
        extract_exploratory(&trace, &tape, config),
        Err(OfflineRlError::ReactiveFrame(1))
    ));

    tape.frames[1].wait_condition = WaitCondition::None;
    tape.frames[1].wait_timeout_ticks = 0;
    let mut trace = trace;
    trace.records[1].simulation_tick += 1;
    trace.records[1].boundary_index += 1;
    assert!(matches!(
        extract_exploratory(&trace, &tape, config),
        Err(OfflineRlError::DiscontinuousTrace { .. })
    ));
}

#[test]
fn rejects_capacity_exhaustion_and_zero_start() {
    let (mut trace, tape) = fixture();
    trace.capacity_exhausted = true;
    let valid_config = config(1, 1);
    assert!(matches!(
        extract_exploratory(&trace, &tape, valid_config),
        Err(OfflineRlError::CapacityExhausted)
    ));
    trace.capacity_exhausted = false;
    assert!(matches!(
        extract_exploratory(
            &trace,
            &tape,
            ExploratoryExtractConfig {
                start_tape_frame: 0,
                ..config(1, 1)
            }
        ),
        Err(OfflineRlError::InvalidRange { .. })
    ));
}
