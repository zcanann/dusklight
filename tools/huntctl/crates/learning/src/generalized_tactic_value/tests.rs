use super::*;
use crate::tactic_asset::TacticAssetAdapter;
use dusklight_control::tape::{InputFrame, InputTape};
use std::collections::BTreeMap;

fn action(
    id: &str,
    path_length: f32,
    displacement: f32,
    turn: f32,
    roll_period: Option<u64>,
    target_x: f32,
) -> OptionActionDescriptor {
    let mut parameters = BTreeMap::from([
        ("duration_ticks".into(), OptionParameter::Unsigned(160)),
        (
            "command_target_first_x".into(),
            OptionParameter::F32Bits(target_x.to_bits()),
        ),
        (
            "command_target_first_z".into(),
            OptionParameter::F32Bits(0.0_f32.to_bits()),
        ),
        (
            "command_target_last_x".into(),
            OptionParameter::F32Bits(displacement.to_bits()),
        ),
        (
            "command_target_last_z".into(),
            OptionParameter::F32Bits(0.0_f32.to_bits()),
        ),
        (
            "command_internal_path_length".into(),
            OptionParameter::F32Bits(path_length.to_bits()),
        ),
        (
            "command_internal_displacement".into(),
            OptionParameter::F32Bits(displacement.to_bits()),
        ),
        (
            "command_internal_turn_radians".into(),
            OptionParameter::F32Bits(turn.to_bits()),
        ),
        (
            "command_target_point_count".into(),
            OptionParameter::Unsigned(4),
        ),
        (
            "command_stick_magnitude".into(),
            OptionParameter::Unsigned(127),
        ),
    ]);
    if let Some(period) = roll_period {
        parameters.insert(
            "command_button_mask".into(),
            OptionParameter::Unsigned(0x0100),
        );
        parameters.insert(
            "button_pulse_period_ticks".into(),
            OptionParameter::Unsigned(period),
        );
        parameters.insert(
            "command_button_active_fraction".into(),
            OptionParameter::F32Bits((1.0 / period as f32).to_bits()),
        );
    }
    OptionActionDescriptor {
        option_id: id.into(),
        option_type: OptionType::Custom("reactive_controller".into()),
        parameters,
    }
}

fn sample(
    action: OptionActionDescriptor,
    reward: f32,
    outcome: GeneralizedTacticOutcome,
) -> GeneralizedTacticTrainingSample {
    GeneralizedTacticTrainingSample {
        state_features: vec![0.0, 1.0],
        context: GeneralizedTacticContext::default(),
        action,
        outcome: GeneralizedTacticOutcome { reward, ..outcome },
    }
}

#[test]
fn shared_multi_action_neighborhood_matches_independent_predictions() {
    let descriptors = [
        action("straight", 100.0, 100.0, 0.0, None, 100.0),
        action("curve", 120.0, 90.0, 0.5, None, 90.0),
        action("roll", 100.0, 100.0, 0.0, Some(8), 100.0),
    ];
    let samples = descriptors
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, descriptor)| GeneralizedTacticTrainingSample {
            state_features: vec![index as f32, 1.0],
            context: GeneralizedTacticContext {
                player_x: index as f32,
                ..GeneralizedTacticContext::default()
            },
            action: descriptor,
            outcome: GeneralizedTacticOutcome {
                terminal: f32::from(index == 2),
                reward: index as f32,
                duration_ticks: (index + 1) as f32,
                ..GeneralizedTacticOutcome::default()
            },
        })
        .collect::<Vec<_>>();
    let model = GeneralizedTacticValueModel::fit(&samples).unwrap();
    let state = [0.5, 1.0];
    let context = GeneralizedTacticContext {
        player_x: 0.5,
        ..GeneralizedTacticContext::default()
    };

    for estimate in model.rank(&state, &context, &descriptors).unwrap() {
        assert_eq!(
            estimate,
            model
                .predict(&state, &context, &estimate.descriptor)
                .unwrap()
        );
    }
    for estimate in model
        .rank_terminal_support(&state, &context, &descriptors)
        .unwrap()
    {
        assert_eq!(
            estimate,
            model
                .predict(&state, &context, &estimate.descriptor)
                .unwrap()
        );
    }
}

#[test]
fn semantic_weights_prevent_bitset_width_from_swamping_movement() {
    let left = vec![0.0; 33];
    let mut movement_difference = left.clone();
    movement_difference[0] = 1.0;
    let mut flag_difference = left.clone();
    flag_difference[1..].fill(1.0);
    let minimum = vec![0.0; 33];
    let range = vec![1.0; 33];
    let mut weights = vec![1.0 / 32.0; 33];
    weights[0] = 1.0;

    let movement =
        weighted_normalized_distance(&left, &movement_difference, &minimum, &range, &weights);
    let flags = weighted_normalized_distance(&left, &flag_difference, &minimum, &range, &weights);
    assert!((movement - flags).abs() < 1.0e-6);
}

#[test]
fn state_range_calibration_resists_single_extreme_outlier() {
    let mut rows = (0..20).map(|value| vec![value as f32]).collect::<Vec<_>>();
    rows.push(vec![1.0e9]);
    let (minimum, range) = feature_ranges(rows.iter().map(Vec::as_slice), 1);

    assert_eq!(minimum, vec![1.0]);
    assert_eq!(range, vec![18.0]);
}

#[test]
fn terminal_support_excludes_censored_and_cyclic_components() {
    let digest = |byte| Digest([byte; 32]);
    let edges = [
        (digest(1), digest(2), false),
        (digest(2), digest(3), true),
        (digest(1), digest(4), false),
        (digest(4), digest(5), false),
        (digest(6), digest(2), false),
        (digest(7), digest(8), false),
        (digest(8), digest(7), false),
    ];

    assert_eq!(
        terminal_supported_edge_indices(&edges),
        BTreeSet::from([0, 1, 4])
    );
}

#[test]
fn fitted_q_can_propagate_beyond_the_configured_minimum() {
    assert_eq!(fitted_q_backup_limit(12, 32), 32);
    assert_eq!(fitted_q_backup_limit(32, 4), 32);
    assert_eq!(fitted_q_backup_limit(12, 1_000), 512);
}

#[test]
fn action_similarity_cannot_pull_value_from_a_remote_state_region() {
    let mut samples = Vec::new();
    for index in 0..STATE_NEIGHBORS {
        samples.push(GeneralizedTacticTrainingSample {
            state_features: vec![0.0, 0.0],
            context: GeneralizedTacticContext::default(),
            action: action(&format!("near-{index}"), 500.0, 10.0, 2.5, Some(7), -100.0),
            outcome: GeneralizedTacticOutcome {
                reward: 1.0,
                ..GeneralizedTacticOutcome::default()
            },
        });
    }
    let target = action("target", 100.0, 100.0, 0.0, None, 100.0);
    for index in 0..STATE_NEIGHBORS {
        let mut remote = target.clone();
        remote.option_id = format!("remote-{index}");
        samples.push(GeneralizedTacticTrainingSample {
            state_features: vec![100.0, 100.0],
            context: GeneralizedTacticContext::default(),
            action: remote,
            outcome: GeneralizedTacticOutcome {
                reward: 100.0,
                ..GeneralizedTacticOutcome::default()
            },
        });
    }

    let prediction = GeneralizedTacticValueModel::fit(&samples)
        .unwrap()
        .predict(&[0.0, 0.0], &GeneralizedTacticContext::default(), &target)
        .unwrap();
    assert_eq!(prediction.outcome.reward, 1.0);
}

#[test]
fn later_action_similarity_cannot_override_exact_local_state_evidence() {
    let target = action("target", 100.0, 100.0, 0.0, None, 100.0);
    let mut samples = Vec::new();
    for index in 0..STATE_NEIGHBORS {
        samples.push(GeneralizedTacticTrainingSample {
            state_features: vec![0.0],
            context: GeneralizedTacticContext::default(),
            action: action(&format!("local-{index}"), 500.0, 10.0, 2.5, Some(7), -100.0),
            outcome: GeneralizedTacticOutcome {
                reward: 1.0,
                ..GeneralizedTacticOutcome::default()
            },
        });
    }
    for index in 0..STATE_NEIGHBORS {
        let mut later = target.clone();
        later.option_id = format!("later-{index}");
        samples.push(GeneralizedTacticTrainingSample {
            state_features: vec![0.25],
            context: GeneralizedTacticContext::default(),
            action: later,
            outcome: GeneralizedTacticOutcome {
                reward: 100.0,
                ..GeneralizedTacticOutcome::default()
            },
        });
    }

    let prediction = GeneralizedTacticValueModel::fit(&samples)
        .unwrap()
        .predict(&[0.0], &GeneralizedTacticContext::default(), &target)
        .unwrap();
    assert_eq!(prediction.outcome.reward, 1.0);
}

#[test]
fn censored_neighbors_calibrate_expected_return_and_not_first_hit_cost() {
    let descriptor = action("query", 100.0, 100.0, 0.0, Some(20), 10.0);
    let mut samples = Vec::new();
    for index in 0..4 {
        let mut supported = descriptor.clone();
        supported.option_id = format!("supported-{index}");
        samples.push(GeneralizedTacticTrainingSample {
            state_features: vec![0.0, 1.0],
            context: GeneralizedTacticContext::default(),
            action: supported,
            outcome: GeneralizedTacticOutcome {
                terminal: 1.0,
                reward: 99.0,
                duration_ticks: 10.0,
                ..GeneralizedTacticOutcome::default()
            },
        });
        let mut censored = descriptor.clone();
        censored.option_id = format!("censored-{index}");
        samples.push(GeneralizedTacticTrainingSample {
            state_features: vec![0.0, 1.0],
            context: GeneralizedTacticContext::default(),
            action: censored,
            outcome: GeneralizedTacticOutcome {
                terminal: 0.0,
                reward: -1.0,
                duration_ticks: 0.0,
                ..GeneralizedTacticOutcome::default()
            },
        });
    }

    let prediction = GeneralizedTacticValueModel::fit(&samples)
        .unwrap()
        .predict(
            &[0.0, 1.0],
            &GeneralizedTacticContext::default(),
            &descriptor,
        )
        .unwrap();
    assert!((prediction.outcome.terminal - 0.5).abs() < 1.0e-6);
    assert!((prediction.outcome.reward - 49.0).abs() < 1.0e-6);
    assert_eq!(prediction.outcome.duration_ticks, 10.0);
}

#[test]
fn action_identity_is_not_a_model_feature() {
    let mut left = action("left", 100.0, 100.0, 0.0, Some(20), 10.0);
    let mut right = action("right", 100.0, 100.0, 0.0, Some(20), 10.0);
    left.parameters.insert(
        "controller_sha256".into(),
        OptionParameter::Digest(crate::artifact::Digest([1; 32])),
    );
    right.parameters.insert(
        "controller_sha256".into(),
        OptionParameter::Digest(crate::artifact::Digest([2; 32])),
    );
    assert_eq!(
        encode_action(&GeneralizedTacticContext::default(), &left).unwrap(),
        encode_action(&GeneralizedTacticContext::default(), &right).unwrap()
    );
}

#[test]
fn recorded_tape_identity_is_not_a_model_feature() {
    let mut frame = InputFrame::default();
    frame.owned_ports = 1;
    frame.pads[0].stick_y = 127;
    frame.pads[0].buttons = 0x0100;
    let left = InputTape {
        frames: vec![frame.clone(), InputFrame::default()],
        ..InputTape::default()
    };
    let right = InputTape {
        tick_rate_numerator: 60,
        frames: vec![frame, InputFrame::default()],
        ..InputTape::default()
    };
    let left = left.describe("recorded-left").unwrap().option;
    let right = right.describe("recorded-right").unwrap().option;
    assert_ne!(
        left.parameters.get("input_tape_sha256"),
        right.parameters.get("input_tape_sha256")
    );
    assert_eq!(
        encode_action(&GeneralizedTacticContext::default(), &left).unwrap(),
        encode_action(&GeneralizedTacticContext::default(), &right).unwrap()
    );
}

#[test]
fn initial_command_heading_distinguishes_setup_from_long_run_mean() {
    let mut right = action("right-setup", 100.0, 100.0, 0.0, None, 10.0);
    let mut left = action("left-setup", 100.0, 100.0, 0.0, None, 10.0);
    for descriptor in [&mut right, &mut left] {
        descriptor.parameters.insert(
            "movement_heading".into(),
            OptionParameter::F32Bits(0.0_f32.to_bits()),
        );
    }
    right.parameters.insert(
        "command_initial_heading".into(),
        OptionParameter::F32Bits(std::f32::consts::FRAC_PI_2.to_bits()),
    );
    left.parameters.insert(
        "command_initial_heading".into(),
        OptionParameter::F32Bits((-std::f32::consts::FRAC_PI_2).to_bits()),
    );

    assert_ne!(
        encode_action(&GeneralizedTacticContext::default(), &right).unwrap(),
        encode_action(&GeneralizedTacticContext::default(), &left).unwrap()
    );
}

#[test]
fn typed_native_targets_magnitude_and_heading_are_state_relative() {
    let context = GeneralizedTacticContext {
        player_x: 10.0,
        player_z: 20.0,
        yaw_cos: 1.0,
        camera_yaw_cos: 1.0,
        ..GeneralizedTacticContext::default()
    };
    let target = OptionActionDescriptor {
        option_id: "native-target".into(),
        option_type: OptionType::Move,
        parameters: BTreeMap::from([
            ("maximum_ticks".into(), OptionParameter::Unsigned(10)),
            (
                "coordinate".into(),
                OptionParameter::Vec3F32Bits([30.0_f32, 0.0, 20.0_f32].map(f32::to_bits)),
            ),
            ("magnitude".into(), OptionParameter::Unsigned(100)),
        ]),
    };
    let encoded_target = encode_action(&context, &target).unwrap();
    assert_eq!(encoded_target[30], 1.0);
    assert_eq!(&encoded_target[31..34], &[20.0, 0.0, 20.0]);
    assert_eq!(encoded_target[44], 1.0);
    assert_eq!(encoded_target[45], 100.0 / 127.0);

    let heading = OptionActionDescriptor {
        option_id: "native-heading".into(),
        option_type: OptionType::MaintainHeading,
        parameters: BTreeMap::from([
            ("maximum_ticks".into(), OptionParameter::Unsigned(10)),
            (
                "heading_radians".into(),
                OptionParameter::F32Bits(std::f32::consts::FRAC_PI_2.to_bits()),
            ),
            ("magnitude".into(), OptionParameter::Unsigned(127)),
        ]),
    };
    let encoded_heading = encode_action(&context, &heading).unwrap();
    assert_eq!(encoded_heading[0], 1.0);
    assert_eq!(encoded_heading[5], 0.0);
    assert_eq!(encoded_heading[47], 1.0);
    assert!((encoded_heading[48] - 1.0).abs() < 1.0e-6);
    assert!(encoded_heading[49].abs() < 1.0e-6);
    assert!((encoded_heading[50] - std::f32::consts::FRAC_PI_2).abs() < 1.0e-6);

    // Heading factors describe the emitted input, not player/camera error.
    let rotated_context = GeneralizedTacticContext {
        yaw_sin: 0.75_f32.sin(),
        yaw_cos: 0.75_f32.cos(),
        camera_yaw_sin: (-1.25_f32).sin(),
        camera_yaw_cos: (-1.25_f32).cos(),
        ..context
    };
    assert_eq!(
        encoded_heading,
        encode_action(&rotated_context, &heading).unwrap()
    );

    let roll = OptionActionDescriptor {
        option_id: "native-roll".into(),
        option_type: OptionType::Roll,
        parameters: BTreeMap::from([
            ("direction_degrees".into(), OptionParameter::Signed(-90)),
            ("magnitude".into(), OptionParameter::Unsigned(127)),
            ("recovery_frames".into(), OptionParameter::Unsigned(3)),
        ]),
    };
    let encoded_roll = encode_action(&context, &roll).unwrap();
    assert!((encoded_roll[29] - 4.0_f32.ln_1p()).abs() < 1.0e-6);
    assert_eq!(encoded_roll[47], 1.0);
    assert!((encoded_roll[48] - 1.0).abs() < 1.0e-6);
    assert!(encoded_roll[49].abs() < 1.0e-6);
    assert_eq!(encoded_roll[45], 1.0);
    assert!((encoded_roll[51] - 0.25).abs() < 1.0e-6);
    assert_eq!(encoded_roll[55 + 8], 1.0);

    let temporal_controller = OptionActionDescriptor {
        option_id: "camera-lock-forward".into(),
        option_type: OptionType::Custom("reactive_controller".into()),
        parameters: BTreeMap::from([
            (
                "command_initial_heading".into(),
                OptionParameter::F32Bits(std::f32::consts::FRAC_PI_2.to_bits()),
            ),
            (
                "movement_heading".into(),
                OptionParameter::F32Bits(0.0_f32.to_bits()),
            ),
        ]),
    };
    let encoded_temporal = encode_action(&context, &temporal_controller).unwrap();
    assert_eq!(encoded_temporal[47], 1.0);
    assert!((encoded_temporal[48] - 1.0).abs() < 1.0e-6);
    assert!(encoded_temporal[49].abs() < 1.0e-6);
}

#[test]
fn learned_objective_return_is_the_only_action_ordering() {
    let faster = GeneralizedTacticOutcome {
        terminal: 1.0,
        reward: 99.0,
        duration_ticks: 100.0,
        path_efficiency: 0.7,
        ..GeneralizedTacticOutcome::default()
    };
    let slower_but_cleaner = GeneralizedTacticOutcome {
        terminal: 1.0,
        reward: 98.0,
        duration_ticks: 110.0,
        path_efficiency: 1.0,
        ..GeneralizedTacticOutcome::default()
    };

    assert_eq!(
        compare_generalized_tactic_outcomes(&faster, &slower_but_cleaner),
        std::cmp::Ordering::Greater
    );
}

#[test]
fn auxiliary_motion_predictions_do_not_define_action_utility() {
    let clean = GeneralizedTacticOutcome {
        goal_progress_per_tick: 20.0,
        path_efficiency: 0.98,
        speed_retention: 0.95,
        duration_ticks: 100.0,
        ..GeneralizedTacticOutcome::default()
    };
    let benign_clip = GeneralizedTacticOutcome {
        wall_contact_fraction: 0.5,
        ..clean
    };
    let slowing_impact = GeneralizedTacticOutcome {
        goal_progress_per_tick: 1.0,
        path_efficiency: 0.1,
        speed_retention: 0.1,
        stalled_command_fraction: 0.8,
        momentum_loss_per_tick: 1.0,
        collision_correction_per_tick: 0.5,
        ..benign_clip
    };

    assert_eq!(
        compare_generalized_tactic_outcomes(&benign_clip, &clean),
        std::cmp::Ordering::Equal
    );
    assert_eq!(
        compare_generalized_tactic_outcomes(&slowing_impact, &clean),
        std::cmp::Ordering::Equal
    );
}

#[test]
fn subresolution_return_noise_defers_to_action_support_distance() {
    let reference = GeneralizedTacticOutcome {
        reward: 98.739_967,
        ..GeneralizedTacticOutcome::default()
    };
    let interpolation_noise = GeneralizedTacticOutcome {
        reward: 98.739_983,
        ..reference
    };
    let one_tick_gain = GeneralizedTacticOutcome {
        reward: reference.reward + 0.01,
        ..reference
    };

    assert_eq!(
        compare_generalized_tactic_outcomes(&interpolation_noise, &reference),
        std::cmp::Ordering::Equal
    );
    assert_eq!(
        compare_generalized_tactic_outcomes(&one_tick_gain, &reference),
        std::cmp::Ordering::Greater
    );
}

#[test]
fn objective_tick_ties_prefer_authenticated_terminal_action_support() {
    let estimate =
        |id: &str, reward: f32, nearest_distance: f32, terminal_support_distance: f32| {
            GeneralizedTacticEstimate {
                descriptor: action(id, 100.0, 100.0, 0.0, None, 100.0),
                outcome: GeneralizedTacticOutcome {
                    reward,
                    ..GeneralizedTacticOutcome::default()
                },
                nearest_distance,
                terminal_support_distance: Some(terminal_support_distance),
                neighbors: NEIGHBORS,
            }
        };
    let supported = estimate("supported", 98.739, 0.5, 0.01);
    let censored_interpolation = estimate("censored", 98.741, 0.0, 0.5);
    let one_tick_faster = estimate("faster", 98.749, 0.5, 0.75);

    assert_eq!(
        compare_generalized_tactic_estimates(&supported, &censored_interpolation, 0.01,),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        compare_generalized_tactic_estimates(&supported, &one_tick_faster, 0.01),
        std::cmp::Ordering::Greater
    );
    assert_eq!(
        compare_terminal_support_estimates(&supported, &one_tick_faster, 0.01),
        std::cmp::Ordering::Less
    );
}

#[test]
fn terminal_support_policy_clones_actions_from_the_nearest_successful_state() {
    let near_action = action("near", 100.0, 100.0, 0.0, None, 100.0);
    let far_action = action("far", 500.0, 10.0, 2.5, Some(7), -100.0);
    let samples = vec![
        GeneralizedTacticTrainingSample {
            state_features: vec![0.0],
            context: GeneralizedTacticContext {
                player_x: 0.0,
                ..GeneralizedTacticContext::default()
            },
            action: near_action.clone(),
            outcome: GeneralizedTacticOutcome {
                terminal: 1.0,
                reward: 1.0,
                ..GeneralizedTacticOutcome::default()
            },
        },
        GeneralizedTacticTrainingSample {
            state_features: vec![1.0],
            context: GeneralizedTacticContext {
                player_x: 1.0,
                ..GeneralizedTacticContext::default()
            },
            action: far_action.clone(),
            outcome: GeneralizedTacticOutcome {
                terminal: 1.0,
                reward: 100.0,
                ..GeneralizedTacticOutcome::default()
            },
        },
    ];
    let model = GeneralizedTacticValueModel::fit(&samples).unwrap();

    let ranked = model
        .rank_terminal_support(
            &[0.01],
            &GeneralizedTacticContext {
                player_x: 0.01,
                ..GeneralizedTacticContext::default()
            },
            &[far_action, near_action.clone()],
        )
        .unwrap();

    assert_eq!(ranked[0].descriptor.option_id, near_action.option_id);
}

#[test]
fn terminal_support_prefers_nearby_state_over_authored_route_phase() {
    let current_phase_action = action("current-phase", 100.0, 100.0, 0.0, None, 100.0);
    let physically_near_past_action =
        action("physically-near-past", 500.0, 10.0, 2.5, Some(7), -100.0);
    let samples = [
        GeneralizedTacticTrainingSample {
            state_features: vec![0.0],
            context: GeneralizedTacticContext {
                player_x: 100.0,
                ..GeneralizedTacticContext::default()
            },
            action: current_phase_action.clone(),
            outcome: GeneralizedTacticOutcome {
                terminal: 1.0,
                reward: 1.0,
                ..GeneralizedTacticOutcome::default()
            },
        },
        GeneralizedTacticTrainingSample {
            state_features: vec![1.0],
            context: GeneralizedTacticContext {
                player_x: 0.0,
                ..GeneralizedTacticContext::default()
            },
            action: physically_near_past_action.clone(),
            outcome: GeneralizedTacticOutcome {
                terminal: 1.0,
                reward: 100.0,
                ..GeneralizedTacticOutcome::default()
            },
        },
    ];
    let model = GeneralizedTacticValueModel::fit(&samples).unwrap();

    let ranked = model
        .rank_terminal_support(
            &[0.0],
            &GeneralizedTacticContext {
                player_x: 0.0,
                ..GeneralizedTacticContext::default()
            },
            &[
                physically_near_past_action.clone(),
                current_phase_action.clone(),
            ],
        )
        .unwrap();

    assert_eq!(
        ranked[0].descriptor.option_id,
        physically_near_past_action.option_id
    );
}

#[test]
fn terminal_support_policy_preserves_prompted_action_availability() {
    let demonstrated_roll = action("demonstrated-roll", 100.0, 100.0, 0.0, Some(20), 100.0);
    let model = GeneralizedTacticValueModel::fit(&[
        GeneralizedTacticTrainingSample {
            state_features: vec![0.0],
            context: GeneralizedTacticContext::default(),
            action: demonstrated_roll.clone(),
            outcome: GeneralizedTacticOutcome {
                terminal: 1.0,
                reward: 1.0,
                ..GeneralizedTacticOutcome::default()
            },
        },
        GeneralizedTacticTrainingSample {
            state_features: vec![1.0],
            context: GeneralizedTacticContext::default(),
            action: demonstrated_roll.clone(),
            outcome: GeneralizedTacticOutcome {
                terminal: 1.0,
                reward: 1.0,
                ..GeneralizedTacticOutcome::default()
            },
        },
    ])
    .unwrap();
    let generated_roll = action("generated-roll", 500.0, 10.0, 2.5, Some(7), -100.0);
    let continuous_match = action("continuous-match", 100.0, 100.0, 0.0, None, 100.0);

    let ranked = model
        .rank_terminal_support(
            &[0.0],
            &GeneralizedTacticContext::default(),
            &[continuous_match, generated_roll.clone()],
        )
        .unwrap();

    assert_eq!(ranked[0].descriptor.option_id, generated_roll.option_id);
}

#[test]
fn terminal_support_transfers_direction_across_camera_lock_modifier() {
    let mut plain = [0.0; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH];
    let mut camera_lock = plain;
    let mut prompted_action = plain;
    let minimum = [0.0; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH];
    let range = [1.0; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH];
    camera_lock[55 + 6] = 1.0; // L
    prompted_action[55 + 8] = 1.0; // A

    let camera_lock_distance =
        behavior_cloning_action_distance(&plain, &camera_lock, &minimum, &range);
    let prompted_action_distance =
        behavior_cloning_action_distance(&plain, &prompted_action, &minimum, &range);

    assert_eq!(
        camera_lock_distance,
        1.0 / GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH as f32
    );
    assert!(
        prompted_action_distance
            > camera_lock_distance + GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH as f32
    );
    plain[55 + 6] = 1.0;
    assert_eq!(
        behavior_cloning_action_distance(&plain, &camera_lock, &minimum, &range),
        0.0
    );
}

#[test]
fn calibrated_return_comparison_is_transitive() {
    let outcomes = [
        98.739_90, 98.739_94, 98.739_967, 98.739_983, 98.740_02, 98.749_97,
    ]
    .map(|reward| GeneralizedTacticOutcome {
        reward,
        ..GeneralizedTacticOutcome::default()
    });

    for left in &outcomes {
        for middle in &outcomes {
            for right in &outcomes {
                let left_middle = compare_generalized_tactic_outcomes(left, middle);
                let middle_right = compare_generalized_tactic_outcomes(middle, right);
                if left_middle != std::cmp::Ordering::Greater
                    && middle_right != std::cmp::Ordering::Greater
                {
                    assert_ne!(
                        compare_generalized_tactic_outcomes(left, right),
                        std::cmp::Ordering::Greater
                    );
                }
            }
        }
    }
}

#[test]
fn held_out_actions_generalize_roll_straightness_and_collision_outcomes() {
    let clean = GeneralizedTacticOutcome {
        terminal: 1.0,
        goal_progress_per_tick: 20.0,
        path_efficiency: 0.98,
        speed_retention: 0.95,
        ..GeneralizedTacticOutcome::default()
    };
    let curved = GeneralizedTacticOutcome {
        goal_progress_per_tick: 12.0,
        path_efficiency: 0.65,
        speed_retention: 0.7,
        ..GeneralizedTacticOutcome::default()
    };
    let wall = GeneralizedTacticOutcome {
        goal_progress_per_tick: 5.0,
        path_efficiency: 0.5,
        speed_retention: 0.3,
        wall_contact_fraction: 0.4,
        momentum_loss_per_tick: 3.0,
        collision_correction_per_tick: 2.0,
        ..GeneralizedTacticOutcome::default()
    };
    let samples = vec![
        sample(
            action("roll-18", 100.0, 100.0, 0.0, Some(18), 10.0),
            99.0,
            clean,
        ),
        sample(
            action("roll-22", 104.0, 100.0, 0.03, Some(22), 12.0),
            98.0,
            clean,
        ),
        sample(action("walk-a", 100.0, 100.0, 0.0, None, 10.0), 5.0, curved),
        sample(
            action("walk-b", 104.0, 100.0, 0.03, None, 12.0),
            5.0,
            curved,
        ),
        sample(
            action("curve-a", 150.0, 100.0, 1.2, Some(20), 10.0),
            20.0,
            curved,
        ),
        sample(
            action("curve-b", 145.0, 100.0, 1.0, Some(24), 12.0),
            20.0,
            curved,
        ),
        sample(
            action("wall-a", 120.0, 90.0, 0.4, Some(20), 90.0),
            -10.0,
            wall,
        ),
        sample(
            action("wall-b", 122.0, 90.0, 0.45, Some(22), 92.0),
            -9.0,
            wall,
        ),
    ];
    let model = GeneralizedTacticValueModel::fit(&samples).unwrap();
    let held_out = vec![
        action("held-roll", 102.0, 100.0, 0.01, Some(20), 11.0),
        action("held-walk", 102.0, 100.0, 0.01, None, 11.0),
        action("held-curve", 148.0, 100.0, 1.1, Some(21), 11.0),
        action("held-wall", 121.0, 90.0, 0.42, Some(21), 91.0),
    ];
    let ranked = model
        .rank(&[0.0, 1.0], &GeneralizedTacticContext::default(), &held_out)
        .unwrap();
    assert_eq!(ranked[0].descriptor.option_id, "held-roll");
    let by_id = |id: &str| {
        ranked
            .iter()
            .find(|estimate| estimate.descriptor.option_id == id)
            .unwrap()
    };
    assert!(by_id("held-roll").outcome.reward > by_id("held-walk").outcome.reward);
    assert!(by_id("held-roll").outcome.reward > by_id("held-curve").outcome.reward);
    assert!(
        by_id("held-wall").outcome.wall_contact_fraction
            > by_id("held-roll").outcome.wall_contact_fraction
    );
    assert!(by_id("held-wall").outcome.reward < by_id("held-roll").outcome.reward);
}
