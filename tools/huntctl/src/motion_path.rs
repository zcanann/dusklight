//! Exact-duration static stick paths with rational sampling phase.

use crate::option_execution::{
    MAX_OPTION_CONDITIONS, OptionCondition, OptionEndReason, OptionExecution, OptionExecutionError,
    OptionParameter, OptionType, TapeRange, validate_condition,
};
use crate::tape::{InputFrame, InputTape, RawPadState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const MOTION_PATH_SCHEMA_V1: &str = "dusklight-motion-path/v1";
pub const MAX_PATH_TICKS: u32 = 10_000;
pub const MAX_PATH_POINTS: usize = 64;
pub const MAX_PHASE_DENOMINATOR: u32 = 10_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StickPoint {
    pub x: i16,
    pub y: i16,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SamplePhase {
    /// Fractional phase within each output tick. `denominator/denominator`
    /// samples the end of the interval and is intentionally distinct from zero.
    pub numerator: u32,
    pub denominator: u32,
}

impl Default for SamplePhase {
    fn default() -> Self {
        Self {
            numerator: 0,
            denominator: 1,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StickPath {
    /// Piecewise-constant samples distributed uniformly across the duration.
    Waypoint { points: Vec<StickPoint> },
    /// Piecewise-linear interpolation through every point.
    Rail { points: Vec<StickPoint> },
    /// Uniform Catmull-Rom interpolation with duplicated endpoint controls.
    Spline { points: Vec<StickPoint> },
    /// One cubic Bézier segment.
    Bezier { control: [StickPoint; 4] },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MotionPathPlan {
    pub schema: String,
    pub path: StickPath,
    pub duration_ticks: u32,
    pub sample_phase: SamplePhase,
    pub cancellation_conditions: Vec<OptionCondition>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PathCancellationHit {
    pub tick: u32,
    pub condition_index: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MotionPathRealization {
    pub planned_ticks: u32,
    pub realized_ticks: u32,
    pub end_reason: OptionEndReason,
    pub frames: Vec<InputFrame>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MotionPathError {
    UnsupportedSchema,
    InvalidDuration,
    InvalidPhase,
    InvalidPointCount,
    InvalidPoint,
    InvalidCancellation,
    ArithmeticOverflow,
    FrameMismatch,
    RangeOverflow,
    Execution(OptionExecutionError),
}

impl fmt::Display for MotionPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema => formatter.write_str("unsupported motion path schema"),
            Self::InvalidDuration => formatter.write_str("motion path duration is invalid"),
            Self::InvalidPhase => formatter.write_str("motion path sample phase is invalid"),
            Self::InvalidPointCount => formatter.write_str("motion path point count is invalid"),
            Self::InvalidPoint => formatter.write_str("motion path point exceeds raw stick range"),
            Self::InvalidCancellation => formatter.write_str("motion path cancellation is invalid"),
            Self::ArithmeticOverflow => formatter.write_str("motion path arithmetic overflowed"),
            Self::FrameMismatch => {
                formatter.write_str("motion path realization does not match the tape range")
            }
            Self::RangeOverflow => formatter.write_str("motion path tape range overflows"),
            Self::Execution(source) => {
                write!(formatter, "motion path execution is invalid: {source}")
            }
        }
    }
}

impl Error for MotionPathError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Execution(source) => Some(source),
            _ => None,
        }
    }
}

impl From<OptionExecutionError> for MotionPathError {
    fn from(value: OptionExecutionError) -> Self {
        Self::Execution(value)
    }
}

impl MotionPathPlan {
    pub fn new(path: StickPath, duration_ticks: u32) -> Self {
        Self {
            schema: MOTION_PATH_SCHEMA_V1.into(),
            path,
            duration_ticks,
            sample_phase: SamplePhase::default(),
            cancellation_conditions: Vec::new(),
        }
    }

    pub fn option_type(&self) -> OptionType {
        match self.path {
            StickPath::Waypoint { .. } => OptionType::Waypoint,
            StickPath::Rail { .. } => OptionType::Rail,
            StickPath::Spline { .. } => OptionType::Spline,
            StickPath::Bezier { .. } => OptionType::Bezier,
        }
    }

    pub fn validate(&self) -> Result<(), MotionPathError> {
        if self.schema != MOTION_PATH_SCHEMA_V1 {
            return Err(MotionPathError::UnsupportedSchema);
        }
        if !(1..=MAX_PATH_TICKS).contains(&self.duration_ticks) {
            return Err(MotionPathError::InvalidDuration);
        }
        if self.sample_phase.denominator == 0
            || self.sample_phase.denominator > MAX_PHASE_DENOMINATOR
            || self.sample_phase.numerator > self.sample_phase.denominator
        {
            return Err(MotionPathError::InvalidPhase);
        }
        let points: &[StickPoint] = match &self.path {
            StickPath::Waypoint { points }
            | StickPath::Rail { points }
            | StickPath::Spline { points } => points,
            StickPath::Bezier { control } => control,
        };
        let minimum = match self.path {
            StickPath::Waypoint { .. } => 1,
            StickPath::Rail { .. } | StickPath::Spline { .. } => 2,
            StickPath::Bezier { .. } => 4,
        };
        if points.len() < minimum || points.len() > MAX_PATH_POINTS {
            return Err(MotionPathError::InvalidPointCount);
        }
        if points
            .iter()
            .any(|point| !(-128..=127).contains(&point.x) || !(-128..=127).contains(&point.y))
        {
            return Err(MotionPathError::InvalidPoint);
        }
        if self.cancellation_conditions.len() > MAX_OPTION_CONDITIONS {
            return Err(MotionPathError::InvalidCancellation);
        }
        for condition in &self.cancellation_conditions {
            validate_condition(condition)?;
        }
        Ok(())
    }

    pub fn realize(
        &self,
        cancellation: Option<PathCancellationHit>,
    ) -> Result<MotionPathRealization, MotionPathError> {
        self.validate()?;
        let (realized_ticks, end_reason) = match cancellation {
            Some(hit)
                if hit.tick > 0
                    && hit.tick < self.duration_ticks
                    && (hit.condition_index as usize) < self.cancellation_conditions.len() =>
            {
                (
                    hit.tick,
                    OptionEndReason::Cancelled {
                        condition_index: hit.condition_index,
                    },
                )
            }
            Some(_) => return Err(MotionPathError::InvalidCancellation),
            None => (self.duration_ticks, OptionEndReason::Completed),
        };
        let mut frames = Vec::with_capacity(realized_ticks as usize);
        for tick in 0..realized_ticks {
            let point = self.sample(tick)?;
            frames.push(owned_frame(RawPadState {
                stick_x: point.x as i8,
                stick_y: point.y as i8,
                ..RawPadState::default()
            }));
        }
        Ok(MotionPathRealization {
            planned_ticks: self.duration_ticks,
            realized_ticks,
            end_reason,
            frames,
        })
    }

    pub fn capture_execution(
        &self,
        option_id: String,
        tape: &InputTape,
        range: TapeRange,
        cancellation: Option<PathCancellationHit>,
    ) -> Result<OptionExecution, MotionPathError> {
        let realization = self.realize(cancellation)?;
        let expected_end = range
            .start_frame
            .checked_add(u64::from(realization.realized_ticks))
            .ok_or(MotionPathError::RangeOverflow)?;
        let start =
            usize::try_from(range.start_frame).map_err(|_| MotionPathError::RangeOverflow)?;
        let end = usize::try_from(expected_end).map_err(|_| MotionPathError::RangeOverflow)?;
        if range.end_frame_exclusive != expected_end
            || end > tape.frames.len()
            || tape.frames[start..end] != realization.frames
        {
            return Err(MotionPathError::FrameMismatch);
        }
        Ok(OptionExecution::capture(
            option_id,
            self.option_type(),
            self.parameters(),
            1,
            self.duration_ticks,
            OptionCondition::DurationElapsed,
            self.cancellation_conditions.clone(),
            realization.end_reason,
            tape,
            range,
        )?)
    }

    fn sample(&self, tick: u32) -> Result<StickPoint, MotionPathError> {
        let global_denominator = u64::from(self.duration_ticks)
            .checked_mul(u64::from(self.sample_phase.denominator))
            .ok_or(MotionPathError::ArithmeticOverflow)?;
        let global_numerator = u64::from(tick)
            .checked_mul(u64::from(self.sample_phase.denominator))
            .and_then(|value| value.checked_add(u64::from(self.sample_phase.numerator)))
            .ok_or(MotionPathError::ArithmeticOverflow)?;
        match &self.path {
            StickPath::Waypoint { points } => {
                let index = uniform_index(global_numerator, global_denominator, points.len())?;
                Ok(points[index])
            }
            StickPath::Rail { points } => {
                let (segment, numerator, denominator) =
                    segment_phase(global_numerator, global_denominator, points.len() - 1)?;
                Ok(StickPoint {
                    x: linear(
                        points[segment].x,
                        points[segment + 1].x,
                        numerator,
                        denominator,
                    )?,
                    y: linear(
                        points[segment].y,
                        points[segment + 1].y,
                        numerator,
                        denominator,
                    )?,
                })
            }
            StickPath::Spline { points } => {
                let (segment, numerator, denominator) =
                    segment_phase(global_numerator, global_denominator, points.len() - 1)?;
                let p0 = points[segment.saturating_sub(1)];
                let p1 = points[segment];
                let p2 = points[segment + 1];
                let p3 = points[(segment + 2).min(points.len() - 1)];
                Ok(StickPoint {
                    x: catmull_rom(p0.x, p1.x, p2.x, p3.x, numerator, denominator)?,
                    y: catmull_rom(p0.y, p1.y, p2.y, p3.y, numerator, denominator)?,
                })
            }
            StickPath::Bezier { control } => Ok(StickPoint {
                x: bezier_axis(
                    control.map(|point| point.x),
                    global_numerator,
                    global_denominator,
                )?,
                y: bezier_axis(
                    control.map(|point| point.y),
                    global_numerator,
                    global_denominator,
                )?,
            }),
        }
    }

    fn parameters(&self) -> BTreeMap<String, OptionParameter> {
        let mut output = BTreeMap::new();
        output.insert(
            "duration_ticks".into(),
            OptionParameter::Unsigned(u64::from(self.duration_ticks)),
        );
        output.insert(
            "sample_phase_numerator".into(),
            OptionParameter::Unsigned(u64::from(self.sample_phase.numerator)),
        );
        output.insert(
            "sample_phase_denominator".into(),
            OptionParameter::Unsigned(u64::from(self.sample_phase.denominator)),
        );
        let points: Vec<StickPoint> = match &self.path {
            StickPath::Waypoint { points }
            | StickPath::Rail { points }
            | StickPath::Spline { points } => points.clone(),
            StickPath::Bezier { control } => control.to_vec(),
        };
        let encoded = points
            .iter()
            .map(|point| format!("{},{}", point.x, point.y))
            .collect::<Vec<_>>()
            .join(";");
        output.insert("stick_points".into(), OptionParameter::Text(encoded));
        output
    }
}

fn uniform_index(numerator: u64, denominator: u64, count: usize) -> Result<usize, MotionPathError> {
    let scaled = numerator
        .checked_mul(count as u64)
        .ok_or(MotionPathError::ArithmeticOverflow)?;
    Ok(((scaled / denominator) as usize).min(count - 1))
}

fn segment_phase(
    numerator: u64,
    denominator: u64,
    segments: usize,
) -> Result<(usize, u64, u64), MotionPathError> {
    let scaled = numerator
        .checked_mul(segments as u64)
        .ok_or(MotionPathError::ArithmeticOverflow)?;
    if scaled >= denominator * segments as u64 {
        return Ok((segments - 1, denominator, denominator));
    }
    Ok((
        (scaled / denominator) as usize,
        scaled % denominator,
        denominator,
    ))
}

fn linear(left: i16, right: i16, numerator: u64, denominator: u64) -> Result<i16, MotionPathError> {
    let numerator = i128::from(left) * i128::from(denominator - numerator)
        + i128::from(right) * i128::from(numerator);
    rounded_axis(numerator, i128::from(denominator))
}

fn bezier_axis(points: [i16; 4], numerator: u64, denominator: u64) -> Result<i16, MotionPathError> {
    let n = i128::from(numerator);
    let d = i128::from(denominator);
    let inverse = d - n;
    let value = i128::from(points[0]) * inverse.pow(3)
        + 3 * i128::from(points[1]) * inverse.pow(2) * n
        + 3 * i128::from(points[2]) * inverse * n.pow(2)
        + i128::from(points[3]) * n.pow(3);
    rounded_axis(value, d.pow(3))
}

fn catmull_rom(
    p0: i16,
    p1: i16,
    p2: i16,
    p3: i16,
    numerator: u64,
    denominator: u64,
) -> Result<i16, MotionPathError> {
    let n = i128::from(numerator);
    let d = i128::from(denominator);
    let value = 2 * i128::from(p1) * d.pow(3)
        + i128::from(-p0 + p2) * n * d.pow(2)
        + i128::from(2 * p0 - 5 * p1 + 4 * p2 - p3) * n.pow(2) * d
        + i128::from(-p0 + 3 * p1 - 3 * p2 + p3) * n.pow(3);
    rounded_axis(value, 2 * d.pow(3))
}

fn rounded_axis(numerator: i128, denominator: i128) -> Result<i16, MotionPathError> {
    if denominator <= 0 {
        return Err(MotionPathError::ArithmeticOverflow);
    }
    let negative = numerator < 0;
    let magnitude = numerator.unsigned_abs();
    let denominator = denominator as u128;
    let mut quotient = magnitude / denominator;
    if (magnitude % denominator) * 2 >= denominator {
        quotient += 1;
    }
    let signed = if negative {
        -(quotient as i128)
    } else {
        quotient as i128
    };
    Ok(signed.clamp(-128, 127) as i16)
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

    fn point(x: i16, y: i16) -> StickPoint {
        StickPoint { x, y }
    }

    fn sticks(realization: &MotionPathRealization) -> Vec<(i8, i8)> {
        realization
            .frames
            .iter()
            .map(|frame| (frame.pads[0].stick_x, frame.pads[0].stick_y))
            .collect()
    }

    #[test]
    fn waypoint_and_rail_have_exact_duration_and_phase() {
        let waypoint = MotionPathPlan::new(
            StickPath::Waypoint {
                points: vec![point(0, 0), point(10, 20)],
            },
            4,
        );
        assert_eq!(
            sticks(&waypoint.realize(None).unwrap()),
            vec![(0, 0), (0, 0), (10, 20), (10, 20)]
        );

        let mut rail = MotionPathPlan::new(
            StickPath::Rail {
                points: vec![point(0, 0), point(8, -8)],
            },
            4,
        );
        rail.sample_phase = SamplePhase {
            numerator: 1,
            denominator: 1,
        };
        assert_eq!(
            sticks(&rail.realize(None).unwrap()),
            vec![(2, -2), (4, -4), (6, -6), (8, -8)]
        );
    }

    #[test]
    fn spline_and_bezier_use_exact_integer_cubic_rounding() {
        let mut spline = MotionPathPlan::new(
            StickPath::Spline {
                points: vec![point(0, 0), point(8, 8), point(16, 0)],
            },
            4,
        );
        spline.sample_phase = SamplePhase {
            numerator: 1,
            denominator: 2,
        };
        assert_eq!(spline.realize(None).unwrap().frames.len(), 4);
        let spline_sticks = sticks(&spline.realize(None).unwrap());
        assert_eq!(spline_sticks[0], (1, 2));
        assert_eq!(spline_sticks[3], (15, 2));

        let mut bezier = MotionPathPlan::new(
            StickPath::Bezier {
                control: [point(0, 0), point(0, 8), point(8, 8), point(8, 0)],
            },
            2,
        );
        bezier.sample_phase = SamplePhase {
            numerator: 1,
            denominator: 1,
        };
        assert_eq!(sticks(&bezier.realize(None).unwrap()), vec![(4, 6), (8, 0)]);
    }

    #[test]
    fn cancellation_and_capture_authenticate_the_exact_path_prefix() {
        let mut plan = MotionPathPlan::new(
            StickPath::Rail {
                points: vec![point(0, 0), point(12, 0)],
            },
            4,
        );
        plan.cancellation_conditions = vec![OptionCondition::TargetReached {
            target: "rail-end".into(),
        }];
        let hit = PathCancellationHit {
            tick: 2,
            condition_index: 0,
        };
        let realization = plan.realize(Some(hit)).unwrap();
        let mut frames = vec![InputFrame::default()];
        frames.extend(realization.frames.clone());
        let tape = InputTape {
            boot: TapeBoot::Process,
            frames,
            ..InputTape::default()
        };
        let range = TapeRange {
            start_frame: 1,
            end_frame_exclusive: 3,
        };
        let execution = plan
            .capture_execution("rail-cancel".into(), &tape, range, Some(hit))
            .unwrap();
        assert_eq!(execution.option_type, OptionType::Rail);
        assert_eq!(execution.emitted_raw_actions, realization.frames);
        execution.validate_against_tape(&tape).unwrap();
    }

    #[test]
    fn validation_rejects_bad_counts_points_phase_and_duration() {
        let mut plan = MotionPathPlan::new(StickPath::Waypoint { points: vec![] }, 1);
        assert_eq!(plan.validate(), Err(MotionPathError::InvalidPointCount));
        plan.path = StickPath::Waypoint {
            points: vec![point(128, 0)],
        };
        assert_eq!(plan.validate(), Err(MotionPathError::InvalidPoint));
        plan.path = StickPath::Waypoint {
            points: vec![point(0, 0)],
        };
        plan.sample_phase = SamplePhase {
            numerator: 2,
            denominator: 1,
        };
        assert_eq!(plan.validate(), Err(MotionPathError::InvalidPhase));
        plan.sample_phase = SamplePhase::default();
        plan.duration_ticks = 0;
        assert_eq!(plan.validate(), Err(MotionPathError::InvalidDuration));
    }
}
