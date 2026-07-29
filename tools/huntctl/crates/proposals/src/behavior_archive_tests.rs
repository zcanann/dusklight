use super::*;
use crate::artifact::Digest;
use crate::candidate_envelope::NamedDigest;
use crate::offline_rl::{
    canonical_movement_pad_v2, movement_action_schema_digest_v2, movement_feature_schema_digest_v1,
};
use crate::search::{Candidate, SegmentProfile};
use crate::tape::{InputFrame, InputTape, RawPadState};
use crate::transition_corpus::{MacroAction, StateReference, StateReferenceKind, Transition};
use dusklight_control::option_execution::{
    OptionCondition, OptionEndReason, OptionExecution, OptionType, TapeRange,
};
use dusklight_evidence::native_episode_shard::{NativeEpisodeShard, NativeObservationPhase};
use dusklight_learning::fact_snapshot::FactSnapshot;
use dusklight_learning::option_transition::OptionTransitionSample;
use std::collections::BTreeMap;

#[test]
fn state_frontier_distance_excludes_action_provenance() {
    let reference = TacticEndpointDescriptor {
        stage: "F_TEST".into(),
        room: 1,
        layer: Some(0),
        player_procedure: Some(7),
        position_bin: [10, 20, 30],
        event_running: Some(false),
        event_id: Some(-1),
        actor_count_bin: 2,
        terminal: false,
        action_identity_sha256: Digest([1; 32]),
    };
    let mut different_action = reference.clone();
    different_action.action_identity_sha256 = Digest([2; 32]);
    let mut spatially_distinct = reference.clone();
    spatially_distinct.position_bin[0] += 1;

    assert_eq!(
        tactic_state_descriptor_distance(&reference, &different_action),
        0
    );
    assert_eq!(
        tactic_state_descriptor_distance(&reference, &spatially_distinct),
        1
    );
    assert!(
        tactic_descriptor_distance(&reference, &different_action)
            > tactic_descriptor_distance(&reference, &spatially_distinct)
    );
}

#[test]
fn reachability_frontier_distance_excludes_nonspatial_modes() {
    let reference = TacticEndpointDescriptor {
        stage: "F_TEST".into(),
        room: 1,
        layer: Some(0),
        player_procedure: Some(7),
        position_bin: [10, 20, 30],
        event_running: Some(false),
        event_id: Some(-1),
        actor_count_bin: 2,
        terminal: false,
        action_identity_sha256: Digest([1; 32]),
    };
    let mut different_mode = reference.clone();
    different_mode.player_procedure = Some(99);
    different_mode.event_running = Some(true);
    different_mode.event_id = Some(8);
    different_mode.actor_count_bin = 17;
    different_mode.action_identity_sha256 = Digest([2; 32]);
    let mut spatially_distinct = reference.clone();
    spatially_distinct.position_bin[0] += 1;

    assert_eq!(
        tactic_reachability_descriptor_distance(&reference, &different_mode),
        0
    );
    assert_eq!(
        tactic_reachability_descriptor_distance(&reference, &spatially_distinct),
        1
    );
    assert!(
        tactic_state_descriptor_distance(&reference, &different_mode)
            > tactic_state_descriptor_distance(&reference, &spatially_distinct)
    );
}

#[test]
fn frontier_cells_are_finer_than_reward_novelty_cells() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let mut first = FactSnapshot::from_native_learning(
        &shard.episodes[0].steps[0].pre_input,
        &[],
        None,
        Vec::new(),
    )
    .unwrap();
    let mut second = first.clone();
    let mut first_position = first.player.position_f32_bits;
    let coarse_base = (f32::from_bits(first_position[0]) / POSITION_BIN_WORLD_UNITS).floor()
        * POSITION_BIN_WORLD_UNITS;
    first_position[0] = (coarse_base + 32.0).to_bits();
    first.player.position_f32_bits = first_position;
    let mut second_position = second.player.position_f32_bits;
    second_position[0] = (coarse_base + 112.0).to_bits();
    second.player.position_f32_bits = second_position;

    assert_eq!(
        tactic_state_descriptor(&first, false),
        tactic_state_descriptor(&second, false)
    );
    assert_ne!(
        tactic_frontier_state_descriptor(&first, false),
        tactic_frontier_state_descriptor(&second, false)
    );
}

#[test]
fn tactic_endpoints_become_restorable_quality_diversity_elites() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let step = &shard.episodes[0].steps[0];
    let before =
        FactSnapshot::from_native_learning(&step.pre_input, &[], None, Vec::new()).unwrap();
    let mut route = InputTape {
        frames: vec![InputFrame::default(); before.tape_frame as usize],
        ..InputTape::default()
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
    route.frames.push(frame);
    let execution = OptionExecution::capture(
        "fixture.tick".into(),
        OptionType::Neutral,
        BTreeMap::new(),
        1,
        1,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &route,
        TapeRange {
            start_frame: before.tape_frame,
            end_frame_exclusive: before.tape_frame + 1,
        },
    )
    .unwrap();
    let mut boundary = step.post_simulation.clone();
    boundary.phase = NativeObservationPhase::PreInput;
    boundary.simulation_tick += 1;
    boundary.tape_frame += 1;
    let after = FactSnapshot::from_native_learning(
        &boundary,
        std::slice::from_ref(&step.pre_input),
        Some(&execution),
        Vec::new(),
    )
    .unwrap();
    let root = Digest([7; 32]);
    let prefix = InputTape {
        frames: route.frames[..before.tape_frame as usize].to_vec(),
        ..route.clone()
    };
    let transition = OptionTransitionSample::capture(
        Digest([9; 32]),
        tactic_route_checkpoint(root, &prefix).unwrap(),
        tactic_route_checkpoint(root, &route).unwrap(),
        before,
        after,
        execution,
        &route,
        1.0,
        step.post_simulation.terminal_reason
            == dusklight_evidence::native_episode_shard::NativeTerminalReason::GoalReached,
        |facts| Ok::<_, &'static str>(vec![facts.tape_frame as f32]),
    )
    .unwrap();

    let mut archive = BehaviorArchive::default();
    archive
        .consider_tactic_endpoint(root, transition.clone(), route.clone(), 0)
        .unwrap();
    assert_eq!(archive.tactic_len(), 1);
    let selected = archive.select_tactic_frontier(&[], 1);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].route_tape, route);
    assert!(
        archive
            .select_tactic_frontier_within_route_frames(&[], 1, route.frames.len() - 1)
            .is_empty()
    );
    assert_eq!(
        selected[0].route_checkpoint_sha256,
        transition.next_checkpoint_sha256
    );
    assert_eq!(archive.tactic_frontiers(), selected);
    assert!(archive.contains_tactic_frontier(
        transition.next_checkpoint_sha256,
        transition.after_state_sha256
    ));
    assert!(!archive.contains_tactic_frontier(Digest([8; 32]), transition.after_state_sha256));

    let mut better = transition.clone();
    better.value_sample.reward = 2.0;
    archive
        .consider_tactic_endpoint(root, better, route.clone(), 1)
        .unwrap();
    assert_eq!(archive.tactic_len(), 1);
    assert_eq!(
        archive.select_tactic_frontier(&[], 1)[0]
            .transition
            .value_sample
            .reward,
        2.0
    );

    let retained = archive.select_tactic_frontier(&[], 1).remove(0);
    let mut shorter_lower_reward = retained.clone();
    shorter_lower_reward.route_tape.frames.pop();
    shorter_lower_reward.transition.value_sample.reward = 1.0;
    assert!(
        tactic_cell_elite_cmp(&shorter_lower_reward, &retained).is_gt(),
        "a shorter route to the same semantic cell must dominate novelty-inflated reward"
    );
    assert!(
        tactic_quality_cmp(&shorter_lower_reward, &retained).is_lt(),
        "cross-cell frontier quality remains reward-first"
    );

    let mut detached = transition;
    detached.next_checkpoint_sha256 = Digest([8; 32]);
    detached.value_sample.next_checkpoint_sha256 = Digest([8; 32]);
    assert!(
        archive
            .consider_tactic_endpoint(root, detached, route, 2)
            .is_err()
    );
}

fn episode(x: f32, procedure: f32, frames: usize) -> QEpisode {
    let disconnected = RawPadState {
        connected: false,
        error: -1,
        ..RawPadState::default()
    };
    let tape = InputTape {
        frames: (0..frames)
            .map(|_| InputFrame {
                owned_ports: 1,
                pads: [
                    canonical_movement_pad_v2(1).unwrap(),
                    disconnected,
                    disconnected,
                    disconnected,
                ],
                ..InputFrame::default()
            })
            .collect(),
        ..InputTape::default()
    };
    let candidate = Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &tape).unwrap();
    let transitions = (0..frames)
        .map(|index| {
            let mut state = vec![0.0; 49];
            state[0] = f32::from(b'F') / 255.0;
            state[16] = procedure;
            state[17] = (x * index as f32 / frames as f32) / 8192.0;
            let mut next_state = state.clone();
            next_state[17] = (x * (index + 1) as f32 / frames as f32) / 8192.0;
            Transition {
                source: StateReference {
                    kind: StateReferenceKind::Boundary,
                    digest: Digest([index as u8 + 1; 32]),
                },
                state,
                action: MacroAction {
                    action_id: 1,
                    macro_kind: 2,
                    parameters: vec![0, 127, 0],
                },
                duration_ticks: 1,
                reward: -1.0,
                next: StateReference {
                    kind: StateReferenceKind::Boundary,
                    digest: Digest([index as u8 + 2; 32]),
                },
                next_state,
                terminal: index + 1 == frames,
            }
        })
        .collect();
    QEpisode {
        candidate,
        corpus: TransitionCorpus::new(
            movement_feature_schema_digest_v1(),
            movement_action_schema_digest_v2(),
            49,
            transitions,
        )
        .unwrap(),
        outcome: crate::episode::EpisodeOutcomeClass::Successful,
        objective: NamedDigest::new("archive-test", Digest([0x90; 32])),
    }
}

fn score(tick: u64) -> LexicographicScore {
    LexicographicScore {
        goal_feasible: true,
        milestone_depth: 2,
        successes: 1,
        attempts: 1,
        median_first_hit_tick: tick,
        best_first_hit_tick: tick,
        tape_frames: tick,
        input_complexity: 0,
        risk_events: None,
        boundary_compatibility: crate::search::BoundaryCompatibility::Unknown,
    }
}

fn contexts_base() -> BehaviorContext {
    BehaviorContext {
        objective_rng_identity: Some("01".repeat(32)),
        actor_population_identity: Some("11".repeat(32)),
        downstream_state_identity: Some("21".repeat(32)),
        ..BehaviorContext::default()
    }
}

#[test]
fn archive_keeps_distinct_paths_and_replaces_same_descriptor_with_faster_episode() {
    let mut archive = BehaviorArchive::default();
    archive
        .consider(episode(100.0, 4.0, 8), score(10), 0)
        .unwrap();
    archive
        .consider(episode(100.0, 4.0, 8), score(9), 1)
        .unwrap();
    archive
        .consider(episode(900.0, 7.0, 12), score(20), 1)
        .unwrap();
    assert_eq!(archive.len(), 2);

    let reference = describe_behavior(&episode(100.0, 4.0, 8).corpus).unwrap();
    let selected = archive
        .select_diverse(&HashSet::new(), &[reference], 1)
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].descriptor.terminal_player_procedure, 7);
}

#[test]
fn empirically_sensitive_coarse_cells_refine_without_a_float_epsilon() {
    let mut archive = BehaviorArchive::default();
    archive
        .consider(episode(100.0, 4.0, 8), score(10), 0)
        .unwrap();
    archive
        .consider(episode(110.0, 4.0, 8), score(9), 1)
        .unwrap();
    assert_eq!(archive.len(), 2);
    assert_eq!(archive.refined_cells.len(), 1);
    assert!(archive.entries.keys().all(|descriptor| {
        descriptor.adaptive_spatial_axis_mask & SPATIAL_AXIS_X != 0
            && descriptor
                .adaptive_spatial_identity
                .as_ref()
                .is_some_and(|identity| identity.len() == 64)
    }));

    let selected = archive.select_diverse(&HashSet::new(), &[], 2).unwrap();
    let summary = archive.summary(&selected).unwrap();
    assert_eq!(summary.refined_cells, 1);
    assert_eq!(summary.adaptive_spatial_axis_mask_union, SPATIAL_AXIS_X);
}

#[test]
fn map_elites_cells_separate_all_semantic_novelty_axes() {
    let mut archive = BehaviorArchive::default();
    let contexts = [
        BehaviorContext {
            objective_rng_identity: Some("01".repeat(32)),
            actor_population_identity: Some("11".repeat(32)),
            downstream_state_identity: Some("21".repeat(32)),
            ..BehaviorContext::default()
        },
        BehaviorContext {
            objective_rng_identity: Some("02".repeat(32)),
            actor_population_identity: Some("11".repeat(32)),
            downstream_state_identity: Some("21".repeat(32)),
            ..BehaviorContext::default()
        },
        BehaviorContext {
            objective_rng_identity: Some("01".repeat(32)),
            actor_population_identity: Some("12".repeat(32)),
            downstream_state_identity: Some("21".repeat(32)),
            ..BehaviorContext::default()
        },
        BehaviorContext {
            objective_rng_identity: Some("01".repeat(32)),
            actor_population_identity: Some("11".repeat(32)),
            downstream_state_identity: Some("22".repeat(32)),
            ..BehaviorContext::default()
        },
        BehaviorContext {
            objective_rng_identity: Some("01".repeat(32)),
            actor_population_identity: Some("11".repeat(32)),
            contact_behavior_identity: Some("31".repeat(32)),
            downstream_state_identity: Some("21".repeat(32)),
            ..BehaviorContext::default()
        },
        BehaviorContext {
            objective_rng_identity: Some("01".repeat(32)),
            actor_population_identity: Some("11".repeat(32)),
            boundary_state_identity: Some("41".repeat(32)),
            downstream_state_identity: Some("21".repeat(32)),
            ..BehaviorContext::default()
        },
        BehaviorContext {
            procedure_sequence_identity: Some("50".repeat(32)),
            ..contexts_base()
        },
        BehaviorContext {
            event_sequence_identity: Some("51".repeat(32)),
            ..contexts_base()
        },
        BehaviorContext {
            state_transition_identity: Some("52".repeat(32)),
            ..contexts_base()
        },
        BehaviorContext {
            actor_relationship_identity: Some("53".repeat(32)),
            ..contexts_base()
        },
        BehaviorContext {
            flag_state_identity: Some("54".repeat(32)),
            ..contexts_base()
        },
        BehaviorContext {
            kinematic_extrema_identity: Some("55".repeat(32)),
            ..contexts_base()
        },
    ];
    for (index, context) in contexts.iter().enumerate() {
        archive
            .consider_with_context(episode(100.0, 4.0, 8), score(10), index as u32, context)
            .unwrap();
    }
    assert_eq!(archive.len(), 12);
    let descriptors = archive.entries.keys().cloned().collect::<Vec<_>>();
    let selected = archive
        .select_diverse(&HashSet::new(), &descriptors[..1], 11)
        .unwrap();
    assert_eq!(selected.len(), 11);
    let summary = archive.summary(&selected).unwrap();
    assert_eq!(summary.schema, "dusklight-behavior-archive/v4");
    assert_eq!(
        summary.policy,
        "one_native_quality_elite_per_adaptive_cell_plus_farthest_first_novelty"
    );
}
