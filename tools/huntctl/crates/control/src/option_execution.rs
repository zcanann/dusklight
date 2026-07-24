//! Versioned semi-Markov option realization records.
//!
//! An option may be a convenient state-aware controller, but its execution
//! record is authoritative only when every emitted raw frame matches the
//! authenticated canonical tape range.

use crate::artifact::Digest;
use crate::tape::{InputFrame, InputTape, TapeError, WaitCondition};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const OPTION_EXECUTION_SCHEMA_V1: &str = "dusklight-option-execution/v1";
pub const MAX_OPTION_PARAMETERS: usize = 64;
pub const MAX_OPTION_CONDITIONS: usize = 64;
pub const MAX_OPTION_TICKS: u32 = 1_000_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionType {
    Move,
    Turn,
    Brake,
    Neutral,
    Align,
    MaintainHeading,
    MaintainDistance,
    Roll,
    JumpAttack,
    Attack,
    Shield,
    Target,
    Interact,
    ItemUse,
    Transform,
    Crawl,
    Climb,
    Swim,
    Mount,
    Boomerang,
    Clawshot,
    Spinner,
    Waypoint,
    Rail,
    Spline,
    Bezier,
    SeekActor,
    MaintainOffset,
    Custom(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum OptionParameter {
    Bool(bool),
    Signed(i64),
    Unsigned(u64),
    /// Exact IEEE-754 binary32 representation, including signed zero.
    F32Bits(u32),
    Vec3F32Bits([u32; 3]),
    Text(String),
    Digest(Digest),
}

impl OptionParameter {
    pub fn f32(value: f32) -> Result<Self, OptionExecutionError> {
        if value.is_finite() {
            Ok(Self::F32Bits(value.to_bits()))
        } else {
            Err(OptionExecutionError::NonFiniteParameter)
        }
    }

    pub fn vec3(value: [f32; 3]) -> Result<Self, OptionExecutionError> {
        if value.iter().all(|component| component.is_finite()) {
            Ok(Self::Vec3F32Bits(value.map(f32::to_bits)))
        } else {
            Err(OptionExecutionError::NonFiniteParameter)
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OptionCondition {
    DurationElapsed,
    Predicate {
        program_sha256: Digest,
        predicate_sha256: Digest,
    },
    Observation {
        observation_schema_sha256: Digest,
        expression_sha256: Digest,
    },
    TargetReached {
        target: String,
    },
    TargetLost {
        target: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionDuration {
    pub minimum_ticks: u32,
    pub maximum_ticks: u32,
    pub realized_ticks: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OptionEndReason {
    Completed,
    Terminated,
    Cancelled { condition_index: u32 },
    MaximumDuration,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TapeRange {
    pub start_frame: u64,
    pub end_frame_exclusive: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionExecution {
    pub schema: String,
    pub option_id: String,
    #[serde(rename = "type")]
    pub option_type: OptionType,
    pub parameters: BTreeMap<String, OptionParameter>,
    pub duration: OptionDuration,
    pub termination_condition: OptionCondition,
    pub cancellation_conditions: Vec<OptionCondition>,
    pub end_reason: OptionEndReason,
    pub emitted_raw_actions: Vec<InputFrame>,
    pub realized_tape_range: TapeRange,
    pub tape_sha256: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OptionExecutionError {
    Tape(TapeError),
    UnsupportedSchema,
    InvalidOptionId,
    InvalidOptionType,
    TooManyParameters,
    InvalidParameterName(String),
    InvalidParameterValue(String),
    NonFiniteParameter,
    InvalidDuration,
    TooManyConditions,
    InvalidCondition,
    InvalidEndReason,
    InvalidRange,
    RangeOutOfBounds,
    ReactiveFrame(u64),
    FrameMismatch,
    TapeDigestMismatch,
}

impl fmt::Display for OptionExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tape(source) => write!(formatter, "option tape is invalid: {source}"),
            Self::UnsupportedSchema => formatter.write_str("unsupported option execution schema"),
            Self::InvalidOptionId => formatter.write_str("option ID is invalid"),
            Self::InvalidOptionType => formatter.write_str("custom option type is invalid"),
            Self::TooManyParameters => formatter.write_str("option has too many parameters"),
            Self::InvalidParameterName(name) => {
                write!(formatter, "option parameter name {name:?} is invalid")
            }
            Self::InvalidParameterValue(name) => {
                write!(formatter, "option parameter {name:?} has an invalid value")
            }
            Self::NonFiniteParameter => formatter.write_str("option parameter is not finite"),
            Self::InvalidDuration => formatter.write_str("option duration is invalid"),
            Self::TooManyConditions => formatter.write_str("option has too many conditions"),
            Self::InvalidCondition => formatter.write_str("option condition is invalid"),
            Self::InvalidEndReason => formatter.write_str("option end reason is inconsistent"),
            Self::InvalidRange => formatter.write_str("realized tape range is invalid"),
            Self::RangeOutOfBounds => formatter.write_str("realized tape range exceeds the tape"),
            Self::ReactiveFrame(frame) => {
                write!(formatter, "emitted raw action {frame} is reactive")
            }
            Self::FrameMismatch => {
                formatter.write_str("emitted raw actions do not match the realized tape range")
            }
            Self::TapeDigestMismatch => {
                formatter.write_str("option tape digest does not match the canonical tape")
            }
        }
    }
}

impl Error for OptionExecutionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Tape(source) => Some(source),
            _ => None,
        }
    }
}

impl From<TapeError> for OptionExecutionError {
    fn from(value: TapeError) -> Self {
        Self::Tape(value)
    }
}

impl OptionExecution {
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        option_id: String,
        option_type: OptionType,
        parameters: BTreeMap<String, OptionParameter>,
        minimum_ticks: u32,
        maximum_ticks: u32,
        termination_condition: OptionCondition,
        cancellation_conditions: Vec<OptionCondition>,
        end_reason: OptionEndReason,
        tape: &InputTape,
        realized_tape_range: TapeRange,
    ) -> Result<Self, OptionExecutionError> {
        tape.validate()?;
        let (start, end) = checked_range(tape, realized_tape_range)?;
        let emitted_raw_actions = tape.frames[start..end].to_vec();
        let realized_ticks = u32::try_from(emitted_raw_actions.len())
            .map_err(|_| OptionExecutionError::InvalidDuration)?;
        let execution = Self {
            schema: OPTION_EXECUTION_SCHEMA_V1.into(),
            option_id,
            option_type,
            parameters,
            duration: OptionDuration {
                minimum_ticks,
                maximum_ticks,
                realized_ticks,
            },
            termination_condition,
            cancellation_conditions,
            end_reason,
            emitted_raw_actions,
            realized_tape_range,
            tape_sha256: canonical_tape_digest(tape)?,
        };
        execution.validate_against_tape(tape)?;
        Ok(execution)
    }

    pub fn validate(&self) -> Result<(), OptionExecutionError> {
        if self.schema != OPTION_EXECUTION_SCHEMA_V1 {
            return Err(OptionExecutionError::UnsupportedSchema);
        }
        if !valid_name(&self.option_id, 96) {
            return Err(OptionExecutionError::InvalidOptionId);
        }
        if let OptionType::Custom(name) = &self.option_type
            && !valid_name(name, 96)
        {
            return Err(OptionExecutionError::InvalidOptionType);
        }
        if self.parameters.len() > MAX_OPTION_PARAMETERS {
            return Err(OptionExecutionError::TooManyParameters);
        }
        for (name, value) in &self.parameters {
            if !valid_name(name, 64) {
                return Err(OptionExecutionError::InvalidParameterName(name.clone()));
            }
            match value {
                OptionParameter::F32Bits(bits) if !f32::from_bits(*bits).is_finite() => {
                    return Err(OptionExecutionError::InvalidParameterValue(name.clone()));
                }
                OptionParameter::Vec3F32Bits(bits)
                    if !bits
                        .iter()
                        .all(|component| f32::from_bits(*component).is_finite()) =>
                {
                    return Err(OptionExecutionError::InvalidParameterValue(name.clone()));
                }
                OptionParameter::Text(value) if !valid_text(value, 1024) => {
                    return Err(OptionExecutionError::InvalidParameterValue(name.clone()));
                }
                OptionParameter::Digest(value) if *value == Digest::ZERO => {
                    return Err(OptionExecutionError::InvalidParameterValue(name.clone()));
                }
                _ => {}
            }
        }
        if self.duration.minimum_ticks == 0
            || self.duration.minimum_ticks > self.duration.maximum_ticks
            || self.duration.maximum_ticks > MAX_OPTION_TICKS
            || self.duration.realized_ticks == 0
            || self.duration.realized_ticks > self.duration.maximum_ticks
            || self.duration.realized_ticks as usize != self.emitted_raw_actions.len()
        {
            return Err(OptionExecutionError::InvalidDuration);
        }
        if self.cancellation_conditions.len() > MAX_OPTION_CONDITIONS {
            return Err(OptionExecutionError::TooManyConditions);
        }
        validate_condition(&self.termination_condition)?;
        for condition in &self.cancellation_conditions {
            validate_condition(condition)?;
        }
        match self.end_reason {
            OptionEndReason::Completed | OptionEndReason::Terminated
                if self.duration.realized_ticks < self.duration.minimum_ticks =>
            {
                return Err(OptionExecutionError::InvalidEndReason);
            }
            OptionEndReason::Cancelled { condition_index }
                if condition_index as usize >= self.cancellation_conditions.len() =>
            {
                return Err(OptionExecutionError::InvalidEndReason);
            }
            OptionEndReason::MaximumDuration
                if self.duration.realized_ticks != self.duration.maximum_ticks =>
            {
                return Err(OptionExecutionError::InvalidEndReason);
            }
            _ => {}
        }
        if self.realized_tape_range.end_frame_exclusive <= self.realized_tape_range.start_frame
            || self.realized_tape_range.end_frame_exclusive - self.realized_tape_range.start_frame
                != u64::from(self.duration.realized_ticks)
        {
            return Err(OptionExecutionError::InvalidRange);
        }
        for (index, frame) in self.emitted_raw_actions.iter().enumerate() {
            if frame.wait_condition != WaitCondition::None || frame.wait_timeout_ticks != 0 {
                return Err(OptionExecutionError::ReactiveFrame(index as u64));
            }
        }
        if self.tape_sha256 == Digest::ZERO {
            return Err(OptionExecutionError::TapeDigestMismatch);
        }
        Ok(())
    }

    pub fn validate_against_tape(&self, tape: &InputTape) -> Result<(), OptionExecutionError> {
        self.validate()?;
        tape.validate()?;
        if canonical_tape_digest(tape)? != self.tape_sha256 {
            return Err(OptionExecutionError::TapeDigestMismatch);
        }
        let (start, end) = checked_range(tape, self.realized_tape_range)?;
        if tape.frames[start..end] != self.emitted_raw_actions {
            return Err(OptionExecutionError::FrameMismatch);
        }
        Ok(())
    }
}

fn canonical_tape_digest(tape: &InputTape) -> Result<Digest, OptionExecutionError> {
    Ok(Digest(Sha256::digest(tape.encode()?).into()))
}

fn checked_range(
    tape: &InputTape,
    range: TapeRange,
) -> Result<(usize, usize), OptionExecutionError> {
    if range.end_frame_exclusive <= range.start_frame {
        return Err(OptionExecutionError::InvalidRange);
    }
    let start =
        usize::try_from(range.start_frame).map_err(|_| OptionExecutionError::RangeOutOfBounds)?;
    let end = usize::try_from(range.end_frame_exclusive)
        .map_err(|_| OptionExecutionError::RangeOutOfBounds)?;
    if end > tape.frames.len() {
        return Err(OptionExecutionError::RangeOutOfBounds);
    }
    Ok((start, end))
}

pub fn validate_condition(condition: &OptionCondition) -> Result<(), OptionExecutionError> {
    match condition {
        OptionCondition::DurationElapsed => Ok(()),
        OptionCondition::Predicate {
            program_sha256,
            predicate_sha256,
        } if *program_sha256 != Digest::ZERO && *predicate_sha256 != Digest::ZERO => Ok(()),
        OptionCondition::Observation {
            observation_schema_sha256,
            expression_sha256,
        } if *observation_schema_sha256 != Digest::ZERO && *expression_sha256 != Digest::ZERO => {
            Ok(())
        }
        OptionCondition::TargetReached { target } | OptionCondition::TargetLost { target }
            if valid_name(target, 128) =>
        {
            Ok(())
        }
        _ => Err(OptionExecutionError::InvalidCondition),
    }
}

fn valid_name(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
        })
}

fn valid_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.chars().all(|character| !character.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::{RawPadState, TapeBoot};

    fn tape() -> InputTape {
        InputTape {
            boot: TapeBoot::Stage {
                stage: "F_SP103".into(),
                room: 1,
                point: 1,
                layer: 3,
                save_slot: None,
                fixture: None,
            },
            frames: (0..6)
                .map(|index| InputFrame {
                    owned_ports: 1,
                    pads: [
                        RawPadState {
                            stick_x: index,
                            ..RawPadState::default()
                        },
                        RawPadState::default(),
                        RawPadState::default(),
                        RawPadState::default(),
                    ],
                    ..InputFrame::default()
                })
                .collect(),
            ..InputTape::default()
        }
    }

    #[test]
    fn captures_round_trips_and_authenticates_an_exact_realized_range() {
        let tape = tape();
        let execution = OptionExecution::capture(
            "move-to-gate".into(),
            OptionType::Move,
            BTreeMap::from([
                (
                    "target".into(),
                    OptionParameter::vec3([1.0, -2.5, 3.0]).unwrap(),
                ),
                ("magnitude".into(), OptionParameter::Unsigned(127)),
            ]),
            2,
            10,
            OptionCondition::Predicate {
                program_sha256: Digest([1; 32]),
                predicate_sha256: Digest([2; 32]),
            },
            vec![OptionCondition::TargetLost {
                target: "gate.actor".into(),
            }],
            OptionEndReason::Terminated,
            &tape,
            TapeRange {
                start_frame: 1,
                end_frame_exclusive: 4,
            },
        )
        .unwrap();
        assert_eq!(execution.duration.realized_ticks, 3);
        assert_eq!(execution.emitted_raw_actions, tape.frames[1..4]);
        let json = serde_json::to_vec_pretty(&execution).unwrap();
        let decoded: OptionExecution = serde_json::from_slice(&json).unwrap();
        assert_eq!(decoded, execution);
        decoded.validate_against_tape(&tape).unwrap();
    }

    #[test]
    fn permits_early_typed_cancellation_but_rejects_wrong_index_and_frames() {
        let tape = tape();
        let mut execution = OptionExecution::capture(
            "seek-exit".into(),
            OptionType::SeekActor,
            BTreeMap::new(),
            4,
            20,
            OptionCondition::TargetReached {
                target: "nearest-exit".into(),
            },
            vec![OptionCondition::TargetLost {
                target: "nearest-exit".into(),
            }],
            OptionEndReason::Cancelled { condition_index: 0 },
            &tape,
            TapeRange {
                start_frame: 0,
                end_frame_exclusive: 2,
            },
        )
        .unwrap();
        execution.end_reason = OptionEndReason::Cancelled { condition_index: 1 };
        assert_eq!(
            execution.validate(),
            Err(OptionExecutionError::InvalidEndReason)
        );
        execution.end_reason = OptionEndReason::Cancelled { condition_index: 0 };
        execution.emitted_raw_actions[0].pads[0].buttons = 0x0100;
        assert_eq!(
            execution.validate_against_tape(&tape),
            Err(OptionExecutionError::FrameMismatch)
        );
    }

    #[test]
    fn rejects_range_digest_duration_and_reactive_mismatches() {
        let tape = tape();
        let execution = OptionExecution::capture(
            "neutral".into(),
            OptionType::Neutral,
            BTreeMap::new(),
            1,
            2,
            OptionCondition::DurationElapsed,
            Vec::new(),
            OptionEndReason::MaximumDuration,
            &tape,
            TapeRange {
                start_frame: 2,
                end_frame_exclusive: 4,
            },
        )
        .unwrap();
        let mut different = tape.clone();
        different.frames[5].pads[0].buttons = 0x0200;
        assert_eq!(
            execution.validate_against_tape(&different),
            Err(OptionExecutionError::TapeDigestMismatch)
        );
        let mut reactive = execution;
        reactive.emitted_raw_actions[0].wait_condition = WaitCondition::NameEntryActive;
        reactive.emitted_raw_actions[0].wait_timeout_ticks = 3;
        assert_eq!(
            reactive.validate(),
            Err(OptionExecutionError::ReactiveFrame(0))
        );
    }
}
