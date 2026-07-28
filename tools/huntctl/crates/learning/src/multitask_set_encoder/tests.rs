use super::*;
use crate::trainable_set_encoder::TypedSetNode;

fn sample(identity: u8, first: f32, second: f32, reverse: bool) -> MultiTaskSetSample {
    let mut nodes = vec![
        TypedSetNode {
            stable_id: 1,
            categorical: vec![10],
            categorical_present: vec![true],
            continuous: vec![first],
            continuous_present: vec![true],
            binary: vec![first > 0.0],
            binary_present: vec![true],
        },
        TypedSetNode {
            stable_id: 2,
            categorical: vec![20],
            categorical_present: vec![true],
            continuous: vec![second],
            continuous_present: vec![true],
            binary: vec![second > 0.0],
            binary_present: vec![true],
        },
    ];
    if reverse {
        nodes.reverse();
    }
    let second_present = !identity.is_multiple_of(5);
    let mut post_nodes = nodes.clone();
    for node in &mut post_nodes {
        node.continuous[0] += first - second;
    }
    let post_sample_sha256 =
        canonical_digest(b"dusklight.synthetic-multitask-post/v1\0", &identity).unwrap();
    let mut action_context = vec![0.0; ACTION_CONTEXT_WIDTH];
    action_context[0] = first;
    action_context[1] = second;
    MultiTaskSetSample {
        input: TypedSetSample {
            sample_sha256: Digest([identity; 32]),
            actor_feature_schema_sha256: Digest([7; 32]),
            base: vec![first - second],
            base_present: vec![true],
            nodes,
            target: 0.0,
        },
        post_input: TypedSetSample {
            sample_sha256: post_sample_sha256,
            actor_feature_schema_sha256: Digest([7; 32]),
            base: vec![first + second],
            base_present: vec![true],
            nodes: post_nodes,
            target: 0.0,
        },
        history: Vec::new(),
        action_context,
        targets: vec![
            first + second,
            if second_present { first - second } else { 0.0 },
        ],
        target_present: vec![true, second_present],
    }
}

fn corpus(start: u8, count: usize) -> Vec<MultiTaskSetSample> {
    (0..count)
        .map(|index| {
            let first = ((index * 17 % 41) as f32 - 20.0) / 10.0;
            let second = ((index * 29 % 37) as f32 - 18.0) / 10.0;
            sample(start + index as u8, first, second, index % 2 == 0)
        })
        .collect()
}

#[test]
fn direct_native_adapter_exposes_generic_event_transition_with_masks() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v22.dseps"
    ))
    .unwrap();
    let observation = &shard.episodes[0].steps[0].pre_input;
    let (base, present) = broad_base(observation);
    assert_eq!(
        &base[62..76],
        &[
            1.0, 2.0, 291.0, 1.0, 0.0, 10.0, 20.0, 30.0, 1.0, 2.0, 1.0, 3.0, 5.0, 2.0
        ]
    );
    assert!(present[62..76].iter().all(|value| *value));

    let mut base = base;
    let mut present = present;
    append_core_temporal_features(&mut base, &mut present, observation, None);

    let reduced =
        NativeEncoderFeatureSpec::excluding([NativeEncoderChannelFamily::CoreEventTransition])
            .unwrap();
    let mut reduced_values = base;
    let mut reduced_present = present;
    retain_feature_families(
        &mut reduced_values,
        &mut reduced_present,
        &native_base_feature_families(),
        &reduced,
    );
    assert_eq!(reduced_values.len(), 234);
    assert_eq!(reduced_present.len(), 234);
}

#[test]
fn direct_native_adapter_exposes_generic_clock_domains_with_masks() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v23.dseps"
    ))
    .unwrap();
    let observation = &shard.episodes[0].steps[0].pre_input;
    let (base, present) = broad_base(observation);
    assert_eq!(
        &base[76..93],
        &[
            1000.0, 900.0, 0.0, 1.0, 1.0, 2.0, 1.0, 0.0, 1.0, 1.0, 40.0, 35.0, 3.0, 1.0, 4.0,
            1234.0, 5000.0,
        ]
    );
    assert!(present[76..93].iter().all(|value| *value));

    let mut base = base;
    let mut present = present;
    append_core_temporal_features(&mut base, &mut present, observation, None);
    let reduced =
        NativeEncoderFeatureSpec::excluding([NativeEncoderChannelFamily::CoreClockDomains])
            .unwrap();
    retain_feature_families(
        &mut base,
        &mut present,
        &native_base_feature_families(),
        &reduced,
    );
    assert_eq!(base.len(), 231);
    assert_eq!(present.len(), 231);
}

#[test]
fn direct_native_adapter_exposes_generic_warp_session_with_masks() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v25.dseps"
    ))
    .unwrap();
    let observation = &shard.episodes[0].steps[0].pre_input;
    let (base, present) = broad_base(observation);
    assert_eq!(
        &base[93..116],
        &[
            3.0, 1.0, 100.0, 200.0, -300.0, 4608.0, 2.0, 4.0, 1.0, 0.0, 1.0, 10.0, 20.0, 30.0,
            -4608.0, 5.0, 3.0, 0.0, 1.0, 9.0, 1.0, 6.0, 0.0,
        ]
    );
    assert!(present[93..116].iter().all(|value| *value));

    let legacy = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v24.dseps"
    ))
    .unwrap();
    let (legacy_base, legacy_present) = broad_base(&legacy.episodes[0].steps[0].pre_input);
    assert!(legacy_base[93..116].iter().all(|value| *value == 0.0));
    assert!(legacy_present[93..116].iter().all(|value| !*value));

    let mut base = base;
    let mut present = present;
    append_core_temporal_features(&mut base, &mut present, observation, None);
    let reduced =
        NativeEncoderFeatureSpec::excluding([NativeEncoderChannelFamily::CoreWarpSession]).unwrap();
    retain_feature_families(
        &mut base,
        &mut present,
        &native_base_feature_families(),
        &reduced,
    );
    assert_eq!(base.len(), 225);
    assert_eq!(present.len(), 225);
}

#[test]
fn direct_native_adapter_keeps_the_complete_typed_actor_population() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v15.dseps"
    ))
    .unwrap();
    let observation = &shard.episodes[0].steps[0].pre_input;
    assert!(!observation.actors_truncated);
    let nodes = native_actor_nodes(observation, None);
    assert_eq!(nodes.len(), observation.actors.len());
    assert_eq!(
        nodes.iter().map(|node| node.stable_id).collect::<Vec<_>>(),
        observation
            .actors
            .iter()
            .map(|actor| actor.runtime_generation)
            .collect::<Vec<_>>()
    );
    assert!(nodes.iter().all(|node| {
        node.categorical.len() == native_actor_categorical_names().len()
            && node.categorical.len() == node.categorical_present.len()
            && node.continuous.len() == native_actor_continuous_names().len()
            && node.continuous.len() == node.continuous_present.len()
            && node.binary.len() == native_actor_binary_names().len()
            && node.binary.len() == node.binary_present.len()
    }));
    let lock_membership = native_actor_binary_names()
        .iter()
        .position(|name| name == "attention_lock_candidate")
        .unwrap();
    assert!(
        nodes
            .iter()
            .all(|node| !node.binary[lock_membership] && !node.binary_present[lock_membership])
    );
    let (base, present) = broad_base(observation);
    assert_eq!(base.len(), 223);
    assert_eq!(present.len(), 223);
    assert!(base[62..93].iter().all(|value| *value == 0.0));
    assert!(present[62..93].iter().all(|value| !*value));
    assert!(base[93..116].iter().all(|value| *value == 0.0));
    assert!(present[93..116].iter().all(|value| !*value));
    assert!(base[183..].iter().all(|value| *value == 0.0));
    assert!(present[183..].iter().all(|value| !*value));
    let mut temporal_base = base.clone();
    let mut temporal_present = present.clone();
    append_core_temporal_features(&mut temporal_base, &mut temporal_present, observation, None);
    let all = NativeEncoderFeatureSpec::all();
    assert_eq!(temporal_base.len(), 248);
    assert_eq!(temporal_present.len(), 248);
    assert_eq!(native_base_feature_names().len(), 248);
    assert_eq!(native_base_feature_families().len(), 248);
    let previous_available = native_base_feature_names()
        .iter()
        .position(|name| name == "temporal_previous_state_available")
        .unwrap();
    assert_eq!(temporal_base[previous_available], 0.0);
    assert!(temporal_present[previous_available]);
    let actor_previous_available = native_actor_binary_names()
        .iter()
        .position(|name| name == "temporal_previous_actor_present")
        .unwrap();
    assert!(!nodes[0].binary[actor_previous_available]);
    assert!(!nodes[0].binary_present[actor_previous_available]);
    let mut post_base = temporal_base.clone();
    let mut post_present = temporal_present.clone();
    suppress_base_family(
        &mut post_base,
        &mut post_present,
        NativeEncoderChannelFamily::CorePreviousInput,
    );
    for (index, family) in native_base_feature_families().into_iter().enumerate() {
        if family == NativeEncoderChannelFamily::CorePreviousInput {
            assert_eq!(post_base[index], 0.0);
            assert!(!post_present[index]);
        } else {
            assert_eq!(post_base[index], temporal_base[index]);
            assert_eq!(post_present[index], temporal_present[index]);
        }
    }
    assert_ne!(native_actor_feature_schema(&all).unwrap(), Digest::ZERO);
    let reduced = NativeEncoderFeatureSpec::excluding([
        NativeEncoderChannelFamily::CoreAttentionCandidates,
        NativeEncoderChannelFamily::ActorAttention,
        NativeEncoderChannelFamily::ActorAttentionCandidates,
        NativeEncoderChannelFamily::ActorEventParticipation,
        NativeEncoderChannelFamily::ActorEnemyBase,
        NativeEncoderChannelFamily::ActorTriggerVolume,
        NativeEncoderChannelFamily::ActorDoor20,
    ])
    .unwrap();
    let mut reduced_node = nodes[0].clone();
    retain_node_feature_families(&mut reduced_node, &reduced);
    assert!(reduced_node.categorical.len() < nodes[0].categorical.len());
    assert!(reduced_node.continuous.len() < nodes[0].continuous.len());
    assert!(reduced_node.binary.len() < nodes[0].binary.len());
    assert_ne!(
        native_actor_feature_schema(&reduced).unwrap(),
        native_actor_feature_schema(&all).unwrap()
    );
}

#[test]
fn direct_native_adapter_exposes_ablatable_door20_with_legacy_masks() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v27.dseps"
    ))
    .unwrap();
    let observation = &shard.episodes[0].steps[0].pre_input;
    let mut nodes = native_actor_nodes(observation, None);
    let categorical_names = native_actor_categorical_names();
    let continuous_names = native_actor_continuous_names();
    let binary_names = native_actor_binary_names();
    let kind = categorical_names
        .iter()
        .position(|name| name == "door20_kind")
        .unwrap();
    let front_switch = categorical_names
        .iter()
        .position(|name| name == "door20_front_switch")
        .unwrap();
    let action = categorical_names
        .iter()
        .position(|name| name == "door20_action")
        .unwrap();
    let angle = continuous_names
        .iter()
        .position(|name| name == "door20_angle_s16")
        .unwrap();
    let present = binary_names
        .iter()
        .position(|name| name == "door20_present")
        .unwrap();
    let front_switch_set = binary_names
        .iter()
        .position(|name| name == "door20_front_switch_set")
        .unwrap();
    let opening = binary_names
        .iter()
        .position(|name| name == "door20_opening_active")
        .unwrap();
    let door = nodes
        .iter()
        .find(|node| node.binary[present])
        .expect("direct DOOR20 node");
    assert!(door.binary_present[present]);
    assert_eq!(door.categorical[kind], 9);
    assert_eq!(door.categorical[front_switch], 0x11);
    assert!(door.categorical_present[front_switch]);
    assert_eq!(door.categorical[action], 3);
    assert_eq!(door.continuous[angle], -1234.0);
    assert!(door.continuous_present[angle]);
    assert!(door.binary[front_switch_set]);
    assert!(door.binary[opening]);

    let spec = NativeEncoderFeatureSpec::new([
        NativeEncoderChannelFamily::ActorPopulation,
        NativeEncoderChannelFamily::ActorDoor20,
    ])
    .unwrap();
    for node in &mut nodes {
        retain_node_feature_families(node, &spec);
        assert_eq!(node.categorical.len(), 21);
        assert_eq!(node.continuous.len(), 1);
        assert_eq!(node.binary.len(), 10);
    }
    assert_eq!(
        selected_feature_names(
            native_actor_categorical_names(),
            &native_actor_categorical_families(),
            &spec,
        )
        .len(),
        21
    );

    let legacy = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v26.dseps"
    ))
    .unwrap();
    let legacy = native_actor_nodes(&legacy.episodes[0].steps[0].pre_input, None);
    assert!(legacy.iter().all(|node| {
        !node.binary[present]
            && node.binary_present[present]
            && node.categorical[kind] == 0
            && !node.categorical_present[kind]
            && node.continuous[angle] == 0.0
            && !node.continuous_present[angle]
            && !node.binary[opening]
            && !node.binary_present[opening]
    }));

    let all = NativeEncoderFeatureSpec::all();
    let without_door =
        NativeEncoderFeatureSpec::excluding([NativeEncoderChannelFamily::ActorDoor20]).unwrap();
    for (names, families, removed) in [
        (
            native_actor_categorical_names(),
            native_actor_categorical_families(),
            21,
        ),
        (
            native_actor_continuous_names(),
            native_actor_continuous_families(),
            1,
        ),
        (
            native_actor_binary_names(),
            native_actor_binary_families(),
            10,
        ),
    ] {
        assert_eq!(
            selected_feature_names(names.clone(), &families, &all).len()
                - selected_feature_names(names, &families, &without_door).len(),
            removed
        );
    }
    assert_ne!(
        native_actor_feature_schema(&all).unwrap(),
        native_actor_feature_schema(&without_door).unwrap()
    );
}

#[test]
fn temporal_features_are_past_only_and_join_actors_by_runtime_generation() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v15.dseps"
    ))
    .unwrap();
    let previous = shard.episodes[0].steps[0].pre_input.clone();
    let mut current = previous.clone();
    current.player_position[0] += 1.25;
    current.player_velocity[2] -= 0.5;
    let actor = current.actors.first_mut().unwrap();
    let stable_id = actor.runtime_generation;
    actor.position[0] += 3.5;
    actor.velocity[1] -= 2.0;
    actor.status ^= 1;

    let (mut base, mut present) = broad_base(&current);
    append_core_temporal_features(&mut base, &mut present, &current, Some(&previous));
    let base_names = native_base_feature_names();
    let player_delta_x = base_names
        .iter()
        .position(|name| name == "temporal_player_position_x")
        .unwrap();
    assert_eq!(base[player_delta_x], 1.25);
    assert!(present[player_delta_x]);

    let node = native_actor_nodes(&current, Some(&previous))
        .into_iter()
        .find(|node| node.stable_id == stable_id)
        .unwrap();
    let continuous_names = native_actor_continuous_names();
    let position_delta_x = continuous_names
        .iter()
        .position(|name| name == "temporal_position_delta_x")
        .unwrap();
    let velocity_delta_y = continuous_names
        .iter()
        .position(|name| name == "temporal_velocity_delta_y")
        .unwrap();
    assert_eq!(node.continuous[position_delta_x], 3.5);
    assert!(node.continuous_present[position_delta_x]);
    assert_eq!(node.continuous[velocity_delta_y], -2.0);
    assert!(node.continuous_present[velocity_delta_y]);

    let binary_names = native_actor_binary_names();
    let previous_actor_present = binary_names
        .iter()
        .position(|name| name == "temporal_previous_actor_present")
        .unwrap();
    let status_changed = binary_names
        .iter()
        .position(|name| name == "temporal_status_changed")
        .unwrap();
    assert!(node.binary[previous_actor_present]);
    assert!(node.binary_present[previous_actor_present]);
    assert!(node.binary[status_changed]);
    assert!(node.binary_present[status_changed]);

    current.room = current.room.wrapping_add(1);
    let context_changed_node = native_actor_nodes(&current, Some(&previous))
        .into_iter()
        .find(|node| node.stable_id == stable_id)
        .unwrap();
    assert!(!context_changed_node.binary[previous_actor_present]);
    assert!(!context_changed_node.binary_present[previous_actor_present]);
    assert!(!context_changed_node.continuous_present[position_delta_x]);
}

#[test]
fn rare_event_metrics_report_recall_and_probability_error() {
    let mut accumulator = BinaryEventAccumulator::default();
    for (expected, score) in [(true, 0.9), (true, 0.2), (false, 0.8), (false, 0.1)] {
        accumulator.observe(expected, score, 0.5);
    }
    let metrics = accumulator.finish().unwrap();
    assert_eq!(metrics.positives, 2);
    assert_eq!(metrics.negatives, 2);
    assert_eq!(metrics.true_positives, 1);
    assert_eq!(metrics.false_positives, 1);
    assert_eq!(metrics.true_negatives, 1);
    assert_eq!(metrics.false_negatives, 1);
    assert_eq!(metrics.precision, Some(0.5));
    assert_eq!(metrics.recall, Some(0.5));
    assert_eq!(metrics.specificity, Some(0.5));
    assert_eq!(metrics.balanced_accuracy, Some(0.5));
    assert_eq!(metrics.f1, Some(0.5));
    assert!((metrics.brier_score - 0.325).abs() < 1.0e-12);
}

#[test]
fn bernoulli_loss_is_stable_and_its_gradient_matches_finite_difference() {
    let logit = 0.37;
    let expected = 1.0;
    let weight = 3.25;
    let epsilon = 1.0e-6;
    let numeric = weight
        * (binary_cross_entropy_from_logit(logit + epsilon, expected)
            - binary_cross_entropy_from_logit(logit - epsilon, expected))
        / (2.0 * epsilon);
    let analytic = weight * (logistic(logit) - expected);
    assert!((numeric - analytic).abs() < 1.0e-9);
    assert!(binary_cross_entropy_from_logit(1_000.0, 0.0).is_finite());
    assert!(binary_cross_entropy_from_logit(-1_000.0, 1.0).is_finite());
}

#[test]
fn balanced_binary_scores_recover_training_prior_and_threshold_on_validation_only() {
    let prevalence = 0.1;
    let positive_weight = 5.0;
    let negative_weight = 5.0 / 9.0;
    assert!(
        (calibrated_binary_probability(0.0, positive_weight, negative_weight) - prevalence).abs()
            < 1.0e-12
    );
    let rows = [(true, 0.9), (true, 0.8), (false, 0.7), (false, 0.2)];
    assert_eq!(select_binary_decision_threshold(&rows).unwrap(), 0.8);
    assert!(select_binary_decision_threshold(&[(true, 0.9)]).is_err());
    assert!(select_binary_decision_threshold(&[(false, 0.1)]).is_err());
}

#[test]
fn bernoulli_normalization_balances_classes_without_changing_regression() {
    let mut samples = corpus(1, 4);
    for (index, sample) in samples.iter_mut().enumerate() {
        sample.targets = vec![f32::from(index == 0), index as f32];
        sample.target_present = vec![true, true];
    }
    let objectives = vec![
        AuxiliaryHeadObjective::ClassBalancedBernoulli,
        AuxiliaryHeadObjective::NormalizedRegression,
    ];
    let normalization = target_normalization(&samples, &objectives).unwrap();
    assert_eq!(normalization.mean, vec![0.25, 1.5]);
    assert_eq!(normalization.inverse_stddev[0], 1.0);
    assert_eq!(normalization.positive_weight[0], 2.0);
    assert!((normalization.negative_weight[0] - 2.0 / 3.0).abs() < 1.0e-12);
    assert_eq!(normalization.positive_weight[1], 1.0);
    assert_eq!(normalization.negative_weight[1], 1.0);

    samples[0].targets[0] = 0.25;
    assert!(target_normalization(&samples, &objectives).is_err());
}

#[test]
fn typed_multitask_fit_binds_balanced_binary_heads_and_probabilities() {
    let mut training = corpus(1, 40);
    let mut held_out = corpus(101, 20);
    for (index, sample) in training.iter_mut().enumerate() {
        sample.targets[0] = f32::from(index % 10 == 0);
    }
    for (index, sample) in held_out.iter_mut().enumerate() {
        sample.targets[0] = f32::from(index % 5 == 0);
    }
    let training_digest = sample_manifest_digest(&training).unwrap();
    let held_out_digest = sample_manifest_digest(&held_out).unwrap();
    let config = TrainableSetConfig {
        epochs: 4,
        node_hidden_width: 4,
        head_hidden_width: 4,
        ..TrainableSetConfig::default()
    };
    let (report, model) = CompleteSetMultiTaskEncoder::fit(
        Digest([7; 32]),
        training_digest,
        held_out_digest,
        vec!["contact_changed".into(), "inverse_difference".into()],
        &training,
        &held_out,
        config,
    )
    .unwrap();
    assert_eq!(
        report.target_objectives,
        vec![
            AuxiliaryHeadObjective::ClassBalancedBernoulli,
            AuxiliaryHeadObjective::NormalizedRegression,
        ]
    );
    assert_eq!(report.target_positive_weights, vec![5.0, 1.0]);
    assert!((report.target_negative_weights[0] - 5.0 / 9.0).abs() < 1.0e-12);
    assert_eq!(report.target_negative_weights[1], 1.0);
    assert_eq!(
        report.target_decision_thresholds[0],
        model.target_decision_thresholds[0]
    );
    assert!(report.target_decision_thresholds[0].is_some());
    assert_eq!(report.target_decision_thresholds[1], None);
    assert!(report.training_objective_loss.is_finite());
    assert!(report.held_out_objective_loss.is_finite());
    assert!(report.held_out_constant_baseline_objective_loss.is_finite());
    assert!((model.constant_baseline_prediction(0) - 0.5).abs() < 1.0e-12);
    let zero_logit_loss = training
        .iter()
        .map(|sample| model.target_loss(0, 0.0, f64::from(sample.targets[0])))
        .sum::<f64>()
        / training.len() as f64;
    let constant_baseline_loss = training
        .iter()
        .map(|sample| model.constant_baseline_loss(0, f64::from(sample.targets[0])))
        .sum::<f64>()
        / training.len() as f64;
    assert!((zero_logit_loss - constant_baseline_loss).abs() < 1.0e-12);
    let probability = model.predict(&held_out[0]).unwrap()[0];
    assert!((0.0..=1.0).contains(&probability));

    let control = fit_shuffled_auxiliary_control(
        Digest([7; 32]),
        vec!["contact_changed".into(), "inverse_difference".into()],
        training,
        held_out_digest,
        &held_out,
        &held_out,
        config,
    )
    .unwrap();
    assert_eq!(control.report.target_objectives, report.target_objectives);
    assert_eq!(
        control.report.target_positive_weights,
        report.target_positive_weights
    );
}

#[test]
fn feature_family_names_round_trip_and_actor_columns_require_population() {
    let target_names = native_target_names();
    assert_eq!(target_names.len(), 15);
    assert_eq!(
        target_conditioning_for_names(&target_names),
        native_target_conditioning()
    );
    let objectives = target_objectives_for_names(&target_names);
    assert_eq!(
        objectives[target_names
            .iter()
            .position(|name| name == "actor_disappearance_occurred")
            .unwrap()],
        AuxiliaryHeadObjective::ClassBalancedBernoulli
    );
    assert_eq!(
        objectives[target_names
            .iter()
            .position(|name| name == "actor_disappearance_count")
            .unwrap()],
        AuxiliaryHeadObjective::NormalizedRegression
    );
    assert_eq!(
        MultiTaskSetPooling::parse("mean-max"),
        Some(MultiTaskSetPooling::MeanMax)
    );
    assert_eq!(
        MultiTaskSetPooling::parse("mean-max-learned-attention"),
        Some(MultiTaskSetPooling::MeanMaxLearnedAttention)
    );
    assert_eq!(
        MultiTaskSetPooling::parse("mean-max-task-attention"),
        Some(MultiTaskSetPooling::MeanMaxTaskAttention)
    );
    assert_eq!(MultiTaskSetPooling::parse("nearest-actor"), None);
    for family in NativeEncoderChannelFamily::ALL {
        assert_eq!(
            NativeEncoderChannelFamily::parse(family.name()),
            Some(family)
        );
    }
    assert!(NativeEncoderChannelFamily::parse("nearest_actor_magic").is_none());
    assert!(NativeEncoderFeatureSpec::new([NativeEncoderChannelFamily::ActorMotion]).is_err());
    assert!(NativeEncoderFeatureSpec::new([NativeEncoderChannelFamily::CorePreviousInput]).is_ok());
    assert!(
        NativeEncoderFeatureSpec::all()
            .with_history_depth(MAX_EPISODE_HISTORY_DEPTH + 1)
            .is_err()
    );
    assert!(
        NativeEncoderFeatureSpec::all()
            .with_recurrent_history(0, DEFAULT_HISTORY_RECURRENT_WIDTH)
            .is_err()
    );
    assert!(
        NativeEncoderFeatureSpec::all()
            .with_recurrent_history(2, 0)
            .is_err()
    );
    assert!(
        NativeEncoderFeatureSpec::all()
            .with_recurrent_history(2, MAX_HISTORY_RECURRENT_WIDTH + 1)
            .is_err()
    );
    assert!(
        NativeEncoderFeatureSpec::all()
            .with_trainable_history(0, DEFAULT_HISTORY_RECURRENT_WIDTH)
            .is_err()
    );
    assert_ne!(
        native_actor_feature_schema(&NativeEncoderFeatureSpec::all()).unwrap(),
        native_actor_feature_schema(
            &NativeEncoderFeatureSpec::all()
                .with_history_depth(2)
                .unwrap()
        )
        .unwrap()
    );
    assert_ne!(
        native_actor_feature_schema(
            &NativeEncoderFeatureSpec::all()
                .with_recurrent_history(2, DEFAULT_HISTORY_RECURRENT_WIDTH)
                .unwrap()
        )
        .unwrap(),
        native_actor_feature_schema(
            &NativeEncoderFeatureSpec::all()
                .with_trainable_history(2, DEFAULT_HISTORY_RECURRENT_WIDTH)
                .unwrap()
        )
        .unwrap()
    );
    assert_ne!(
        native_actor_feature_schema(
            &NativeEncoderFeatureSpec::all()
                .with_history_depth(2)
                .unwrap()
        )
        .unwrap(),
        native_actor_feature_schema(
            &NativeEncoderFeatureSpec::all()
                .with_recurrent_history(2, DEFAULT_HISTORY_RECURRENT_WIDTH)
                .unwrap()
        )
        .unwrap()
    );
    assert!(
        NativeEncoderFeatureSpec {
            families: vec![
                NativeEncoderChannelFamily::CoreGoal,
                NativeEncoderChannelFamily::CorePlayerMotion,
            ],
            history_depth: 0,
            history_encoding: NativeEncoderHistoryEncoding::None,
            history_recurrent_width: 0,
        }
        .validate()
        .is_err()
    );
}

#[test]
fn stacked_history_is_past_only_right_aligned_and_masked_at_episode_start() {
    let mut shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v14.dseps"
    ))
    .unwrap();
    let prototype = shard.episodes[0].steps[0].clone();
    shard.episodes[0].steps = vec![prototype; 3];
    let episode = &shard.episodes[0];
    let history = NativeEpisodeHistoryView::build(&shard, 2).unwrap();
    let spec = NativeEncoderFeatureSpec::all()
        .with_history_depth(2)
        .unwrap();
    let names = native_history_feature_names(&spec);
    assert_eq!(names.len() % 2, 0);
    let slot_width = names.len() / 2;

    let mut start_values = Vec::new();
    let mut start_present = Vec::new();
    append_episode_history_features(&mut start_values, &mut start_present, episode, &[], &spec)
        .unwrap();
    assert_eq!(start_values.len(), names.len());
    assert_eq!(start_values[0], 0.0);
    assert_eq!(start_values[slot_width], 0.0);
    assert!(start_present[0] && start_present[slot_width]);
    assert!(start_present[1..slot_width].iter().all(|present| !*present));
    assert!(
        start_present[slot_width + 1..]
            .iter()
            .all(|present| !*present)
    );

    let decision = &history.decisions[2];
    assert_eq!(decision.episode_id, episode.id);
    assert_eq!(decision.step_index, 2);
    let completed = decision
        .completed_transition_indices
        .iter()
        .map(|index| &history.transitions[*index as usize])
        .collect::<Vec<_>>();
    let mut values = Vec::new();
    let mut present = Vec::new();
    append_episode_history_features(&mut values, &mut present, episode, &completed, &spec).unwrap();
    assert_eq!(values[0], 1.0);
    assert_eq!(values[slot_width], 1.0);
    assert!(present[1..].iter().any(|present| *present));

    let mut changed_current = episode.clone();
    changed_current.steps[2].consumed_pad.buttons ^= 0xffff;
    changed_current.steps[2].post_simulation.player_position[0] += 10_000.0;
    let mut unchanged_values = Vec::new();
    let mut unchanged_present = Vec::new();
    append_episode_history_features(
        &mut unchanged_values,
        &mut unchanged_present,
        &changed_current,
        &completed,
        &spec,
    )
    .unwrap();
    assert_eq!(unchanged_values, values);
    assert_eq!(unchanged_present, present);
}

#[test]
fn recurrent_history_is_bounded_deterministic_and_past_only() {
    let mut shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v14.dseps"
    ))
    .unwrap();
    let prototype = shard.episodes[0].steps[0].clone();
    shard.episodes[0].steps = vec![prototype; 3];
    let episode = &shard.episodes[0];
    let history = NativeEpisodeHistoryView::build(&shard, 2).unwrap();
    let spec = NativeEncoderFeatureSpec::all()
        .with_recurrent_history(2, 4)
        .unwrap();
    let reservoir = native_recurrent_history_reservoir(&spec).unwrap().unwrap();
    assert_eq!(native_history_feature_names(&spec).len(), 6);

    let mut start_values = Vec::new();
    let mut start_present = Vec::new();
    append_encoded_episode_history_features(
        &mut start_values,
        &mut start_present,
        episode,
        &[],
        &spec,
        Some(&reservoir),
    )
    .unwrap();
    assert_eq!(start_values, vec![0.0; 6]);
    assert_eq!(start_present, vec![true; 6]);

    let decision = &history.decisions[2];
    let completed = decision
        .completed_transition_indices
        .iter()
        .map(|index| &history.transitions[*index as usize])
        .collect::<Vec<_>>();
    let mut values = Vec::new();
    let mut present = Vec::new();
    append_encoded_episode_history_features(
        &mut values,
        &mut present,
        episode,
        &completed,
        &spec,
        Some(&reservoir),
    )
    .unwrap();
    assert_eq!(values.len(), 6);
    assert_eq!(values[0], 1.0);
    assert_eq!(values[1], 1.0);
    assert!(values[2..].iter().all(|value| value.is_finite()));
    assert!(values[2..].iter().any(|value| *value != 0.0));
    assert_eq!(present, vec![true; 6]);

    let mut repeated_values = Vec::new();
    let mut repeated_present = Vec::new();
    append_encoded_episode_history_features(
        &mut repeated_values,
        &mut repeated_present,
        episode,
        &completed,
        &spec,
        Some(&reservoir),
    )
    .unwrap();
    assert_eq!(repeated_values, values);
    assert_eq!(repeated_present, present);

    let mut changed_current = episode.clone();
    changed_current.steps[2].consumed_pad.buttons ^= 0xffff;
    changed_current.steps[2].post_simulation.player_position[0] += 10_000.0;
    let mut unchanged_values = Vec::new();
    let mut unchanged_present = Vec::new();
    append_encoded_episode_history_features(
        &mut unchanged_values,
        &mut unchanged_present,
        &changed_current,
        &completed,
        &spec,
        Some(&reservoir),
    )
    .unwrap();
    assert_eq!(unchanged_values, values);
    assert_eq!(unchanged_present, present);
}

#[test]
fn trainable_history_shares_complete_states_and_excludes_the_current_transition() {
    let mut shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v14.dseps"
    ))
    .unwrap();
    let prototype = shard.episodes[0].steps[0].clone();
    shard.episodes[0].steps = vec![prototype; 3];
    let episode = &shard.episodes[0];
    let history = NativeEpisodeHistoryView::build(&shard, 2).unwrap();
    let spec = NativeEncoderFeatureSpec::all()
        .with_trainable_history(2, 4)
        .unwrap();
    let schema = native_actor_feature_schema(&spec).unwrap();
    let mut states = BTreeMap::new();

    let first_decision = &history.decisions[1];
    let first_completed = first_decision
        .completed_transition_indices
        .iter()
        .map(|index| &history.transitions[*index as usize])
        .collect::<Vec<_>>();
    let first =
        trainable_episode_history_steps(episode, &first_completed, &spec, schema, &mut states)
            .unwrap();
    assert_eq!(first.len(), 1);
    assert!(!first[0].state.nodes.is_empty());

    let decision = &history.decisions[2];
    let completed = decision
        .completed_transition_indices
        .iter()
        .map(|index| &history.transitions[*index as usize])
        .collect::<Vec<_>>();
    let second =
        trainable_episode_history_steps(episode, &completed, &spec, schema, &mut states).unwrap();
    assert_eq!(second.len(), 2);
    assert!(Arc::ptr_eq(&first[0].state, &second[0].state));
    assert_eq!(states.len(), 2);

    let mut changed_current = episode.clone();
    changed_current.steps[2].consumed_pad.buttons ^= 0xffff;
    changed_current.steps[2].post_simulation.player_position[0] += 10_000.0;
    let mut changed_states = BTreeMap::new();
    let unchanged = trainable_episode_history_steps(
        &changed_current,
        &completed,
        &spec,
        schema,
        &mut changed_states,
    )
    .unwrap();
    assert_eq!(
        unchanged
            .iter()
            .map(|step| (step.transition_sha256, step.state.sample_sha256))
            .collect::<Vec<_>>(),
        second
            .iter()
            .map(|step| (step.transition_sha256, step.state.sample_sha256))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        unchanged
            .iter()
            .map(|step| &step.action_context)
            .collect::<Vec<_>>(),
        second
            .iter()
            .map(|step| &step.action_context)
            .collect::<Vec<_>>()
    );
}

#[test]
fn historical_actor_state_receives_gradient_through_the_gru() {
    let mut sample = sample(201, 1.25, -0.5, false);
    let mut history_state = sample.input.clone();
    history_state.sample_sha256 = Digest([202; 32]);
    sample.input.nodes.clear();
    sample.post_input.nodes.clear();
    sample.history = vec![MultiTaskHistoryStep {
        transition_sha256: Digest([203; 32]),
        state: Arc::new(history_state),
        action_context: vec![0.0; ACTION_CONTEXT_WIDTH],
    }];
    sample.targets = vec![1.0, 0.0];
    sample.target_present = vec![true, false];
    let dimensions = Dimensions {
        categorical: 1,
        continuous: 1,
        binary: 1,
        base: 1,
    };
    let layout = FeatureLayout::fit(sample_model_states(&sample), dimensions).unwrap();
    let config = TrainableSetConfig {
        epochs: 1,
        node_hidden_width: 4,
        head_hidden_width: 4,
        l2_penalty: 0.0,
        ..TrainableSetConfig::default()
    };
    let temporal = MultiTaskTemporalConfig::gated_recurrent(2, 4);
    let mut model = CompleteSetMultiTaskEncoder::initialized(
        Digest([7; 32]),
        layout,
        config,
        vec![
            "actor_disappearance_occurred".into(),
            "inverse_stick_x".into(),
        ],
        vec![
            AuxiliaryHeadConditioning::PreStateAndAction,
            AuxiliaryHeadConditioning::PreAndPostState,
        ],
        vec![
            AuxiliaryHeadObjective::ClassBalancedBernoulli,
            AuxiliaryHeadObjective::NormalizedRegression,
        ],
        vec![0.1, 0.0],
        vec![1.0; 2],
        vec![2.0, 1.0],
        vec![0.5, 1.0],
        MultiTaskSetPooling::MeanMax,
        temporal,
    )
    .unwrap();
    model.output_weights.fill(0.0);
    let history_offset = config.head_hidden_width * 2 + ACTION_CONTEXT_WIDTH;
    model.output_weights[history_offset] = 1.0;
    let before = model.node_weights.clone();
    model.train_one(&sample).unwrap();
    let gradient_l1 = model
        .node_weights
        .iter()
        .zip(before)
        .map(|(after, before)| (after - before).abs())
        .sum::<f64>();
    assert!(gradient_l1 > 0.0);
}

#[test]
fn trainable_history_refits_and_actor_permutations_are_exact() {
    let attach_history = |samples: &mut [MultiTaskSetSample]| {
        for sample in samples {
            let mut state = sample.input.clone();
            state.sample_sha256 = canonical_digest(
                b"dusklight.synthetic-history-state/v1\0",
                &sample.input.sample_sha256,
            )
            .unwrap();
            sample.history = vec![MultiTaskHistoryStep {
                transition_sha256: canonical_digest(
                    b"dusklight.synthetic-history-transition/v1\0",
                    &sample.input.sample_sha256,
                )
                .unwrap(),
                state: Arc::new(state),
                action_context: sample.action_context.clone(),
            }];
        }
    };
    let mut training = corpus(1, 48);
    let mut held_out = corpus(101, 16);
    attach_history(&mut training);
    attach_history(&mut held_out);
    let training_digest = sample_manifest_digest(&training).unwrap();
    let held_out_digest = sample_manifest_digest(&held_out).unwrap();
    let config = TrainableSetConfig {
        epochs: 4,
        node_hidden_width: 4,
        head_hidden_width: 4,
        ..TrainableSetConfig::default()
    };
    let temporal = MultiTaskTemporalConfig::gated_recurrent(2, 4);
    let fit = || {
        CompleteSetMultiTaskEncoder::fit_with_pooling_and_temporal(
            Digest([7; 32]),
            training_digest,
            held_out_digest,
            vec!["sum".into(), "inverse_difference".into()],
            &training,
            &held_out,
            config,
            MultiTaskSetPooling::MeanMax,
            temporal,
        )
        .unwrap()
    };
    let (first_report, first) = fit();
    let (second_report, second) = fit();
    assert_eq!(first_report.temporal, temporal);
    assert_eq!(first_report.report_sha256, second_report.report_sha256);
    assert_eq!(
        first.model_sha256().unwrap(),
        second.model_sha256().unwrap()
    );

    let original = first.predict(&held_out[0]).unwrap();
    let mut permuted = held_out[0].clone();
    permuted.input.nodes.reverse();
    permuted.post_input.nodes.reverse();
    let mut history_state = (*permuted.history[0].state).clone();
    history_state.nodes.reverse();
    permuted.history[0].state = Arc::new(history_state);
    assert_eq!(first.predict(&permuted).unwrap(), original);
}

#[test]
fn direct_native_adapter_joins_attention_candidates_without_selecting_one() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v20.dseps"
    ))
    .unwrap();
    let observation = &shard.episodes[0].steps[0].pre_input;
    let node = native_actor_nodes(observation, None)
        .into_iter()
        .find(|node| node.stable_id == 7)
        .unwrap();
    let categorical_names = native_actor_categorical_names();
    let continuous_names = native_actor_continuous_names();
    let binary_names = native_actor_binary_names();
    let categorical = |name: &str| {
        let index = categorical_names
            .iter()
            .position(|value| value == name)
            .unwrap();
        (node.categorical[index], node.categorical_present[index])
    };
    let continuous = |name: &str| {
        let index = continuous_names
            .iter()
            .position(|value| value == name)
            .unwrap();
        (node.continuous[index], node.continuous_present[index])
    };
    let binary = |name: &str| {
        let index = binary_names.iter().position(|value| value == name).unwrap();
        (node.binary[index], node.binary_present[index])
    };

    assert_eq!(categorical("attention_lock_type"), (1, true));
    assert_eq!(categorical("attention_lock_rank"), (0, true));
    assert_eq!(categorical("attention_action_type"), (6, true));
    assert_eq!(categorical("attention_action_rank"), (0, true));
    assert_eq!(categorical("attention_check_type"), (0, false));
    assert_eq!(continuous("attention_lock_weight"), (0.25, true));
    assert_eq!(continuous("attention_lock_distance"), (80.0, true));
    assert_eq!(continuous("attention_lock_angle_s16"), (-256.0, true));
    assert_eq!(continuous("attention_action_weight"), (0.5, true));
    assert_eq!(continuous("attention_action_distance"), (90.0, true));
    assert_eq!(continuous("attention_action_angle_s16"), (512.0, true));
    assert_eq!(continuous("attention_check_weight"), (0.0, false));
    assert_eq!(binary("attention_lock_candidate"), (true, true));
    assert_eq!(binary("attention_action_candidate"), (true, true));
    assert_eq!(binary("attention_check_candidate"), (false, true));
    let (base, base_present) = broad_base(observation);
    assert_eq!(base.len(), 223);
    assert!(base_present[62..93].iter().all(|value| !*value));
    assert!(base_present[93..116].iter().all(|value| !*value));
    assert!(base_present[183..].iter().all(|value| *value));
    assert_eq!(base[183 + 2], 1.0);
    assert_eq!(base[183 + 4], 1.0);
    assert_eq!(base[215], 2.0);
    assert_eq!(base[216], 3.0);
    assert_eq!(base[217], 1.0);
    assert_eq!(base[219], 1.0);
    assert_eq!(base[221], 0.0);
}

#[test]
fn shuffled_control_rebinds_targets_without_changing_support() {
    let training = corpus(1, 96);
    let validation = corpus(130, 32);
    let test = corpus(170, 32);
    let original_digest = sample_manifest_digest(&training).unwrap();
    let config = TrainableSetConfig {
        epochs: 2,
        node_hidden_width: 8,
        head_hidden_width: 8,
        minimum_relative_improvement: 1.0,
        ..TrainableSetConfig::default()
    };
    let control = fit_shuffled_auxiliary_control(
        Digest([7; 32]),
        vec!["forward_sum".into(), "inverse_difference".into()],
        training,
        sample_manifest_digest(&validation).unwrap(),
        &validation,
        &test,
        config,
    )
    .unwrap();
    assert_eq!(control.schema, SHUFFLED_AUXILIARY_CONTROL_SCHEMA_V1);
    assert_ne!(control.shuffled_training_dataset_sha256, original_digest);
    assert_eq!(control.report.target_support_training, vec![96, 77]);
    assert_eq!(control.test_evaluation.samples, 32);
    assert_eq!(
        control.report.decision,
        MultiTaskEncoderDecision::RetainTrainingMeanBaseline
    );
}

#[test]
fn actorless_control_does_not_leak_set_cardinality() {
    let mut training = corpus(1, 32);
    let mut held_out = corpus(80, 16);
    for sample in training.iter_mut().chain(&mut held_out) {
        sample.input.actor_feature_schema_sha256 = Digest([6; 32]);
        sample.input.nodes.clear();
        sample.post_input.actor_feature_schema_sha256 = Digest([6; 32]);
        sample.post_input.nodes.clear();
    }
    let (report, model) = CompleteSetMultiTaskEncoder::fit(
        Digest([6; 32]),
        Digest([8; 32]),
        Digest([9; 32]),
        vec!["forward_sum".into(), "inverse_difference".into()],
        &training,
        &held_out,
        TrainableSetConfig {
            epochs: 2,
            node_hidden_width: 8,
            head_hidden_width: 8,
            ..TrainableSetConfig::default()
        },
    )
    .unwrap();
    assert_eq!(report.maximum_training_nodes, 0);
    assert_eq!(report.maximum_held_out_nodes, 0);
    assert_eq!(model.encode(&held_out[0].input).unwrap().len(), 8);
}

#[test]
fn shared_complete_set_encoder_learns_masked_heads_on_held_out_rows() {
    let training = corpus(1, 96);
    let held_out = corpus(130, 32);
    let config = TrainableSetConfig {
        epochs: 180,
        node_hidden_width: 12,
        head_hidden_width: 16,
        learning_rate: 0.003,
        minimum_relative_improvement: 0.25,
        ..TrainableSetConfig::default()
    };
    let (report, model) = CompleteSetMultiTaskEncoder::fit(
        Digest([7; 32]),
        Digest([8; 32]),
        Digest([9; 32]),
        vec!["forward_sum".into(), "inverse_difference".into()],
        &training,
        &held_out,
        config,
    )
    .unwrap();
    assert_eq!(report.target_support_training, vec![96, 77]);
    assert_eq!(report.target_support_held_out, vec![32, 25]);
    assert_eq!(
        report.target_conditioning,
        vec![
            AuxiliaryHeadConditioning::PreStateAndAction,
            AuxiliaryHeadConditioning::PreAndPostState,
        ]
    );
    assert!(report.relative_held_out_improvement > 0.25);
    assert_eq!(
        report.decision,
        MultiTaskEncoderDecision::SharedEncoderCandidate
    );
    assert_eq!(model.encode(&held_out[0].input).unwrap().len(), 16);
    assert_eq!(model.predict(&held_out[0]).unwrap().len(), 2);
    let baseline = model.predict(&held_out[0]).unwrap();
    let mut changed_post = held_out[0].clone();
    changed_post.post_input.base[0] += 1000.0;
    changed_post.post_input.nodes[0].continuous[0] -= 1000.0;
    let post_prediction = model.predict(&changed_post).unwrap();
    assert_eq!(baseline[0], post_prediction[0]);
    let mut changed_action = held_out[0].clone();
    changed_action.action_context.fill(0.75);
    let action_prediction = model.predict(&changed_action).unwrap();
    assert_eq!(baseline[1], action_prediction[1]);
    let evaluation = model.evaluate(&held_out).unwrap();
    assert_eq!(evaluation.samples, 32);
    assert!(evaluation.relative_improvement > 0.25);
    assert!(!report.promotion_authority);
    assert_ne!(report.report_sha256, Digest::ZERO);
}

#[test]
fn learned_attention_pooling_is_seeded_trainable_and_permutation_invariant() {
    let training = corpus(1, 48);
    let held_out = corpus(100, 16);
    let config = TrainableSetConfig {
        epochs: 4,
        node_hidden_width: 8,
        head_hidden_width: 8,
        ..TrainableSetConfig::default()
    };
    let fit = || {
        CompleteSetMultiTaskEncoder::fit_with_pooling(
            Digest([7; 32]),
            Digest([8; 32]),
            Digest([9; 32]),
            vec!["forward_sum".into(), "inverse_difference".into()],
            &training,
            &held_out,
            config,
            MultiTaskSetPooling::MeanMaxLearnedAttention,
        )
        .unwrap()
    };
    let (report, mut model) = fit();
    let (_, repeated) = fit();
    assert_eq!(report.pooling, MultiTaskSetPooling::MeanMaxLearnedAttention);
    assert_eq!(report.held_out_attention.len(), LEARNED_ATTENTION_HEADS);
    assert_eq!(
        model.model_sha256().unwrap(),
        repeated.model_sha256().unwrap()
    );
    for head in &report.held_out_attention {
        assert_eq!(head.target, None);
        assert_eq!(head.conditioning, None);
        assert_eq!(head.observation_support, held_out.len());
        assert!(head.query_l2_norm.is_finite() && head.query_l2_norm > 0.0);
        assert!((0.0..=1.0).contains(&head.mean_normalized_entropy));
        assert!((0.0..=1.0).contains(&head.mean_maximum_weight));
    }

    let baseline = model.predict(&held_out[0]).unwrap();
    let mut permuted = held_out[0].clone();
    permuted.input.nodes.reverse();
    permuted.post_input.nodes.reverse();
    assert_eq!(baseline, model.predict(&permuted).unwrap());

    let queries_before = model.attention_queries.clone();
    model.train_one(&training[0]).unwrap();
    assert_ne!(queries_before, model.attention_queries);
}

#[test]
fn task_attention_is_target_bound_phase_correct_and_permutation_invariant() {
    let training = corpus(1, 48);
    let held_out = corpus(100, 16);
    let config = TrainableSetConfig {
        epochs: 4,
        node_hidden_width: 8,
        head_hidden_width: 8,
        ..TrainableSetConfig::default()
    };
    let fit = || {
        CompleteSetMultiTaskEncoder::fit_with_pooling(
            Digest([7; 32]),
            Digest([8; 32]),
            Digest([9; 32]),
            vec!["forward_sum".into(), "inverse_difference".into()],
            &training,
            &held_out,
            config,
            MultiTaskSetPooling::MeanMaxTaskAttention,
        )
        .unwrap()
    };
    let (report, mut model) = fit();
    let (_, repeated) = fit();
    assert_eq!(report.pooling, MultiTaskSetPooling::MeanMaxTaskAttention);
    assert_eq!(report.held_out_attention.len(), 2);
    assert_eq!(
        model.model_sha256().unwrap(),
        repeated.model_sha256().unwrap()
    );
    for (target, head) in report.held_out_attention.iter().enumerate() {
        assert_eq!(
            head.target.as_deref(),
            Some(report.target_names[target].as_str())
        );
        assert_eq!(head.conditioning, Some(report.target_conditioning[target]));
        let phase_multiplier = usize::from(
            report.target_conditioning[target] == AuxiliaryHeadConditioning::PreAndPostState,
        ) + 1;
        assert_eq!(
            head.observation_support,
            report.target_support_held_out[target] * phase_multiplier
        );
    }

    let baseline = model.predict(&held_out[0]).unwrap();
    let mut permuted = held_out[0].clone();
    permuted.input.nodes.reverse();
    permuted.post_input.nodes.reverse();
    assert_eq!(baseline, model.predict(&permuted).unwrap());

    let mut changed_post = held_out[0].clone();
    changed_post.post_input.nodes[0].continuous[0] += 1000.0;
    assert_eq!(baseline[0], model.predict(&changed_post).unwrap()[0]);
    let mut changed_action = held_out[0].clone();
    changed_action.action_context.fill(0.75);
    assert_eq!(baseline[1], model.predict(&changed_action).unwrap()[1]);

    let queries_before = model.attention_queries.clone();
    model.train_one(&training[0]).unwrap();
    assert_ne!(queries_before, model.attention_queries);
}

#[test]
fn rejects_cross_split_identity_and_unsupported_target() {
    let training = corpus(1, 8);
    let mut held_out = corpus(40, 4);
    held_out[0].input.sample_sha256 = training[0].input.sample_sha256;
    assert!(
        CompleteSetMultiTaskEncoder::fit(
            Digest([7; 32]),
            Digest([8; 32]),
            Digest([9; 32]),
            vec!["forward_sum".into(), "inverse_difference".into()],
            &training,
            &held_out,
            TrainableSetConfig::default(),
        )
        .is_err()
    );
    let mut unsupported = corpus(40, 4);
    for sample in &mut unsupported {
        sample.target_present[1] = false;
        sample.targets[1] = 0.0;
    }
    assert!(
        CompleteSetMultiTaskEncoder::fit(
            Digest([7; 32]),
            Digest([8; 32]),
            Digest([9; 32]),
            vec!["forward_sum".into(), "inverse_difference".into()],
            &training,
            &unsupported,
            TrainableSetConfig::default(),
        )
        .is_err()
    );
}
