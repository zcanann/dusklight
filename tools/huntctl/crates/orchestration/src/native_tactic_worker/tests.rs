use super::*;
use crate::native_suffix_result::ValidatedNativeSuffixCandidate;
use dusklight_control::controller_program::ControllerProgram;
use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
use dusklight_control::option_execution::{OptionParameter, OptionType};
use dusklight_learning::tactic_asset::{TacticAssetSource, TacticCatalogEntry};
use dusklight_learning::tactic_exploration::TacticSelectionReason;
use dusklight_learning::{
    native_generic_tactic::{
        GenericTactic, NATIVE_GENERIC_TACTIC_SCHEMA_V1, NativeGenericTacticPlan,
    },
    tactic_exploration::TACTIC_EXPLORATION_SCHEMA_V1,
};
use std::collections::BTreeMap;

#[test]
fn authenticated_terminal_can_cancel_a_static_tactic_before_its_minimum_duration() {
    let option_tape = InputTape {
        frames: vec![
            InputFrame {
                owned_ports: 1,
                ..InputFrame::default()
            };
            8
        ],
        ..InputTape::default()
    };
    let execution = OptionExecution::capture(
        "macro/example".into(),
        OptionType::Custom("macro".into()),
        BTreeMap::new(),
        8,
        8,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &option_tape,
        TapeRange {
            start_frame: 0,
            end_frame_exclusive: 8,
        },
    )
    .unwrap();
    let prepared = PreparedNativeTactic {
        option_tape,
        execution,
        duration: TacticDurationBounds {
            minimum_ticks: 8,
            maximum_ticks: 8,
        },
    };

    let (end_reason, cancellation_conditions) = realized_option_end(&prepared, 6, true).unwrap();

    assert_eq!(
        end_reason,
        OptionEndReason::Cancelled { condition_index: 0 }
    );
    assert_eq!(
        cancellation_conditions,
        vec![OptionCondition::TargetReached {
            target: "authenticated_terminal".into(),
        }]
    );
    let realized_tape = InputTape {
        frames: prepared.option_tape.frames[..6].to_vec(),
        ..InputTape::default()
    };
    OptionExecution::capture(
        "macro/example".into(),
        OptionType::Custom("macro".into()),
        BTreeMap::new(),
        8,
        8,
        OptionCondition::DurationElapsed,
        cancellation_conditions,
        end_reason,
        &realized_tape,
        TapeRange {
            start_frame: 0,
            end_frame_exclusive: 6,
        },
    )
    .unwrap();
}

#[test]
fn selected_static_tactic_becomes_one_exact_variable_horizon_batch() {
    let catalog = TacticAssetCatalog::new(vec![
        TacticCatalogEntry::new(
            "shield",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield { frames: 3 })),
        )
        .unwrap(),
    ])
    .unwrap();
    let descriptor = catalog
        .entry("shield")
        .unwrap()
        .description()
        .option
        .clone();
    let selected = SelectedTactic {
        schema: dusklight_learning::tactic_exploration::TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: Digest([1; 32]),
        decision_index: 4,
        descriptor,
        reason: TacticSelectionReason::Greedy,
        exploration_draw: 900_000,
    };
    let PreparedNativeExecution::Static(prepared) =
        prepare_selected(&selected, &catalog, &[]).unwrap()
    else {
        panic!("shield must remain static");
    };
    let identity = NativeSuffixWorkerIdentity {
        executable_sha256: Digest([1; 32]),
        game_data_sha256: Digest([2; 32]),
        input_tape_sha256: Digest([3; 32]),
        milestone_program_sha256: Digest([4; 32]),
        card_fixture_sha256: Digest([5; 32]),
        world_context_sha256: Digest([6; 32]),
        source_frame: 440,
        source_boundary_fingerprint: "7".repeat(32),
        checkpoint_validation_kind: "recorded_replay_window".into(),
        checkpoint_validation_ticks: 2,
        maximum_ticks: 99,
        terminal: crate::native_suffix_result::NativeTerminalBinding {
            goal: "goal".into(),
            program_sha256: Digest([8; 32]),
            definition_sha256: Digest([9; 32]),
        },
    };
    let batch = tactic_batch(
        &identity,
        &selected,
        &prepared.option_tape,
        None,
        NativeTacticCheckpointRetention::PortableImage,
    )
    .unwrap();
    assert_eq!(batch.maximum_ticks, 3);
    assert_eq!(batch.candidates.len(), 1);
    assert_eq!(
        batch.candidates[0]
            .actions
            .iter()
            .map(|action| match action {
                MacroAction::PadRun { frames, .. } => *frames,
                _ => 0,
            })
            .sum::<u32>(),
        3
    );
    assert!(!batch.verify_state_hashes);

    let checkpoint_source = NativeTacticCheckpointSource {
        restore_identity: "a".repeat(32),
        boundary_fingerprint: "b".repeat(32),
        route_ticks: 40,
        storage: NativeTacticCheckpointStorage::PortableImage,
    };
    let restored = tactic_batch(
        &identity,
        &selected,
        &prepared.option_tape,
        Some(&checkpoint_source),
        NativeTacticCheckpointRetention::PortableImage,
    )
    .unwrap();
    let cache = restored
        .checkpoint_cache
        .expect("native tactic batches always declare their cache policy");
    assert_eq!(restored.schema, NATIVE_CACHED_SUFFIX_BATCH_SCHEMA);
    assert_eq!(restored.source_frame, 440);
    assert_eq!(restored.source_boundary_fingerprint, "b".repeat(32));
    assert_eq!(restored.maximum_ticks, 3);
    assert_eq!(
        cache.source_identity.as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert_eq!(cache.source_route_ticks, 40);
    assert!(cache.retain_candidate_checkpoints);
    assert!(!cache.retain_live_endpoint);

    let live = tactic_batch(
        &identity,
        &selected,
        &prepared.option_tape,
        Some(&checkpoint_source),
        NativeTacticCheckpointRetention::LiveEndpoint,
    )
    .unwrap();
    let live_cache = live
        .checkpoint_cache
        .expect("live batches declare their bounded endpoint policy");
    assert!(!live_cache.retain_candidate_checkpoints);
    assert!(live_cache.retain_live_endpoint);

    let evaluation_only = tactic_batch(
        &identity,
        &selected,
        &prepared.option_tape,
        Some(&checkpoint_source),
        NativeTacticCheckpointRetention::None,
    )
    .unwrap();
    let evaluation_cache = evaluation_only
        .checkpoint_cache
        .expect("evaluation batches still declare their cache policy");
    assert!(!evaluation_cache.retain_candidate_checkpoints);
    assert!(!evaluation_cache.retain_live_endpoint);
    assert!(
        candidate_prefix_frames(
            &InputTape {
                frames: vec![InputFrame::default(); 443],
                ..InputTape::default()
            },
            440,
            Some(&checkpoint_source),
        )
        .is_empty()
    );
}

#[test]
fn relative_heading_becomes_one_linear_native_controller_candidate() {
    let authored_heading = 0.375_f32;
    let plan = NativeGenericTacticPlan {
        schema: NATIVE_GENERIC_TACTIC_SCHEMA_V1.into(),
        tactic: GenericTactic::MaintainRelativeHeading {
            heading_radians_f32_bits: authored_heading.to_bits(),
            magnitude: 96,
        },
        minimum_ticks: 1,
        maximum_ticks: 16,
    };
    let duration = TacticDurationBounds {
        minimum_ticks: 1,
        maximum_ticks: 16,
    };
    let program = native_generic_controller_program(&plan, duration)
        .unwrap()
        .expect("relative heading has a native controller equivalent");
    let Operation::MaintainHeading {
        frame,
        heading_radians,
        magnitude,
        ..
    } = program.layers[0].operation
    else {
        panic!("relative heading must compile to maintain-heading");
    };
    assert_eq!(frame, CoordinateFrame::Player);
    assert_eq!(heading_radians.to_bits(), (-authored_heading).to_bits());
    assert_eq!(magnitude, 96);

    let endpoint_plan = NativeGenericTacticPlan {
        tactic: GenericTactic::MaintainRelativeHeading {
            heading_radians_f32_bits: (-std::f32::consts::PI).to_bits(),
            magnitude: 96,
        },
        ..plan.clone()
    };
    let endpoint_program = native_generic_controller_program(&endpoint_plan, duration)
        .unwrap()
        .expect("the -pi catalog endpoint must remain encodable");
    let Operation::MaintainHeading {
        heading_radians, ..
    } = endpoint_program.layers[0].operation
    else {
        panic!("relative heading must compile to maintain-heading");
    };
    assert_eq!(heading_radians.to_bits(), (-std::f32::consts::PI).to_bits());

    let descriptor = plan.descriptor("heading".into()).unwrap();
    let selected = SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: Digest([1; 32]),
        decision_index: 5,
        descriptor,
        reason: TacticSelectionReason::Epsilon,
        exploration_draw: 1,
    };
    let identity = NativeSuffixWorkerIdentity {
        executable_sha256: Digest([1; 32]),
        game_data_sha256: Digest([2; 32]),
        input_tape_sha256: Digest([3; 32]),
        milestone_program_sha256: Digest([4; 32]),
        card_fixture_sha256: Digest([5; 32]),
        world_context_sha256: Digest([6; 32]),
        source_frame: 440,
        source_boundary_fingerprint: "7".repeat(32),
        checkpoint_validation_kind: "recorded_replay_window".into(),
        checkpoint_validation_ticks: 2,
        maximum_ticks: 99,
        terminal: crate::native_suffix_result::NativeTerminalBinding {
            goal: "goal".into(),
            program_sha256: Digest([8; 32]),
            definition_sha256: Digest([9; 32]),
        },
    };
    let prefix = vec![
        InputFrame {
            owned_ports: 1,
            ..InputFrame::default()
        };
        3
    ];
    let batch = tactic_controller_batch(
        &identity,
        &selected,
        &prefix,
        &program,
        None,
        NativeTacticCheckpointRetention::PortableImage,
    )
    .unwrap();

    assert_eq!(batch.schema, NATIVE_CACHED_SUFFIX_BATCH_SCHEMA);
    assert_eq!(batch.maximum_ticks, 19);
    assert_eq!(batch.candidates.len(), 1);
    assert_eq!(
        batch.candidates[0]
            .actions
            .iter()
            .map(|action| match action {
                MacroAction::PadRun { frames, .. } => *frames,
                _ => 0,
            })
            .sum::<u32>(),
        3
    );
    assert!(batch.candidates[0].controller_program_hex.is_some());
}

#[test]
fn seek_coordinate_becomes_one_linear_native_controller_candidate() {
    let target = [-1842.25_f32, 717.0, -4739.5];
    let plan = NativeGenericTacticPlan::new(
        GenericTactic::SeekCoordinate {
            coordinate_f32_bits: target.map(f32::to_bits),
            tolerance_f32_bits: 24.0_f32.to_bits(),
            magnitude: 127,
        },
        80,
    );
    let program = native_generic_controller_program(
        &plan,
        TacticDurationBounds {
            minimum_ticks: 1,
            maximum_ticks: 80,
        },
    )
    .unwrap()
    .expect("seek-coordinate has a native controller equivalent");
    let Operation::SeekCoordinate {
        frame,
        target: actual,
        offset,
        stop_radius,
        magnitude,
        ..
    } = program.layers[0].operation
    else {
        panic!("seek coordinate must compile to the same controller operation");
    };
    assert_eq!(frame, CoordinateFrame::World);
    assert_eq!(actual, target);
    assert_eq!(offset, [0.0; 3]);
    assert_eq!(stop_radius.to_bits(), 24.0_f32.to_bits());
    assert_eq!(magnitude, 127);
    assert_eq!(program.duration_frames, 80);
}

#[test]
fn seek_coordinate_sequence_becomes_one_linear_native_controller_candidate() {
    let coordinates = [
        [-2501.3477_f32, 717.0, -3931.8281],
        [-2534.9966, 717.0, -4164.2246],
        [-2568.6455, 717.0, -4396.6206],
        [-1842.2203, 717.0, -4739.0684],
    ];
    let plan = NativeGenericTacticPlan::new(
        GenericTactic::SeekCoordinateSequence {
            coordinates_f32_bits: coordinates
                .map(|coordinate| coordinate.map(f32::to_bits))
                .to_vec(),
            intermediate_tolerance_f32_bits: 96.0_f32.to_bits(),
            final_tolerance_f32_bits: 32.0_f32.to_bits(),
            magnitude: 127,
            stall_grace_ticks: 40,
            stationary_window_ticks: 16,
            stationary_window_distance_f32_bits: 1.0_f32.to_bits(),
        },
        40,
    );
    let program = native_generic_controller_program(
        &plan,
        TacticDurationBounds {
            minimum_ticks: 1,
            maximum_ticks: 40,
        },
    )
    .unwrap()
    .expect("a four-point coordinate sequence has a native controller equivalent");
    let Operation::SeekCoordinateSequence {
        coordinates_xz,
        intermediate_stop_radius,
        final_stop_radius,
        magnitude,
        ..
    } = &program.layers[0].operation
    else {
        panic!("coordinate sequence must compile to one sequence controller operation");
    };
    assert_eq!(
        coordinates_xz,
        &coordinates
            .iter()
            .map(|coordinate| [coordinate[0], coordinate[2]])
            .collect::<Vec<_>>()
    );
    assert_eq!(intermediate_stop_radius.to_bits(), 96.0_f32.to_bits());
    assert_eq!(final_stop_radius.to_bits(), 32.0_f32.to_bits());
    assert_eq!(*magnitude, 127);
    assert_eq!(program.duration_frames, 40);

    let mut early_stall_plan = plan.clone();
    let GenericTactic::SeekCoordinateSequence {
        stall_grace_ticks, ..
    } = &mut early_stall_plan.tactic
    else {
        unreachable!();
    };
    *stall_grace_ticks = 20;
    assert!(
        native_generic_controller_program(
            &early_stall_plan,
            TacticDurationBounds {
                minimum_ticks: 1,
                maximum_ticks: 40,
            },
        )
        .unwrap()
        .is_none(),
        "a sequence with an observable early-stall boundary must keep the Rust observation loop"
    );
    let delayed_stop_plan = NativeGenericTacticPlan {
        minimum_ticks: 2,
        ..plan.clone()
    };
    assert!(
        native_generic_controller_program(
            &delayed_stop_plan,
            TacticDurationBounds {
                minimum_ticks: 2,
                maximum_ticks: 40,
            },
        )
        .unwrap()
        .is_none(),
        "a sequence with a delayed stopping boundary must keep the Rust observation loop"
    );
    assert!(
        native_generic_controller_program_for_strategy(
            &plan,
            TacticDurationBounds {
                minimum_ticks: 1,
                maximum_ticks: 40,
            },
            NativeGenericExecutionStrategy::ProgressiveAudit,
        )
        .unwrap()
        .is_none(),
        "the audit strategy must force the progressive observation loop"
    );
}

#[test]
fn selected_native_generic_tactic_dispatches_to_the_live_stepper() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let before = FactSnapshot::from_native_learning(
        &shard.episodes[0].steps[0].pre_input,
        &[],
        None,
        Vec::new(),
    )
    .unwrap();
    let plan = NativeGenericTacticPlan {
        schema: NATIVE_GENERIC_TACTIC_SCHEMA_V1.into(),
        tactic: GenericTactic::ShortCurve {
            control: [[37, -21]; 4],
        },
        minimum_ticks: 1,
        maximum_ticks: 1,
    };
    let catalog = TacticAssetCatalog::new(vec![
        TacticCatalogEntry::new("native.curve", TacticAssetSource::NativeGenericTactic(plan))
            .unwrap(),
    ])
    .unwrap();
    let selected = SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: before.content_sha256().unwrap(),
        decision_index: 2,
        descriptor: catalog
            .entry("native.curve")
            .unwrap()
            .description()
            .option
            .clone(),
        reason: TacticSelectionReason::Greedy,
        exploration_draw: 0,
    };

    let PreparedNativeExecution::NativeGeneric {
        mut stepper,
        duration,
        ..
    } = prepare_selected(&selected, &catalog, &[]).unwrap()
    else {
        panic!("native generic tactic must keep its observation loop");
    };
    let step = stepper
        .step(before.to_native_tactic_observation().unwrap())
        .unwrap();

    assert_eq!(duration.maximum_ticks, 1);
    assert_eq!(step.frame.pads[0].stick_x, 37);
    assert_eq!(step.frame.pads[0].stick_y, -21);
    assert_eq!(step.end_reason, Some(OptionEndReason::MaximumDuration));
    assert_eq!(step.query.local_tick, 0);
}

#[test]
fn selected_reactive_controller_dispatches_to_the_observed_program_stepper() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let before = FactSnapshot::from_native_learning(
        &shard.episodes[0].steps[0].pre_input,
        &[],
        None,
        Vec::new(),
    )
    .unwrap();
    let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new(
                "controller.seek",
                TacticAssetSource::ReactiveController(
                    ControllerProgram::parse(
                        "duskcontrol 1\nframes 1\nseek coordinate replace from 0 for 1 frame world target 100 0 0 offset 0 0 0 magnitude 90 stop 1\n",
                    )
                    .unwrap(),
                ),
            )
            .unwrap(),
        ])
        .unwrap();
    let selected = SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: before.content_sha256().unwrap(),
        decision_index: 3,
        descriptor: catalog
            .entry("controller.seek")
            .unwrap()
            .description()
            .option
            .clone(),
        reason: TacticSelectionReason::Greedy,
        exploration_draw: 0,
    };
    let PreparedNativeExecution::ReactiveController {
        program,
        mut stepper,
        duration,
        ..
    } = prepare_selected(&selected, &catalog, &[]).unwrap()
    else {
        panic!("reactive controller must keep its observed program");
    };
    let step = stepper
        .step(&controller_observation_from_facts(&before).unwrap())
        .unwrap();

    assert_eq!(duration.maximum_ticks, 1);
    assert!(step.frame.is_some());
    assert_eq!(step.end, Some(ControllerRuntimeEnd::MaximumDuration));
    assert_eq!(step.query.controller_frame, 0);
    assert!(step.query.queried_fields.contains(
        &dusklight_control::controller_compilation::ControllerObservationField::PlayerPosition
    ));
    assert_eq!(program.duration_frames, 1);
    assert!(reactive_controller_uses_native_strategy(
        NativeGenericExecutionStrategy::NativeController,
        &[],
    ));
    assert!(!reactive_controller_uses_native_strategy(
        NativeGenericExecutionStrategy::ProgressiveAudit,
        &[],
    ));
    assert!(!reactive_controller_uses_native_strategy(
        NativeGenericExecutionStrategy::NativeController,
        &[OptionCondition::TargetLost {
            target: "controller_exact_actor".into(),
        }],
    ));
}

#[test]
fn native_episode_observes_the_real_stop_and_next_fact_boundary() {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../tests/fixtures/automation/native_episode_v28.dseps")
        .canonicalize()
        .unwrap();
    let bytes = fs::read(&fixture_path).unwrap();
    let shard = NativeEpisodeShard::decode(&bytes).unwrap();
    let episode = &shard.episodes[0];
    let step = &episode.steps[0];
    assert_eq!(step.chosen_pad, step.consumed_pad);

    let before =
        FactSnapshot::from_native_learning(&step.pre_input, &[], None, Vec::new()).unwrap();
    let selected = SelectedTactic {
        schema: dusklight_learning::tactic_exploration::TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: before.content_sha256().unwrap(),
        decision_index: 0,
        descriptor: dusklight_learning::option_values::OptionActionDescriptor {
            option_id: "fixture.tick".into(),
            option_type: OptionType::Neutral,
            parameters: BTreeMap::<String, OptionParameter>::new(),
        },
        reason: TacticSelectionReason::Greedy,
        exploration_draw: 999_999,
    };
    let mut frame = InputFrame {
        owned_ports: 1,
        ..InputFrame::default()
    };
    frame.pads[0] = RawPadState {
        buttons: step.chosen_pad.buttons,
        stick_x: step.chosen_pad.stick_x,
        stick_y: step.chosen_pad.stick_y,
        substick_x: step.chosen_pad.substick_x,
        substick_y: step.chosen_pad.substick_y,
        trigger_left: step.chosen_pad.trigger_left,
        trigger_right: step.chosen_pad.trigger_right,
        analog_a: step.chosen_pad.analog_a,
        analog_b: step.chosen_pad.analog_b,
        connected: step.chosen_pad.connected,
        error: step.chosen_pad.error,
    };
    let option_tape = InputTape {
        frames: vec![frame],
        ..InputTape::default()
    };
    let local_execution = OptionExecution::capture(
        selected.descriptor.option_id.clone(),
        selected.descriptor.option_type.clone(),
        selected.descriptor.parameters.clone(),
        1,
        1,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &option_tape,
        TapeRange {
            start_frame: 0,
            end_frame_exclusive: 1,
        },
    )
    .unwrap();
    let prepared = PreparedNativeTactic {
        option_tape: option_tape.clone(),
        execution: local_execution,
        duration: TacticDurationBounds {
            minimum_ticks: 1,
            maximum_ticks: 1,
        },
    };
    let request = NativeSuffixBatch {
        schema: dusklight_search::suffix_batch::NATIVE_SUFFIX_BATCH_SCHEMA.into(),
        source_frame: before.tape_frame as usize,
        source_boundary_fingerprint: "a".repeat(32),
        checkpoint_validation: NativeCheckpointValidation {
            kind: "recorded_replay_window".into(),
            ticks: 1,
        },
        maximum_ticks: 1,
        verify_state_hashes: true,
        checkpoint_cache: None,
        candidates: vec![NativeSuffixCandidate {
            id: episode.id.clone(),
            actions: pad_runs(&option_tape.frames).unwrap(),
            controller_program_hex: None,
        }],
    };
    let validated = ValidatedNativeSuffixBatch {
        restore_identity: shard.metadata.checkpoint_identity.clone(),
        checkpoint_bytes: 1,
        simulated_ticks: 1,
        restore_micros: vec![1],
        checkpoint_cache: None,
        episode_shard_path: fixture_path.to_string_lossy().into_owned(),
        candidates: vec![ValidatedNativeSuffixCandidate {
            id: episode.id.clone(),
            simulated_ticks: 1,
            first_hit_tick: episode.first_hit_tick.map(u64::from),
            state_sequence_digest: Some("b".repeat(64)),
            terminal_boundary_fingerprint: "c".repeat(32),
            behavior_sha256: Digest([7; 32]),
            retained_checkpoint: None,
        }],
    };
    let route_prefix = InputTape {
        frames: vec![InputFrame::default(); before.tape_frame as usize],
        ..InputTape::default()
    };
    let loaded_episode = load_candidate_episode(&request, &validated).unwrap();
    let outcome = observe_outcome(
        Digest([12; 32]),
        &selected,
        &before,
        &route_prefix,
        prepared,
        option_tape,
        0,
        request,
        validated,
        Some(loaded_episode),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(outcome.execution.duration.realized_ticks, 1);
    assert_eq!(
        outcome.execution.realized_tape_range,
        TapeRange {
            start_frame: before.tape_frame,
            end_frame_exclusive: before.tape_frame + 1,
        }
    );
    assert_eq!(outcome.next_facts.tape_frame, before.tape_frame + 1);
    assert_eq!(outcome.terminal, episode.success);
    assert_eq!(
        outcome.route_tape.frames.len(),
        before.tape_frame as usize + 1
    );
}
