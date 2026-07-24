//! Route-agnostic bounded tactic catalog used to bootstrap route learning.

use crate::native_generic_tactic::{GenericTactic, NativeGenericTacticPlan};
use crate::tactic_asset::{
    TacticAssetCatalog, TacticAssetError, TacticAssetSource, TacticCatalogEntry,
};
use dusklight_control::controller_program::ControllerProgram;
use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
use dusklight_control::roll_option::RollOptionPlan;
use std::f32::consts::TAU;

pub const DEFAULT_ROUTE_TACTIC_COUNT: usize = 128;
pub const MAX_GOAL_SEEK_TARGETS: usize = 64;

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
        let first = stick_heading(curve_index, 100);
        let second = stick_heading((curve_index + 1) % 8, 100);
        push(
            &mut entries,
            format!("move.curve.clockwise.{curve_index:02}"),
            TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan::new(
                GenericTactic::ShortCurve {
                    control: [first, first, second, second],
                },
                8,
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
    maximum_ticks: u32,
) -> Result<TacticAssetCatalog, TacticAssetError> {
    if targets.is_empty() || targets.len() > MAX_GOAL_SEEK_TARGETS || maximum_ticks == 0 {
        return Err(TacticAssetError::InvalidAsset(
            "goal seek targets or duration are invalid".into(),
        ));
    }
    let mut entries = default_route_tactic_catalog()?.entries().to_vec();
    for (index, coordinate) in targets.iter().copied().enumerate() {
        if coordinate.iter().any(|value| !value.is_finite()) {
            return Err(TacticAssetError::InvalidAsset(
                "goal seek target is non-finite".into(),
            ));
        }
        push(
            &mut entries,
            format!("goal.seek.coordinate.{index:02}"),
            TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan::new(
                GenericTactic::SeekCoordinate {
                    coordinate_f32_bits: coordinate.map(f32::to_bits),
                    // Goal-conditioned corridor actions retain a fixed native
                    // batch duration. A zero radius makes reaching an
                    // intermediate point emit neutral PAD without ending the
                    // auditable batch early; the binary terminal may still
                    // stop the batch at any tick.
                    tolerance_f32_bits: 0.0_f32.to_bits(),
                    magnitude: 127,
                },
                maximum_ticks,
            )),
        )?;
    }
    TacticAssetCatalog::new(entries)
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
    }

    #[test]
    fn goal_conditioned_catalog_exposes_derived_seek_targets_as_ordinary_actions() {
        let target = [-1842.0, 717.0, -4739.0];
        let catalog = goal_conditioned_route_tactic_catalog(&[target], 160).unwrap();
        assert_eq!(catalog.entries().len(), DEFAULT_ROUTE_TACTIC_COUNT + 1);
        let entry = catalog.entry("goal.seek.coordinate.00").unwrap();
        assert_eq!(entry.description().duration.maximum_ticks, 160);
        assert!(matches!(
            entry.source(),
            TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan {
                tactic: GenericTactic::SeekCoordinate {
                    coordinate_f32_bits,
                    ..
                },
                ..
            }) if coordinate_f32_bits.map(f32::from_bits) == target
        ));
    }

    #[test]
    fn goal_conditioned_catalog_rejects_missing_or_non_finite_targets() {
        assert!(goal_conditioned_route_tactic_catalog(&[], 160).is_err());
        assert!(goal_conditioned_route_tactic_catalog(&[[f32::NAN, 0.0, 0.0]], 160).is_err());
    }
}
