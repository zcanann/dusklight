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
}
