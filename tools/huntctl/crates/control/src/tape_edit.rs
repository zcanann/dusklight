//! Deterministic editing and comparison for absolute input tapes.

use crate::tape::{InputFrame, InputTape, PORT_COUNT, TapeBoot, TapeError, WaitCondition};
use serde::Serialize;
use serde_json::{Value, json};
use std::error::Error;
use std::fmt;

pub const CANONICAL_TICK_RATE_NUMERATOR: u32 = 30;
pub const CANONICAL_TICK_RATE_DENOMINATOR: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditError {
    InvalidBase(TapeError),
    InvalidOverlay(TapeError),
    ReactiveBase {
        frame: usize,
    },
    ReactiveOverlay {
        frame: usize,
    },
    TickRateMismatch,
    BootMismatch {
        base: Box<TapeBoot>,
        overlay: Box<TapeBoot>,
    },
    RangeOverflow,
    RangeOutOfBounds {
        end: usize,
        base_frames: usize,
    },
    TooManyFrames,
}

impl fmt::Display for EditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBase(source) => write!(formatter, "base tape is invalid: {source}"),
            Self::InvalidOverlay(source) => write!(formatter, "overlay tape is invalid: {source}"),
            Self::ReactiveBase { frame } => {
                write!(
                    formatter,
                    "base frame {frame} has a reactive wait condition"
                )
            }
            Self::ReactiveOverlay { frame } => {
                write!(
                    formatter,
                    "overlay frame {frame} has a reactive wait condition"
                )
            }
            Self::TickRateMismatch => {
                formatter.write_str("base and overlay tape tick rates differ")
            }
            Self::BootMismatch { base, overlay } => write!(
                formatter,
                "overlay boot origin {overlay:?} conflicts with base origin {base:?}"
            ),
            Self::RangeOverflow => formatter.write_str("layer range overflows"),
            Self::RangeOutOfBounds { end, base_frames } => write!(
                formatter,
                "layer ends at frame {end}, beyond the base tape's {base_frames} frames"
            ),
            Self::TooManyFrames => formatter.write_str("resampled tape is too large"),
        }
    }
}

impl Error for EditError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidBase(source) | Self::InvalidOverlay(source) => Some(source),
            _ => None,
        }
    }
}

/// Replaces complete native PAD states for the ports owned by `overlay`.
///
/// The base supplies boot identity, duration, wait metadata, and every port not
/// owned by the overlay. Both tapes must be absolute because a reactive frame
/// has no stable tick at which an overlay could be applied.
pub fn layer_at(
    mut base: InputTape,
    overlay: InputTape,
    start: usize,
) -> Result<InputTape, EditError> {
    validate_absolute(&base, false)?;
    validate_absolute(&overlay, true)?;
    if !rates_equal(
        base.tick_rate_numerator,
        base.tick_rate_denominator,
        overlay.tick_rate_numerator,
        overlay.tick_rate_denominator,
    ) {
        return Err(EditError::TickRateMismatch);
    }
    if !matches!(&overlay.boot, TapeBoot::Process) && overlay.boot != base.boot {
        return Err(EditError::BootMismatch {
            base: Box::new(base.boot),
            overlay: Box::new(overlay.boot),
        });
    }
    let end = start
        .checked_add(overlay.frames.len())
        .ok_or(EditError::RangeOverflow)?;
    if end > base.frames.len() {
        return Err(EditError::RangeOutOfBounds {
            end,
            base_frames: base.frames.len(),
        });
    }
    for (target, source) in base.frames[start..end].iter_mut().zip(overlay.frames) {
        for port in 0..PORT_COUNT {
            let mask = 1_u8 << port;
            if source.owned_ports & mask != 0 {
                target.pads[port] = source.pads[port];
                target.owned_ports |= mask;
            }
        }
    }
    Ok(base)
}

/// Converts an absolute authoring tape to canonical 30 Hz with a deterministic
/// zero-order hold sampled at each output tick's start time.
pub fn resample_to_canonical(tape: InputTape) -> Result<InputTape, EditError> {
    validate_absolute(&tape, false)?;
    if rates_equal(
        tape.tick_rate_numerator,
        tape.tick_rate_denominator,
        CANONICAL_TICK_RATE_NUMERATOR,
        CANONICAL_TICK_RATE_DENOMINATOR,
    ) {
        return Ok(InputTape {
            tick_rate_numerator: CANONICAL_TICK_RATE_NUMERATOR,
            tick_rate_denominator: CANONICAL_TICK_RATE_DENOMINATOR,
            ..tape
        });
    }

    let source_count = tape.frames.len() as u128;
    let output_count = source_count
        .checked_mul(u128::from(tape.tick_rate_denominator))
        .and_then(|value| value.checked_mul(u128::from(CANONICAL_TICK_RATE_NUMERATOR)))
        .and_then(|value| {
            let divisor = u128::from(tape.tick_rate_numerator);
            value.checked_add(divisor - 1).map(|value| value / divisor)
        })
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(EditError::TooManyFrames)?;
    let source_step_numerator = u128::from(tape.tick_rate_numerator);
    let source_step_denominator =
        u128::from(tape.tick_rate_denominator) * u128::from(CANONICAL_TICK_RATE_NUMERATOR);
    let mut frames = Vec::with_capacity(output_count);
    for output_index in 0..output_count {
        let source_index =
            ((output_index as u128) * source_step_numerator / source_step_denominator) as usize;
        frames.push(tape.frames[source_index.min(tape.frames.len() - 1)].clone());
    }
    Ok(InputTape {
        boot: tape.boot,
        tick_rate_numerator: CANONICAL_TICK_RATE_NUMERATOR,
        tick_rate_denominator: CANONICAL_TICK_RATE_DENOMINATOR,
        frames,
    })
}

fn validate_absolute(tape: &InputTape, overlay: bool) -> Result<(), EditError> {
    tape.validate().map_err(|source| {
        if overlay {
            EditError::InvalidOverlay(source)
        } else {
            EditError::InvalidBase(source)
        }
    })?;
    if let Some((frame, _)) = tape
        .frames
        .iter()
        .enumerate()
        .find(|(_, frame)| frame.wait_condition != WaitCondition::None)
    {
        return Err(if overlay {
            EditError::ReactiveOverlay { frame }
        } else {
            EditError::ReactiveBase { frame }
        });
    }
    Ok(())
}

fn rates_equal(
    left_numerator: u32,
    left_denominator: u32,
    right_numerator: u32,
    right_denominator: u32,
) -> bool {
    u128::from(left_numerator) * u128::from(right_denominator)
        == u128::from(right_numerator) * u128::from(left_denominator)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TapeDiff {
    pub schema: &'static str,
    pub identical: bool,
    pub metadata: Vec<FieldDifference>,
    pub left_frame_count: usize,
    pub right_frame_count: usize,
    pub differing_frame_count: usize,
    pub frames: Vec<FrameDifference>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FrameDifference {
    pub frame: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_only: Option<InputFrame>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_only: Option<InputFrame>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldDifference>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FieldDifference {
    pub field: String,
    pub left: Value,
    pub right: Value,
}

/// Produces an exhaustive typed field diff, including all four native PADs.
pub fn diff(left: &InputTape, right: &InputTape) -> TapeDiff {
    let mut metadata = Vec::new();
    push_difference(&mut metadata, "boot", json!(&left.boot), json!(&right.boot));
    push_difference(
        &mut metadata,
        "tick_rate.numerator",
        json!(left.tick_rate_numerator),
        json!(right.tick_rate_numerator),
    );
    push_difference(
        &mut metadata,
        "tick_rate.denominator",
        json!(left.tick_rate_denominator),
        json!(right.tick_rate_denominator),
    );

    let mut frames = Vec::new();
    for frame in 0..left.frames.len().max(right.frames.len()) {
        match (left.frames.get(frame), right.frames.get(frame)) {
            (Some(left_frame), Some(right_frame)) if left_frame != right_frame => {
                frames.push(FrameDifference {
                    frame,
                    left_only: None,
                    right_only: None,
                    fields: frame_differences(left_frame, right_frame),
                })
            }
            (Some(left_frame), None) => frames.push(FrameDifference {
                frame,
                left_only: Some(left_frame.clone()),
                right_only: None,
                fields: Vec::new(),
            }),
            (None, Some(right_frame)) => frames.push(FrameDifference {
                frame,
                left_only: None,
                right_only: Some(right_frame.clone()),
                fields: Vec::new(),
            }),
            _ => {}
        }
    }
    TapeDiff {
        schema: "huntctl-tape-diff/v1",
        identical: metadata.is_empty() && frames.is_empty(),
        metadata,
        left_frame_count: left.frames.len(),
        right_frame_count: right.frames.len(),
        differing_frame_count: frames.len(),
        frames,
    }
}

fn frame_differences(left: &InputFrame, right: &InputFrame) -> Vec<FieldDifference> {
    let mut fields = Vec::new();
    push_difference(
        &mut fields,
        "owned_ports",
        json!(left.owned_ports),
        json!(right.owned_ports),
    );
    push_difference(
        &mut fields,
        "wait_condition",
        json!(left.wait_condition.as_str()),
        json!(right.wait_condition.as_str()),
    );
    push_difference(
        &mut fields,
        "wait_timeout_ticks",
        json!(left.wait_timeout_ticks),
        json!(right.wait_timeout_ticks),
    );
    for port in 0..PORT_COUNT {
        let left = left.pads[port];
        let right = right.pads[port];
        let prefix = format!("p{port}");
        push_difference(
            &mut fields,
            format!("{prefix}.buttons"),
            json!(left.buttons),
            json!(right.buttons),
        );
        push_difference(
            &mut fields,
            format!("{prefix}.stick_x"),
            json!(left.stick_x),
            json!(right.stick_x),
        );
        push_difference(
            &mut fields,
            format!("{prefix}.stick_y"),
            json!(left.stick_y),
            json!(right.stick_y),
        );
        push_difference(
            &mut fields,
            format!("{prefix}.substick_x"),
            json!(left.substick_x),
            json!(right.substick_x),
        );
        push_difference(
            &mut fields,
            format!("{prefix}.substick_y"),
            json!(left.substick_y),
            json!(right.substick_y),
        );
        push_difference(
            &mut fields,
            format!("{prefix}.trigger_left"),
            json!(left.trigger_left),
            json!(right.trigger_left),
        );
        push_difference(
            &mut fields,
            format!("{prefix}.trigger_right"),
            json!(left.trigger_right),
            json!(right.trigger_right),
        );
        push_difference(
            &mut fields,
            format!("{prefix}.analog_a"),
            json!(left.analog_a),
            json!(right.analog_a),
        );
        push_difference(
            &mut fields,
            format!("{prefix}.analog_b"),
            json!(left.analog_b),
            json!(right.analog_b),
        );
        push_difference(
            &mut fields,
            format!("{prefix}.connected"),
            json!(left.connected),
            json!(right.connected),
        );
        push_difference(
            &mut fields,
            format!("{prefix}.error"),
            json!(left.error),
            json!(right.error),
        );
    }
    fields
}

fn push_difference(
    fields: &mut Vec<FieldDifference>,
    field: impl Into<String>,
    left: Value,
    right: Value,
) {
    if left != right {
        fields.push(FieldDifference {
            field: field.into(),
            left,
            right,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::RawPadState;

    fn frame(value: i8, port: usize) -> InputFrame {
        let mut frame = InputFrame {
            owned_ports: 1 << port,
            ..InputFrame::default()
        };
        frame.pads[port] = RawPadState {
            stick_x: value,
            ..RawPadState::default()
        };
        frame
    }

    #[test]
    fn layers_owned_ports_without_touching_other_native_state() {
        let mut base = InputTape {
            frames: vec![frame(1, 0), frame(2, 0), frame(3, 0)],
            ..InputTape::default()
        };
        base.frames[1].pads[3].error = -1;
        let overlay = InputTape {
            frames: vec![frame(40, 2), frame(50, 2)],
            ..InputTape::default()
        };
        let layered = layer_at(base.clone(), overlay, 1).unwrap();
        assert_eq!(layered.frames[0], base.frames[0]);
        assert_eq!(layered.frames[1].pads[0], base.frames[1].pads[0]);
        assert_eq!(layered.frames[1].pads[2].stick_x, 40);
        assert_eq!(layered.frames[2].pads[2].stick_x, 50);
        assert_eq!(layered.frames[1].pads[3].error, -1);
        assert_eq!(layered.frames[1].owned_ports, 0b0101);
    }

    #[test]
    fn resamples_authoring_rates_to_exact_30_hz() {
        let sixty_hz = InputTape {
            tick_rate_numerator: 60,
            frames: (0..6).map(|value| frame(value, 0)).collect(),
            ..InputTape::default()
        };
        let downsampled = resample_to_canonical(sixty_hz).unwrap();
        assert_eq!(downsampled.tick_rate_numerator, 30);
        assert_eq!(downsampled.tick_rate_denominator, 1);
        assert_eq!(
            downsampled
                .frames
                .iter()
                .map(|frame| frame.pads[0].stick_x)
                .collect::<Vec<_>>(),
            [0, 2, 4]
        );

        let fifteen_hz = InputTape {
            tick_rate_numerator: 15,
            frames: vec![frame(7, 0), frame(9, 0)],
            ..InputTape::default()
        };
        let upsampled = resample_to_canonical(fifteen_hz).unwrap();
        assert_eq!(
            upsampled
                .frames
                .iter()
                .map(|frame| frame.pads[0].stick_x)
                .collect::<Vec<_>>(),
            [7, 7, 9, 9]
        );
    }

    #[test]
    fn reports_exact_metadata_and_pad_field_differences() {
        let left = InputTape {
            frames: vec![frame(1, 0)],
            ..InputTape::default()
        };
        let mut right = left.clone();
        right.tick_rate_numerator = 60;
        right.tick_rate_denominator = 2;
        right.frames[0].pads[0].stick_x = -8;
        right.frames[0].pads[3].connected = false;
        right.frames.push(frame(2, 1));
        let report = diff(&left, &right);
        assert!(!report.identical);
        assert_eq!(report.metadata.len(), 2);
        assert_eq!(report.differing_frame_count, 2);
        assert_eq!(report.frames[0].fields[0].field, "p0.stick_x");
        assert_eq!(report.frames[0].fields[1].field, "p3.connected");
        assert!(report.frames[1].right_only.is_some());
    }
}
