//! Small route-agnostic action surface for cold-start route learning.

use crate::native_generic_tactic::{GenericTactic, NativeGenericTacticPlan};
use crate::tactic_asset::{
    TacticAssetCatalog, TacticAssetError, TacticAssetSource, TacticCatalogEntry,
};
use dusklight_control::controller_program::ControllerProgram;
use dusklight_control::roll_option::RollOptionPlan;
use std::f32::consts::{PI, TAU};

pub const SCRATCH_HEADING_COUNT: usize = 16;
pub const SCRATCH_ACTIONS_PER_HEADING: usize = 16;
pub const SCRATCH_ACTION_COUNT: usize = SCRATCH_HEADING_COUNT * SCRATCH_ACTIONS_PER_HEADING;
pub const MAX_SCRATCH_REFINEMENT_HEADINGS: usize = 32;

/// Builds one fixed action universe without goals, coordinates, demonstrations,
/// route fragments, or state-conditioned proposal rules.
pub fn scratch_action_catalog() -> Result<TacticAssetCatalog, TacticAssetError> {
    scratch_action_catalog_with_heading_count(SCRATCH_HEADING_COUNT)
}

/// Builds the same route-agnostic action families at a finer angular resolution
/// for local incumbent refinement. Cold-start learning keeps the fixed 16-bin
/// catalog so this does not multiply its exploration surface.
pub fn scratch_action_catalog_with_heading_count(
    heading_count: usize,
) -> Result<TacticAssetCatalog, TacticAssetError> {
    if !(SCRATCH_HEADING_COUNT..=MAX_SCRATCH_REFINEMENT_HEADINGS).contains(&heading_count)
        || !heading_count.is_power_of_two()
    {
        return Err(TacticAssetError::InvalidAsset(
            "scratch heading count must be a power of two from 16 through 32".into(),
        ));
    }
    let mut entries = Vec::with_capacity(heading_count * SCRATCH_ACTIONS_PER_HEADING);
    for heading_index in 0..heading_count {
        let heading = TAU * heading_index as f32 / heading_count as f32;
        let direction_degrees = (-heading * 180.0 / PI).round() as i16;
        let direction_degrees = if direction_degrees < -180 {
            direction_degrees + 360
        } else {
            direction_degrees
        };
        push_raw(&mut entries, heading_index, heading)?;
        for ticks in [4, 8, 16] {
            push_move(&mut entries, heading_index, heading, ticks, "move")?;
        }
        for recovery_frames in [3, 7] {
            entries.push(TacticCatalogEntry::new(
                format!("scratch.roll.h{heading_index:02}.r{recovery_frames:02}"),
                TacticAssetSource::Roll(RollOptionPlan::new(
                    direction_degrees,
                    127,
                    recovery_frames,
                )),
            )?);
        }
        let initial_stick = stick(heading, 127);
        for duration in [8, 16] {
            for lock_frame in [0, 1] {
                entries.push(TacticCatalogEntry::new(
                    format!("scratch.camera_move.h{heading_index:02}.t{duration:02}.l{lock_frame}"),
                    TacticAssetSource::ReactiveController(camera_lock_move_program(
                        initial_stick,
                        duration,
                        lock_frame,
                    )?),
                )?);
            }
            for timing in [0, 1, 2] {
                entries.push(TacticCatalogEntry::new(
                    format!("scratch.camera_roll.h{heading_index:02}.t{duration:02}.s{timing}"),
                    TacticAssetSource::ReactiveController(camera_lock_roll_program(
                        initial_stick,
                        duration,
                        timing,
                    )?),
                )?);
            }
        }
    }
    debug_assert_eq!(entries.len(), heading_count * SCRATCH_ACTIONS_PER_HEADING);
    TacticAssetCatalog::new(entries)
}

pub fn scratch_action_heading_index(
    catalog: &TacticAssetCatalog,
    action_index: usize,
) -> Result<usize, TacticAssetError> {
    let entry = catalog
        .entries()
        .get(action_index)
        .ok_or_else(|| TacticAssetError::InvalidAsset("scratch action index is invalid".into()))?;
    let (_, heading, _) = split_heading(entry.option_id())?;
    Ok(heading)
}

pub fn scratch_action_index_with_heading(
    catalog: &TacticAssetCatalog,
    action_index: usize,
    replacement_heading_index: usize,
) -> Result<usize, TacticAssetError> {
    let entry = catalog
        .entries()
        .get(action_index)
        .ok_or_else(|| TacticAssetError::InvalidAsset("scratch action index is invalid".into()))?;
    let (prefix, _, suffix) = split_heading(entry.option_id())?;
    let replacement = format!("{prefix}{replacement_heading_index:02}{suffix}");
    catalog
        .entries()
        .binary_search_by_key(&replacement.as_str(), |entry| entry.option_id())
        .map_err(|_| {
            TacticAssetError::InvalidAsset("scratch replacement heading is invalid".into())
        })
}

pub fn map_scratch_action_to_finer_catalog(
    source: &TacticAssetCatalog,
    target: &TacticAssetCatalog,
    action_index: usize,
) -> Result<usize, TacticAssetError> {
    let entry = source
        .entries()
        .get(action_index)
        .ok_or_else(|| TacticAssetError::InvalidAsset("scratch action index is invalid".into()))?;
    let (prefix, heading, suffix) = split_heading(entry.option_id())?;
    let replacement = format!("{prefix}{:02}{suffix}", heading.saturating_mul(2));
    target
        .entries()
        .binary_search_by_key(&replacement.as_str(), |entry| entry.option_id())
        .map_err(|_| TacticAssetError::InvalidAsset("finer scratch action is missing".into()))
}

fn split_heading(option_id: &str) -> Result<(&str, usize, &str), TacticAssetError> {
    let marker = option_id
        .rfind(".h")
        .ok_or_else(|| TacticAssetError::InvalidAsset("scratch action has no heading".into()))?;
    let digits_start = marker + 2;
    let digits_end = option_id[digits_start..]
        .find('.')
        .map(|offset| digits_start + offset)
        .ok_or_else(|| TacticAssetError::InvalidAsset("scratch heading is malformed".into()))?;
    let heading = option_id[digits_start..digits_end]
        .parse()
        .map_err(|_| TacticAssetError::InvalidAsset("scratch heading is malformed".into()))?;
    Ok((
        &option_id[..digits_start],
        heading,
        &option_id[digits_end..],
    ))
}

fn push_raw(
    entries: &mut Vec<TacticCatalogEntry>,
    heading_index: usize,
    heading: f32,
) -> Result<(), TacticAssetError> {
    let raw = stick(heading, 127);
    entries.push(TacticCatalogEntry::new(
        format!("scratch.raw.h{heading_index:02}.t01"),
        TacticAssetSource::ReactiveController(
            ControllerProgram::parse(&format!(
                "duskcontrol 1\nframes 1\nbezier replace from 0 for 1 p0 {x} {y} p1 {x} {y} p2 {x} {y} p3 {x} {y}\n",
                x = raw[0],
                y = raw[1],
            ))
            .map_err(|error| TacticAssetError::InvalidAsset(error.to_string()))?,
        ),
    )?);
    Ok(())
}

fn push_move(
    entries: &mut Vec<TacticCatalogEntry>,
    heading_index: usize,
    heading: f32,
    ticks: u32,
    family: &str,
) -> Result<(), TacticAssetError> {
    entries.push(TacticCatalogEntry::new(
        format!("scratch.{family}.h{heading_index:02}.t{ticks:02}"),
        TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan::new(
            GenericTactic::MaintainRelativeHeading {
                heading_radians_f32_bits: heading.to_bits(),
                magnitude: 127,
            },
            ticks,
        )),
    )?);
    Ok(())
}

fn camera_lock_move_program(
    initial_stick: [i8; 2],
    duration: u32,
    lock_frame: u32,
) -> Result<ControllerProgram, TacticAssetError> {
    if lock_frame > 1 || lock_frame >= duration {
        return Err(TacticAssetError::InvalidAsset(
            "scratch camera-lock frame is invalid".into(),
        ));
    }
    let forward_start = lock_frame + 1;
    let mut source = format!(
        "duskcontrol 1\nframes {duration}\n\
         bezier replace from 0 for 1 p0 {x} {y} p1 {x} {y} p2 {x} {y} p3 {x} {y}\n\
         buttons from {lock_frame} for 1 L\n",
        x = initial_stick[0],
        y = initial_stick[1],
    );
    if duration > forward_start {
        source.push_str(&format!(
            "bezier replace from {forward_start} for {} p0 0 127 p1 0 127 p2 0 127 p3 0 127\n",
            duration - forward_start
        ));
    }
    ControllerProgram::parse(&source)
        .map_err(|error| TacticAssetError::InvalidAsset(error.to_string()))
}

fn camera_lock_roll_program(
    initial_stick: [i8; 2],
    duration: u32,
    timing: u32,
) -> Result<ControllerProgram, TacticAssetError> {
    if timing > 2 || timing >= duration {
        return Err(TacticAssetError::InvalidAsset(
            "scratch camera-lock roll timing is invalid".into(),
        ));
    }
    let mut source = format!(
        "duskcontrol 1\nframes {duration}\n\
         bezier replace from 0 for 1 p0 {x} {y} p1 {x} {y} p2 {x} {y} p3 {x} {y}\n",
        x = initial_stick[0],
        y = initial_stick[1],
    );
    match timing {
        0 => source.push_str("buttons from 0 for 1 L A\n"),
        1 => source.push_str("buttons from 1 for 1 L A\n"),
        2 => source.push_str("buttons from 1 for 1 L\nbuttons from 2 for 1 A\n"),
        _ => unreachable!(),
    }
    let forward_start = timing + 1;
    if duration > forward_start {
        source.push_str(&format!(
            "bezier replace from {forward_start} for {} p0 0 127 p1 0 127 p2 0 127 p3 0 127\n",
            duration - forward_start
        ));
    }
    ControllerProgram::parse(&source)
        .map_err(|error| TacticAssetError::InvalidAsset(error.to_string()))
}

fn stick(angle: f32, magnitude: i8) -> [i8; 2] {
    [
        (-angle.sin() * f32::from(magnitude)).round() as i8,
        (angle.cos() * f32::from(magnitude)).round() as i8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_fixed_bounded_and_fully_executable() {
        let first = scratch_action_catalog().unwrap();
        let second = scratch_action_catalog().unwrap();
        assert_eq!(first, second);
        assert_eq!(first.entries().len(), SCRATCH_ACTION_COUNT);
        assert_eq!(first.action_schema_sha256(), second.action_schema_sha256());
        assert!(first.entries().iter().all(|entry| {
            entry.option_id().starts_with("scratch.")
                && entry.description().duration.maximum_ticks <= 16
                && first.prepare_execution(entry.option_id()).is_ok()
        }));
        assert!(first.entries().iter().all(|entry| {
            !entry.option_id().contains("seek") && !entry.option_id().contains("target")
        }));
    }

    #[test]
    fn refinement_catalog_preserves_variants_and_inserts_exact_midpoints() {
        let coarse = scratch_action_catalog().unwrap();
        let fine = scratch_action_catalog_with_heading_count(32).unwrap();
        assert_eq!(fine.entries().len(), 2 * coarse.entries().len());
        for action_index in 0..coarse.entries().len() {
            let coarse_entry = &coarse.entries()[action_index];
            let fine_index =
                map_scratch_action_to_finer_catalog(&coarse, &fine, action_index).unwrap();
            let fine_entry = &fine.entries()[fine_index];
            assert_eq!(
                scratch_action_heading_index(&fine, fine_index).unwrap(),
                2 * scratch_action_heading_index(&coarse, action_index).unwrap()
            );
            assert_eq!(
                coarse_entry.description().content_sha256,
                fine_entry.description().content_sha256
            );
            assert_eq!(
                coarse_entry.description().option.parameters,
                fine_entry.description().option.parameters
            );
            assert_eq!(
                coarse_entry.description().duration,
                fine_entry.description().duration
            );
            assert_eq!(
                coarse_entry.description().executor,
                fine_entry.description().executor
            );
        }
        assert!(scratch_action_catalog_with_heading_count(24).is_err());
        assert!(scratch_action_catalog_with_heading_count(64).is_err());
        assert!(scratch_action_catalog_with_heading_count(512).is_err());
    }

    #[test]
    fn camera_roll_includes_all_three_generic_schedules() {
        let catalog = scratch_action_catalog().unwrap();
        let schedules = catalog
            .entries()
            .iter()
            .filter(|entry| entry.option_id().starts_with("scratch.camera_roll.h00.t08"))
            .map(|entry| {
                entry
                    .exact_static_realization()
                    .unwrap()
                    .unwrap()
                    .tape
                    .frames
                    .iter()
                    .take(3)
                    .map(|frame| frame.pads[0].buttons)
                    .collect::<Vec<_>>()
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            schedules,
            std::collections::BTreeSet::from([
                vec![0x0140, 0, 0],
                vec![0, 0x0140, 0],
                vec![0, 0x0040, 0x0100],
            ])
        );
    }
}
