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
use crate::tactic_blueprint::TacticBlueprint;
use dusklight_control::controller_program::ControllerProgram;
use dusklight_control::roll_option::RollOptionPlan;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::f32::consts::{PI, TAU};

pub const PARAMETERIZED_TACTIC_FAMILY_SCHEMA_V3: &str =
    "dusklight-parameterized-tactic-families/v3";
pub const MAX_PARAMETERIZED_PROPOSALS: usize = 48;
const MAX_PARAMETERIZED_TACTIC_TICKS: u32 = 4_096;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ParameterizedTacticFamily {
    SeekTarget,
    RelativeHeading,
    ShortCurve,
    Roll,
    Neutral,
}

impl ParameterizedTacticFamily {
    fn slug(self) -> &'static str {
        match self {
            Self::SeekTarget => "seek-target",
            Self::RelativeHeading => "relative-heading",
            Self::ShortCurve => "short-curve",
            Self::Roll => "roll",
            Self::Neutral => "neutral",
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
    pub feedback: Option<ParameterizedTacticFeedback>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterizedTacticFeedback {
    pub previous_reward: f32,
    pub goal_progress: f32,
    pub ensemble_uncertainty: Option<f32>,
    pub endpoint_novel: bool,
    pub terminal: bool,
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
            || self.feedback.is_some_and(|feedback| {
                !feedback.previous_reward.is_finite()
                    || !feedback.goal_progress.is_finite()
                    || feedback
                        .ensemble_uncertainty
                        .is_some_and(|value| !value.is_finite() || value < 0.0)
                    || feedback.terminal
            })
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
    hasher.update(PARAMETERIZED_TACTIC_FAMILY_SCHEMA_V3.as_bytes());
    hasher.update((MAX_PARAMETERIZED_PROPOSALS as u64).to_le_bytes());
    hasher.update(MAX_PARAMETERIZED_TACTIC_TICKS.to_le_bytes());
    for family in [
        ParameterizedTacticFamily::SeekTarget,
        ParameterizedTacticFamily::RelativeHeading,
        ParameterizedTacticFamily::ShortCurve,
        ParameterizedTacticFamily::Roll,
        ParameterizedTacticFamily::Neutral,
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
    // Candidate coverage is independent of observed reward, goal progress,
    // novelty, and auxiliary prediction heads. Those signals remain available
    // to learned models, but cannot hand-author the next action set.
    let jitter_degrees = 5.0;
    let centered_draw = (draw % 2_001) as f32 / 1_000.0 - 1.0;
    let angular_jitter = centered_draw * jitter_degrees * PI / 180.0;
    let central_heading = normalize_angle(goal_heading + angular_jitter);
    let mut entries = BTreeMap::<String, TacticCatalogEntry>::new();

    let seek_durations = [4_u32, 8, 16, 40].map(|ticks| ticks.min(context.maximum_ticks));
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

    // One exact goal-relative heading plus a complete direction lattice keeps
    // straight movement available without assigning it utility. Noncentral
    // bins retain seeded continuous jitter for off-grid exploration.
    let heading_offsets = [
        0.0,
        -PI / 8.0,
        PI / 8.0,
        -PI / 4.0,
        PI / 4.0,
        -3.0 * PI / 8.0,
        3.0 * PI / 8.0,
        -PI / 2.0,
        PI / 2.0,
        -5.0 * PI / 8.0,
        5.0 * PI / 8.0,
        -3.0 * PI / 4.0,
        3.0 * PI / 4.0,
        -7.0 * PI / 8.0,
        7.0 * PI / 8.0,
        PI,
    ];
    let durations = [4_u32];
    for (index, offset) in heading_offsets.iter().copied().enumerate() {
        let magnitude = 127;
        let heading = if index == 0 {
            goal_heading
        } else {
            normalize_angle(central_heading + offset)
        };
        for duration in durations {
            insert(
                &mut entries,
                ParameterizedTacticFamily::RelativeHeading,
                TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan::new(
                    GenericTactic::MaintainRelativeHeading {
                        heading_radians_f32_bits: heading.to_bits(),
                        magnitude,
                    },
                    duration.min(context.maximum_ticks),
                )),
                context.maximum_ticks,
            )?;
        }
    }

    for clockwise in [false, true] {
        let start = stick(central_heading, 127);
        let bend_angle = PI / 4.0;
        let bend = stick(
            normalize_angle(central_heading + if clockwise { bend_angle } else { -bend_angle }),
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

    // Roll is an atomic prompted action layered over directional movement.
    // Expose the same generic direction lattice as ordinary movement so the
    // learner can decide both whether and where to roll. This is state-local
    // applicability, not a pre-authored cadence or route composition.
    for (index, offset) in heading_offsets.iter().copied().enumerate() {
        let heading = if index == 0 {
            goal_heading
        } else {
            normalize_angle(central_heading + offset)
        };
        for recovery_frames in [3_u32] {
            // RollOptionPlan's camera-relative +90 semantic emits positive raw
            // stick X, while world-heading controllers use the game's -sin X
            // convention. Negate the world-relative heading so roll, seek,
            // curve, and maintained-heading instances command one direction.
            let direction_degrees = (-heading * 180.0 / PI).round().clamp(-180.0, 180.0) as i16;
            insert(
                &mut entries,
                ParameterizedTacticFamily::Roll,
                TacticAssetSource::Roll(RollOptionPlan::new(
                    direction_degrees,
                    127,
                    recovery_frames,
                )),
                context.maximum_ticks,
            )?;
        }
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
    Ok(ParameterizedTacticProposalCatalog {
        family_schema_sha256,
        catalog,
        // Useful compositions are mined and promoted from authenticated
        // replay. The proposal generator must not pre-author sequences whose
        // apparent value comes from the benchmark designer.
        blueprints: Vec::new(),
    })
}

fn insert(
    entries: &mut BTreeMap<String, TacticCatalogEntry>,
    family: ParameterizedTacticFamily,
    source: TacticAssetSource,
    maximum_ticks: u32,
) -> Result<(), TacticAssetError> {
    let canonical = source.canonical_bytes()?;
    let mut hasher = Sha256::new();
    hasher.update(PARAMETERIZED_TACTIC_FAMILY_SCHEMA_V3.as_bytes());
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
    hasher.update(PARAMETERIZED_TACTIC_FAMILY_SCHEMA_V3.as_bytes());
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
        (-angle.sin() * f32::from(magnitude)).round() as i8,
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
            feedback: None,
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
        assert!(first.blueprints.is_empty());
    }

    #[test]
    fn proposals_cover_parameters_instead_of_blessed_grid_ids() {
        let proposal_context = context(19, 4);
        let expected_straight_heading = goal_relative_heading(proposal_context);
        let proposals = propose_parameterized_tactics(proposal_context).unwrap();
        let descriptors = proposals.catalog.option_descriptors().collect::<Vec<_>>();
        let types = descriptors
            .iter()
            .map(|descriptor| descriptor.option_type.clone())
            .collect::<Vec<_>>();

        assert!(types.contains(&OptionType::MaintainHeading));
        assert!(types.contains(&OptionType::Move));
        assert!(types.contains(&OptionType::Roll));
        assert!(!types.contains(&OptionType::Interact));
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
        assert!(descriptors.iter().any(|descriptor| {
            matches!(
                (
                    descriptor.parameters.get("heading_radians"),
                    descriptor.parameters.get("magnitude")
                ),
                (
                    Some(OptionParameter::F32Bits(heading)),
                    Some(OptionParameter::Unsigned(127))
                ) if *heading == expected_straight_heading.to_bits()
            )
        }));
        assert!(descriptors.iter().all(|descriptor| {
            descriptor.option_id.starts_with("family/")
                && !descriptor.option_id.starts_with("move.heading.")
                && !descriptor.option_id.starts_with("roll.direction.")
        }));
        let mut heading_durations = BTreeMap::<u32, BTreeSet<u64>>::new();
        for descriptor in descriptors
            .iter()
            .filter(|descriptor| descriptor.option_type == OptionType::MaintainHeading)
        {
            let Some(OptionParameter::F32Bits(heading)) =
                descriptor.parameters.get("heading_radians")
            else {
                panic!("maintained heading has no typed heading");
            };
            let Some(OptionParameter::Unsigned(duration)) =
                descriptor.parameters.get("maximum_ticks")
            else {
                panic!("maintained heading has no typed duration");
            };
            heading_durations
                .entry(*heading)
                .or_default()
                .insert(*duration);
        }
        assert_eq!(heading_durations.len(), 16);
        assert!(
            heading_durations
                .values()
                .all(|durations| durations == &BTreeSet::from([4]))
        );
        assert!(proposals.blueprints.is_empty());
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

    #[test]
    fn measured_outcome_and_auxiliary_signals_do_not_author_parameter_instances() {
        let baseline = propose_parameterized_tactics(context(29, 5)).unwrap();
        let mut adapted_context = context(29, 5);
        adapted_context.feedback = Some(ParameterizedTacticFeedback {
            previous_reward: -0.25,
            goal_progress: -12.0,
            ensemble_uncertainty: Some(0.81),
            endpoint_novel: false,
            terminal: false,
        });
        let adapted = propose_parameterized_tactics(adapted_context).unwrap();
        let ids = |proposals: &ParameterizedTacticProposalCatalog| {
            proposals
                .catalog
                .entries()
                .iter()
                .map(|entry| entry.option_id().to_owned())
                .collect::<BTreeSet<_>>()
        };

        assert_eq!(ids(&baseline), ids(&adapted));
        assert_eq!(baseline.family_schema_sha256, adapted.family_schema_sha256);
    }

    #[test]
    fn world_heading_uses_one_main_stick_x_convention_across_atomic_families() {
        assert_eq!(stick(PI / 2.0, 100), [-100, 0]);

        let mut proposal_context = context(31, 2);
        proposal_context.player_position = [0.0; 3];
        proposal_context.camera_yaw_radians = Some(0.0);
        proposal_context.goal_coordinate = [100.0, 0.0, 0.0];
        let proposals = propose_parameterized_tactics(proposal_context).unwrap();
        let roll_directions = proposals
            .catalog
            .option_descriptors()
            .filter(|descriptor| descriptor.option_type == OptionType::Roll)
            .map(|descriptor| descriptor.parameters.get("direction_degrees").cloned())
            .collect::<Vec<_>>();

        assert_eq!(roll_directions.len(), 16);
        assert!(
            roll_directions
                .iter()
                .any(|direction| matches!(direction, Some(OptionParameter::Signed(-90))))
        );
    }
}
