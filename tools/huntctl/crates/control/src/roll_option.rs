//! Deterministic typed roll option planning and exact tape realization.

use crate::option_execution::{
    MAX_OPTION_CONDITIONS, OptionCondition, OptionEndReason, OptionExecution, OptionExecutionError,
    OptionParameter, OptionType, TapeRange, validate_condition,
};
use crate::tape::{InputFrame, InputTape, RawPadState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const ROLL_OPTION_SCHEMA_V1: &str = "dusklight-roll-option/v1";
pub const MAX_ROLL_TICKS: u32 = 10_000;
const BUTTON_A: u16 = 0x0100;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RollSpacing {
    /// Button ticks must occupy this absolute-timeline period.
    pub period_ticks: u32,
    /// Required `absolute_button_tick % period_ticks`.
    pub phase_tick: u32,
}

impl Default for RollSpacing {
    fn default() -> Self {
        Self {
            period_ticks: 1,
            phase_tick: 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RollOptionPlan {
    pub schema: String,
    /// Camera-relative direction: zero is forward and +90 is right.
    pub direction_degrees: i16,
    pub magnitude: u8,
    /// Zero-based option tick on which the GameCube A action button is pressed.
    pub button_frame: u32,
    /// Direction-only ticks emitted after the A frame.
    pub recovery_frames: u32,
    pub spacing: RollSpacing,
    pub cancellation_conditions: Vec<OptionCondition>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RollCancellationHit {
    /// Cancellation is observed before input is emitted for this local tick.
    pub tick: u32,
    pub condition_index: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RollRealization {
    pub planned_ticks: u32,
    pub realized_ticks: u32,
    pub absolute_button_tick: u64,
    pub button_emitted: bool,
    pub end_reason: OptionEndReason,
    pub frames: Vec<InputFrame>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RollOptionError {
    UnsupportedSchema,
    InvalidDirection,
    InvalidMagnitude,
    InvalidTiming,
    InvalidSpacing,
    PhaseMismatch {
        absolute_button_tick: u64,
        period_ticks: u32,
        phase_tick: u32,
    },
    InvalidCancellation,
    FrameMismatch,
    RangeOverflow,
    Execution(OptionExecutionError),
}

impl fmt::Display for RollOptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema => formatter.write_str("unsupported roll option schema"),
            Self::InvalidDirection => {
                formatter.write_str("roll direction must be in [-180, 180] degrees")
            }
            Self::InvalidMagnitude => formatter.write_str("roll magnitude must be in 1..=127"),
            Self::InvalidTiming => formatter.write_str("roll timing is invalid or unbounded"),
            Self::InvalidSpacing => formatter.write_str("roll spacing period or phase is invalid"),
            Self::PhaseMismatch {
                absolute_button_tick,
                period_ticks,
                phase_tick,
            } => write!(
                formatter,
                "roll button tick {absolute_button_tick} is not phase {phase_tick} modulo {period_ticks}"
            ),
            Self::InvalidCancellation => formatter.write_str("roll cancellation hit is invalid"),
            Self::FrameMismatch => {
                formatter.write_str("roll realization does not match the requested tape range")
            }
            Self::RangeOverflow => formatter.write_str("roll absolute tape range overflows"),
            Self::Execution(source) => write!(formatter, "roll execution is invalid: {source}"),
        }
    }
}

impl Error for RollOptionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Execution(source) => Some(source),
            _ => None,
        }
    }
}

impl From<OptionExecutionError> for RollOptionError {
    fn from(value: OptionExecutionError) -> Self {
        Self::Execution(value)
    }
}

impl RollOptionPlan {
    pub fn new(direction_degrees: i16, magnitude: u8, recovery_frames: u32) -> Self {
        Self {
            schema: ROLL_OPTION_SCHEMA_V1.into(),
            direction_degrees,
            magnitude,
            button_frame: 0,
            recovery_frames,
            spacing: RollSpacing::default(),
            cancellation_conditions: Vec::new(),
        }
    }

    pub fn planned_ticks(&self) -> Result<u32, RollOptionError> {
        self.button_frame
            .checked_add(1)
            .and_then(|ticks| ticks.checked_add(self.recovery_frames))
            .filter(|ticks| (1..=MAX_ROLL_TICKS).contains(ticks))
            .ok_or(RollOptionError::InvalidTiming)
    }

    pub fn validate(&self) -> Result<(), RollOptionError> {
        if self.schema != ROLL_OPTION_SCHEMA_V1 {
            return Err(RollOptionError::UnsupportedSchema);
        }
        if !(-180..=180).contains(&self.direction_degrees) {
            return Err(RollOptionError::InvalidDirection);
        }
        if !(1..=127).contains(&self.magnitude) {
            return Err(RollOptionError::InvalidMagnitude);
        }
        self.planned_ticks()?;
        if self.spacing.period_ticks == 0
            || self.spacing.period_ticks > MAX_ROLL_TICKS
            || self.spacing.phase_tick >= self.spacing.period_ticks
        {
            return Err(RollOptionError::InvalidSpacing);
        }
        if self.cancellation_conditions.len() > MAX_OPTION_CONDITIONS {
            return Err(RollOptionError::InvalidCancellation);
        }
        for condition in &self.cancellation_conditions {
            validate_condition(condition)?;
        }
        Ok(())
    }

    pub fn realize(
        &self,
        absolute_start_tick: u64,
        cancellation: Option<RollCancellationHit>,
    ) -> Result<RollRealization, RollOptionError> {
        self.validate()?;
        let planned_ticks = self.planned_ticks()?;
        let absolute_button_tick = absolute_start_tick
            .checked_add(u64::from(self.button_frame))
            .ok_or(RollOptionError::RangeOverflow)?;
        if absolute_button_tick % u64::from(self.spacing.period_ticks)
            != u64::from(self.spacing.phase_tick)
        {
            return Err(RollOptionError::PhaseMismatch {
                absolute_button_tick,
                period_ticks: self.spacing.period_ticks,
                phase_tick: self.spacing.phase_tick,
            });
        }

        let (realized_ticks, end_reason) = match cancellation {
            Some(hit)
                if hit.tick > 0
                    && hit.tick < planned_ticks
                    && (hit.condition_index as usize) < self.cancellation_conditions.len() =>
            {
                (
                    hit.tick,
                    OptionEndReason::Cancelled {
                        condition_index: hit.condition_index,
                    },
                )
            }
            Some(_) => return Err(RollOptionError::InvalidCancellation),
            None => (planned_ticks, OptionEndReason::Completed),
        };

        let direction = direction_pad(self.direction_degrees, self.magnitude);
        let mut frames = Vec::with_capacity(realized_ticks as usize);
        for tick in 0..realized_ticks {
            let mut pad = direction;
            if tick == self.button_frame {
                pad.buttons = BUTTON_A;
            }
            frames.push(owned_frame(pad));
        }
        Ok(RollRealization {
            planned_ticks,
            realized_ticks,
            absolute_button_tick,
            button_emitted: self.button_frame < realized_ticks,
            end_reason,
            frames,
        })
    }

    pub fn capture_execution(
        &self,
        option_id: String,
        tape: &InputTape,
        range: TapeRange,
        cancellation: Option<RollCancellationHit>,
    ) -> Result<OptionExecution, RollOptionError> {
        let realization = self.realize(range.start_frame, cancellation)?;
        let expected_end = range
            .start_frame
            .checked_add(u64::from(realization.realized_ticks))
            .ok_or(RollOptionError::RangeOverflow)?;
        let start =
            usize::try_from(range.start_frame).map_err(|_| RollOptionError::RangeOverflow)?;
        let end = usize::try_from(expected_end).map_err(|_| RollOptionError::RangeOverflow)?;
        if range.end_frame_exclusive != expected_end
            || end > tape.frames.len()
            || tape.frames[start..end] != realization.frames
        {
            return Err(RollOptionError::FrameMismatch);
        }

        let mut parameters = BTreeMap::new();
        parameters.insert(
            "direction_degrees".into(),
            OptionParameter::Signed(i64::from(self.direction_degrees)),
        );
        parameters.insert(
            "magnitude".into(),
            OptionParameter::Unsigned(u64::from(self.magnitude)),
        );
        parameters.insert(
            "button_frame".into(),
            OptionParameter::Unsigned(u64::from(self.button_frame)),
        );
        parameters.insert(
            "recovery_frames".into(),
            OptionParameter::Unsigned(u64::from(self.recovery_frames)),
        );
        parameters.insert(
            "spacing_period_ticks".into(),
            OptionParameter::Unsigned(u64::from(self.spacing.period_ticks)),
        );
        parameters.insert(
            "spacing_phase_tick".into(),
            OptionParameter::Unsigned(u64::from(self.spacing.phase_tick)),
        );

        Ok(OptionExecution::capture(
            option_id,
            OptionType::Roll,
            parameters,
            1,
            self.planned_ticks()?,
            OptionCondition::DurationElapsed,
            self.cancellation_conditions.clone(),
            realization.end_reason,
            tape,
            range,
        )?)
    }
}

fn direction_pad(direction_degrees: i16, magnitude: u8) -> RawPadState {
    let radians = f64::from(direction_degrees).to_radians();
    let magnitude = f64::from(magnitude);
    RawPadState {
        stick_x: (radians.sin() * magnitude).round().clamp(-127.0, 127.0) as i8,
        stick_y: (radians.cos() * magnitude).round().clamp(-127.0, 127.0) as i8,
        ..RawPadState::default()
    }
}

fn owned_frame(pad: RawPadState) -> InputFrame {
    let mut frame = InputFrame {
        owned_ports: 0x0f,
        ..InputFrame::default()
    };
    frame.pads[0] = pad;
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::TapeBoot;

    #[test]
    fn realizes_direction_button_recovery_and_absolute_spacing_phase() {
        let plan = RollOptionPlan {
            schema: ROLL_OPTION_SCHEMA_V1.into(),
            direction_degrees: 90,
            magnitude: 100,
            button_frame: 2,
            recovery_frames: 3,
            spacing: RollSpacing {
                period_ticks: 6,
                phase_tick: 1,
            },
            cancellation_conditions: Vec::new(),
        };
        let realization = plan.realize(5, None).unwrap();
        assert_eq!(realization.planned_ticks, 6);
        assert_eq!(realization.realized_ticks, 6);
        assert_eq!(realization.absolute_button_tick, 7);
        assert!(realization.button_emitted);
        assert_eq!(realization.end_reason, OptionEndReason::Completed);
        assert_eq!(realization.frames[0].pads[0].stick_x, 100);
        assert_eq!(realization.frames[1].pads[0].buttons, 0);
        assert_eq!(realization.frames[2].pads[0].buttons, BUTTON_A);
        assert_eq!(realization.frames[5].pads[0].buttons, 0);
        assert!(matches!(
            plan.realize(4, None),
            Err(RollOptionError::PhaseMismatch { .. })
        ));
    }

    #[test]
    fn cancellation_is_pre_input_typed_and_can_prevent_the_button() {
        let mut plan = RollOptionPlan::new(0, 127, 4);
        plan.button_frame = 2;
        plan.cancellation_conditions = vec![OptionCondition::TargetLost {
            target: "roll-control".into(),
        }];
        let realization = plan
            .realize(
                0,
                Some(RollCancellationHit {
                    tick: 2,
                    condition_index: 0,
                }),
            )
            .unwrap();
        assert_eq!(realization.realized_ticks, 2);
        assert!(!realization.button_emitted);
        assert!(
            realization
                .frames
                .iter()
                .all(|frame| frame.pads[0].buttons == 0)
        );
        assert_eq!(
            realization.end_reason,
            OptionEndReason::Cancelled { condition_index: 0 }
        );
        assert!(
            plan.realize(
                0,
                Some(RollCancellationHit {
                    tick: 0,
                    condition_index: 0,
                })
            )
            .is_err()
        );
    }

    #[test]
    fn capture_authenticates_the_exact_realized_roll_range() {
        let mut plan = RollOptionPlan::new(-90, 80, 2);
        plan.button_frame = 1;
        let realization = plan.realize(3, None).unwrap();
        let mut frames = vec![InputFrame::default(); 3];
        frames.extend(realization.frames.clone());
        frames.push(InputFrame::default());
        let tape = InputTape {
            boot: TapeBoot::Process,
            frames,
            ..InputTape::default()
        };
        let range = TapeRange {
            start_frame: 3,
            end_frame_exclusive: 7,
        };
        let execution = plan
            .capture_execution("roll-left".into(), &tape, range, None)
            .unwrap();
        assert_eq!(execution.option_type, OptionType::Roll);
        assert_eq!(execution.emitted_raw_actions, realization.frames);
        execution.validate_against_tape(&tape).unwrap();

        let mut tampered = tape;
        tampered.frames[4].pads[0].buttons = 0;
        assert!(
            plan.capture_execution("roll-left".into(), &tampered, range, None)
                .is_err()
        );
    }

    #[test]
    fn rejects_unbounded_invalid_and_ambiguous_roll_plans() {
        let mut plan = RollOptionPlan::new(181, 1, 0);
        assert_eq!(plan.validate(), Err(RollOptionError::InvalidDirection));
        plan.direction_degrees = 0;
        plan.magnitude = 0;
        assert_eq!(plan.validate(), Err(RollOptionError::InvalidMagnitude));
        plan.magnitude = 1;
        plan.button_frame = MAX_ROLL_TICKS;
        assert_eq!(plan.validate(), Err(RollOptionError::InvalidTiming));
        plan.button_frame = 0;
        plan.spacing = RollSpacing {
            period_ticks: 4,
            phase_tick: 4,
        };
        assert_eq!(plan.validate(), Err(RollOptionError::InvalidSpacing));
    }
}
