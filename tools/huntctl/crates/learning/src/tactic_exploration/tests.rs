use super::*;
use crate::learner_state::LearnerActionMaskEntry;
use crate::option_values::{AvailableOptionRanking, RankedOption};
use crate::tactic_asset::TacticDurationBounds;
use crate::tactic_blueprint::ConcreteTacticChoiceKind;
use dusklight_control::option_execution::{OptionParameter, OptionType};
use std::collections::{BTreeMap, BTreeSet};

fn descriptor(id: &str, option_type: OptionType) -> OptionActionDescriptor {
    OptionActionDescriptor {
        option_id: id.into(),
        option_type,
        parameters: BTreeMap::new(),
    }
}

fn descriptor_with_duration(
    id: &str,
    option_type: OptionType,
    duration_ticks: u64,
) -> OptionActionDescriptor {
    let mut descriptor = descriptor(id, option_type);
    descriptor.parameters.insert(
        "duration_ticks".into(),
        OptionParameter::Unsigned(duration_ticks),
    );
    descriptor
}

fn descriptor_with_duration_and_buttons(
    id: &str,
    option_type: OptionType,
    duration_ticks: u64,
    button_mask: u64,
) -> OptionActionDescriptor {
    let mut descriptor = descriptor_with_duration(id, option_type, duration_ticks);
    descriptor.parameters.insert(
        "command_button_mask".into(),
        OptionParameter::Unsigned(button_mask),
    );
    descriptor
}

fn choice(descriptor: OptionActionDescriptor) -> LearnerActionMaskEntry {
    LearnerActionMaskEntry {
        choice_id: descriptor.option_id.clone(),
        kind: ConcreteTacticChoiceKind::CatalogEntry,
        descriptor,
        duration: TacticDurationBounds {
            minimum_ticks: 1,
            maximum_ticks: 1,
        },
        applicable: true,
    }
}

#[test]
fn zero_epsilon_is_greedy_and_seeded_exploration_is_reproducible() {
    let wait = descriptor("wait", OptionType::Neutral);
    let roll = descriptor("roll", OptionType::Roll);
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([1; 32]),
        action_universe_sha256: Digest([2; 32]),
        choices: vec![choice(wait.clone()), choice(roll.clone())],
        values: AvailableOptionRanking {
            ranked: vec![
                RankedOption {
                    action_id: 1,
                    descriptor: roll.clone(),
                    mean_q: 5.0,
                    ensemble_variance: 0.0,
                },
                RankedOption {
                    action_id: 0,
                    descriptor: wait,
                    mean_q: 1.0,
                    ensemble_variance: 0.0,
                },
            ],
            unsupported: Vec::new(),
        },
    };
    let greedy = choose_tactic(
        &ranking,
        7,
        TacticExplorationConfig {
            seed: 99,
            epsilon_per_million: 0,
        },
    )
    .unwrap();
    assert_eq!(greedy.descriptor, roll);
    assert_eq!(greedy.reason, TacticSelectionReason::Greedy);

    let config = TacticExplorationConfig {
        seed: 99,
        epsilon_per_million: EPSILON_SCALE,
    };
    assert_eq!(
        choose_tactic(&ranking, 7, config).unwrap(),
        choose_tactic(&ranking, 7, config).unwrap()
    );
    assert_eq!(
        choose_tactic(&ranking, 7, config).unwrap().reason,
        TacticSelectionReason::Epsilon
    );
}

#[test]
fn stratified_epsilon_bounds_finite_campaign_exploration_gaps() {
    for seed in 0..32 {
        let quarter = (0..20)
            .map(|decision| stratified_exploration_draw(seed, decision, 250_000))
            .collect::<Vec<_>>();
        for cycle in quarter.chunks_exact(4) {
            assert_eq!(cycle.iter().filter(|draw| **draw < 250_000).count(), 1);
        }

        let tenth = (0..30)
            .map(|decision| stratified_exploration_draw(seed, decision, 100_000))
            .collect::<Vec<_>>();
        for cycle in tenth.chunks_exact(10) {
            assert_eq!(cycle.iter().filter(|draw| **draw < 100_000).count(), 1);
        }
    }
}

#[test]
fn an_untrained_catalog_bootstraps_without_fabricating_q() {
    let wait = descriptor("wait", OptionType::Neutral);
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([1; 32]),
        action_universe_sha256: Digest([2; 32]),
        choices: vec![choice(wait.clone())],
        values: AvailableOptionRanking {
            ranked: Vec::new(),
            unsupported: vec![wait.clone()],
        },
    };
    let selected = choose_tactic(&ranking, 0, TacticExplorationConfig::default()).unwrap();
    assert_eq!(selected.descriptor, wait);
    assert_eq!(selected.reason, TacticSelectionReason::UnsupportedBootstrap);
}

#[test]
fn epsilon_remains_behaviorally_authoritative_without_exact_q_support() {
    let wait = descriptor("wait", OptionType::Neutral);
    let move_forward = descriptor("move", OptionType::Move);
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([1; 32]),
        action_universe_sha256: Digest([2; 32]),
        choices: vec![choice(move_forward.clone()), choice(wait.clone())],
        values: AvailableOptionRanking {
            ranked: Vec::new(),
            unsupported: vec![wait, move_forward],
        },
    };

    let selected = choose_tactic(
        &ranking,
        0,
        TacticExplorationConfig {
            seed: 7,
            epsilon_per_million: 1_000_000,
        },
    )
    .unwrap();

    assert_eq!(selected.reason, TacticSelectionReason::Epsilon);
}

#[test]
fn nonpositive_known_values_bootstrap_an_unsupported_tactic() {
    let wait = descriptor("wait", OptionType::Neutral);
    let move_forward = descriptor("move", OptionType::Move);
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([1; 32]),
        action_universe_sha256: Digest([2; 32]),
        choices: vec![choice(move_forward.clone()), choice(wait.clone())],
        values: AvailableOptionRanking {
            ranked: vec![RankedOption {
                action_id: 0,
                descriptor: wait,
                mean_q: -0.01,
                ensemble_variance: 0.0,
            }],
            unsupported: vec![move_forward.clone()],
        },
    };
    let selected = choose_tactic(
        &ranking,
        0,
        TacticExplorationConfig {
            seed: 7,
            epsilon_per_million: 0,
        },
    )
    .unwrap();
    assert_eq!(selected.descriptor, move_forward);
    assert_eq!(selected.reason, TacticSelectionReason::UnsupportedBootstrap);
}

#[test]
fn epsilon_exploration_prioritizes_untried_tactics() {
    let known = descriptor("known", OptionType::Neutral);
    let fresh = descriptor("fresh", OptionType::Move);
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([3; 32]),
        action_universe_sha256: Digest([4; 32]),
        choices: vec![choice(fresh.clone()), choice(known.clone())],
        values: AvailableOptionRanking {
            ranked: vec![RankedOption {
                action_id: 1,
                descriptor: known,
                mean_q: 5.0,
                ensemble_variance: 0.0,
            }],
            unsupported: vec![fresh.clone()],
        },
    };
    for seed in 0..16 {
        let selected = choose_tactic(
            &ranking,
            0,
            TacticExplorationConfig {
                seed,
                epsilon_per_million: EPSILON_SCALE,
            },
        )
        .unwrap();
        assert_eq!(selected.descriptor, fresh);
        assert_eq!(selected.reason, TacticSelectionReason::Epsilon);
    }
}

#[test]
fn unsupported_coverage_does_not_privilege_authored_action_semantics() {
    let known = descriptor("known", OptionType::Neutral);
    let mut directional = descriptor("directional", OptionType::MaintainHeading);
    directional
        .parameters
        .insert("heading_radians".into(), OptionParameter::F32Bits(0));
    directional
        .parameters
        .insert("magnitude".into(), OptionParameter::Unsigned(127));
    directional
        .parameters
        .insert("maximum_ticks".into(), OptionParameter::Unsigned(16));
    let mut short = descriptor("short", OptionType::MaintainHeading);
    short
        .parameters
        .insert("heading_radians".into(), OptionParameter::F32Bits(0));
    short
        .parameters
        .insert("magnitude".into(), OptionParameter::Unsigned(80));
    short
        .parameters
        .insert("maximum_ticks".into(), OptionParameter::Unsigned(4));
    let mut curve = descriptor("curve", OptionType::Bezier);
    curve
        .parameters
        .insert("control".into(), OptionParameter::Text("symmetric".into()));
    let mut spatial = descriptor("spatial", OptionType::Move);
    spatial.parameters.insert(
        "coordinate".into(),
        OptionParameter::Vec3F32Bits([1.0_f32.to_bits(), 2.0_f32.to_bits(), 3.0_f32.to_bits()]),
    );
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([5; 32]),
        action_universe_sha256: Digest([6; 32]),
        choices: vec![
            choice(directional.clone()),
            choice(known.clone()),
            choice(curve.clone()),
            choice(short.clone()),
            choice(spatial.clone()),
        ],
        values: AvailableOptionRanking {
            ranked: vec![RankedOption {
                action_id: 1,
                descriptor: known,
                mean_q: 5.0,
                ensemble_variance: 0.0,
            }],
            unsupported: vec![curve.clone(), directional.clone(), short, spatial.clone()],
        },
    };
    let mut selected_ids = std::collections::BTreeSet::new();
    for seed in 0..64 {
        let selected = choose_tactic(
            &ranking,
            0,
            TacticExplorationConfig {
                seed,
                epsilon_per_million: EPSILON_SCALE,
            },
        )
        .unwrap();
        selected_ids.insert(selected.descriptor.option_id);
        assert_eq!(selected.reason, TacticSelectionReason::Epsilon);
    }
    assert_eq!(
        selected_ids,
        std::collections::BTreeSet::from([
            "curve".into(),
            "directional".into(),
            "short".into(),
            "spatial".into(),
        ])
    );
}

#[test]
fn epsilon_exploration_covers_actions_untried_in_the_current_state_cell() {
    let globally_best = descriptor("globally-best", OptionType::Move);
    let locally_untried = descriptor("locally-untried", OptionType::Bezier);
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([7; 32]),
        action_universe_sha256: Digest([8; 32]),
        choices: vec![
            choice(globally_best.clone()),
            choice(locally_untried.clone()),
        ],
        values: AvailableOptionRanking {
            ranked: vec![
                RankedOption {
                    action_id: 0,
                    descriptor: globally_best,
                    mean_q: 5.0,
                    ensemble_variance: 0.0,
                },
                RankedOption {
                    action_id: 1,
                    descriptor: locally_untried.clone(),
                    mean_q: 1.0,
                    ensemble_variance: 0.0,
                },
            ],
            unsupported: Vec::new(),
        },
    };
    let selected = choose_tactic_with_state_untried(
        &ranking,
        0,
        TacticExplorationConfig {
            seed: 11,
            epsilon_per_million: EPSILON_SCALE,
        },
        std::slice::from_ref(&locally_untried),
    )
    .unwrap();
    assert_eq!(selected.descriptor, locally_untried);
    assert_eq!(selected.reason, TacticSelectionReason::Epsilon);
}

#[test]
fn parallel_acquisition_partitions_distinct_route_families() {
    let routes = (0..3)
        .map(|route| {
            descriptor(
                &format!("goal.seek.route.{route:02}.roll.period.20.phase.00"),
                OptionType::Custom("reactive_controller".into()),
            )
        })
        .collect::<Vec<_>>();
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([21; 32]),
        action_universe_sha256: Digest([22; 32]),
        choices: routes.iter().cloned().map(choice).collect(),
        values: AvailableOptionRanking {
            ranked: Vec::new(),
            unsupported: routes.clone(),
        },
    };
    let primary = SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: ranking.learner_snapshot_sha256,
        decision_index: 0,
        descriptor: routes[2].clone(),
        reason: TacticSelectionReason::UnsupportedBootstrap,
        exploration_draw: 17,
    };
    let covered = (0..3)
        .map(|partition| {
            let mut proposals = vec![primary.clone()];
            ensure_route_family_partition(&ranking, &routes, partition, 3, &mut proposals).unwrap();
            assert_eq!(proposals[0], primary);
            let target = format!("goal.seek.route.{partition:02}");
            assert!(proposals.iter().any(|proposal| {
                route_family_id(&proposal.descriptor.option_id).as_deref() == Some(target.as_str())
            }));
            target
        })
        .collect::<BTreeSet<_>>();

    assert_eq!(
        covered,
        BTreeSet::from([
            "goal.seek.route.00".into(),
            "goal.seek.route.01".into(),
            "goal.seek.route.02".into(),
        ])
    );
}

#[test]
fn terminal_route_compositions_receive_a_dedicated_acquisition_lane() {
    let incumbent = descriptor(
        "goal.seek.route.01.roll.period.23.phase.00",
        OptionType::Custom("reactive_controller".into()),
    );
    let forward = descriptor(
        "goal.seek.route.01.crossover.04.split.01.roll.period.23.phase.00.radius.096",
        OptionType::Custom("reactive_controller".into()),
    );
    let reverse = descriptor(
        "goal.seek.route.04.crossover.01.split.01.roll.period.23.phase.00.radius.096",
        OptionType::Custom("reactive_controller".into()),
    );
    let unrelated = descriptor(
        "goal.seek.route.02.crossover.03.split.01.roll.period.23.phase.00.radius.096",
        OptionType::Custom("reactive_controller".into()),
    );
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([27; 32]),
        action_universe_sha256: Digest([28; 32]),
        choices: [
            incumbent.clone(),
            forward.clone(),
            reverse.clone(),
            unrelated.clone(),
        ]
        .into_iter()
        .map(choice)
        .collect(),
        values: AvailableOptionRanking {
            ranked: vec![RankedOption {
                action_id: 0,
                descriptor: incumbent.clone(),
                mean_q: 98.5,
                ensemble_variance: 0.0,
            }],
            unsupported: vec![forward.clone(), reverse.clone(), unrelated.clone()],
        },
    };
    let untried = vec![forward.clone(), reverse.clone(), unrelated];
    let selected = (0..2)
        .map(|partition| {
            let mut proposals = vec![SelectedTactic {
                schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
                learner_snapshot_sha256: ranking.learner_snapshot_sha256,
                decision_index: 0,
                descriptor: incumbent.clone(),
                reason: TacticSelectionReason::Greedy,
                exploration_draw: 0,
            }];
            ensure_route_composition_refinement(
                &ranking,
                &untried,
                Some(&incumbent),
                partition,
                4,
                &mut proposals,
            )
            .unwrap();
            assert_eq!(proposals[0].descriptor, incumbent);
            assert_eq!(proposals[1].reason, TacticSelectionReason::BatchDiversity);
            proposals[1].descriptor.option_id.clone()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        selected,
        BTreeSet::from([forward.option_id, reverse.option_id])
    );
}

#[test]
fn terminal_cost_refinement_preserves_control_and_selects_nearest_untried_variant() {
    fn rolling_route(period: u64) -> OptionActionDescriptor {
        let mut route = descriptor(
            &format!("goal.seek.route.00.roll.period.{period:02}.phase.00"),
            OptionType::Custom("reactive_controller".into()),
        );
        route.parameters.insert(
            "program_sha256".into(),
            OptionParameter::Digest(Digest([period as u8; 32])),
        );
        route.parameters.insert(
            "controller_base_sha256".into(),
            OptionParameter::Digest(Digest([9; 32])),
        );
        route.parameters.insert(
            "controller_structure_sha256".into(),
            OptionParameter::Digest(Digest([8; 32])),
        );
        route
            .parameters
            .insert("duration_ticks".into(), OptionParameter::Unsigned(160));
        route
            .parameters
            .insert("button_pulse_mask".into(), OptionParameter::Unsigned(8));
        route.parameters.insert(
            "button_pulse_period_ticks".into(),
            OptionParameter::Unsigned(period),
        );
        route.parameters.insert(
            "button_pulse_phase_tick".into(),
            OptionParameter::Unsigned(0),
        );
        route.parameters.insert(
            "waypoint_switch_radius".into(),
            OptionParameter::F32Bits(96.0_f32.to_bits()),
        );
        route
    }

    let incumbent = rolling_route(22);
    let period_20 = rolling_route(20);
    let period_24 = rolling_route(24);
    let mut other_path = rolling_route(22);
    other_path.option_id = "goal.seek.route.01.roll.period.22.phase.00".into();
    other_path.parameters.insert(
        "controller_base_sha256".into(),
        OptionParameter::Digest(Digest([10; 32])),
    );
    other_path.parameters.insert(
        "controller_structure_sha256".into(),
        OptionParameter::Digest(Digest([10; 32])),
    );
    let escape = descriptor("interact", OptionType::Interact);
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([31; 32]),
        action_universe_sha256: Digest([32; 32]),
        choices: vec![
            choice(incumbent.clone()),
            choice(period_20.clone()),
            choice(period_24.clone()),
            choice(other_path.clone()),
            choice(escape.clone()),
        ],
        values: AvailableOptionRanking {
            ranked: vec![RankedOption {
                action_id: 0,
                descriptor: incumbent.clone(),
                mean_q: 98.5,
                ensemble_variance: 0.0,
            }],
            unsupported: vec![
                period_20.clone(),
                period_24.clone(),
                other_path.clone(),
                escape.clone(),
            ],
        },
    };
    let mut proposals = choose_tactic_batch_with_state_untried(
        &ranking,
        4,
        TacticExplorationConfig {
            seed: 7,
            epsilon_per_million: 0,
        },
        &[period_20.clone(), period_24, other_path, escape],
        1,
    )
    .unwrap();
    let mut coverage = proposals[0].clone();
    coverage.descriptor = descriptor("interact", OptionType::Interact);
    coverage.reason = TacticSelectionReason::BatchCoverage;
    proposals.push(coverage);

    ensure_terminal_cost_refinement(
        &ranking,
        &[period_20.clone(), rolling_route(24)],
        Some(&incumbent),
        0,
        3,
        &mut proposals,
    )
    .unwrap();

    assert_eq!(proposals.len(), 3);
    assert_eq!(proposals[0].descriptor, incumbent);
    assert!(proposals[1].descriptor == period_20 || proposals[1].descriptor == rolling_route(24));
    assert_eq!(
        proposals[1].reason,
        TacticSelectionReason::TerminalCostRefinement
    );

    let selected_neighbors = (0..2)
        .map(|acquisition_partition| {
            let mut proposals = proposals.clone();
            proposals.remove(1);
            ensure_terminal_cost_refinement(
                &ranking,
                &[period_20.clone(), rolling_route(24)],
                Some(&incumbent),
                acquisition_partition,
                3,
                &mut proposals,
            )
            .unwrap();
            proposals[1].descriptor.option_id.clone()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        selected_neighbors,
        BTreeSet::from([
            period_20.option_id.clone(),
            rolling_route(24).option_id.clone()
        ])
    );

    let period_21 = rolling_route(21);
    let period_23 = rolling_route(23);
    let period_30 = rolling_route(30);
    let supported_24 = rolling_route(24);
    let learned_ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([39; 32]),
        action_universe_sha256: Digest([40; 32]),
        choices: [
            period_20.clone(),
            incumbent.clone(),
            supported_24.clone(),
            period_21.clone(),
            period_23.clone(),
            period_30.clone(),
        ]
        .into_iter()
        .map(choice)
        .collect(),
        values: AvailableOptionRanking {
            ranked: vec![
                RankedOption {
                    action_id: 1,
                    descriptor: incumbent.clone(),
                    mean_q: 99.0,
                    ensemble_variance: 0.0,
                },
                RankedOption {
                    action_id: 2,
                    descriptor: supported_24,
                    mean_q: 98.5,
                    ensemble_variance: 0.0,
                },
                RankedOption {
                    action_id: 0,
                    descriptor: period_20.clone(),
                    mean_q: 98.0,
                    ensemble_variance: 0.0,
                },
            ],
            unsupported: vec![period_21.clone(), period_23.clone(), period_30.clone()],
        },
    };
    let mut learned_proposals = vec![SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: learned_ranking.learner_snapshot_sha256,
        decision_index: 0,
        descriptor: incumbent.clone(),
        reason: TacticSelectionReason::Greedy,
        exploration_draw: 0,
    }];
    ensure_terminal_cost_refinement(
        &learned_ranking,
        &[period_21.clone(), period_23.clone(), period_30],
        Some(&incumbent),
        0,
        4,
        &mut learned_proposals,
    )
    .unwrap();
    assert_eq!(learned_proposals[1].descriptor, period_21);

    let wide_candidates = (12..=32)
        .filter(|period| *period != 22)
        .map(rolling_route)
        .collect::<Vec<_>>();
    let wide_ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([41; 32]),
        action_universe_sha256: Digest([42; 32]),
        choices: std::iter::once(choice(incumbent.clone()))
            .chain(wide_candidates.iter().cloned().map(choice))
            .collect(),
        values: AvailableOptionRanking {
            ranked: vec![RankedOption {
                action_id: 0,
                descriptor: incumbent.clone(),
                mean_q: 98.5,
                ensemble_variance: 0.0,
            }],
            unsupported: wide_candidates.clone(),
        },
    };
    let spread = (0..4)
        .map(|partition| {
            let mut proposals = vec![SelectedTactic {
                schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
                learner_snapshot_sha256: wide_ranking.learner_snapshot_sha256,
                decision_index: 0,
                descriptor: incumbent.clone(),
                reason: TacticSelectionReason::Greedy,
                exploration_draw: 0,
            }];
            ensure_terminal_cost_refinement(
                &wide_ranking,
                &wide_candidates,
                Some(&incumbent),
                partition,
                4,
                &mut proposals,
            )
            .unwrap();
            proposals[1].descriptor.option_id.clone()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(spread.len(), 4);
    assert!(spread.contains("goal.seek.route.00.roll.period.32.phase.00"));

    let mut phase_01 = incumbent.clone();
    phase_01.option_id = "goal.seek.route.00.roll.period.22.phase.01".into();
    phase_01.parameters.insert(
        "button_pulse_phase_tick".into(),
        OptionParameter::Unsigned(1),
    );
    let mut phase_21 = incumbent.clone();
    phase_21.option_id = "goal.seek.route.00.roll.period.22.phase.21".into();
    phase_21.parameters.insert(
        "button_pulse_phase_tick".into(),
        OptionParameter::Unsigned(21),
    );
    let mut radius_80 = incumbent.clone();
    radius_80.option_id = "goal.seek.route.00.roll.period.22.phase.00.radius.080".into();
    radius_80.parameters.insert(
        "controller_base_sha256".into(),
        OptionParameter::Digest(Digest([80; 32])),
    );
    radius_80.parameters.insert(
        "waypoint_switch_radius".into(),
        OptionParameter::F32Bits(80.0_f32.to_bits()),
    );
    let mut radius_112 = incumbent.clone();
    radius_112.option_id = "goal.seek.route.00.roll.period.22.phase.00.radius.112".into();
    radius_112.parameters.insert(
        "controller_base_sha256".into(),
        OptionParameter::Digest(Digest([112; 32])),
    );
    radius_112.parameters.insert(
        "waypoint_switch_radius".into(),
        OptionParameter::F32Bits(112.0_f32.to_bits()),
    );
    let axis_candidates = [
        wide_candidates,
        vec![
            phase_01.clone(),
            phase_21.clone(),
            radius_80.clone(),
            radius_112.clone(),
        ],
    ]
    .concat();
    let axis_ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([43; 32]),
        action_universe_sha256: Digest([44; 32]),
        choices: std::iter::once(choice(incumbent.clone()))
            .chain(axis_candidates.iter().cloned().map(choice))
            .collect(),
        values: AvailableOptionRanking {
            ranked: vec![RankedOption {
                action_id: 0,
                descriptor: incumbent.clone(),
                mean_q: 98.5,
                ensemble_variance: 0.0,
            }],
            unsupported: axis_candidates.clone(),
        },
    };
    let axis_selections = (0..4)
        .map(|partition| {
            let mut proposals = vec![SelectedTactic {
                schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
                learner_snapshot_sha256: axis_ranking.learner_snapshot_sha256,
                decision_index: 0,
                descriptor: incumbent.clone(),
                reason: TacticSelectionReason::Greedy,
                exploration_draw: 0,
            }];
            ensure_terminal_cost_refinement(
                &axis_ranking,
                &axis_candidates,
                Some(&incumbent),
                partition,
                4,
                &mut proposals,
            )
            .unwrap();
            proposals[1].descriptor.clone()
        })
        .collect::<Vec<_>>();
    assert_ne!(
        axis_selections[0]
            .parameters
            .get("button_pulse_period_ticks"),
        Some(&OptionParameter::Unsigned(22))
    );
    assert_eq!(
        axis_selections[1]
            .parameters
            .get("button_pulse_period_ticks"),
        Some(&OptionParameter::Unsigned(22))
    );
    assert!(
        axis_selections[1] == phase_01 || axis_selections[1] == phase_21,
        "phase lane did not preserve the incumbent period"
    );
    assert_eq!(
        axis_selections[3].option_id,
        "goal.seek.route.00.roll.period.32.phase.00"
    );
    let mut radius_proposals = vec![SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: axis_ranking.learner_snapshot_sha256,
        decision_index: 0,
        descriptor: incumbent.clone(),
        reason: TacticSelectionReason::Greedy,
        exploration_draw: 0,
    }];
    ensure_terminal_cost_refinement(
        &axis_ranking,
        &axis_candidates,
        Some(&incumbent),
        5,
        4,
        &mut radius_proposals,
    )
    .unwrap();
    assert!(
        radius_proposals[1].descriptor == radius_80 || radius_proposals[1].descriptor == radius_112,
        "successive local-axis generations did not rotate to waypoint lookahead"
    );

    let lower_incumbent = rolling_route(12);
    let mut lower_phase = lower_incumbent.clone();
    lower_phase.option_id = "goal.seek.route.00.roll.period.12.phase.01".into();
    lower_phase.parameters.insert(
        "button_pulse_phase_tick".into(),
        OptionParameter::Unsigned(1),
    );
    let lower_candidates = (13..=32)
        .flat_map(|period| {
            let phase_00 = rolling_route(period);
            let mut half_phase = phase_00.clone();
            half_phase.option_id = format!(
                "goal.seek.route.00.roll.period.{period:02}.phase.{:02}",
                period / 2
            );
            half_phase.parameters.insert(
                "button_pulse_phase_tick".into(),
                OptionParameter::Unsigned(period / 2),
            );
            [phase_00, half_phase]
        })
        .chain(std::iter::once(lower_phase))
        .collect::<Vec<_>>();
    let lower_ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([45; 32]),
        action_universe_sha256: Digest([46; 32]),
        choices: std::iter::once(choice(lower_incumbent.clone()))
            .chain(lower_candidates.iter().cloned().map(choice))
            .collect(),
        values: AvailableOptionRanking {
            ranked: vec![RankedOption {
                action_id: 0,
                descriptor: lower_incumbent.clone(),
                mean_q: 98.5,
                ensemble_variance: 0.0,
            }],
            unsupported: lower_candidates.clone(),
        },
    };
    let mut midpoint_proposals = vec![SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: lower_ranking.learner_snapshot_sha256,
        decision_index: 0,
        descriptor: lower_incumbent.clone(),
        reason: TacticSelectionReason::Greedy,
        exploration_draw: 0,
    }];
    ensure_terminal_cost_refinement(
        &lower_ranking,
        &lower_candidates,
        Some(&lower_incumbent),
        2,
        4,
        &mut midpoint_proposals,
    )
    .unwrap();
    assert_eq!(
        midpoint_proposals[1].descriptor.option_id,
        "goal.seek.route.00.roll.period.23.phase.00"
    );
}

#[test]
fn new_cells_explore_factorized_action_types_without_route_priority() {
    let mut supported_route = descriptor(
        "supported-route",
        OptionType::Custom("seek_coordinate_sequence".into()),
    );
    supported_route
        .parameters
        .insert("coordinates".into(), OptionParameter::Text("[]".into()));
    let mut fresh_route = supported_route.clone();
    fresh_route.option_id = "fresh-route".into();
    let roll = descriptor("roll", OptionType::Roll);
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([9; 32]),
        action_universe_sha256: Digest([10; 32]),
        choices: vec![
            choice(supported_route.clone()),
            choice(fresh_route.clone()),
            choice(roll.clone()),
        ],
        values: AvailableOptionRanking {
            ranked: vec![RankedOption {
                action_id: 0,
                descriptor: supported_route,
                mean_q: 1.0,
                ensemble_variance: 0.0,
            }],
            unsupported: vec![fresh_route.clone(), roll.clone()],
        },
    };

    let mut saw_roll = false;
    let mut saw_route = false;
    for seed in 0..64 {
        let selected = choose_tactic_with_state_untried(
            &ranking,
            0,
            TacticExplorationConfig {
                seed,
                epsilon_per_million: EPSILON_SCALE,
            },
            &[fresh_route.clone(), roll.clone()],
        )
        .unwrap();
        assert_eq!(selected.reason, TacticSelectionReason::Epsilon);
        saw_roll |= selected.descriptor.option_type == OptionType::Roll;
        saw_route |= selected.descriptor.option_type
            == OptionType::Custom("seek_coordinate_sequence".into());
    }

    assert!(saw_roll);
    assert!(saw_route);
}

#[test]
fn proposal_batch_preserves_primary_and_prioritizes_distinct_types() {
    let move_a = descriptor("move-a", OptionType::Move);
    let move_b = descriptor("move-b", OptionType::Move);
    let roll = descriptor("roll", OptionType::Roll);
    let wait = descriptor("wait", OptionType::Neutral);
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([11; 32]),
        action_universe_sha256: Digest([12; 32]),
        choices: vec![
            choice(move_b.clone()),
            choice(wait.clone()),
            choice(move_a.clone()),
            choice(roll.clone()),
        ],
        values: AvailableOptionRanking {
            ranked: vec![RankedOption {
                action_id: 0,
                descriptor: move_a.clone(),
                mean_q: 5.0,
                ensemble_variance: 0.0,
            }],
            unsupported: vec![move_b, roll, wait],
        },
    };
    let config = TacticExplorationConfig {
        seed: 42,
        epsilon_per_million: 0,
    };

    let primary = choose_tactic(&ranking, 3, config).unwrap();
    let first = choose_tactic_batch_with_state_untried(&ranking, 3, config, &[], 3).unwrap();
    let second = choose_tactic_batch_with_state_untried(&ranking, 3, config, &[], 3).unwrap();

    assert_eq!(first, second);
    assert_eq!(first[0], primary);
    assert_eq!(first.len(), 3);
    assert_eq!(first[1].reason, TacticSelectionReason::BatchCoverage);
    assert_eq!(first[2].reason, TacticSelectionReason::BatchCoverage);
    assert!(first[1].descriptor.option_type != OptionType::Move);
    assert!(first[2].descriptor.option_type != OptionType::Move);
    assert!(first[1].descriptor.option_type != first[2].descriptor.option_type);
}

#[test]
fn proposal_batch_has_separate_uncertainty_value_and_coverage_lanes() {
    let greedy = descriptor("greedy", OptionType::Move);
    let uncertain = descriptor("uncertain", OptionType::Bezier);
    let valuable = descriptor("valuable", OptionType::Roll);
    let fresh = descriptor("fresh", OptionType::Interact);
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([15; 32]),
        action_universe_sha256: Digest([16; 32]),
        choices: vec![
            choice(fresh.clone()),
            choice(valuable.clone()),
            choice(greedy.clone()),
            choice(uncertain.clone()),
        ],
        values: AvailableOptionRanking {
            ranked: vec![
                RankedOption {
                    action_id: 0,
                    descriptor: greedy.clone(),
                    mean_q: 9.0,
                    ensemble_variance: 0.1,
                },
                RankedOption {
                    action_id: 1,
                    descriptor: valuable.clone(),
                    mean_q: 7.0,
                    ensemble_variance: 0.2,
                },
                RankedOption {
                    action_id: 2,
                    descriptor: uncertain.clone(),
                    mean_q: 1.0,
                    ensemble_variance: 8.0,
                },
            ],
            unsupported: vec![fresh.clone()],
        },
    };
    let batch = choose_tactic_batch_with_state_untried(
        &ranking,
        0,
        TacticExplorationConfig {
            seed: 19,
            epsilon_per_million: 0,
        },
        &[uncertain.clone(), valuable.clone(), fresh.clone()],
        4,
    )
    .unwrap();

    assert_eq!(batch[0].descriptor, greedy);
    assert_eq!(batch[1].descriptor, uncertain);
    assert_eq!(batch[1].reason, TacticSelectionReason::BatchUncertainty);
    assert_eq!(batch[2].descriptor, valuable);
    assert_eq!(batch[2].reason, TacticSelectionReason::BatchValue);
    assert_eq!(batch[3].descriptor, fresh);
    assert_eq!(batch[3].reason, TacticSelectionReason::BatchCoverage);
}

#[test]
fn proposal_batch_covers_an_untried_action_before_remeasuring_ranked_actions() {
    let greedy = descriptor("greedy", OptionType::Move);
    let uncertain = descriptor("uncertain", OptionType::Bezier);
    let fresh = descriptor("fresh", OptionType::Interact);
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([17; 32]),
        action_universe_sha256: Digest([18; 32]),
        choices: vec![
            choice(fresh.clone()),
            choice(greedy.clone()),
            choice(uncertain.clone()),
        ],
        values: AvailableOptionRanking {
            ranked: vec![
                RankedOption {
                    action_id: 0,
                    descriptor: greedy.clone(),
                    mean_q: 9.0,
                    ensemble_variance: 0.1,
                },
                RankedOption {
                    action_id: 1,
                    descriptor: uncertain,
                    mean_q: 1.0,
                    ensemble_variance: 8.0,
                },
            ],
            unsupported: vec![fresh.clone()],
        },
    };

    let batch = choose_tactic_batch_with_state_untried(
        &ranking,
        0,
        TacticExplorationConfig {
            seed: 23,
            epsilon_per_million: 0,
        },
        std::slice::from_ref(&fresh),
        2,
    )
    .unwrap();

    assert_eq!(batch[0].descriptor, greedy);
    assert_eq!(batch[1].descriptor, fresh);
    assert_eq!(batch[1].reason, TacticSelectionReason::BatchCoverage);
}

#[test]
fn exploratory_batch_keeps_one_greedy_control() {
    let greedy = descriptor("greedy", OptionType::Move);
    let fresh = descriptor("fresh", OptionType::Interact);
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([19; 32]),
        action_universe_sha256: Digest([20; 32]),
        choices: vec![choice(fresh.clone()), choice(greedy.clone())],
        values: AvailableOptionRanking {
            ranked: vec![RankedOption {
                action_id: 0,
                descriptor: greedy.clone(),
                mean_q: 9.0,
                ensemble_variance: 0.1,
            }],
            unsupported: vec![fresh.clone()],
        },
    };

    let batch = choose_tactic_batch_with_state_untried(
        &ranking,
        0,
        TacticExplorationConfig {
            seed: 29,
            epsilon_per_million: EPSILON_SCALE,
        },
        std::slice::from_ref(&fresh),
        2,
    )
    .unwrap();

    assert_eq!(batch[0].descriptor, fresh);
    assert_eq!(batch[0].reason, TacticSelectionReason::Epsilon);
    assert_eq!(batch[1].descriptor, greedy);
    assert_eq!(batch[1].reason, TacticSelectionReason::BatchValue);
}

#[test]
fn proposal_batch_rejects_zero_capacity_and_never_duplicates_actions() {
    let move_a = descriptor("move-a", OptionType::Move);
    let move_b = descriptor("move-b", OptionType::Move);
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([13; 32]),
        action_universe_sha256: Digest([14; 32]),
        choices: vec![choice(move_b.clone()), choice(move_a.clone())],
        values: AvailableOptionRanking {
            ranked: vec![RankedOption {
                action_id: 0,
                descriptor: move_a,
                mean_q: 1.0,
                ensemble_variance: 0.0,
            }],
            unsupported: vec![move_b],
        },
    };
    let config = TacticExplorationConfig {
        seed: 7,
        epsilon_per_million: 0,
    };

    assert_eq!(
        choose_tactic_batch_with_state_untried(&ranking, 0, config, &[], 0),
        Err(TacticExplorationError::InvalidInput)
    );
    let batch =
        choose_tactic_batch_with_state_untried(&ranking, 0, config, &[], usize::MAX).unwrap();
    assert_eq!(batch.len(), 2);
    assert_ne!(batch[0].descriptor, batch[1].descriptor);
}

#[test]
fn equal_budget_baselines_ignore_learned_values_and_remain_seeded() {
    let choices = [
        descriptor("move", OptionType::Move),
        descriptor("roll", OptionType::Roll),
        descriptor("wait", OptionType::Neutral),
        descriptor("interact", OptionType::Interact),
    ];
    let mut ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([21; 32]),
        action_universe_sha256: Digest([22; 32]),
        choices: choices.iter().cloned().map(choice).collect(),
        values: AvailableOptionRanking {
            ranked: vec![RankedOption {
                action_id: 0,
                descriptor: choices[0].clone(),
                mean_q: 100.0,
                ensemble_variance: 0.0,
            }],
            unsupported: choices[1..].to_vec(),
        },
    };
    let config = TacticExplorationConfig {
        seed: 104_729,
        epsilon_per_million: 350_000,
    };
    let random = choose_tactic_batch_for_policy(
        &ranking,
        7,
        config,
        &choices,
        3,
        TacticProposalPolicy::RandomValid,
    )
    .unwrap();
    let structured = choose_tactic_batch_for_policy(
        &ranking,
        7,
        config,
        &choices,
        3,
        TacticProposalPolicy::StructuredNonLearning,
    )
    .unwrap();
    let learned = choose_tactic_batch_for_policy(
        &ranking,
        7,
        config,
        &choices,
        3,
        TacticProposalPolicy::Learned,
    )
    .unwrap();
    let frozen = choose_tactic_batch_for_policy(
        &ranking,
        7,
        config,
        &choices,
        3,
        TacticProposalPolicy::FrozenPolicy,
    )
    .unwrap();
    assert_eq!(frozen, learned);
    ranking.values.ranked[0].mean_q = -100.0;
    assert_eq!(
        random,
        choose_tactic_batch_for_policy(
            &ranking,
            7,
            config,
            &choices,
            3,
            TacticProposalPolicy::RandomValid,
        )
        .unwrap()
    );
    assert_eq!(
        structured,
        choose_tactic_batch_for_policy(
            &ranking,
            7,
            config,
            &choices,
            3,
            TacticProposalPolicy::StructuredNonLearning,
        )
        .unwrap()
    );
    assert!(
        random
            .iter()
            .all(|proposal| proposal.reason == TacticSelectionReason::RandomBaseline)
    );
    assert!(
        structured
            .iter()
            .all(|proposal| { proposal.reason == TacticSelectionReason::StructuredBaseline })
    );
    assert_eq!(
        random
            .iter()
            .map(|proposal| proposal.descriptor.option_id.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
}

#[test]
fn random_valid_baseline_obeys_the_live_applicability_mask() {
    let available = descriptor("available", OptionType::Move);
    let unavailable = descriptor("unavailable", OptionType::Roll);
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([23; 32]),
        action_universe_sha256: Digest([24; 32]),
        choices: vec![
            choice(available.clone()),
            LearnerActionMaskEntry {
                applicable: false,
                ..choice(unavailable)
            },
        ],
        values: AvailableOptionRanking {
            ranked: Vec::new(),
            unsupported: vec![available.clone()],
        },
    };
    let selected = choose_tactic_batch_for_policy(
        &ranking,
        0,
        TacticExplorationConfig {
            seed: 104_729,
            epsilon_per_million: 0,
        },
        std::slice::from_ref(&available),
        4,
        TacticProposalPolicy::RandomValid,
    )
    .unwrap();

    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].descriptor, available);
}

#[test]
fn learned_and_structured_batches_obey_the_live_applicability_mask() {
    let available = descriptor("available", OptionType::Move);
    let unavailable = descriptor("unavailable", OptionType::Roll);
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([25; 32]),
        action_universe_sha256: Digest([26; 32]),
        choices: vec![
            choice(available.clone()),
            LearnerActionMaskEntry {
                applicable: false,
                ..choice(unavailable.clone())
            },
        ],
        values: AvailableOptionRanking {
            ranked: vec![
                RankedOption {
                    action_id: 1,
                    descriptor: unavailable,
                    mean_q: 100.0,
                    ensemble_variance: 0.0,
                },
                RankedOption {
                    action_id: 0,
                    descriptor: available.clone(),
                    mean_q: 1.0,
                    ensemble_variance: 0.0,
                },
            ],
            unsupported: Vec::new(),
        },
    };
    let config = TacticExplorationConfig {
        seed: 104_729,
        epsilon_per_million: 0,
    };

    for policy in [
        TacticProposalPolicy::Learned,
        TacticProposalPolicy::FrozenPolicy,
        TacticProposalPolicy::StructuredNonLearning,
    ] {
        let selected = choose_tactic_batch_for_policy(
            &ranking,
            0,
            config,
            std::slice::from_ref(&available),
            4,
            policy,
        )
        .unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].descriptor, available);
    }
}

#[test]
fn generalized_value_can_confirm_a_supported_control_or_add_an_applicable_peer() {
    let control = descriptor("known/control", OptionType::Move);
    let duplicate = control.clone();
    let held_out_roll = descriptor("unseen/roll", OptionType::Roll);
    let fallback = descriptor("unseen/fallback", OptionType::Move);
    let mut proposals = vec![SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: Digest([31; 32]),
        decision_index: 7,
        descriptor: control.clone(),
        reason: TacticSelectionReason::Greedy,
        exploration_draw: 0,
    }];

    ensure_generalized_value_acquisition(
        &[duplicate, held_out_roll.clone(), fallback],
        0,
        2,
        &mut proposals,
    )
    .unwrap();

    assert_eq!(proposals[0].descriptor, control);
    assert_eq!(proposals[0].reason, TacticSelectionReason::GeneralizedValue);
    assert_eq!(proposals.len(), 1);

    proposals[0].reason = TacticSelectionReason::Greedy;
    ensure_generalized_value_acquisition(
        &[control.clone(), held_out_roll.clone()],
        1,
        2,
        &mut proposals,
    )
    .unwrap();

    assert_eq!(proposals[0].descriptor, control);
    assert_eq!(proposals[0].reason, TacticSelectionReason::Greedy);
    assert_eq!(proposals[1].descriptor, held_out_roll);
    assert_eq!(proposals[1].reason, TacticSelectionReason::GeneralizedValue);
}

#[test]
fn generalized_value_can_become_primary_without_dropping_its_control() {
    let control = descriptor("known/control", OptionType::Move);
    let predicted = descriptor("unseen/predicted", OptionType::Roll);
    let mut proposals = vec![
        SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: Digest([31; 32]),
            decision_index: 7,
            descriptor: control.clone(),
            reason: TacticSelectionReason::UnsupportedBootstrap,
            exploration_draw: 0,
        },
        SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: Digest([31; 32]),
            decision_index: 7,
            descriptor: predicted.clone(),
            reason: TacticSelectionReason::GeneralizedValue,
            exploration_draw: 0,
        },
    ];

    retain_generalized_value_acquisition(&mut proposals).unwrap();

    assert_eq!(proposals[0].descriptor, predicted);
    assert_eq!(proposals[0].reason, TacticSelectionReason::GeneralizedValue);
    assert_eq!(proposals[1].descriptor, control);
    assert_eq!(
        proposals[1].reason,
        TacticSelectionReason::UnsupportedBootstrap
    );
}

#[test]
fn goal_reachability_is_an_explicit_preterminal_primary() {
    let control = descriptor("known/control", OptionType::Move);
    let reachable = descriptor("unseen/reachable", OptionType::Roll);
    let mut proposals = vec![SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: Digest([31; 32]),
        decision_index: 7,
        descriptor: control.clone(),
        reason: TacticSelectionReason::UnsupportedBootstrap,
        exploration_draw: 0,
    }];

    ensure_goal_reachability_acquisition(std::slice::from_ref(&reachable), 0, 2, &mut proposals)
        .unwrap();
    retain_goal_reachability_acquisition(&mut proposals).unwrap();

    assert_eq!(proposals[0].descriptor, reachable);
    assert_eq!(proposals[0].reason, TacticSelectionReason::GoalReachability);
    assert_eq!(proposals[1].descriptor, control);
}

#[test]
fn unproven_goal_reachability_is_sibling_evidence_only() {
    let control = descriptor("known/control", OptionType::Move);
    let reachable = descriptor("unseen/reachable", OptionType::Roll);
    let proposal = |descriptor, reason| SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: Digest([31; 32]),
        decision_index: 7,
        descriptor,
        reason,
        exploration_draw: 0,
    };

    let mut proposals = vec![proposal(
        control.clone(),
        TacticSelectionReason::UnsupportedBootstrap,
    )];
    ensure_goal_reachability_evidence(std::slice::from_ref(&reachable), 2, &mut proposals).unwrap();
    assert_eq!(proposals[0].descriptor, control);
    assert_eq!(
        proposals[0].reason,
        TacticSelectionReason::UnsupportedBootstrap
    );
    assert_eq!(proposals[1].descriptor, reachable);
    assert_eq!(proposals[1].reason, TacticSelectionReason::GoalReachability);

    let mut already_present = vec![proposal(
        control.clone(),
        TacticSelectionReason::UnsupportedBootstrap,
    )];
    ensure_goal_reachability_evidence(&[control.clone()], 2, &mut already_present).unwrap();
    assert_eq!(already_present.len(), 1);
    assert_eq!(already_present[0].descriptor, control);
    assert_eq!(
        already_present[0].reason,
        TacticSelectionReason::UnsupportedBootstrap
    );
}

#[test]
fn goal_reachability_keeps_the_top_prediction_across_worker_partitions() {
    let control = descriptor("known/control", OptionType::Neutral);
    let best = descriptor("predicted/best", OptionType::Move);
    let second_same_type = descriptor("predicted/second", OptionType::Move);
    let third_other_type = descriptor("predicted/third", OptionType::Roll);
    let proposal = || SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: Digest([31; 32]),
        decision_index: 7,
        descriptor: control.clone(),
        reason: TacticSelectionReason::UnsupportedBootstrap,
        exploration_draw: 0,
    };
    let ranked = [best.clone(), second_same_type, third_other_type];

    for partition in [0, 1, 12, 127] {
        let mut proposals = vec![proposal()];
        ensure_goal_reachability_acquisition(&ranked, partition, 2, &mut proposals).unwrap();
        retain_goal_reachability_acquisition(&mut proposals).unwrap();
        assert_eq!(proposals[0].descriptor, best);
        assert_eq!(proposals[0].reason, TacticSelectionReason::GoalReachability);
    }
}

#[test]
fn generalized_value_does_not_override_an_epsilon_primary() {
    let exploratory = descriptor("unseen/epsilon", OptionType::Move);
    let predicted = descriptor("unseen/predicted", OptionType::Roll);
    let mut proposals = vec![
        SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: Digest([31; 32]),
            decision_index: 7,
            descriptor: exploratory.clone(),
            reason: TacticSelectionReason::Epsilon,
            exploration_draw: 1,
        },
        SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: Digest([31; 32]),
            decision_index: 7,
            descriptor: predicted.clone(),
            reason: TacticSelectionReason::GeneralizedValue,
            exploration_draw: 1,
        },
    ];

    retain_generalized_value_acquisition(&mut proposals).unwrap();

    assert_eq!(proposals[0].descriptor, exploratory);
    assert_eq!(proposals[0].reason, TacticSelectionReason::Epsilon);
    assert_eq!(proposals[1].descriptor, predicted);
    assert_eq!(proposals[1].reason, TacticSelectionReason::GeneralizedValue);

    let mut confirmed = vec![proposals[0].clone()];
    ensure_generalized_value_acquisition(std::slice::from_ref(&exploratory), 0, 2, &mut confirmed)
        .unwrap();
    retain_generalized_value_acquisition(&mut confirmed).unwrap();
    assert_eq!(confirmed[0].descriptor, exploratory);
    assert_eq!(confirmed[0].reason, TacticSelectionReason::Epsilon);
}

#[test]
fn generalized_value_does_not_override_a_supported_greedy_primary() {
    let supported = descriptor("known/terminal", OptionType::Target);
    let predicted = descriptor("unseen/predicted", OptionType::Bezier);
    let mut proposals = vec![
        SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: Digest([31; 32]),
            decision_index: 7,
            descriptor: supported.clone(),
            reason: TacticSelectionReason::Greedy,
            exploration_draw: 1,
        },
        SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: Digest([31; 32]),
            decision_index: 7,
            descriptor: predicted.clone(),
            reason: TacticSelectionReason::GeneralizedValue,
            exploration_draw: 1,
        },
    ];

    retain_generalized_value_acquisition(&mut proposals).unwrap();

    assert_eq!(proposals[0].descriptor, supported);
    assert_eq!(proposals[0].reason, TacticSelectionReason::Greedy);
    assert_eq!(proposals[1].descriptor, predicted);
    assert_eq!(proposals[1].reason, TacticSelectionReason::GeneralizedValue);
}

#[test]
fn generalized_value_partitions_ranked_acquisition_across_workers() {
    let control = descriptor("known/control", OptionType::Move);
    let ranked = (0..140)
        .map(|index| descriptor(&format!("unseen/{index:03}"), OptionType::Move))
        .collect::<Vec<_>>();
    let proposal = || SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: Digest([31; 32]),
        decision_index: 7,
        descriptor: control.clone(),
        reason: TacticSelectionReason::Greedy,
        exploration_draw: 0,
    };

    let mut first = vec![proposal()];
    ensure_generalized_value_acquisition(&ranked, 0, 2, &mut first).unwrap();
    let mut last = vec![proposal()];
    ensure_generalized_value_acquisition(&ranked, 127, 2, &mut last).unwrap();
    let mut wrapped = vec![proposal()];
    ensure_generalized_value_acquisition(&ranked, 128, 2, &mut wrapped).unwrap();

    assert_eq!(first[1].descriptor, ranked[0]);
    assert_eq!(last[1].descriptor, ranked[127]);
    assert_eq!(wrapped[1].descriptor, ranked[0]);
}

#[test]
fn generalized_value_interleaves_typed_actions_before_parameter_variants() {
    let ranked = vec![
        descriptor("roll/best", OptionType::Roll),
        descriptor("roll/second", OptionType::Roll),
        descriptor("move/best", OptionType::Move),
        descriptor("roll/third", OptionType::Roll),
        descriptor("neutral/best", OptionType::Neutral),
        descriptor("move/second", OptionType::Move),
    ];

    let interleaved = interleave_ranked_action_types(&ranked)
        .into_iter()
        .map(|descriptor| descriptor.option_id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        interleaved,
        vec![
            "roll/best",
            "move/best",
            "neutral/best",
            "roll/second",
            "move/second",
            "roll/third",
        ]
    );
}

#[test]
fn terminal_support_batch_transfers_to_distinct_learned_action_factors() {
    let control = descriptor("move/control", OptionType::Move);
    let ranked = vec![
        descriptor("move/best", OptionType::Move),
        descriptor("move/second", OptionType::Move),
        descriptor("target/best", OptionType::Target),
        descriptor("roll/best", OptionType::Roll),
        descriptor("neutral/best", OptionType::Neutral),
        descriptor("curve/best", OptionType::Bezier),
    ];
    let mut proposals = vec![
        SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: Digest([31; 32]),
            decision_index: 7,
            descriptor: control.clone(),
            reason: TacticSelectionReason::Greedy,
            exploration_draw: 0,
        },
        SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: Digest([31; 32]),
            decision_index: 7,
            descriptor: descriptor("coverage/random", OptionType::Roll),
            reason: TacticSelectionReason::BatchCoverage,
            exploration_draw: 0,
        },
    ];

    ensure_terminal_support_factor_acquisitions(&ranked, 4, &mut proposals).unwrap();

    assert_eq!(proposals[0].descriptor, control);
    assert_eq!(
        proposals[1..]
            .iter()
            .map(|proposal| proposal.descriptor.option_id.as_str())
            .collect::<Vec<_>>(),
        vec!["roll/best", "target/best", "neutral/best"]
    );
    assert!(
        proposals[1..]
            .iter()
            .all(|proposal| proposal.reason == TacticSelectionReason::GeneralizedValue)
    );
}

#[test]
fn terminal_support_factor_transfer_preserves_epsilon_authority() {
    let exploratory = descriptor("move/epsilon", OptionType::Move);
    let ranked = vec![
        descriptor("target/best", OptionType::Target),
        descriptor("roll/best", OptionType::Roll),
    ];
    let mut proposals = vec![SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: Digest([31; 32]),
        decision_index: 7,
        descriptor: exploratory.clone(),
        reason: TacticSelectionReason::Epsilon,
        exploration_draw: 1,
    }];

    ensure_terminal_support_factor_acquisitions(&ranked, 4, &mut proposals).unwrap();
    retain_generalized_value_acquisition(&mut proposals).unwrap();

    assert_eq!(proposals[0].descriptor, exploratory);
    assert_eq!(proposals[0].reason, TacticSelectionReason::Epsilon);
    assert_eq!(proposals.len(), 3);
}

#[test]
fn terminal_support_factor_transfer_preserves_long_movement_probe() {
    let exploratory = descriptor_with_duration("move/epsilon-short", OptionType::Move, 4);
    let ranked = vec![
        descriptor_with_duration("move/best-short", OptionType::Move, 4),
        descriptor_with_duration("move/second-short", OptionType::Move, 4),
        descriptor_with_duration("neutral/short", OptionType::Neutral, 4),
        descriptor_with_duration("roll/short", OptionType::Roll, 4),
        descriptor_with_duration("curve/short", OptionType::Bezier, 4),
        descriptor_with_duration("move/long", OptionType::Move, 40),
    ];
    let mut proposals = vec![SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: Digest([31; 32]),
        decision_index: 7,
        descriptor: exploratory.clone(),
        reason: TacticSelectionReason::Epsilon,
        exploration_draw: 1,
    }];

    ensure_terminal_support_factor_acquisitions(&ranked, 4, &mut proposals).unwrap();
    retain_generalized_value_acquisition(&mut proposals).unwrap();

    assert_eq!(proposals[0].descriptor, exploratory);
    assert_eq!(proposals[0].reason, TacticSelectionReason::Epsilon);
    assert_eq!(
        proposals[1..]
            .iter()
            .map(|proposal| proposal.descriptor.option_id.as_str())
            .collect::<Vec<_>>(),
        vec!["roll/short", "move/long", "neutral/short"]
    );
}

#[test]
fn terminal_support_factor_transfer_covers_each_prompted_button_mask() {
    let exploratory =
        descriptor_with_duration_and_buttons("move/epsilon", OptionType::MaintainHeading, 4, 0);
    let ranked = vec![
        descriptor_with_duration_and_buttons("curve/best", OptionType::Bezier, 24, 0),
        descriptor_with_duration_and_buttons(
            "target-roll/best",
            OptionType::Custom("target_roll".into()),
            16,
            0x0140,
        ),
        descriptor_with_duration_and_buttons("roll/best", OptionType::Roll, 4, 0x0100),
        descriptor_with_duration_and_buttons("target/best", OptionType::Target, 40, 0x0040),
        descriptor_with_duration_and_buttons("move/second", OptionType::Move, 40, 0),
    ];
    let mut proposals = vec![SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: Digest([31; 32]),
        decision_index: 7,
        descriptor: exploratory.clone(),
        reason: TacticSelectionReason::Epsilon,
        exploration_draw: 1,
    }];

    ensure_terminal_support_factor_acquisitions(&ranked, 4, &mut proposals).unwrap();
    retain_generalized_value_acquisition(&mut proposals).unwrap();

    assert_eq!(proposals[0].descriptor, exploratory);
    assert_eq!(proposals[0].reason, TacticSelectionReason::Epsilon);
    assert_eq!(
        proposals[1..]
            .iter()
            .map(|proposal| (
                proposal.descriptor.option_id.as_str(),
                action_button_mask(&proposal.descriptor),
            ))
            .collect::<Vec<_>>(),
        vec![
            ("target-roll/best", 0x0140),
            ("roll/best", 0x0100),
            ("target/best", 0x0040),
        ]
    );
}

#[test]
fn adding_parameter_variants_does_not_change_epsilon_action_type() {
    let base = vec![
        descriptor("a/move", OptionType::Move),
        descriptor("b/roll", OptionType::Roll),
        descriptor("c/neutral", OptionType::Neutral),
    ];
    let mut expanded = base.clone();
    for index in 0..16 {
        expanded.push(descriptor(&format!("b/roll/{index:02}"), OptionType::Roll));
    }
    let base = canonical_candidates(&base);
    let expanded = canonical_candidates(&expanded);

    for seed in 0..64 {
        let selected_base = deterministic_factorized_candidate(&base, seed, 7, Digest([9; 32]));
        let selected_expanded =
            deterministic_factorized_candidate(&expanded, seed, 7, Digest([9; 32]));
        assert_eq!(
            selected_base.option_type, selected_expanded.option_type,
            "seed {seed}"
        );
    }
}

#[test]
fn coverage_ties_preserve_seeded_candidate_rotation() {
    let first_after_rotation = descriptor(
        "route/z-after-rotation",
        OptionType::Custom("controller".into()),
    );
    let lexicographically_first = descriptor("route/a", OptionType::Custom("controller".into()));
    let ranking = LiveTacticRanking {
        learner_snapshot_sha256: Digest([41; 32]),
        action_universe_sha256: Digest([42; 32]),
        choices: vec![
            choice(first_after_rotation.clone()),
            choice(lexicographically_first.clone()),
        ],
        values: AvailableOptionRanking {
            ranked: Vec::new(),
            unsupported: vec![
                first_after_rotation.clone(),
                lexicographically_first.clone(),
            ],
        },
    };
    let candidates = vec![first_after_rotation, lexicographically_first];

    assert_eq!(
        select_batch_candidate(
            &candidates,
            &ranking,
            &candidates,
            &[OptionType::Custom("controller".into())],
            BatchAcquisitionLane::Coverage,
        ),
        Some(0)
    );
}
