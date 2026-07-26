//! Bounded state-conditioned instances of stable executable tactic families.
//!
//! Families are the learner-facing action contract. Concrete entries are
//! ephemeral, content-addressed instantiations backed by the existing typed
//! tactic executors; they are not a permanently enumerated action grid.

use crate::artifact::Digest;
use crate::native_generic_tactic::{GenericTactic, NativeGenericTacticPlan};
use crate::tactic_asset::{
    TacticAssetAdapter, TacticAssetCatalog, TacticAssetError, TacticAssetSource, TacticCatalogEntry,
};
use crate::tactic_blueprint::{TacticBlueprint, TacticBlueprintNode};
use dusklight_control::controller_program::ControllerProgram;
use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
use dusklight_control::roll_option::RollOptionPlan;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::f32::consts::{PI, TAU};

pub const PARAMETERIZED_TACTIC_FAMILY_SCHEMA_V1: &str =
    "dusklight-parameterized-tactic-families/v1";
pub const MAX_PARAMETERIZED_PROPOSALS: usize = 32;
const MAX_PARAMETERIZED_TACTIC_TICKS: u32 = 4_096;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ParameterizedTacticFamily {
    SeekTarget,
    RelativeHeading,
    ShortCurve,
    Roll,
    Interact,
    Neutral,
    Sequence,
}

impl ParameterizedTacticFamily {
    fn slug(self) -> &'static str {
        match self {
            Self::SeekTarget => "seek-target",
            Self::RelativeHeading => "relative-heading",
            Self::ShortCurve => "short-curve",
            Self::Roll => "roll",
            Self::Interact => "interact",
            Self::Neutral => "neutral",
            Self::Sequence => "sequence",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParameterizedTacticProposalContext {
    pub seed: u64,
    pub decision_index: u64,
    pub state_sha256: Digest,
    pub player_position: [f32; 3],
    pub camera_yaw_radians: Option<f32>,
    pub goal_coordinate: [f32; 3],
    pub maximum_ticks: u32,
}

impl ParameterizedTacticProposalContext {
    fn validate(self) -> Result<Self, TacticAssetError> {
        if self.state_sha256 == Digest::ZERO
            || self.maximum_ticks == 0
            || self.maximum_ticks > MAX_PARAMETERIZED_TACTIC_TICKS
            || self
                .player_position
                .iter()
                .chain(self.goal_coordinate.iter())
                .any(|value| !value.is_finite())
            || self
                .camera_yaw_radians
                .is_some_and(|value| !value.is_finite())
        {
            return Err(TacticAssetError::InvalidAsset(
                "parameterized tactic proposal context is invalid".into(),
            ));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParameterizedTacticProposalCatalog {
    pub family_schema_sha256: Digest,
    pub catalog: TacticAssetCatalog,
    pub blueprints: Vec<TacticBlueprint>,
}

pub fn parameterized_tactic_family_schema_sha256() -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(PARAMETERIZED_TACTIC_FAMILY_SCHEMA_V1.as_bytes());
    hasher.update((MAX_PARAMETERIZED_PROPOSALS as u64).to_le_bytes());
    hasher.update(MAX_PARAMETERIZED_TACTIC_TICKS.to_le_bytes());
    for family in [
        ParameterizedTacticFamily::SeekTarget,
        ParameterizedTacticFamily::RelativeHeading,
        ParameterizedTacticFamily::ShortCurve,
        ParameterizedTacticFamily::Roll,
        ParameterizedTacticFamily::Interact,
        ParameterizedTacticFamily::Neutral,
        ParameterizedTacticFamily::Sequence,
    ] {
        hasher.update(family.slug().as_bytes());
        hasher.update([0]);
    }
    Digest(hasher.finalize().into())
}

pub fn propose_parameterized_tactics(
    context: ParameterizedTacticProposalContext,
) -> Result<ParameterizedTacticProposalCatalog, TacticAssetError> {
    let context = context.validate()?;
    let family_schema_sha256 = parameterized_tactic_family_schema_sha256();
    let draw = proposal_draw(context);
    let goal_heading = goal_relative_heading(context);
    let angular_jitter = ((draw % 21) as i32 - 10) as f32 * PI / 180.0;
    let central_heading = normalize_angle(goal_heading + angular_jitter);
    let mut entries = BTreeMap::<String, TacticCatalogEntry>::new();

    let seek_durations = [context.maximum_ticks.min(16), context.maximum_ticks.min(40)];
    for (index, duration) in seek_durations.into_iter().enumerate() {
        let magnitude = if (draw.rotate_left(index as u32) & 1) == 0 {
            96
        } else {
            127
        };
        insert(
            &mut entries,
            ParameterizedTacticFamily::SeekTarget,
            TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan::new(
                GenericTactic::SeekCoordinate {
                    coordinate_f32_bits: context.goal_coordinate.map(f32::to_bits),
                    tolerance_f32_bits: 0.0_f32.to_bits(),
                    magnitude,
                },
                duration,
            )),
            context.maximum_ticks,
        )?;
    }

    let heading_offsets = [-PI / 3.0, -PI / 9.0, PI / 9.0, PI / 3.0];
    let durations = [4_u32, 8, 12, 16];
    for (index, offset) in heading_offsets.into_iter().enumerate() {
        let magnitude = if ((draw >> index) & 1) == 0 { 80 } else { 127 };
        insert(
            &mut entries,
            ParameterizedTacticFamily::RelativeHeading,
            TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan::new(
                GenericTactic::MaintainRelativeHeading {
                    heading_radians_f32_bits: normalize_angle(central_heading + offset).to_bits(),
                    magnitude,
                },
                durations[(index + (draw as usize & 3)) % durations.len()]
                    .min(context.maximum_ticks),
            )),
            context.maximum_ticks,
        )?;
    }

    for clockwise in [false, true] {
        let start = stick(central_heading, 127);
        let bend = stick(
            normalize_angle(central_heading + if clockwise { PI / 5.0 } else { -PI / 5.0 }),
            127,
        );
        insert(
            &mut entries,
            ParameterizedTacticFamily::ShortCurve,
            TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan::new(
                GenericTactic::ShortCurve {
                    control: [start, start, bend, bend],
                },
                context.maximum_ticks.min(24),
            )),
            context.maximum_ticks,
        )?;
    }

    for recovery_frames in [3_u32, 7] {
        let direction_degrees = (central_heading * 180.0 / PI).round().clamp(-180.0, 180.0) as i16;
        insert(
            &mut entries,
            ParameterizedTacticFamily::Roll,
            TacticAssetSource::Roll(RollOptionPlan::new(direction_degrees, 127, recovery_frames)),
            context.maximum_ticks,
        )?;
    }

    for (press_frames, recovery_frames) in [(1_u32, 1_u32), (2, 4)] {
        insert(
            &mut entries,
            ParameterizedTacticFamily::Interact,
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Interact {
                press_frames,
                recovery_frames,
            })),
            context.maximum_ticks,
        )?;
    }

    insert(
        &mut entries,
        ParameterizedTacticFamily::Neutral,
        TacticAssetSource::ReactiveController(
            ControllerProgram::parse("duskcontrol 1\nframes 4\nneutral replace from 0 for 4\n")
                .map_err(|error| TacticAssetError::InvalidAsset(error.to_string()))?,
        ),
        context.maximum_ticks,
    )?;

    if entries.is_empty() || entries.len() > MAX_PARAMETERIZED_PROPOSALS {
        return Err(TacticAssetError::InvalidAsset(
            "parameterized tactic proposal batch is empty or oversized".into(),
        ));
    }
    let catalog = TacticAssetCatalog::new(entries.into_values().collect())?;
    let blueprints = propose_bounded_compositions(&catalog, context.maximum_ticks)?;
    if catalog.entries().len().saturating_add(blueprints.len()) > MAX_PARAMETERIZED_PROPOSALS {
        return Err(TacticAssetError::InvalidAsset(
            "parameterized tactic proposal batch is oversized".into(),
        ));
    }
    Ok(ParameterizedTacticProposalCatalog {
        family_schema_sha256,
        catalog,
        blueprints,
    })
}

fn propose_bounded_compositions(
    catalog: &TacticAssetCatalog,
    maximum_ticks: u32,
) -> Result<Vec<TacticBlueprint>, TacticAssetError> {
    let roll = catalog
        .entries()
        .iter()
        .find(|entry| entry.option_id().starts_with("family/roll/"));
    let interact = catalog
        .entries()
        .iter()
        .find(|entry| entry.option_id().starts_with("family/interact/"));
    let (Some(roll), Some(interact)) = (roll, interact) else {
        return Ok(Vec::new());
    };
    let mut blueprints = Vec::new();
    for option_ids in [
        [roll.option_id(), interact.option_id()],
        [interact.option_id(), roll.option_id()],
    ] {
        let duration = option_ids.iter().try_fold(0_u32, |total, option_id| {
            catalog
                .entry(option_id)
                .and_then(|entry| total.checked_add(entry.description().duration.maximum_ticks))
        });
        if !duration.is_some_and(|duration| duration <= maximum_ticks) {
            continue;
        }
        let mut hasher = Sha256::new();
        hasher.update(PARAMETERIZED_TACTIC_FAMILY_SCHEMA_V1.as_bytes());
        hasher.update(ParameterizedTacticFamily::Sequence.slug().as_bytes());
        for option_id in option_ids {
            hasher.update((option_id.len() as u64).to_le_bytes());
            hasher.update(option_id.as_bytes());
        }
        let digest = hasher.finalize();
        let suffix = digest[..10]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let blueprint = TacticBlueprint::new(
            format!("generated-sequence-{suffix}"),
            TacticBlueprintNode::Sequence {
                steps: option_ids
                    .into_iter()
                    .map(|option_id| TacticBlueprintNode::Invoke {
                        option_id: option_id.into(),
                    })
                    .collect(),
            },
        )
        .map_err(|error| TacticAssetError::InvalidAsset(error.to_string()))?;
        let compiled = blueprint
            .compile_static(catalog)
            .map_err(|error| TacticAssetError::InvalidAsset(error.to_string()))?;
        if compiled.tape.frames.len() <= maximum_ticks as usize {
            blueprints.push(blueprint);
        }
    }
    Ok(blueprints)
}

fn insert(
    entries: &mut BTreeMap<String, TacticCatalogEntry>,
    family: ParameterizedTacticFamily,
    source: TacticAssetSource,
    maximum_ticks: u32,
) -> Result<(), TacticAssetError> {
    let canonical = source.canonical_bytes()?;
    let mut hasher = Sha256::new();
    hasher.update(PARAMETERIZED_TACTIC_FAMILY_SCHEMA_V1.as_bytes());
    hasher.update(family.slug().as_bytes());
    hasher.update((canonical.len() as u64).to_le_bytes());
    hasher.update(canonical);
    let digest = hasher.finalize();
    let suffix = digest[..10]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let option_id = format!("family/{}/{suffix}", family.slug());
    let entry = TacticCatalogEntry::new(option_id.clone(), source)?;
    if entry.description().duration.maximum_ticks <= maximum_ticks {
        entries.insert(option_id, entry);
    }
    Ok(())
}

fn proposal_draw(context: ParameterizedTacticProposalContext) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(PARAMETERIZED_TACTIC_FAMILY_SCHEMA_V1.as_bytes());
    hasher.update(context.seed.to_le_bytes());
    hasher.update(context.decision_index.to_le_bytes());
    hasher.update(context.state_sha256.0);
    hasher.update(
        context
            .goal_coordinate
            .map(f32::to_bits)
            .map(u32::to_le_bytes)
            .concat(),
    );
    u64::from_le_bytes(hasher.finalize()[..8].try_into().unwrap())
}

fn goal_relative_heading(context: ParameterizedTacticProposalContext) -> f32 {
    let delta_x = context.goal_coordinate[0] - context.player_position[0];
    let delta_z = context.goal_coordinate[2] - context.player_position[2];
    let world_heading = delta_x.atan2(delta_z);
    normalize_angle(world_heading - context.camera_yaw_radians.unwrap_or(0.0))
}

fn normalize_angle(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
}

fn stick(angle: f32, magnitude: i8) -> [i8; 2] {
    [
        (angle.sin() * f32::from(magnitude)).round() as i8,
        (angle.cos() * f32::from(magnitude)).round() as i8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_control::option_execution::{OptionParameter, OptionType};
    use std::collections::BTreeSet;

    fn context(seed: u64, decision_index: u64) -> ParameterizedTacticProposalContext {
        ParameterizedTacticProposalContext {
            seed,
            decision_index,
            state_sha256: Digest([7; 32]),
            player_position: [10.0, 20.0, 30.0],
            camera_yaw_radians: Some(0.25),
            goal_coordinate: [90.0, 25.0, -40.0],
            maximum_ticks: 40,
        }
    }

    #[test]
    fn state_conditioned_family_proposals_are_deterministic_bounded_and_executable() {
        let first = propose_parameterized_tactics(context(11, 3)).unwrap();
        let second = propose_parameterized_tactics(context(11, 3)).unwrap();

        assert_eq!(first, second);
        assert_eq!(
            first.family_schema_sha256,
            parameterized_tactic_family_schema_sha256()
        );
        assert!(!first.catalog.entries().is_empty());
        assert!(
            first
                .catalog
                .entries()
                .len()
                .saturating_add(first.blueprints.len())
                <= MAX_PARAMETERIZED_PROPOSALS
        );
        for entry in first.catalog.entries() {
            assert!(entry.option_id().starts_with("family/"));
            assert!(entry.description().duration.maximum_ticks <= 40);
            first.catalog.prepare_execution(entry.option_id()).unwrap();
        }
        assert!(!first.blueprints.is_empty());
        for blueprint in &first.blueprints {
            let compiled = blueprint.compile_static(&first.catalog).unwrap();
            assert!(compiled.tape.frames.len() <= 40);
        }
    }

    #[test]
    fn proposals_cover_parameters_instead_of_blessed_grid_ids() {
        let proposals = propose_parameterized_tactics(context(19, 4)).unwrap();
        let descriptors = proposals.catalog.option_descriptors().collect::<Vec<_>>();
        let types = descriptors
            .iter()
            .map(|descriptor| descriptor.option_type.clone())
            .collect::<Vec<_>>();

        assert!(types.contains(&OptionType::MaintainHeading));
        assert!(types.contains(&OptionType::Move));
        assert!(types.contains(&OptionType::Roll));
        assert!(types.contains(&OptionType::Interact));
        assert!(
            proposals
                .catalog
                .entries()
                .iter()
                .any(|entry| entry.option_id().starts_with("family/neutral/"))
        );
        assert!(descriptors.iter().any(|descriptor| {
            descriptor
                .parameters
                .get("coordinate")
                .is_some_and(|parameter| matches!(parameter, OptionParameter::Vec3F32Bits(_)))
        }));
        assert!(descriptors.iter().any(|descriptor| {
            descriptor
                .parameters
                .get("heading_radians")
                .is_some_and(|parameter| matches!(parameter, OptionParameter::F32Bits(_)))
        }));
        assert!(descriptors.iter().all(|descriptor| {
            descriptor.option_id.starts_with("family/")
                && !descriptor.option_id.starts_with("move.heading.")
                && !descriptor.option_id.starts_with("roll.direction.")
        }));
        assert!(
            proposals
                .blueprints
                .iter()
                .all(|blueprint| blueprint.asset_id.starts_with("generated-sequence-"))
        );
    }

    #[test]
    fn seed_and_decision_generate_new_bounded_instances_under_one_family_schema() {
        let first = propose_parameterized_tactics(context(23, 1)).unwrap();
        let second = propose_parameterized_tactics(context(23, 2)).unwrap();
        let first_ids = first
            .catalog
            .entries()
            .iter()
            .map(TacticCatalogEntry::option_id)
            .collect::<BTreeSet<_>>();
        let second_ids = second
            .catalog
            .entries()
            .iter()
            .map(TacticCatalogEntry::option_id)
            .collect::<BTreeSet<_>>();

        assert_eq!(first.family_schema_sha256, second.family_schema_sha256);
        assert_ne!(first_ids, second_ids);
    }

    #[test]
    fn invalid_context_fails_without_emitting_a_fallback_catalog() {
        let mut invalid = context(1, 0);
        invalid.goal_coordinate[0] = f32::NAN;
        assert!(propose_parameterized_tactics(invalid).is_err());
        invalid = context(1, 0);
        invalid.maximum_ticks = MAX_PARAMETERIZED_TACTIC_TICKS + 1;
        assert!(propose_parameterized_tactics(invalid).is_err());
    }
}
