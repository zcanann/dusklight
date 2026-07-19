//! Proof-anchored local refinement of typed option executions.

use crate::option_execution::{OptionExecution, OptionExecutionError};
use crate::roll_option::{
    MAX_ROLL_TICKS, RollCancellationHit, RollOptionError, RollOptionPlan, RollRealization,
};
use crate::tape::InputTape;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::error::Error;
use std::fmt;

pub const OPTION_GOLF_SCHEMA_V1: &str = "dusklight-option-relative-golf/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RollGolfSteps {
    pub heading_degrees: u16,
    pub magnitude: u8,
    pub duration_ticks: u32,
    pub phase_ticks: u32,
    pub button_ticks: u32,
    pub cancellation_ticks: u32,
}

impl Default for RollGolfSteps {
    fn default() -> Self {
        Self {
            heading_degrees: 1,
            magnitude: 1,
            duration_ticks: 1,
            phase_ticks: 1,
            button_ticks: 1,
            cancellation_ticks: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RollGolfDimension {
    Heading,
    Magnitude,
    Duration,
    Phase,
    ButtonTiming,
    Cancellation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RollGolfProposal {
    pub schema: String,
    pub dimension: RollGolfDimension,
    pub delta: i64,
    pub absolute_start_tick: u64,
    pub plan: RollOptionPlan,
    pub cancellation: Option<RollCancellationHit>,
    pub realization: RollRealization,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OptionGolfError {
    InvalidSteps,
    SeedExecution(OptionExecutionError),
    SeedPlan(RollOptionError),
    SeedMismatch,
}

impl fmt::Display for OptionGolfError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSteps => formatter.write_str("option-golf steps must all be nonzero"),
            Self::SeedExecution(source) => write!(formatter, "invalid seed execution: {source}"),
            Self::SeedPlan(source) => write!(formatter, "invalid seed roll plan: {source}"),
            Self::SeedMismatch => formatter.write_str(
                "seed roll plan and cancellation do not reproduce the successful execution",
            ),
        }
    }
}

impl Error for OptionGolfError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::SeedExecution(source) => Some(source),
            Self::SeedPlan(source) => Some(source),
            Self::InvalidSteps | Self::SeedMismatch => None,
        }
    }
}

impl From<OptionExecutionError> for OptionGolfError {
    fn from(value: OptionExecutionError) -> Self {
        Self::SeedExecution(value)
    }
}

impl From<RollOptionError> for OptionGolfError {
    fn from(value: RollOptionError) -> Self {
        Self::SeedPlan(value)
    }
}

/// Builds the finite one-step semantic neighborhood around a proved roll.
///
/// The successful execution is authenticated against `tape`, then reproduced
/// from `seed_plan` and `seed_cancellation` before any proposal is returned.
/// Each proposal changes exactly one named option-relative dimension and
/// includes the exact raw realization that an evaluator must place and replay.
pub fn golf_roll_option(
    seed_plan: &RollOptionPlan,
    seed_cancellation: Option<RollCancellationHit>,
    successful_execution: &OptionExecution,
    tape: &InputTape,
    steps: RollGolfSteps,
) -> Result<Vec<RollGolfProposal>, OptionGolfError> {
    if steps.heading_degrees == 0
        || steps.magnitude == 0
        || steps.duration_ticks == 0
        || steps.phase_ticks == 0
        || steps.button_ticks == 0
        || steps.cancellation_ticks == 0
    {
        return Err(OptionGolfError::InvalidSteps);
    }
    successful_execution.validate_against_tape(tape)?;
    let expected = seed_plan.capture_execution(
        successful_execution.option_id.clone(),
        tape,
        successful_execution.realized_tape_range,
        seed_cancellation,
    )?;
    if expected != *successful_execution {
        return Err(OptionGolfError::SeedMismatch);
    }

    let start = successful_execution.realized_tape_range.start_frame;
    let mut proposals = Vec::new();
    let mut identities = HashSet::new();

    for sign in [-1_i64, 1] {
        let mut plan = seed_plan.clone();
        let delta = sign * i64::from(steps.heading_degrees);
        plan.direction_degrees = normalize_heading(i64::from(plan.direction_degrees) + delta);
        push_proposal(
            &mut proposals,
            &mut identities,
            RollGolfDimension::Heading,
            delta,
            start,
            plan,
            seed_cancellation,
        );

        let mut plan = seed_plan.clone();
        let delta = sign * i64::from(steps.magnitude);
        plan.magnitude = (i64::from(plan.magnitude) + delta)
            .clamp(1, 127)
            .try_into()
            .unwrap();
        push_proposal(
            &mut proposals,
            &mut identities,
            RollGolfDimension::Magnitude,
            delta,
            start,
            plan,
            seed_cancellation,
        );

        if let Some(recovery) = checked_adjust_u32(
            seed_plan.recovery_frames,
            sign * i64::from(steps.duration_ticks),
            0,
            MAX_ROLL_TICKS,
        ) {
            let mut plan = seed_plan.clone();
            plan.recovery_frames = recovery;
            push_proposal(
                &mut proposals,
                &mut identities,
                RollGolfDimension::Duration,
                sign * i64::from(steps.duration_ticks),
                start,
                plan,
                seed_cancellation,
            );
        }

        if let Some(phase_start) = checked_adjust_u64(start, sign * i64::from(steps.phase_ticks)) {
            let mut plan = seed_plan.clone();
            plan.spacing.phase_tick = ((phase_start + u64::from(plan.button_frame))
                % u64::from(plan.spacing.period_ticks))
                as u32;
            push_proposal(
                &mut proposals,
                &mut identities,
                RollGolfDimension::Phase,
                sign * i64::from(steps.phase_ticks),
                phase_start,
                plan,
                seed_cancellation,
            );
        }

        let planned_ticks = seed_plan.planned_ticks()?;
        if let Some(button_frame) = checked_adjust_u32(
            seed_plan.button_frame,
            sign * i64::from(steps.button_ticks),
            0,
            planned_ticks - 1,
        ) {
            let mut plan = seed_plan.clone();
            plan.button_frame = button_frame;
            plan.recovery_frames = planned_ticks - button_frame - 1;
            plan.spacing.phase_tick =
                ((start + u64::from(button_frame)) % u64::from(plan.spacing.period_ticks)) as u32;
            push_proposal(
                &mut proposals,
                &mut identities,
                RollGolfDimension::ButtonTiming,
                sign * i64::from(steps.button_ticks),
                start,
                plan,
                seed_cancellation,
            );
        }

        if let Some(hit) = seed_cancellation
            && let Some(tick) = checked_adjust_u32(
                hit.tick,
                sign * i64::from(steps.cancellation_ticks),
                1,
                planned_ticks - 1,
            )
        {
            push_proposal(
                &mut proposals,
                &mut identities,
                RollGolfDimension::Cancellation,
                sign * i64::from(steps.cancellation_ticks),
                start,
                seed_plan.clone(),
                Some(RollCancellationHit { tick, ..hit }),
            );
        }
    }
    Ok(proposals)
}

#[allow(clippy::too_many_arguments)]
fn push_proposal(
    proposals: &mut Vec<RollGolfProposal>,
    identities: &mut HashSet<Vec<u8>>,
    dimension: RollGolfDimension,
    delta: i64,
    absolute_start_tick: u64,
    plan: RollOptionPlan,
    cancellation: Option<RollCancellationHit>,
) {
    let Ok(realization) = plan.realize(absolute_start_tick, cancellation) else {
        return;
    };
    let Ok(identity) = serde_json::to_vec(&(absolute_start_tick, &plan, cancellation)) else {
        return;
    };
    if identities.insert(identity) {
        proposals.push(RollGolfProposal {
            schema: OPTION_GOLF_SCHEMA_V1.into(),
            dimension,
            delta,
            absolute_start_tick,
            plan,
            cancellation,
            realization,
        });
    }
}

fn checked_adjust_u32(value: u32, delta: i64, minimum: u32, maximum: u32) -> Option<u32> {
    let adjusted = i64::from(value).checked_add(delta)?;
    (i64::from(minimum)..=i64::from(maximum))
        .contains(&adjusted)
        .then_some(adjusted as u32)
}

fn checked_adjust_u64(value: u64, delta: i64) -> Option<u64> {
    if delta < 0 {
        value.checked_sub(delta.unsigned_abs())
    } else {
        value.checked_add(delta as u64)
    }
}

fn normalize_heading(value: i64) -> i16 {
    ((value + 180).rem_euclid(360) - 180) as i16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::option_execution::{OptionCondition, TapeRange};
    use crate::roll_option::{ROLL_OPTION_SCHEMA_V1, RollSpacing};
    use crate::tape::{InputFrame, RawPadState, TapeBoot};
    use std::collections::BTreeSet;

    fn seed() -> (
        RollOptionPlan,
        RollCancellationHit,
        InputTape,
        OptionExecution,
    ) {
        let plan = RollOptionPlan {
            schema: ROLL_OPTION_SCHEMA_V1.into(),
            direction_degrees: 30,
            magnitude: 90,
            button_frame: 2,
            recovery_frames: 5,
            spacing: RollSpacing {
                period_ticks: 4,
                phase_tick: 0,
            },
            cancellation_conditions: vec![OptionCondition::TargetReached {
                target: "door".into(),
            }],
        };
        let cancellation = RollCancellationHit {
            tick: 5,
            condition_index: 0,
        };
        let realization = plan.realize(10, Some(cancellation)).unwrap();
        let mut frames = vec![InputFrame::default(); 10];
        frames.extend(realization.frames);
        frames.push(InputFrame {
            owned_ports: 0x0f,
            pads: [RawPadState::default(); 4],
            ..InputFrame::default()
        });
        let tape = InputTape {
            boot: TapeBoot::Process,
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            frames,
        };
        let execution = plan
            .capture_execution(
                "door-roll".into(),
                &tape,
                TapeRange {
                    start_frame: 10,
                    end_frame_exclusive: 15,
                },
                Some(cancellation),
            )
            .unwrap();
        (plan, cancellation, tape, execution)
    }

    #[test]
    fn proved_roll_generates_every_semantic_local_axis() {
        let (plan, cancellation, tape, execution) = seed();
        let proposals = golf_roll_option(
            &plan,
            Some(cancellation),
            &execution,
            &tape,
            RollGolfSteps::default(),
        )
        .unwrap();
        let dimensions: BTreeSet<_> = proposals
            .iter()
            .map(|proposal| format!("{:?}", proposal.dimension))
            .collect();
        assert_eq!(
            dimensions,
            [
                "ButtonTiming",
                "Cancellation",
                "Duration",
                "Heading",
                "Magnitude",
                "Phase",
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        );
        assert_eq!(proposals.len(), 12);
        for proposal in &proposals {
            assert_eq!(proposal.schema, OPTION_GOLF_SCHEMA_V1);
            assert_eq!(
                proposal.realization,
                proposal
                    .plan
                    .realize(proposal.absolute_start_tick, proposal.cancellation)
                    .unwrap()
            );
        }

        let phases: Vec<_> = proposals
            .iter()
            .filter(|proposal| proposal.dimension == RollGolfDimension::Phase)
            .collect();
        assert_eq!(phases[0].absolute_start_tick, 9);
        assert_eq!(phases[0].plan.spacing.phase_tick, 3);
        assert_eq!(phases[1].absolute_start_tick, 11);
        assert_eq!(phases[1].plan.spacing.phase_tick, 1);

        let button_moves: Vec<_> = proposals
            .iter()
            .filter(|proposal| proposal.dimension == RollGolfDimension::ButtonTiming)
            .collect();
        assert!(
            button_moves
                .iter()
                .all(|proposal| proposal.plan.planned_ticks().unwrap() == 8)
        );
    }

    #[test]
    fn seed_must_match_the_authenticated_success_exactly() {
        let (plan, cancellation, tape, mut execution) = seed();
        execution.parameters.remove("magnitude");
        assert!(matches!(
            golf_roll_option(
                &plan,
                Some(cancellation),
                &execution,
                &tape,
                RollGolfSteps::default()
            ),
            Err(OptionGolfError::SeedMismatch)
        ));
        assert!(matches!(
            golf_roll_option(
                &plan,
                Some(cancellation),
                &plan
                    .capture_execution(
                        "door-roll".into(),
                        &tape,
                        TapeRange {
                            start_frame: 10,
                            end_frame_exclusive: 15,
                        },
                        Some(cancellation)
                    )
                    .unwrap(),
                &tape,
                RollGolfSteps {
                    heading_degrees: 0,
                    ..RollGolfSteps::default()
                }
            ),
            Err(OptionGolfError::InvalidSteps)
        ));
    }
}
