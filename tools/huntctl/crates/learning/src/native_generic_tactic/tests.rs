use super::*;
use crate::fqi::FqiConfig;
use crate::option_values::{OptionValueConfig, OptionValueSample};
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;

fn observation(tick: u64, position: [f32; 3]) -> NativeTacticObservation {
    NativeTacticObservation {
        boundary_index: 4 + tick,
        simulation_tick: 100 + tick,
        tape_frame: 20 + tick,
        state_identity: [tick as u8; 16],
        stage: "F_SP104".into(),
        room: 1,
        player_position_f32_bits: position.map(f32::to_bits),
        player_yaw: 0,
        player_procedure: 7,
        player_mode_flags: 3,
        player_contacts: 1,
        camera_yaw_radians_f32_bits: Some(0.0_f32.to_bits()),
        action_lanes: Vec::new(),
        actor_set_complete: true,
        actors: Vec::new(),
    }
}

fn model(candidate: &NativeGenericTacticCandidate) -> OptionValueModel {
    let samples = [0.0_f32, 1.0]
        .into_iter()
        .enumerate()
        .map(|(index, state)| OptionValueSample {
            action: candidate.descriptor.clone(),
            state: vec![state],
            duration_ticks: 3,
            reward: 1.0,
            next_state: vec![state + 1.0],
            terminal: true,
            before_state_sha256: Digest([index as u8 + 3; 32]),
            after_state_sha256: Digest([index as u8 + 4; 32]),
            source_checkpoint_sha256: Digest([index as u8 + 5; 32]),
            next_checkpoint_sha256: Digest([index as u8 + 6; 32]),
            realized_tape_range: TapeRange {
                start_frame: 0,
                end_frame_exclusive: 3,
            },
            realized_tape_sha256: Digest([index as u8 + 1; 32]),
        })
        .collect::<Vec<_>>();
    OptionValueModel::fit(
        1,
        &samples,
        &[1, 2],
        &OptionValueConfig {
            fitted_q: FqiConfig {
                iterations: 2,
                trees_per_action: 2,
                bootstrap: false,
                seed: 4,
                ..FqiConfig::default()
            },
        },
    )
    .unwrap()
}

#[test]
fn projects_a_post_simulation_row_onto_the_next_tactic_boundary() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let step = &shard.episodes[0].steps[0];
    let before = NativeTacticObservation::from_native(&step.pre_input).unwrap();
    let after =
        NativeTacticObservation::from_post_simulation_boundary(&step.post_simulation).unwrap();

    assert_eq!(after.boundary_index, before.boundary_index + 1);
    assert_eq!(after.simulation_tick, before.simulation_tick + 1);
    assert_eq!(after.tape_frame, before.tape_frame + 1);
    assert_eq!(after.state_identity, step.post_simulation.state_identity);
}

#[test]
fn policy_selects_seek_and_records_every_native_query_and_pad() {
    let plan = NativeGenericTacticPlan::new(
        GenericTactic::SeekCoordinate {
            coordinate_f32_bits: [3.0_f32.to_bits(), 0.0_f32.to_bits(), 0.0_f32.to_bits()],
            tolerance_f32_bits: 0.25_f32.to_bits(),
            magnitude: 100,
        },
        4,
    );
    let candidate = NativeGenericTacticCandidate::new("seek".into(), plan).unwrap();
    let observations = [
        observation(0, [0.0, 0.0, 0.0]),
        observation(1, [1.5, 0.0, 0.0]),
        observation(2, [3.0, 0.0, 0.0]),
    ];
    let result = select_and_execute_generic(
        &model(&candidate),
        &[0.0],
        &[candidate],
        &InputTape::default(),
        &observations,
    )
    .unwrap();
    assert_eq!(result.execution.duration.realized_ticks, 3);
    assert_eq!(result.execution.end_reason, OptionEndReason::Terminated);
    assert_eq!(
        result.queries.len(),
        result.execution.emitted_raw_actions.len()
    );
    assert_eq!(result.tape.frames, result.execution.emitted_raw_actions);
    assert_eq!(result.tape.frames[0].pads[0].stick_x, -100);
    assert_eq!(result.tape.frames[0].pads[0].stick_y, 0);
    assert!(
        result
            .queries
            .iter()
            .all(|query| query.queried_fields.contains(&"player_position".into()))
    );
    assert!(!result.gameplay_write_authority);
    assert!(!result.terminal_authority);
    result
        .execution
        .validate_against_tape(&result.tape)
        .unwrap();
}

#[test]
fn maintained_heading_uses_the_same_main_stick_axis_as_world_seek() {
    let plan = NativeGenericTacticPlan::new(
        GenericTactic::MaintainRelativeHeading {
            heading_radians_f32_bits: std::f32::consts::FRAC_PI_2.to_bits(),
            magnitude: 100,
        },
        1,
    );
    let (frames, _, reason) = realize(&plan, &[observation(0, [0.0; 3])]).unwrap();

    assert_eq!(reason, OptionEndReason::MaximumDuration);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].pads[0].stick_x, -100);
    assert_eq!(frames[0].pads[0].stick_y, 0);
}

#[test]
fn coordinate_sequence_advances_without_exposing_private_state() {
    let plan = NativeGenericTacticPlan::new(
        GenericTactic::SeekCoordinateSequence {
            coordinates_f32_bits: vec![
                [1.0_f32.to_bits(), 0.0_f32.to_bits(), 0.0_f32.to_bits()],
                [3.0_f32.to_bits(), 0.0_f32.to_bits(), 0.0_f32.to_bits()],
            ],
            intermediate_tolerance_f32_bits: 0.25_f32.to_bits(),
            final_tolerance_f32_bits: 0.25_f32.to_bits(),
            stall_grace_ticks: 4,
            stationary_window_ticks: 2,
            stationary_window_distance_f32_bits: 2.0_f32.to_bits(),
            magnitude: 100,
        },
        8,
    );
    let observations = [
        observation(0, [0.0, 0.0, 0.0]),
        observation(1, [1.0, 0.0, 0.0]),
        observation(2, [2.0, 0.0, 0.0]),
        observation(3, [3.0, 0.0, 0.0]),
    ];
    let (frames, queries, reason) = realize(&plan, &observations).unwrap();
    assert_eq!(reason, OptionEndReason::Terminated);
    assert_eq!(frames.len(), 4);
    assert_eq!(frames[0].pads[0].stick_x, -100);
    assert_eq!(frames[1].pads[0].stick_x, -100);
    assert_eq!(frames[3].pads[0].stick_x, 0);
    assert!(!queries[0].target_reached);
    assert!(!queries[1].target_reached);
    assert!(queries[3].target_reached);
}

#[test]
fn coordinate_sequence_resumes_at_the_nearest_forward_waypoint() {
    let plan = NativeGenericTacticPlan::new(
        GenericTactic::SeekCoordinateSequence {
            coordinates_f32_bits: vec![
                [1.0_f32.to_bits(), 0.0_f32.to_bits(), 0.0_f32.to_bits()],
                [3.0_f32.to_bits(), 0.0_f32.to_bits(), 0.0_f32.to_bits()],
                [5.0_f32.to_bits(), 0.0_f32.to_bits(), 0.0_f32.to_bits()],
            ],
            intermediate_tolerance_f32_bits: 0.25_f32.to_bits(),
            final_tolerance_f32_bits: 0.25_f32.to_bits(),
            stall_grace_ticks: 4,
            stationary_window_ticks: 2,
            stationary_window_distance_f32_bits: 2.0_f32.to_bits(),
            magnitude: 100,
        },
        8,
    );
    let (frames, queries, reason) = realize(
        &plan,
        &[
            observation(0, [2.25, 0.0, 0.0]),
            observation(1, [3.0, 0.0, 0.0]),
            observation(2, [5.0, 0.0, 0.0]),
        ],
    )
    .unwrap();

    assert_eq!(reason, OptionEndReason::Terminated);
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].pads[0].stick_x, -100);
    assert!(!queries[0].target_reached);
    assert!(queries[2].target_reached);
}

#[test]
fn coordinate_sequence_does_not_backtrack_after_passing_a_distant_waypoint() {
    let plan = NativeGenericTacticPlan::new(
        GenericTactic::SeekCoordinateSequence {
            coordinates_f32_bits: vec![
                [
                    350.0_f32.to_bits(),
                    0.0_f32.to_bits(),
                    (-10_150.0_f32).to_bits(),
                ],
                [
                    350.0_f32.to_bits(),
                    0.0_f32.to_bits(),
                    (-17_050.0_f32).to_bits(),
                ],
                [
                    (-441.0_f32).to_bits(),
                    0.0_f32.to_bits(),
                    (-19_270.0_f32).to_bits(),
                ],
            ],
            intermediate_tolerance_f32_bits: 256.0_f32.to_bits(),
            final_tolerance_f32_bits: 256.0_f32.to_bits(),
            stall_grace_ticks: 4,
            stationary_window_ticks: 2,
            stationary_window_distance_f32_bits: 2.0_f32.to_bits(),
            magnitude: 100,
        },
        8,
    );
    let (frames, _, _) = realize(
        &plan,
        &[
            observation(0, [506.9, 0.0, -11_545.5]),
            observation(1, [350.0, 0.0, -17_050.0]),
            observation(2, [-441.0, 0.0, -19_270.0]),
        ],
    )
    .unwrap();

    assert!(frames[0].pads[0].stick_y < 0);
}

#[test]
fn coordinate_sequence_stops_after_bounded_collision_jitter() {
    let plan = NativeGenericTacticPlan::new(
        GenericTactic::SeekCoordinateSequence {
            coordinates_f32_bits: vec![[100.0_f32.to_bits(), 0.0_f32.to_bits(), 0.0_f32.to_bits()]],
            intermediate_tolerance_f32_bits: 0.25_f32.to_bits(),
            final_tolerance_f32_bits: 0.25_f32.to_bits(),
            stall_grace_ticks: 40,
            stationary_window_ticks: 16,
            stationary_window_distance_f32_bits: 16.0_f32.to_bits(),
            magnitude: 100,
        },
        64,
    );
    let observations = (0..64)
        .map(|tick| observation(tick, [tick as f32 * 0.5, 0.0, 0.0]))
        .collect::<Vec<_>>();
    let (frames, queries, reason) = realize(&plan, &observations).unwrap();

    assert_eq!(reason, OptionEndReason::Completed);
    assert_eq!(frames.len(), 40);
    assert!(queries.iter().all(|query| !query.target_reached));
}

#[test]
fn portable_actor_target_refuses_ambiguous_or_truncated_absence() {
    let selector = PlacedActorSelector {
        stage: "F_SP104".into(),
        home_room: 1,
        set_id: 2,
        actor_name: 3,
    };
    let plan = NativeGenericTacticPlan::new(
        GenericTactic::SeekActor {
            target: selector.clone(),
            tolerance_f32_bits: 1.0_f32.to_bits(),
            magnitude: 80,
        },
        1,
    );
    let mut observed = observation(0, [0.0; 3]);
    observed.actor_set_complete = false;
    assert_eq!(
        realize(&plan, &[observed.clone()]).unwrap_err(),
        NativeTacticError::TargetUnknown
    );
    let actor = NativeTacticActor {
        selector,
        runtime_generation: 10,
        current_room: 1,
        position_f32_bits: [1.0_f32.to_bits(), 0.0_f32.to_bits(), 0.0_f32.to_bits()],
    };
    observed.actor_set_complete = true;
    observed.actors = vec![actor.clone(), actor];
    assert_eq!(
        realize(&plan, &[observed]).unwrap_err(),
        NativeTacticError::TargetAmbiguous
    );
}

#[test]
fn synchronizes_one_button_edge_to_observed_action_phase() {
    let plan = NativeGenericTacticPlan::new(
        GenericTactic::SynchronizeButtonEdge {
            button_mask: 0x0100,
            procedure_id: 7,
            animation_resource_id: 12,
            phase_f32_bits: 4.0_f32.to_bits(),
            movement_heading_radians_f32_bits: None,
            movement_magnitude: 0,
        },
        4,
    );
    let mut observations = [observation(0, [0.0; 3]), observation(1, [0.0; 3])];
    observations[0].action_lanes = vec![NativeTacticActionLane {
        resource_id: 12,
        frame_f32_bits: 3.5_f32.to_bits(),
    }];
    observations[1].action_lanes = vec![NativeTacticActionLane {
        resource_id: 12,
        frame_f32_bits: 4.25_f32.to_bits(),
    }];
    let (frames, queries, reason) = realize(&plan, &observations).unwrap();
    assert_eq!(reason, OptionEndReason::Terminated);
    assert_eq!(frames[0].pads[0].buttons, 0);
    assert_eq!(frames[1].pads[0].buttons, 0x0100);
    assert_eq!(queries[1].action_lane.unwrap().resource_id, 12);
}

#[test]
fn mines_initiation_and_termination_without_route_coordinates() {
    let mut success_start = observation(0, [12.0, 0.0, -3.0]);
    success_start.player_procedure = 7;
    let mut success_end = observation(1, [99.0, 0.0, 42.0]);
    success_end.player_procedure = 9;
    let mut failure_start = observation(2, [-400.0, 0.0, 8.0]);
    failure_start.player_procedure = 5;
    let failure_end = failure_start.clone();
    let mined = mine_tactic_conditions(&[
        TacticExperience {
            successful: true,
            start: success_start,
            end: success_end,
            end_reason: OptionEndReason::Terminated,
        },
        TacticExperience {
            successful: false,
            start: failure_start,
            end: failure_end,
            end_reason: OptionEndReason::MaximumDuration,
        },
    ])
    .unwrap();
    assert!(
        mined
            .initiation
            .contains(&MinedObservationPredicate::PlayerProcedure(7))
    );
    assert!(
        mined
            .termination
            .contains(&MinedObservationPredicate::PlayerProcedure(9))
    );
    assert!(!mined.coordinate_literals_embedded);
    assert!(!mined.published_procedures_embedded);
}
