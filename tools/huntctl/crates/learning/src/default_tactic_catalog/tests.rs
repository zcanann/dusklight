use super::*;
use crate::tactic_asset::PreparedTacticExecution;
use dusklight_control::option_execution::OptionParameter;
use std::collections::BTreeSet;

#[test]
fn default_catalog_is_complete_bounded_and_route_agnostic() {
    let catalog = default_route_tactic_catalog().unwrap();
    assert_eq!(catalog.entries().len(), DEFAULT_ROUTE_TACTIC_COUNT);
    assert_eq!(
        catalog
            .option_descriptors()
            .map(|descriptor| &descriptor.option_id)
            .collect::<BTreeSet<_>>()
            .len(),
        DEFAULT_ROUTE_TACTIC_COUNT
    );
    let canonical = serde_json::to_string(
        &catalog
            .entries()
            .iter()
            .map(TacticCatalogEntry::description)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(!canonical.contains("coordinate"));
    assert!(!canonical.contains("actor"));
    assert!(!canonical.contains("route"));
    assert!(matches!(
        catalog.prepare_execution("wait.neutral.04").unwrap(),
        PreparedTacticExecution::Static(_)
    ));
    assert!(matches!(
        catalog
            .prepare_execution("move.heading.00.magnitude.080.ticks.04")
            .unwrap(),
        PreparedTacticExecution::NativeGeneric(_)
    ));
    assert!(catalog.entry("move.curve.clockwise.00").is_some());
    assert!(catalog.entry("move.curve.counterclockwise.00").is_some());
    assert!(matches!(
        catalog.entry("move.curve.clockwise.00").unwrap().source(),
        TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan {
            tactic: GenericTactic::ShortCurve { control },
            maximum_ticks: 32,
            ..
        }) if control[0] == [0, 127] && control[3] == [90, 90]
    ));
}

#[test]
fn goal_conditioned_catalog_exposes_derived_seek_targets_as_ordinary_actions() {
    let goal = [-1842.0, 717.0, -4739.0];
    let waypoint = [-900.0, 750.0, -3600.0];
    let catalog =
        goal_conditioned_route_tactic_catalog(&[goal, waypoint], &[vec![waypoint, goal]], 160, 160)
            .unwrap();
    assert_eq!(
        catalog.entries().len(),
        DEFAULT_ROUTE_TACTIC_COUNT
            + 3
            + ((MAX_GOAL_ROUTE_ROLL_PERIOD - MIN_GOAL_ROUTE_ROLL_PERIOD + 1) as usize) * 2
    );
    for period in MIN_GOAL_ROUTE_ROLL_PERIOD..=MAX_GOAL_ROUTE_ROLL_PERIOD {
        for phase in [0, period / 2] {
            assert!(
                catalog
                    .entry(&format!(
                        "goal.seek.route.00.roll.period.{period:02}.phase.{phase:02}"
                    ))
                    .is_some(),
                "missing integer roll cadence period={period} phase={phase}"
            );
        }
    }
    assert!(matches!(
        catalog.entry("goal.seek.route.00").unwrap().source(),
        TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan {
            tactic: GenericTactic::SeekCoordinateSequence {
                coordinates_f32_bits,
                stall_grace_ticks,
                stationary_window_ticks: GOAL_ROUTE_STATIONARY_WINDOW_TICKS,
                stationary_window_distance_f32_bits,
                ..
            },
            maximum_ticks: 160,
            ..
        }) if coordinates_f32_bits
            .iter()
            .map(|coordinate| coordinate.map(f32::from_bits))
            .collect::<Vec<_>>() == vec![waypoint, goal]
            && *stall_grace_ticks == 160
            && f32::from_bits(*stationary_window_distance_f32_bits)
                == GOAL_ROUTE_STATIONARY_WINDOW_DISTANCE
    ));
    let rolling = catalog
        .entry("goal.seek.route.00.roll.period.20.phase.00")
        .expect("goal route must expose a layered rolling composition");
    let TacticAssetSource::ReactiveController(program) = rolling.source() else {
        panic!("rolling goal route must be one native controller program");
    };
    assert_eq!(
        rolling
            .description()
            .option
            .parameters
            .get("button_pulse_period_ticks"),
        Some(&OptionParameter::Unsigned(20))
    );
    assert_eq!(
        rolling
            .description()
            .option
            .parameters
            .get("button_pulse_phase_tick"),
        Some(&OptionParameter::Unsigned(0))
    );
    assert_eq!(
        rolling
            .description()
            .option
            .parameters
            .get("waypoint_switch_radius"),
        Some(&OptionParameter::F32Bits(
            INTERMEDIATE_GOAL_SEEK_TOLERANCE.to_bits()
        ))
    );
    assert_eq!(
        rolling
            .description()
            .option
            .parameters
            .get("controller_base_sha256"),
        catalog
            .entry("goal.seek.route.00.roll.period.22.phase.00")
            .unwrap()
            .description()
            .option
            .parameters
            .get("controller_base_sha256")
    );
    assert_eq!(
        rolling
            .description()
            .option
            .parameters
            .get("controller_structure_sha256"),
        catalog
            .entry("goal.seek.route.00.roll.period.22.phase.00")
            .unwrap()
            .description()
            .option
            .parameters
            .get("controller_structure_sha256")
    );
    let phase_variants =
        goal_route_roll_phase_variants(&catalog, &rolling.description().option).unwrap();
    assert_eq!(phase_variants.len(), 18);
    assert!(
        phase_variants
            .iter()
            .any(|entry| entry.option_id() == "goal.seek.route.00.roll.period.20.phase.01")
    );
    assert!(
        phase_variants
            .iter()
            .any(|entry| entry.option_id() == "goal.seek.route.00.roll.period.20.phase.19")
    );
    assert!(phase_variants.iter().all(|entry| {
        entry
            .description()
            .option
            .parameters
            .get("controller_base_sha256")
            == rolling
                .description()
                .option
                .parameters
                .get("controller_base_sha256")
            && entry
                .description()
                .option
                .parameters
                .get("button_pulse_period_ticks")
                == Some(&OptionParameter::Unsigned(20))
    }));
    let waypoint_variants =
        goal_route_waypoint_switch_variants(&catalog, &rolling.description().option).unwrap();
    assert_eq!(
        waypoint_variants.len(),
        GOAL_ROUTE_WAYPOINT_SWITCH_RADII.len()
    );
    let radius_80 = waypoint_variants
        .iter()
        .find(|entry| entry.option_id() == "goal.seek.route.00.roll.period.20.phase.00.radius.080")
        .unwrap();
    assert_eq!(
        radius_80
            .description()
            .option
            .parameters
            .get("waypoint_switch_radius"),
        Some(&OptionParameter::F32Bits(80.0_f32.to_bits()))
    );
    assert_eq!(
        radius_80
            .description()
            .option
            .parameters
            .get("controller_structure_sha256"),
        rolling
            .description()
            .option
            .parameters
            .get("controller_structure_sha256")
    );
    assert_ne!(
        radius_80
            .description()
            .option
            .parameters
            .get("controller_base_sha256"),
        rolling
            .description()
            .option
            .parameters
            .get("controller_base_sha256")
    );
    let radius_phases =
        goal_route_roll_phase_variants(&catalog, &radius_80.description().option).unwrap();
    assert_eq!(radius_phases.len(), 19);
    assert!(radius_phases.iter().any(|entry| {
        entry.option_id() == "goal.seek.route.00.roll.period.20.phase.01.radius.080"
    }));
    assert!(matches!(
        program.layers[0].operation,
        Operation::SeekCoordinateSequence { .. }
    ));
    assert_eq!(
        program
            .layers
            .iter()
            .filter(|layer| matches!(
                layer.operation,
                Operation::Buttons {
                    mask: GOAL_ROUTE_ROLL_BUTTON_MASK
                }
            ))
            .count(),
        8
    );
    let goal_entry = catalog.entry("goal.seek.coordinate.00").unwrap();
    assert_eq!(goal_entry.description().duration.maximum_ticks, 160);
    assert!(matches!(
        goal_entry.source(),
        TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan {
            tactic: GenericTactic::SeekCoordinate {
                coordinate_f32_bits,
                tolerance_f32_bits,
                ..
            },
            ..
        }) if coordinate_f32_bits.map(f32::from_bits) == goal
            && f32::from_bits(*tolerance_f32_bits) == 0.0
    ));
    assert!(matches!(
        catalog.entry("goal.seek.coordinate.01").unwrap().source(),
        TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan {
            tactic: GenericTactic::SeekCoordinate {
                coordinate_f32_bits,
                tolerance_f32_bits,
                ..
            },
            minimum_ticks,
            maximum_ticks,
            ..
        }) if coordinate_f32_bits.map(f32::from_bits) == waypoint
            && f32::from_bits(*tolerance_f32_bits) == INTERMEDIATE_GOAL_SEEK_TOLERANCE
            && minimum_ticks == maximum_ticks
    ));
}

#[test]
fn long_goal_routes_are_composed_inside_the_existing_controller_wire_limit() {
    let route = (0..8)
        .map(|index| [index as f32 * 100.0, 10.0, index as f32 * -200.0])
        .collect::<Vec<_>>();
    let catalog = goal_conditioned_route_tactic_catalog(&[route[7]], &[route], 160, 160).unwrap();

    for option_id in [
        "goal.seek.route.00",
        "goal.seek.route.00.roll.period.20.phase.00",
    ] {
        let TacticAssetSource::ReactiveController(program) =
            catalog.entry(option_id).unwrap().source()
        else {
            panic!("long routes must be one composed native controller program");
        };
        let sequence_layers = program
            .layers
            .iter()
            .filter(|layer| matches!(layer.operation, Operation::SeekCoordinateSequence { .. }))
            .collect::<Vec<_>>();
        assert_eq!(sequence_layers.len(), 2);
        assert_eq!(
            sequence_layers
                .iter()
                .map(|layer| (layer.start_frame, layer.duration_frames))
                .collect::<Vec<_>>(),
            vec![(0, 80), (80, 80)]
        );
        assert!(sequence_layers.iter().all(|layer| {
            matches!(
                &layer.operation,
                Operation::SeekCoordinateSequence { coordinates_xz, .. }
                    if coordinates_xz.len() == 4
            )
        }));
        assert!(matches!(
            sequence_layers[0].operation,
            Operation::SeekCoordinateSequence {
                final_stop_radius: INTERMEDIATE_GOAL_SEEK_TOLERANCE,
                ..
            }
        ));
        assert!(matches!(
            sequence_layers[1].operation,
            Operation::SeekCoordinateSequence {
                final_stop_radius: 0.0,
                ..
            }
        ));
    }
}

#[test]
fn terminal_route_crossovers_compose_compatible_waypoint_sequences() {
    let left = vec![
        [0.0, 10.0, 0.0],
        [100.0, 10.0, -100.0],
        [200.0, 10.0, -200.0],
        [300.0, 10.0, -300.0],
    ];
    let right = vec![
        [0.0, 20.0, 40.0],
        [120.0, 20.0, -80.0],
        [240.0, 20.0, -180.0],
        [300.0, 20.0, -300.0],
    ];
    let catalog = goal_conditioned_route_tactic_catalog(
        &[[300.0, 10.0, -300.0]],
        &[left.clone(), right.clone()],
        160,
        160,
    )
    .unwrap();
    let incumbent = &catalog
        .entry("goal.seek.route.00.roll.period.20.phase.00")
        .unwrap()
        .description()
        .option;

    let variants = goal_route_crossover_variants(&catalog, incumbent).unwrap();

    assert!(variants.len() <= MAX_GOAL_ROUTE_CROSSOVER_VARIANTS);
    assert!(variants.iter().any(|entry| {
        entry.option_id()
            == "goal.seek.route.01.crossover.00.split.01.roll.period.17.phase.00.radius.096"
    }));
    assert!(variants.iter().any(|entry| {
        entry.option_id()
            == "goal.seek.route.01.crossover.00.split.01.roll.period.20.phase.00.radius.112"
    }));
    let composed = variants
        .iter()
        .find(|entry| {
            entry.option_id()
                == "goal.seek.route.00.crossover.01.split.01.roll.period.20.phase.00.radius.096"
        })
        .unwrap();
    let TacticAssetSource::ReactiveController(program) = composed.source() else {
        panic!("route crossover must remain one reactive controller");
    };
    assert!(matches!(
        &program.layers[0].operation,
        Operation::SeekCoordinateSequence {
            coordinates_xz,
            intermediate_stop_radius: INTERMEDIATE_GOAL_SEEK_TOLERANCE,
            ..
        } if coordinates_xz
            == &vec![
                [left[0][0], left[0][2]],
                [right[1][0], right[1][2]],
                [right[2][0], right[2][2]],
                [right[3][0], right[3][2]],
            ]
    ));
    assert_eq!(
        program
            .layers
            .iter()
            .filter(|layer| matches!(
                layer.operation,
                Operation::Buttons {
                    mask: GOAL_ROUTE_ROLL_BUTTON_MASK
                }
            ))
            .count(),
        8
    );

    let dynamic_incumbent = composed.description().option.clone();
    let refinements = goal_route_crossover_variants(&catalog, &dynamic_incumbent).unwrap();
    let blended = refinements
            .iter()
            .find(|entry| {
                entry.option_id()
                    == "goal.seek.route.00.crossover.01.split.01.blend.050.roll.period.20.phase.00.radius.096"
            })
            .unwrap();
    let TacticAssetSource::ReactiveController(program) = blended.source() else {
        panic!("blended crossover must remain one reactive controller");
    };
    assert!(matches!(
        &program.layers[0].operation,
        Operation::SeekCoordinateSequence { coordinates_xz, .. }
            if coordinates_xz[0]
                == [
                    (left[0][0] + right[0][0]) * 0.5,
                    (left[0][2] + right[0][2]) * 0.5,
                ]
    ));
    let local_refinements =
        goal_route_crossover_variants(&catalog, &blended.description().option).unwrap();
    assert!(local_refinements.len() <= MAX_GOAL_ROUTE_CROSSOVER_VARIANTS);
    for blend in [40, 45, 55, 60] {
        assert!(local_refinements.iter().any(|entry| {
                entry.option_id()
                    == format!(
                        "goal.seek.route.00.crossover.01.split.01.blend.{blend:03}.roll.period.20.phase.00.radius.096"
                    )
            }));
    }
    assert!(local_refinements.iter().any(|entry| {
            entry.option_id()
                == "goal.seek.route.00.crossover.01.split.01.blend.050.roll.period.20.phase.00.radius.640"
        }));
    assert!(local_refinements.iter().any(|entry| {
            entry.option_id()
                == "goal.seek.route.00.crossover.01.split.01.blend.050.roll.period.20.phase.19.radius.096"
        }));
    assert!(local_refinements.iter().all(|entry| {
        entry
            .description()
            .option
            .parameters
            .get("button_pulse_period_ticks")
            == Some(&OptionParameter::Unsigned(20))
    }));
}

#[test]
fn crossover_catalog_retains_the_complete_five_cadence_neighborhood() {
    let routes = (0..5)
        .map(|route| {
            (0..4)
                .map(|point| {
                    [
                        point as f32 * 100.0 + route as f32 * 10.0,
                        10.0,
                        -(point as f32 * 100.0) - route as f32 * 5.0,
                    ]
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let catalog =
        goal_conditioned_route_tactic_catalog(&[[300.0, 10.0, -300.0]], &routes, 160, 160).unwrap();
    let incumbent = &catalog
        .entry("goal.seek.route.01.roll.period.23.phase.00")
        .unwrap()
        .description()
        .option;

    let variants = goal_route_crossover_variants(&catalog, incumbent).unwrap();

    assert_eq!(variants.len(), MAX_GOAL_ROUTE_CROSSOVER_VARIANTS);
    for period in [20, 21, 22, 23, 24] {
        assert!(variants.iter().any(|entry| {
                entry.option_id()
                    == format!(
                        "goal.seek.route.04.crossover.01.split.01.roll.period.{period:02}.phase.00.radius.096"
                    )
            }));
    }
}

#[test]
fn goal_conditioned_catalog_rejects_missing_or_non_finite_targets() {
    assert!(goal_conditioned_route_tactic_catalog(&[], &[], 160, 640).is_err());
    assert!(goal_conditioned_route_tactic_catalog(&[[f32::NAN, 0.0, 0.0]], &[], 160, 640).is_err());
    assert!(
        goal_conditioned_route_tactic_catalog(
            &[[0.0, 0.0, 1.0]],
            &[vec![[f32::NAN, 0.0, 0.0]]],
            160,
            640,
        )
        .is_err()
    );
    assert!(
        goal_conditioned_route_tactic_catalog(
            &[[0.0, 0.0, 1.0]],
            &[vec![[0.0, 0.0, 1.0]]],
            160,
            40,
        )
        .is_err()
    );
}
