//! Route-agnostic bounded tactic catalog used to bootstrap route learning.

use crate::native_generic_tactic::{GenericTactic, NativeGenericTacticPlan};
use crate::tactic_asset::{
    TacticAssetCatalog, TacticAssetError, TacticAssetSource, TacticCatalogEntry,
};
use dusklight_control::controller_program::{
    ControllerProgram, Layer, MAX_LAYERS, MAX_SEEK_COORDINATE_SEQUENCE_POINTS, Operation,
    StickBlend,
};
use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
use dusklight_control::roll_option::RollOptionPlan;
use std::f32::consts::TAU;

pub const DEFAULT_ROUTE_TACTIC_COUNT: usize = 136;
pub const MAX_GOAL_SEEK_TARGETS: usize = 64;
const INTERMEDIATE_GOAL_SEEK_TOLERANCE: f32 = 96.0;
const GOAL_ROUTE_STATIONARY_WINDOW_TICKS: u32 = 16;
const GOAL_ROUTE_STATIONARY_WINDOW_DISTANCE: f32 = 16.0;
const GOAL_ROUTE_ROLL_BUTTON_MASK: u16 = 0x0100;
const MIN_GOAL_ROUTE_ROLL_PERIOD: u32 = 12;
const MAX_GOAL_ROUTE_ROLL_PERIOD: u32 = 32;

/// Builds the finite catalog offered to a fresh route learner.
///
/// It contains no world coordinates, route indices, actor identities, or
/// preferred sequence. Movement is expressed in camera-relative headings,
/// short stick curves, and rolls; ordinary action buttons and a neutral wait
/// complete the catalog.
pub fn default_route_tactic_catalog() -> Result<TacticAssetCatalog, TacticAssetError> {
    let mut entries = Vec::with_capacity(DEFAULT_ROUTE_TACTIC_COUNT);

    for heading_index in 0..16 {
        let heading = TAU * heading_index as f32 / 16.0;
        for magnitude in [80_u8, 127] {
            for ticks in [4_u32, 8, 16] {
                push(
                    &mut entries,
                    format!(
                        "move.heading.{heading_index:02}.magnitude.{magnitude:03}.ticks.{ticks:02}"
                    ),
                    TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan::new(
                        GenericTactic::MaintainRelativeHeading {
                            heading_radians_f32_bits: heading.to_bits(),
                            magnitude,
                        },
                        ticks,
                    )),
                )?;
            }
        }
    }

    for direction_index in 0..8 {
        let unsigned_direction = direction_index * 45;
        let direction = if unsigned_direction > 180 {
            unsigned_direction - 360
        } else {
            unsigned_direction
        };
        for recovery_frames in [3_u32, 7] {
            push(
                &mut entries,
                format!("roll.direction.{direction_index:02}.recovery.{recovery_frames:02}"),
                TacticAssetSource::Roll(RollOptionPlan::new(
                    direction as i16,
                    127,
                    recovery_frames,
                )),
            )?;
        }
    }

    for curve_index in 0..8 {
        let first = stick_heading(curve_index, 127);
        let clockwise = stick_heading((curve_index + 1) % 8, 127);
        let counterclockwise = stick_heading((curve_index + 7) % 8, 127);
        push(
            &mut entries,
            format!("move.curve.clockwise.{curve_index:02}"),
            TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan::new(
                GenericTactic::ShortCurve {
                    control: [first, first, clockwise, clockwise],
                },
                32,
            )),
        )?;
        push(
            &mut entries,
            format!("move.curve.counterclockwise.{curve_index:02}"),
            TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan::new(
                GenericTactic::ShortCurve {
                    control: [first, first, counterclockwise, counterclockwise],
                },
                32,
            )),
        )?;
    }

    push(
        &mut entries,
        "wait.neutral.04".into(),
        TacticAssetSource::ReactiveController(
            ControllerProgram::parse("duskcontrol 1\nframes 4\nneutral replace from 0 for 4\n")
                .map_err(|error| TacticAssetError::InvalidAsset(error.to_string()))?,
        ),
    )?;
    push(
        &mut entries,
        "defend.shield.04".into(),
        game(GameTactic::Shield { frames: 4 }),
    )?;
    push(
        &mut entries,
        "target.hold.04".into(),
        game(GameTactic::Target { frames: 4 }),
    )?;
    push(
        &mut entries,
        "interact.short".into(),
        game(GameTactic::Interact {
            press_frames: 1,
            recovery_frames: 1,
        }),
    )?;
    push(
        &mut entries,
        "interact.long".into(),
        game(GameTactic::Interact {
            press_frames: 2,
            recovery_frames: 4,
        }),
    )?;
    push(
        &mut entries,
        "attack.normal".into(),
        game(GameTactic::NormalAttack {
            direction_degrees: 0,
            magnitude: 100,
            press_frames: 1,
            recovery_frames: 3,
        }),
    )?;
    push(
        &mut entries,
        "attack.jump".into(),
        game(GameTactic::JumpAttack {
            direction_degrees: 0,
            magnitude: 100,
            windup_frames: 1,
            press_frames: 1,
            recovery_frames: 4,
        }),
    )?;
    push(
        &mut entries,
        "attack.combo".into(),
        game(GameTactic::AttackCombo {
            direction_degrees: 0,
            magnitude: 100,
            hits: 2,
            press_frames: 1,
            gap_frames: 2,
            recovery_frames: 3,
        }),
    )?;

    debug_assert_eq!(entries.len(), DEFAULT_ROUTE_TACTIC_COUNT);
    TacticAssetCatalog::new(entries)
}

/// Extends the route-agnostic bootstrap catalog with concrete coordinates
/// resolved from the authenticated goal and world mechanics.
///
/// These are goal-conditioned actions, not route hints: callers must derive
/// every target from the current objective and pinned world inventory. Keeping
/// the extension here makes the resulting coordinates part of the ordinary
/// typed action schema seen by the learner.
pub fn goal_conditioned_route_tactic_catalog(
    targets: &[[f32; 3]],
    route_sequences: &[Vec<[f32; 3]>],
    maximum_ticks: u32,
    route_sequence_maximum_ticks: u32,
) -> Result<TacticAssetCatalog, TacticAssetError> {
    if targets.is_empty()
        || targets.len() > MAX_GOAL_SEEK_TARGETS
        || route_sequences.len() > MAX_GOAL_SEEK_TARGETS
        || maximum_ticks == 0
        || route_sequence_maximum_ticks < maximum_ticks
    {
        return Err(TacticAssetError::InvalidAsset(
            "goal seek targets or duration are invalid".into(),
        ));
    }
    let mut entries = default_route_tactic_catalog()?.entries().to_vec();
    for (index, coordinates) in route_sequences.iter().enumerate() {
        if coordinates.is_empty()
            || coordinates.len() > MAX_GOAL_SEEK_TARGETS
            || coordinates.iter().flatten().any(|value| !value.is_finite())
        {
            return Err(TacticAssetError::InvalidAsset(
                "goal seek route sequence is invalid".into(),
            ));
        }
        let sequence_layers =
            coordinate_sequence_layers(coordinates, route_sequence_maximum_ticks)?;
        let route_source = if coordinates.len() <= MAX_SEEK_COORDINATE_SEQUENCE_POINTS {
            TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan::new(
                GenericTactic::SeekCoordinateSequence {
                    coordinates_f32_bits: coordinates
                        .iter()
                        .map(|coordinate| coordinate.map(f32::to_bits))
                        .collect(),
                    intermediate_tolerance_f32_bits: INTERMEDIATE_GOAL_SEEK_TOLERANCE.to_bits(),
                    final_tolerance_f32_bits: 0.0_f32.to_bits(),
                    // The native controller owns this bounded composition in
                    // one process call. Keep stall detection beyond the
                    // declared horizon so the coordinator never falls back to
                    // replaying one progressively longer prefix per tick.
                    stall_grace_ticks: route_sequence_maximum_ticks,
                    stationary_window_ticks: GOAL_ROUTE_STATIONARY_WINDOW_TICKS,
                    stationary_window_distance_f32_bits: GOAL_ROUTE_STATIONARY_WINDOW_DISTANCE
                        .to_bits(),
                    magnitude: 127,
                },
                route_sequence_maximum_ticks,
            ))
        } else {
            TacticAssetSource::ReactiveController(ControllerProgram {
                duration_frames: route_sequence_maximum_ticks,
                layers: sequence_layers.clone(),
            })
        };
        push(
            &mut entries,
            format!("goal.seek.route.{index:02}"),
            route_source,
        )?;
        // This is a compact integer parameter domain, not a preferred cadence
        // list. The learner must be able to compare adjacent controller
        // values instead of being restricted to a hand-picked even grid.
        for period in MIN_GOAL_ROUTE_ROLL_PERIOD..=MAX_GOAL_ROUTE_ROLL_PERIOD {
            for phase in [0, period / 2] {
                let mut layers = sequence_layers.clone();
                let mut pulse = phase;
                while pulse < route_sequence_maximum_ticks && layers.len() < MAX_LAYERS {
                    layers.push(Layer {
                        start_frame: pulse,
                        duration_frames: 1,
                        operation: Operation::Buttons {
                            mask: GOAL_ROUTE_ROLL_BUTTON_MASK,
                        },
                    });
                    pulse = pulse.saturating_add(period);
                }
                if pulse < route_sequence_maximum_ticks {
                    continue;
                }
                let program = ControllerProgram {
                    duration_frames: route_sequence_maximum_ticks,
                    layers,
                };
                program
                    .validate()
                    .map_err(|error| TacticAssetError::InvalidAsset(error.to_string()))?;
                push(
                    &mut entries,
                    format!("goal.seek.route.{index:02}.roll.period.{period:02}.phase.{phase:02}"),
                    TacticAssetSource::ReactiveController(program),
                )?;
            }
        }
    }
    for (index, coordinate) in targets.iter().copied().enumerate() {
        if coordinate.iter().any(|value| !value.is_finite()) {
            return Err(TacticAssetError::InvalidAsset(
                "goal seek target is non-finite".into(),
            ));
        }
        let mut plan = NativeGenericTacticPlan::new(
            GenericTactic::SeekCoordinate {
                coordinate_f32_bits: coordinate.map(f32::to_bits),
                // The first target is the exact terminal trigger and must keep
                // seeking until native terminal evidence stops it. A derived
                // corridor waypoint emits neutral PAD once it is reached, so
                // the fixed native batch cannot orbit the waypoint.
                tolerance_f32_bits: if index == 0 {
                    0.0_f32
                } else {
                    INTERMEDIATE_GOAL_SEEK_TOLERANCE
                }
                .to_bits(),
                magnitude: 127,
            },
            maximum_ticks,
        );
        if index > 0 {
            // Native suffix requests have a fixed auditable duration. Delay
            // semantic termination until that boundary while `SeekCoordinate`
            // holds neutral inside the tolerance radius.
            plan.minimum_ticks = maximum_ticks;
        }
        push(
            &mut entries,
            format!("goal.seek.coordinate.{index:02}"),
            TacticAssetSource::NativeGenericTactic(plan),
        )?;
    }
    TacticAssetCatalog::new(entries)
}

fn coordinate_sequence_layers(
    coordinates: &[[f32; 3]],
    duration_frames: u32,
) -> Result<Vec<Layer>, TacticAssetError> {
    if coordinates.is_empty() || duration_frames == 0 {
        return Err(TacticAssetError::InvalidAsset(
            "coordinate sequence composition is empty".into(),
        ));
    }
    let chunk_count = coordinates
        .len()
        .div_ceil(MAX_SEEK_COORDINATE_SEQUENCE_POINTS);
    if chunk_count > MAX_LAYERS || duration_frames < chunk_count as u32 {
        return Err(TacticAssetError::InvalidAsset(
            "coordinate sequence composition exceeds the controller bounds".into(),
        ));
    }
    let chunk_size = coordinates.len().div_ceil(chunk_count);
    let mut layers = Vec::with_capacity(chunk_count);
    for (chunk_index, chunk) in coordinates.chunks(chunk_size).enumerate() {
        let start_frame = duration_frames * chunk_index as u32 / chunk_count as u32;
        let end_frame = duration_frames * (chunk_index as u32 + 1) / chunk_count as u32;
        layers.push(Layer {
            start_frame,
            duration_frames: end_frame - start_frame,
            operation: Operation::SeekCoordinateSequence {
                blend: StickBlend::Replace,
                coordinates_xz: chunk
                    .iter()
                    .map(|coordinate| [coordinate[0], coordinate[2]])
                    .collect(),
                intermediate_stop_radius: INTERMEDIATE_GOAL_SEEK_TOLERANCE,
                final_stop_radius: if chunk_index + 1 == chunk_count {
                    0.0
                } else {
                    INTERMEDIATE_GOAL_SEEK_TOLERANCE
                },
                magnitude: 127,
            },
        });
    }
    Ok(layers)
}

fn push(
    entries: &mut Vec<TacticCatalogEntry>,
    option_id: String,
    source: TacticAssetSource,
) -> Result<(), TacticAssetError> {
    entries.push(TacticCatalogEntry::new(option_id, source)?);
    Ok(())
}

fn game(tactic: GameTactic) -> TacticAssetSource {
    TacticAssetSource::GameTactic(GameTacticPlan::new(tactic))
}

fn stick_heading(index: usize, magnitude: i8) -> [i8; 2] {
    let angle = TAU * index as f32 / 8.0;
    [
        (angle.sin() * f32::from(magnitude)).round() as i8,
        (angle.cos() * f32::from(magnitude)).round() as i8,
    ]
}

#[cfg(test)]
mod tests {
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
        let catalog = goal_conditioned_route_tactic_catalog(
            &[goal, waypoint],
            &[vec![waypoint, goal]],
            160,
            160,
        )
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
                .get("controller_base_sha256"),
            catalog
                .entry("goal.seek.route.00.roll.period.22.phase.00")
                .unwrap()
                .description()
                .option
                .parameters
                .get("controller_base_sha256")
        );
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
        let catalog =
            goal_conditioned_route_tactic_catalog(&[route[7]], &[route], 160, 160).unwrap();

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
    fn goal_conditioned_catalog_rejects_missing_or_non_finite_targets() {
        assert!(goal_conditioned_route_tactic_catalog(&[], &[], 160, 640).is_err());
        assert!(
            goal_conditioned_route_tactic_catalog(&[[f32::NAN, 0.0, 0.0]], &[], 160, 640).is_err()
        );
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
}
