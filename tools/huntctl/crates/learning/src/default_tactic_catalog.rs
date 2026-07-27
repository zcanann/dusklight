//! Route-agnostic bounded tactic catalog used to bootstrap route learning.

use crate::native_generic_tactic::{GenericTactic, NativeGenericTacticPlan};
use crate::option_values::OptionActionDescriptor;
use crate::tactic_asset::{
    TacticAssetCatalog, TacticAssetError, TacticAssetSource, TacticCatalogEntry,
};
use dusklight_control::controller_program::{
    ControllerProgram, Layer, MAX_LAYERS, MAX_SEEK_COORDINATE_SEQUENCE_POINTS, Operation,
    StickBlend,
};
use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
use dusklight_control::option_execution::OptionParameter;
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
const GOAL_ROUTE_WAYPOINT_SWITCH_RADII: [u32; 16] = [
    32, 48, 64, 80, 112, 128, 144, 160, 192, 224, 256, 320, 384, 448, 512, 640,
];
// Five cadence values times 24 structural candidates is 120. Composed
// incumbents additionally expose every phase of their winning cadence before
// continuing geometry refinement. The cap retains both bounded domains while
// the complete state-local catalog stays below its global bound.
const MAX_GOAL_ROUTE_CROSSOVER_VARIANTS: usize = 160;

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

/// Materialize the complete phase domain only for the best terminal
/// controller-period family observed at the current learner state.
///
/// The base goal catalog keeps two bootstrap phases per period and remains
/// below the global finite-choice bound. Once native evidence identifies a
/// useful structural family, this bounded state-local extension exposes every
/// remaining integer phase for that one period without eagerly constructing
/// the route × period × phase Cartesian product.
pub fn goal_route_roll_phase_variants(
    catalog: &TacticAssetCatalog,
    incumbent: &OptionActionDescriptor,
) -> Result<Vec<TacticCatalogEntry>, TacticAssetError> {
    let Some(route_prefix) = incumbent
        .option_id
        .split_once(".roll.period.")
        .map(|(prefix, _)| prefix)
        .filter(|prefix| prefix.starts_with("goal.seek.route."))
    else {
        return Ok(Vec::new());
    };
    let Some(OptionParameter::Unsigned(period)) =
        incumbent.parameters.get("button_pulse_period_ticks")
    else {
        return Ok(Vec::new());
    };
    let period = u32::try_from(*period).map_err(|_| {
        TacticAssetError::InvalidAsset("route roll period exceeds controller bounds".into())
    })?;
    if !(MIN_GOAL_ROUTE_ROLL_PERIOD..=MAX_GOAL_ROUTE_ROLL_PERIOD).contains(&period) {
        return Ok(Vec::new());
    }
    let Some(structural_identity) = incumbent
        .parameters
        .get("controller_structure_sha256")
        .or_else(|| incumbent.parameters.get("controller_base_sha256"))
    else {
        return Ok(Vec::new());
    };
    let Some(OptionParameter::Unsigned(mask)) = incumbent.parameters.get("button_pulse_mask")
    else {
        return Ok(Vec::new());
    };
    let mask = u16::try_from(*mask).map_err(|_| {
        TacticAssetError::InvalidAsset("route roll button mask exceeds controller bounds".into())
    })?;
    let source = catalog
        .entries()
        .iter()
        .find(|entry| {
            entry.option_id().starts_with(route_prefix)
                && entry
                    .description()
                    .option
                    .parameters
                    .get("controller_structure_sha256")
                    .or_else(|| {
                        entry
                            .description()
                            .option
                            .parameters
                            .get("controller_base_sha256")
                    })
                    == Some(structural_identity)
                && entry
                    .description()
                    .option
                    .parameters
                    .get("button_pulse_period_ticks")
                    == Some(&OptionParameter::Unsigned(u64::from(period)))
        })
        .ok_or_else(|| {
            TacticAssetError::InvalidAsset(
                "terminal route controller family is absent from its live catalog".into(),
            )
        })?;
    let TacticAssetSource::ReactiveController(source_program) = source.source() else {
        return Err(TacticAssetError::InvalidAsset(
            "terminal route controller family is not reactive".into(),
        ));
    };
    let mut base_layers = source_program.layers.clone();
    base_layers.retain(|layer| !matches!(layer.operation, Operation::Buttons { .. }));
    let waypoint_switch_radius =
        incumbent
            .parameters
            .get("waypoint_switch_radius")
            .and_then(|parameter| match parameter {
                OptionParameter::F32Bits(bits) => Some(f32::from_bits(*bits)),
                _ => None,
            });
    if let Some(radius) = waypoint_switch_radius {
        set_waypoint_switch_radius(&mut base_layers, radius);
    }
    let mut variants = Vec::new();
    for phase in 0..period {
        let option_id = roll_variant_option_id(
            route_prefix,
            period,
            phase,
            incumbent.option_id.contains(".radius."),
            waypoint_switch_radius,
        );
        if option_id == incumbent.option_id || catalog.entry(&option_id).is_some() {
            continue;
        }
        let mut layers = base_layers.clone();
        let mut pulse = phase;
        while pulse < source_program.duration_frames && layers.len() < MAX_LAYERS {
            layers.push(Layer {
                start_frame: pulse,
                duration_frames: 1,
                operation: Operation::Buttons { mask },
            });
            pulse = pulse.saturating_add(period);
        }
        if pulse < source_program.duration_frames {
            continue;
        }
        variants.push(TacticCatalogEntry::new(
            option_id,
            TacticAssetSource::ReactiveController(ControllerProgram {
                duration_frames: source_program.duration_frames,
                layers,
            }),
        )?);
    }
    Ok(variants)
}

/// Materialize a bounded waypoint-lookahead neighborhood only for the best
/// terminal route, cadence, and phase observed in the current learner cell.
pub fn goal_route_waypoint_switch_variants(
    catalog: &TacticAssetCatalog,
    incumbent: &OptionActionDescriptor,
) -> Result<Vec<TacticCatalogEntry>, TacticAssetError> {
    let Some(route_prefix) = incumbent
        .option_id
        .split_once(".roll.period.")
        .map(|(prefix, _)| prefix)
        .filter(|prefix| prefix.starts_with("goal.seek.route."))
    else {
        return Ok(Vec::new());
    };
    let (
        Some(OptionParameter::Unsigned(period)),
        Some(OptionParameter::Unsigned(phase)),
        Some(OptionParameter::Unsigned(mask)),
        Some(OptionParameter::F32Bits(current_radius_bits)),
    ) = (
        incumbent.parameters.get("button_pulse_period_ticks"),
        incumbent.parameters.get("button_pulse_phase_tick"),
        incumbent.parameters.get("button_pulse_mask"),
        incumbent.parameters.get("waypoint_switch_radius"),
    )
    else {
        return Ok(Vec::new());
    };
    let period = u32::try_from(*period).map_err(|_| {
        TacticAssetError::InvalidAsset("route roll period exceeds controller bounds".into())
    })?;
    let phase = u32::try_from(*phase).map_err(|_| {
        TacticAssetError::InvalidAsset("route roll phase exceeds controller bounds".into())
    })?;
    let mask = u16::try_from(*mask).map_err(|_| {
        TacticAssetError::InvalidAsset("route roll button mask exceeds controller bounds".into())
    })?;
    let Some(structural_identity) = incumbent
        .parameters
        .get("controller_structure_sha256")
        .or_else(|| incumbent.parameters.get("controller_base_sha256"))
    else {
        return Ok(Vec::new());
    };
    let source = catalog
        .entries()
        .iter()
        .find(|entry| {
            entry.option_id().starts_with(route_prefix)
                && entry
                    .description()
                    .option
                    .parameters
                    .get("controller_structure_sha256")
                    .or_else(|| {
                        entry
                            .description()
                            .option
                            .parameters
                            .get("controller_base_sha256")
                    })
                    == Some(structural_identity)
        })
        .ok_or_else(|| {
            TacticAssetError::InvalidAsset(
                "terminal route controller structure is absent from its live catalog".into(),
            )
        })?;
    let TacticAssetSource::ReactiveController(source_program) = source.source() else {
        return Err(TacticAssetError::InvalidAsset(
            "terminal route controller structure is not reactive".into(),
        ));
    };
    let mut base_layers = source_program.layers.clone();
    base_layers.retain(|layer| !matches!(layer.operation, Operation::Buttons { .. }));
    if !base_layers
        .iter()
        .any(|layer| matches!(layer.operation, Operation::SeekCoordinateSequence { .. }))
    {
        return Ok(Vec::new());
    }
    let mut variants = Vec::new();
    for radius in GOAL_ROUTE_WAYPOINT_SWITCH_RADII {
        let radius = radius as f32;
        if radius.to_bits() == *current_radius_bits {
            continue;
        }
        let mut layers = base_layers.clone();
        set_waypoint_switch_radius(&mut layers, radius);
        let mut pulse = phase;
        while pulse < source_program.duration_frames && layers.len() < MAX_LAYERS {
            layers.push(Layer {
                start_frame: pulse,
                duration_frames: 1,
                operation: Operation::Buttons { mask },
            });
            pulse = pulse.saturating_add(period);
        }
        if pulse < source_program.duration_frames {
            continue;
        }
        variants.push(TacticCatalogEntry::new(
            roll_variant_option_id(route_prefix, period, phase, true, Some(radius)),
            TacticAssetSource::ReactiveController(ControllerProgram {
                duration_frames: source_program.duration_frames,
                layers,
            }),
        )?);
    }
    Ok(variants)
}

/// Compose compatible waypoint prefixes and suffixes around the best terminal
/// route observed in the current learner cell.
///
/// World route extraction produces several complete path hypotheses, but a
/// useful route can enter on one path and leave on another. Keep this bounded
/// and state-local: only equal-length, single-layer coordinate sequences are
/// crossed, and at most 128 joint geometry/cadence/lookahead actions
/// are materialized.
pub fn goal_route_crossover_variants(
    catalog: &TacticAssetCatalog,
    incumbent: &OptionActionDescriptor,
) -> Result<Vec<TacticCatalogEntry>, TacticAssetError> {
    let Some(route_prefix) = incumbent
        .option_id
        .split_once(".roll.period.")
        .map(|(prefix, _)| prefix)
        .filter(|prefix| prefix.starts_with("goal.seek.route."))
    else {
        return Ok(Vec::new());
    };
    let Some(source_route_id) = route_prefix
        .strip_prefix("goal.seek.route.")
        .and_then(|suffix| suffix.split('.').next())
    else {
        return Ok(Vec::new());
    };
    let base_route_prefix = format!("goal.seek.route.{source_route_id}");
    let crossover_parts = route_prefix.split('.').collect::<Vec<_>>();
    let incumbent_crossover = (crossover_parts.len() >= 8
        && crossover_parts[4] == "crossover"
        && crossover_parts[6] == "split")
        .then(|| {
            crossover_parts[7]
                .parse::<usize>()
                .ok()
                .map(|split| (crossover_parts[5].to_owned(), split))
        })
        .flatten();
    let incumbent_blend = (crossover_parts.len() >= 10 && crossover_parts[8] == "blend")
        .then(|| crossover_parts[9].parse::<u32>().ok())
        .flatten()
        .filter(|blend| (1..100).contains(blend));
    let (
        Some(OptionParameter::Unsigned(period)),
        Some(OptionParameter::Unsigned(phase)),
        Some(OptionParameter::Unsigned(mask)),
    ) = (
        incumbent.parameters.get("button_pulse_period_ticks"),
        incumbent.parameters.get("button_pulse_phase_tick"),
        incumbent.parameters.get("button_pulse_mask"),
    )
    else {
        return Ok(Vec::new());
    };
    let incumbent_period = u32::try_from(*period).map_err(|_| {
        TacticAssetError::InvalidAsset("route roll period exceeds controller bounds".into())
    })?;
    let phase = u32::try_from(*phase).map_err(|_| {
        TacticAssetError::InvalidAsset("route roll phase exceeds controller bounds".into())
    })?;
    let mask = u16::try_from(*mask).map_err(|_| {
        TacticAssetError::InvalidAsset("route roll button mask exceeds controller bounds".into())
    })?;
    let source = catalog
        .entries()
        .iter()
        .find(|entry| entry.option_id() == incumbent.option_id)
        .or_else(|| {
            let structure = incumbent
                .parameters
                .get("controller_structure_sha256")
                .or_else(|| incumbent.parameters.get("controller_base_sha256"))?;
            catalog.entries().iter().find(|entry| {
                entry.option_id().starts_with(route_prefix)
                    && entry
                        .description()
                        .option
                        .parameters
                        .get("controller_structure_sha256")
                        .or_else(|| {
                            entry
                                .description()
                                .option
                                .parameters
                                .get("controller_base_sha256")
                        })
                        == Some(structure)
            })
        })
        .or_else(|| {
            catalog.entries().iter().find(|entry| {
                entry.option_id().starts_with(&base_route_prefix)
                    && matches!(entry.source(), TacticAssetSource::ReactiveController(_))
            })
        })
        .ok_or_else(|| {
            TacticAssetError::InvalidAsset(
                "terminal route controller structure is absent from its live catalog".into(),
            )
        })?;
    let TacticAssetSource::ReactiveController(source_program) = source.source() else {
        return Ok(Vec::new());
    };
    let Some(source_coordinates) = single_layer_route_coordinates(source_program) else {
        return Ok(Vec::new());
    };
    if source_coordinates.len() < 2 {
        return Ok(Vec::new());
    }
    let radius = incumbent
        .parameters
        .get("waypoint_switch_radius")
        .and_then(|parameter| match parameter {
            OptionParameter::F32Bits(bits) => Some(f32::from_bits(*bits)),
            _ => None,
        })
        .filter(|radius| radius.is_finite() && *radius >= 0.0)
        .unwrap_or(INTERMEDIATE_GOAL_SEEK_TOLERANCE);
    let mut peers = Vec::new();
    let mut seen_peer_routes = std::collections::BTreeSet::new();
    for entry in catalog.entries() {
        let Some(peer_suffix) = entry.option_id().strip_prefix("goal.seek.route.") else {
            continue;
        };
        let Some(peer_route_id) = peer_suffix.split('.').next() else {
            continue;
        };
        if peer_route_id == source_route_id || seen_peer_routes.contains(peer_route_id) {
            continue;
        }
        if incumbent_crossover
            .as_ref()
            .is_some_and(|(incumbent_peer, _)| incumbent_peer != peer_route_id)
        {
            continue;
        }
        let TacticAssetSource::ReactiveController(peer_program) = entry.source() else {
            continue;
        };
        let Some(peer_coordinates) = single_layer_route_coordinates(peer_program) else {
            continue;
        };
        if peer_coordinates.len() != source_coordinates.len() {
            continue;
        }
        seen_peer_routes.insert(peer_route_id.to_owned());
        peers.push((peer_route_id.to_owned(), peer_coordinates));
    }
    peers.sort_by(|left, right| left.0.cmp(&right.0));

    let mut variants = Vec::new();
    if let Some((incumbent_peer, split)) = incumbent_crossover.as_ref() {
        if let Some((_, peer_coordinates)) = peers
            .iter()
            .find(|(peer_route_id, _)| peer_route_id == incumbent_peer)
        {
            let mut coordinates = source_coordinates[..*split].to_vec();
            coordinates.extend_from_slice(&peer_coordinates[*split..]);
            if let Some(blend_percent) = incumbent_blend {
                let weight = blend_percent as f32 / 100.0;
                let index = split - 1;
                coordinates[index] = [
                    source_coordinates[index][0] * (1.0 - weight)
                        + peer_coordinates[index][0] * weight,
                    source_coordinates[index][1] * (1.0 - weight)
                        + peer_coordinates[index][1] * weight,
                ];
            }
            for candidate_phase in 0..incumbent_period {
                if candidate_phase == phase {
                    continue;
                }
                let mut layers = vec![Layer {
                    start_frame: 0,
                    duration_frames: source_program.duration_frames,
                    operation: Operation::SeekCoordinateSequence {
                        blend: StickBlend::Replace,
                        coordinates_xz: coordinates.clone(),
                        intermediate_stop_radius: radius,
                        final_stop_radius: 0.0,
                        magnitude: 127,
                    },
                }];
                let mut pulse = candidate_phase;
                while pulse < source_program.duration_frames && layers.len() < MAX_LAYERS {
                    layers.push(Layer {
                        start_frame: pulse,
                        duration_frames: 1,
                        operation: Operation::Buttons { mask },
                    });
                    pulse = pulse.saturating_add(incumbent_period);
                }
                if pulse < source_program.duration_frames {
                    continue;
                }
                let blend_id = incumbent_blend
                    .map(|blend| format!(".blend.{blend:03}"))
                    .unwrap_or_default();
                variants.push(TacticCatalogEntry::new(
                    format!(
                        "goal.seek.route.{source_route_id}.crossover.{incumbent_peer}.split.{split:02}{blend_id}.roll.period.{incumbent_period:02}.phase.{candidate_phase:02}.radius.{:03}",
                        radius.round() as u32
                    ),
                    TacticAssetSource::ReactiveController(ControllerProgram {
                        duration_frames: source_program.duration_frames,
                        layers,
                    }),
                )?);
            }
        }
    }

    // Couple every crossover geometry to a small cadence neighborhood.
    // Faster pulses receive the deeper side of the asymmetric neighborhood
    // because the terminal objective minimizes ticks; one slower neighbor
    // remains as a control for resonance.
    let mut cadences = Vec::new();
    for candidate in [
        incumbent_period,
        incumbent_period.saturating_sub(1),
        incumbent_period.saturating_add(1),
        incumbent_period.saturating_sub(2),
        incumbent_period.saturating_sub(3),
    ] {
        if (MIN_GOAL_ROUTE_ROLL_PERIOD..=MAX_GOAL_ROUTE_ROLL_PERIOD).contains(&candidate)
            && phase < candidate
            && !cadences.contains(&candidate)
        {
            cadences.push(candidate);
        }
    }
    let joint_parameters = if incumbent_blend.is_some() {
        // A measured interpolation already couples the useful geometry and
        // cadence. Spend the bounded action budget on a broad lookahead sweep
        // (including both edges) instead of allowing nearby cadence products
        // to crowd the long straight-line controls out of the live catalog.
        let mut lookaheads = vec![radius];
        lookaheads.extend(
            GOAL_ROUTE_WAYPOINT_SWITCH_RADII
                .into_iter()
                .map(|radius| radius as f32)
                .filter(|lookahead| *lookahead > radius),
        );
        const MAX_LOCAL_LOOKAHEADS: usize = 9;
        if lookaheads.len() > MAX_LOCAL_LOOKAHEADS {
            let source = lookaheads;
            let last = source.len() - 1;
            lookaheads = (0..MAX_LOCAL_LOOKAHEADS)
                .map(|index| {
                    let numerator = index * last + (MAX_LOCAL_LOOKAHEADS - 1) / 2;
                    source[numerator / (MAX_LOCAL_LOOKAHEADS - 1)]
                })
                .collect();
        }
        lookaheads
            .into_iter()
            .map(|lookahead| (incumbent_period, lookahead))
            .collect::<Vec<_>>()
    } else {
        let mut parameters = cadences
            .into_iter()
            .map(|period| (period, radius))
            .collect::<Vec<_>>();
        parameters.extend(
            GOAL_ROUTE_WAYPOINT_SWITCH_RADII
                .into_iter()
                .map(|radius| radius as f32)
                .filter(|lookahead| *lookahead > radius)
                .map(|lookahead| (incumbent_period, lookahead)),
        );
        parameters
    };
    'parameters: for (period, lookahead) in joint_parameters {
        for (peer_route_id, peer_coordinates) in &peers {
            for split in 1..source_coordinates.len() {
                if incumbent_crossover
                    .as_ref()
                    .is_some_and(|(_, incumbent_split)| *incumbent_split != split)
                {
                    continue;
                }
                let directions = if incumbent_crossover.is_some() {
                    vec![(
                        source_route_id,
                        &source_coordinates,
                        peer_route_id.as_str(),
                        peer_coordinates,
                    )]
                } else {
                    vec![
                        (
                            source_route_id,
                            &source_coordinates,
                            peer_route_id.as_str(),
                            peer_coordinates,
                        ),
                        (
                            peer_route_id.as_str(),
                            peer_coordinates,
                            source_route_id,
                            &source_coordinates,
                        ),
                    ]
                };
                for (prefix_id, prefix, suffix_id, suffix) in directions {
                    let blends = if let Some(incumbent_blend) = incumbent_blend {
                        // Once an interpolation wins, turn the coarse 25-point
                        // bracket into a bounded local geometry search. This
                        // keeps refinement generic while allowing measured
                        // trajectory improvements to promote intermediate
                        // waypoint positions.
                        let mut local = vec![None];
                        for blend in [
                            incumbent_blend.saturating_sub(10),
                            incumbent_blend.saturating_sub(5),
                            incumbent_blend,
                            incumbent_blend.saturating_add(5),
                            incumbent_blend.saturating_add(10),
                        ] {
                            if (1..100).contains(&blend) && !local.contains(&Some(blend)) {
                                local.push(Some(blend));
                            }
                        }
                        local
                    } else if incumbent_crossover.is_some() {
                        vec![None, Some(25_u32), Some(50), Some(75)]
                    } else {
                        vec![None]
                    };
                    for blend_percent in blends {
                        let mut coordinates = prefix[..split].to_vec();
                        coordinates.extend_from_slice(&suffix[split..]);
                        if let Some(blend_percent) = blend_percent {
                            let weight = blend_percent as f32 / 100.0;
                            let index = split - 1;
                            coordinates[index] = [
                                prefix[index][0] * (1.0 - weight) + suffix[index][0] * weight,
                                prefix[index][1] * (1.0 - weight) + suffix[index][1] * weight,
                            ];
                        }
                        if coordinates.as_slice() == prefix.as_slice()
                            || coordinates.as_slice() == suffix.as_slice()
                        {
                            continue;
                        }
                        let mut layers = vec![Layer {
                            start_frame: 0,
                            duration_frames: source_program.duration_frames,
                            operation: Operation::SeekCoordinateSequence {
                                blend: StickBlend::Replace,
                                coordinates_xz: coordinates,
                                intermediate_stop_radius: lookahead,
                                final_stop_radius: 0.0,
                                magnitude: 127,
                            },
                        }];
                        let mut pulse = phase;
                        while pulse < source_program.duration_frames && layers.len() < MAX_LAYERS {
                            layers.push(Layer {
                                start_frame: pulse,
                                duration_frames: 1,
                                operation: Operation::Buttons { mask },
                            });
                            pulse = pulse.saturating_add(period);
                        }
                        if pulse < source_program.duration_frames {
                            continue;
                        }
                        let blend_id = blend_percent
                            .map(|blend| format!(".blend.{blend:03}"))
                            .unwrap_or_default();
                        let option_id = format!(
                            "goal.seek.route.{prefix_id}.crossover.{suffix_id}.split.{split:02}{blend_id}.roll.period.{period:02}.phase.{phase:02}.radius.{:03}",
                            lookahead.round() as u32
                        );
                        if option_id == incumbent.option_id
                            || variants
                                .iter()
                                .any(|variant| variant.option_id() == option_id)
                        {
                            continue;
                        }
                        variants.push(TacticCatalogEntry::new(
                            option_id,
                            TacticAssetSource::ReactiveController(ControllerProgram {
                                duration_frames: source_program.duration_frames,
                                layers,
                            }),
                        )?);
                        if variants.len() == MAX_GOAL_ROUTE_CROSSOVER_VARIANTS {
                            break 'parameters;
                        }
                    }
                }
            }
        }
    }
    Ok(variants)
}

fn single_layer_route_coordinates(program: &ControllerProgram) -> Option<Vec<[f32; 2]>> {
    program
        .layers
        .iter()
        .find_map(|layer| match &layer.operation {
            Operation::SeekCoordinateSequence { coordinates_xz, .. }
                if layer.start_frame == 0
                    && layer.duration_frames == program.duration_frames
                    && coordinates_xz.len() <= MAX_SEEK_COORDINATE_SEQUENCE_POINTS =>
            {
                Some(coordinates_xz.clone())
            }
            _ => None,
        })
}

fn set_waypoint_switch_radius(layers: &mut [Layer], radius: f32) {
    for layer in layers {
        if let Operation::SeekCoordinateSequence {
            intermediate_stop_radius,
            ..
        } = &mut layer.operation
        {
            *intermediate_stop_radius = radius;
        }
    }
}

fn roll_variant_option_id(
    route_prefix: &str,
    period: u32,
    phase: u32,
    include_radius: bool,
    radius: Option<f32>,
) -> String {
    let base = format!("{route_prefix}.roll.period.{period:02}.phase.{phase:02}");
    if include_radius {
        format!(
            "{base}.radius.{:03}",
            radius.unwrap_or_default().round() as u32
        )
    } else {
        base
    }
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
            .find(|entry| {
                entry.option_id() == "goal.seek.route.00.roll.period.20.phase.00.radius.080"
            })
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
            goal_conditioned_route_tactic_catalog(&[[300.0, 10.0, -300.0]], &routes, 160, 160)
                .unwrap();
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
