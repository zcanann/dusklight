//! Proof-anchored deterministic neighborhoods for static motion paths.

use crate::motion_path::{
    MAX_PATH_TICKS, MotionPathError, MotionPathPlan, MotionPathRealization, PathCancellationHit,
    StickPath,
};
use crate::option_execution::{OptionExecution, OptionExecutionError};
use crate::tape::InputTape;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::error::Error;
use std::fmt;

pub const MOTION_PATH_GOLF_SCHEMA_V1: &str = "dusklight-motion-path-relative-golf/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MotionPathGolfSteps {
    pub point_units: u16,
    pub duration_ticks: u32,
    pub phase_units: u32,
    pub cancellation_ticks: u32,
}

impl Default for MotionPathGolfSteps {
    fn default() -> Self {
        Self {
            point_units: 1,
            duration_ticks: 1,
            phase_units: 1,
            cancellation_ticks: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionPathGolfDimension {
    PointX,
    PointY,
    Duration,
    SamplePhase,
    Cancellation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MotionPathGolfProposal {
    pub schema: String,
    pub dimension: MotionPathGolfDimension,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point_index: Option<usize>,
    pub delta: i64,
    pub plan: MotionPathPlan,
    pub cancellation: Option<PathCancellationHit>,
    pub realization: MotionPathRealization,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MotionPathGolfError {
    InvalidSteps,
    SeedExecution(OptionExecutionError),
    SeedPlan(MotionPathError),
    SeedMismatch,
}

impl fmt::Display for MotionPathGolfError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSteps => formatter.write_str("motion-path golf steps must all be nonzero"),
            Self::SeedExecution(error) => write!(formatter, "invalid seed execution: {error}"),
            Self::SeedPlan(error) => write!(formatter, "invalid seed motion path: {error}"),
            Self::SeedMismatch => formatter.write_str(
                "seed motion path and cancellation do not reproduce the successful execution",
            ),
        }
    }
}

impl Error for MotionPathGolfError {}

impl From<OptionExecutionError> for MotionPathGolfError {
    fn from(value: OptionExecutionError) -> Self {
        Self::SeedExecution(value)
    }
}

impl From<MotionPathError> for MotionPathGolfError {
    fn from(value: MotionPathError) -> Self {
        Self::SeedPlan(value)
    }
}

/// Authenticates the successful seed and returns a bounded one-axis
/// neighborhood. No proposal executes the game or receives special proof
/// status; evaluators must place and cold-replay its exact realization.
pub fn golf_motion_path(
    seed_plan: &MotionPathPlan,
    seed_cancellation: Option<PathCancellationHit>,
    successful_execution: &OptionExecution,
    tape: &InputTape,
    steps: MotionPathGolfSteps,
) -> Result<Vec<MotionPathGolfProposal>, MotionPathGolfError> {
    if steps.point_units == 0
        || steps.duration_ticks == 0
        || steps.phase_units == 0
        || steps.cancellation_ticks == 0
    {
        return Err(MotionPathGolfError::InvalidSteps);
    }
    successful_execution.validate_against_tape(tape)?;
    let expected = seed_plan.capture_execution(
        successful_execution.option_id.clone(),
        tape,
        successful_execution.realized_tape_range,
        seed_cancellation,
    )?;
    if expected != *successful_execution {
        return Err(MotionPathGolfError::SeedMismatch);
    }

    let mut proposals = Vec::new();
    let mut identities = HashSet::new();
    let point_count = path_points(seed_plan).len();
    for index in 0..point_count {
        for sign in [-1_i64, 1] {
            for dimension in [
                MotionPathGolfDimension::PointX,
                MotionPathGolfDimension::PointY,
            ] {
                let mut plan = seed_plan.clone();
                let delta = sign * i64::from(steps.point_units);
                let point = &mut path_points_mut(&mut plan)[index];
                let axis = match dimension {
                    MotionPathGolfDimension::PointX => &mut point.x,
                    MotionPathGolfDimension::PointY => &mut point.y,
                    _ => unreachable!(),
                };
                let adjusted = (i64::from(*axis) + delta).clamp(-128, 127) as i16;
                if adjusted != *axis {
                    *axis = adjusted;
                    push_proposal(
                        &mut proposals,
                        &mut identities,
                        dimension,
                        Some(index),
                        delta,
                        plan,
                        seed_cancellation,
                    );
                }
            }
        }
    }
    for sign in [-1_i64, 1] {
        let duration_delta = sign * i64::from(steps.duration_ticks);
        if let Some(duration) =
            adjust_u32(seed_plan.duration_ticks, duration_delta, 1, MAX_PATH_TICKS)
        {
            let mut plan = seed_plan.clone();
            plan.duration_ticks = duration;
            push_proposal(
                &mut proposals,
                &mut identities,
                MotionPathGolfDimension::Duration,
                None,
                duration_delta,
                plan,
                seed_cancellation.filter(|hit| hit.tick < duration),
            );
        }
        let phase_delta = sign * i64::from(steps.phase_units);
        if let Some(numerator) = adjust_u32(
            seed_plan.sample_phase.numerator,
            phase_delta,
            0,
            seed_plan.sample_phase.denominator,
        ) {
            let mut plan = seed_plan.clone();
            plan.sample_phase.numerator = numerator;
            push_proposal(
                &mut proposals,
                &mut identities,
                MotionPathGolfDimension::SamplePhase,
                None,
                phase_delta,
                plan,
                seed_cancellation,
            );
        }
        if let Some(hit) = seed_cancellation {
            let cancellation_delta = sign * i64::from(steps.cancellation_ticks);
            if let Some(tick) = adjust_u32(
                hit.tick,
                cancellation_delta,
                1,
                seed_plan.duration_ticks - 1,
            ) {
                push_proposal(
                    &mut proposals,
                    &mut identities,
                    MotionPathGolfDimension::Cancellation,
                    None,
                    cancellation_delta,
                    seed_plan.clone(),
                    Some(PathCancellationHit { tick, ..hit }),
                );
            }
        }
    }
    Ok(proposals)
}

fn path_points(plan: &MotionPathPlan) -> &[crate::motion_path::StickPoint] {
    match &plan.path {
        StickPath::Waypoint { points }
        | StickPath::Rail { points }
        | StickPath::Spline { points } => points,
        StickPath::Bezier { control } => control,
    }
}

fn path_points_mut(plan: &mut MotionPathPlan) -> &mut [crate::motion_path::StickPoint] {
    match &mut plan.path {
        StickPath::Waypoint { points }
        | StickPath::Rail { points }
        | StickPath::Spline { points } => points,
        StickPath::Bezier { control } => control,
    }
}

fn push_proposal(
    proposals: &mut Vec<MotionPathGolfProposal>,
    identities: &mut HashSet<Vec<u8>>,
    dimension: MotionPathGolfDimension,
    point_index: Option<usize>,
    delta: i64,
    plan: MotionPathPlan,
    cancellation: Option<PathCancellationHit>,
) {
    let Ok(realization) = plan.realize(cancellation) else {
        return;
    };
    let Ok(identity) = serde_json::to_vec(&(&plan, cancellation)) else {
        return;
    };
    if identities.insert(identity) {
        proposals.push(MotionPathGolfProposal {
            schema: MOTION_PATH_GOLF_SCHEMA_V1.into(),
            dimension,
            point_index,
            delta,
            plan,
            cancellation,
            realization,
        });
    }
}

fn adjust_u32(value: u32, delta: i64, minimum: u32, maximum: u32) -> Option<u32> {
    let adjusted = i64::from(value).checked_add(delta)?;
    (i64::from(minimum)..=i64::from(maximum))
        .contains(&adjusted)
        .then_some(adjusted as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion_path::{MOTION_PATH_SCHEMA_V1, SamplePhase, StickPoint};
    use crate::option_execution::{OptionCondition, TapeRange};
    use crate::tape::{InputFrame, TapeBoot};
    use std::collections::BTreeSet;

    fn seed(path: StickPath) -> (MotionPathPlan, InputTape, OptionExecution) {
        let plan = MotionPathPlan {
            schema: MOTION_PATH_SCHEMA_V1.into(),
            path,
            duration_ticks: 4,
            sample_phase: SamplePhase {
                numerator: 1,
                denominator: 4,
            },
            cancellation_conditions: vec![OptionCondition::TargetReached {
                target: "door".into(),
            }],
        };
        let cancellation = PathCancellationHit {
            tick: 3,
            condition_index: 0,
        };
        let realization = plan.realize(Some(cancellation)).unwrap();
        let mut frames = vec![InputFrame::default(); 2];
        frames.extend(realization.frames);
        let tape = InputTape {
            boot: TapeBoot::Process,
            frames,
            ..InputTape::default()
        };
        let execution = plan
            .capture_execution(
                "path".into(),
                &tape,
                TapeRange {
                    start_frame: 2,
                    end_frame_exclusive: 5,
                },
                Some(cancellation),
            )
            .unwrap();
        (plan, tape, execution)
    }

    #[test]
    fn waypoint_and_spline_emit_every_deterministic_local_axis() {
        for path in [
            StickPath::Waypoint {
                points: vec![StickPoint { x: 0, y: 20 }, StickPoint { x: 20, y: 40 }],
            },
            StickPath::Spline {
                points: vec![StickPoint { x: 0, y: 20 }, StickPoint { x: 20, y: 40 }],
            },
        ] {
            let (plan, tape, execution) = seed(path);
            let proposals = golf_motion_path(
                &plan,
                Some(PathCancellationHit {
                    tick: 3,
                    condition_index: 0,
                }),
                &execution,
                &tape,
                MotionPathGolfSteps::default(),
            )
            .unwrap();
            let dimensions = proposals
                .iter()
                .map(|proposal| proposal.dimension)
                .collect::<BTreeSet<_>>();
            assert_eq!(
                dimensions,
                BTreeSet::from([
                    MotionPathGolfDimension::PointX,
                    MotionPathGolfDimension::PointY,
                    MotionPathGolfDimension::Duration,
                    MotionPathGolfDimension::SamplePhase,
                    MotionPathGolfDimension::Cancellation,
                ])
            );
            assert!(
                proposals
                    .iter()
                    .all(|proposal| proposal.plan.validate().is_ok())
            );
        }
    }

    #[test]
    fn changed_seed_is_rejected_before_proposal_generation() {
        let (mut plan, tape, execution) = seed(StickPath::Waypoint {
            points: vec![StickPoint { x: 0, y: 20 }],
        });
        plan.duration_ticks += 1;
        assert!(matches!(
            golf_motion_path(
                &plan,
                Some(PathCancellationHit {
                    tick: 3,
                    condition_index: 0,
                }),
                &execution,
                &tape,
                MotionPathGolfSteps::default(),
            ),
            Err(MotionPathGolfError::SeedMismatch | MotionPathGolfError::SeedPlan(_))
        ));
    }
}
