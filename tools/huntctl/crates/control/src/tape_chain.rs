//! Deterministic compact-tape composition and milestone-addressed splicing.

use crate::tape::{InputTape, TapeBoot, TapeError, WaitCondition};
use crate::tape_program::Marker;
use serde::Serialize;
use std::error::Error;
use std::fmt;

const VALID_PORTS: u8 = 0x0f;

/// The source-frame window retained from one chain segment.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SegmentFrames {
    /// Retain the complete source tape.
    #[default]
    All,
    /// Retain frames `0..=tape_frame`. The observed frame is included because
    /// its input produced the post-frame memory observation.
    ThroughMilestone { tape_frame: u64 },
    /// Retain frames after the observed frame, `(tape_frame + 1)..`. This is
    /// the continuation paired with a newly generated upstream prefix.
    AfterMilestone { tape_frame: u64 },
}

/// One compact tape and any marker sidecar metadata available for it.
#[derive(Clone, Debug)]
pub struct ChainSegment {
    pub name: Option<String>,
    pub tape: InputTape,
    pub markers: Vec<Marker>,
    pub frames: SegmentFrames,
}

impl ChainSegment {
    pub fn all(tape: InputTape) -> Self {
        Self {
            name: None,
            tape,
            markers: Vec::new(),
            frames: SegmentFrames::All,
        }
    }

    pub fn through_milestone(tape: InputTape, tape_frame: u64) -> Self {
        Self {
            frames: SegmentFrames::ThroughMilestone { tape_frame },
            ..Self::all(tape)
        }
    }

    pub fn after_milestone(tape: InputTape, tape_frame: u64) -> Self {
        Self {
            frames: SegmentFrames::AfterMilestone { tape_frame },
            ..Self::all(tape)
        }
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_markers(mut self, markers: Vec<Marker>) -> Self {
        self.markers = markers;
        self
    }
}

/// Composition policy. Port masks normally remain a per-frame tape property;
/// callers may require an exact ownership mask for every source frame.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ComposeOptions {
    pub required_owned_ports: Option<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ChainedMarker {
    pub name: String,
    pub tick: u64,
    pub segment_index: usize,
    pub segment_name: Option<String>,
    pub source_tick: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SegmentBoundary {
    pub segment_index: usize,
    pub segment_name: Option<String>,
    pub chain_start_tick: u64,
    pub chain_end_tick: u64,
    pub source_start_tick: u64,
    pub source_end_tick: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChainedTape {
    pub tape: InputTape,
    pub markers: Vec<ChainedMarker>,
    pub segments: Vec<SegmentBoundary>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChainError {
    Empty,
    InvalidRequiredPorts(u8),
    InvalidTape {
        segment_index: usize,
        source: TapeError,
    },
    ReactiveFrame {
        segment_index: usize,
        tape_frame: u64,
    },
    TickRateMismatch {
        segment_index: usize,
        expected_numerator: u32,
        expected_denominator: u32,
        actual_numerator: u32,
        actual_denominator: u32,
    },
    BootMismatch {
        segment_index: usize,
        expected: Box<TapeBoot>,
        actual: Box<TapeBoot>,
    },
    PortMismatch {
        segment_index: usize,
        tape_frame: u64,
        expected: u8,
        actual: u8,
    },
    MilestoneFrameOutOfRange {
        segment_index: usize,
        tape_frame: u64,
        frame_count: u64,
    },
    InvalidMarker {
        segment_index: usize,
        name: String,
        tick: u64,
        frame_count: u64,
    },
    TooManyFrames,
}

impl fmt::Display for ChainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("a tape chain requires at least one segment"),
            Self::InvalidRequiredPorts(mask) => write!(
                formatter,
                "required controller ownership mask 0x{mask:02x} addresses ports above 3"
            ),
            Self::InvalidTape {
                segment_index,
                source,
            } => write!(formatter, "segment {segment_index} is invalid: {source}"),
            Self::ReactiveFrame {
                segment_index,
                tape_frame,
            } => write!(
                formatter,
                "segment {segment_index} frame {tape_frame} is reactive and cannot be chained"
            ),
            Self::TickRateMismatch {
                segment_index,
                expected_numerator,
                expected_denominator,
                actual_numerator,
                actual_denominator,
            } => write!(
                formatter,
                "segment {segment_index} tick rate {actual_numerator}/{actual_denominator} does not match {expected_numerator}/{expected_denominator}"
            ),
            Self::BootMismatch {
                segment_index,
                expected,
                actual,
            } => write!(
                formatter,
                "segment {segment_index} boot origin {actual:?} conflicts with chain origin {expected:?}"
            ),
            Self::PortMismatch {
                segment_index,
                tape_frame,
                expected,
                actual,
            } => write!(
                formatter,
                "segment {segment_index} frame {tape_frame} owns ports 0x{actual:02x}; expected 0x{expected:02x}"
            ),
            Self::MilestoneFrameOutOfRange {
                segment_index,
                tape_frame,
                frame_count,
            } => write!(
                formatter,
                "segment {segment_index} milestone frame {tape_frame} is outside its {frame_count}-frame tape"
            ),
            Self::InvalidMarker {
                segment_index,
                name,
                tick,
                frame_count,
            } => write!(
                formatter,
                "segment {segment_index} marker {name:?} tick {tick} is outside 0..={frame_count}"
            ),
            Self::TooManyFrames => formatter.write_str("composed tape is too large"),
        }
    }
}

impl Error for ChainError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidTape { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Concatenate absolute compact tapes using their existing per-frame port masks.
pub fn concatenate(segments: Vec<ChainSegment>) -> Result<ChainedTape, ChainError> {
    concatenate_with_options(segments, ComposeOptions::default())
}

/// Concatenate absolute compact tapes with optional exact port ownership.
pub fn concatenate_with_options(
    segments: Vec<ChainSegment>,
    options: ComposeOptions,
) -> Result<ChainedTape, ChainError> {
    if segments.is_empty() {
        return Err(ChainError::Empty);
    }
    if let Some(mask) = options.required_owned_ports
        && mask & !VALID_PORTS != 0
    {
        return Err(ChainError::InvalidRequiredPorts(mask));
    }

    let expected_numerator = segments[0].tape.tick_rate_numerator;
    let expected_denominator = segments[0].tape.tick_rate_denominator;
    let output_boot = segments[0].tape.boot.clone();
    let mut windows = Vec::with_capacity(segments.len());
    let mut output_capacity = 0_usize;

    for (segment_index, segment) in segments.iter().enumerate() {
        segment
            .tape
            .validate()
            .map_err(|source| ChainError::InvalidTape {
                segment_index,
                source,
            })?;
        if !rates_equal(
            expected_numerator,
            expected_denominator,
            segment.tape.tick_rate_numerator,
            segment.tape.tick_rate_denominator,
        ) {
            return Err(ChainError::TickRateMismatch {
                segment_index,
                expected_numerator,
                expected_denominator,
                actual_numerator: segment.tape.tick_rate_numerator,
                actual_denominator: segment.tape.tick_rate_denominator,
            });
        }
        // Process-boot continuation tapes carry no fixture assertion. A later
        // explicit fixture must, however, describe the same origin as the
        // first segment; silently discarding a different map start would make
        // the composed artifact lie about its initial state.
        if segment_index != 0
            && !matches!(&segment.tape.boot, TapeBoot::Process)
            && segment.tape.boot != output_boot
        {
            return Err(ChainError::BootMismatch {
                segment_index,
                expected: Box::new(output_boot.clone()),
                actual: Box::new(segment.tape.boot.clone()),
            });
        }
        for (tape_frame, frame) in segment.tape.frames.iter().enumerate() {
            if frame.wait_condition != WaitCondition::None {
                return Err(ChainError::ReactiveFrame {
                    segment_index,
                    tape_frame: tape_frame as u64,
                });
            }
            if let Some(expected) = options.required_owned_ports
                && frame.owned_ports != expected
            {
                return Err(ChainError::PortMismatch {
                    segment_index,
                    tape_frame: tape_frame as u64,
                    expected,
                    actual: frame.owned_ports,
                });
            }
        }
        for marker in &segment.markers {
            if marker.name.is_empty() || marker.tick > segment.tape.frames.len() as u64 {
                return Err(ChainError::InvalidMarker {
                    segment_index,
                    name: marker.name.clone(),
                    tick: marker.tick,
                    frame_count: segment.tape.frames.len() as u64,
                });
            }
        }
        let window = source_window(segment_index, segment)?;
        output_capacity = output_capacity
            .checked_add(window.1 - window.0)
            .ok_or(ChainError::TooManyFrames)?;
        windows.push(window);
    }

    let mut frames = Vec::with_capacity(output_capacity);
    let mut markers = Vec::new();
    let mut boundaries = Vec::with_capacity(segments.len());
    for (segment_index, (segment, (source_start, source_end))) in
        segments.into_iter().zip(windows).enumerate()
    {
        let ChainSegment {
            name,
            tape,
            markers: source_markers,
            ..
        } = segment;
        let chain_start = frames.len();
        frames.extend(
            tape.frames
                .into_iter()
                .skip(source_start)
                .take(source_end - source_start),
        );
        let chain_end = frames.len();
        let chain_start_u64 = chain_start as u64;
        let source_start_u64 = source_start as u64;
        for marker in source_markers
            .into_iter()
            .filter(|marker| marker.tick >= source_start as u64 && marker.tick <= source_end as u64)
        {
            let tick = chain_start_u64
                .checked_add(marker.tick - source_start_u64)
                .ok_or(ChainError::TooManyFrames)?;
            markers.push(ChainedMarker {
                name: marker.name,
                tick,
                segment_index,
                segment_name: name.clone(),
                source_tick: marker.tick,
            });
        }
        boundaries.push(SegmentBoundary {
            segment_index,
            segment_name: name,
            chain_start_tick: chain_start as u64,
            chain_end_tick: chain_end as u64,
            source_start_tick: source_start as u64,
            source_end_tick: source_end as u64,
        });
    }

    Ok(ChainedTape {
        tape: InputTape {
            boot: output_boot,
            tick_rate_numerator: expected_numerator,
            tick_rate_denominator: expected_denominator,
            frames,
        },
        markers,
        segments: boundaries,
    })
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

fn source_window(
    segment_index: usize,
    segment: &ChainSegment,
) -> Result<(usize, usize), ChainError> {
    let frame_count = segment.tape.frames.len();
    let milestone = match segment.frames {
        SegmentFrames::All => return Ok((0, frame_count)),
        SegmentFrames::ThroughMilestone { tape_frame }
        | SegmentFrames::AfterMilestone { tape_frame } => tape_frame,
    };
    if milestone >= frame_count as u64 {
        return Err(ChainError::MilestoneFrameOutOfRange {
            segment_index,
            tape_frame: milestone,
            frame_count: frame_count as u64,
        });
    }
    let boundary = usize::try_from(milestone)
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or(ChainError::TooManyFrames)?;
    match segment.frames {
        SegmentFrames::All => unreachable!(),
        SegmentFrames::ThroughMilestone { .. } => Ok((0, boundary)),
        SegmentFrames::AfterMilestone { .. } => Ok((boundary, frame_count)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::{InputFrame, RawPadState};

    fn frame(value: i8, owned_ports: u8) -> InputFrame {
        InputFrame {
            owned_ports,
            pads: [
                RawPadState {
                    stick_x: value,
                    ..RawPadState::default()
                },
                RawPadState::default(),
                RawPadState::default(),
                RawPadState::default(),
            ],
            ..InputFrame::default()
        }
    }

    fn tape(values: &[i8]) -> InputTape {
        InputTape {
            frames: values.iter().map(|value| frame(*value, 0x0f)).collect(),
            ..InputTape::default()
        }
    }

    #[test]
    fn splices_prefix_and_continuation_at_observed_frames() {
        let upstream = ChainSegment::through_milestone(tape(&[1, 2, 3, 4]), 2)
            .named("new-upstream")
            .with_markers(vec![
                Marker {
                    name: "start".into(),
                    tick: 0,
                },
                Marker {
                    name: "hit".into(),
                    tick: 3,
                },
                Marker {
                    name: "trimmed".into(),
                    tick: 4,
                },
            ]);
        let continuation = ChainSegment::after_milestone(tape(&[5, 6, 7, 8]), 1)
            .named("old-downstream")
            .with_markers(vec![
                Marker {
                    name: "continuation".into(),
                    tick: 2,
                },
                Marker {
                    name: "end".into(),
                    tick: 4,
                },
            ]);

        let chained = concatenate(vec![upstream, continuation]).unwrap();
        let values = chained
            .tape
            .frames
            .iter()
            .map(|frame| frame.pads[0].stick_x)
            .collect::<Vec<_>>();
        assert_eq!(values, [1, 2, 3, 7, 8]);
        assert_eq!(
            chained
                .markers
                .iter()
                .map(|marker| (marker.name.as_str(), marker.tick, marker.source_tick))
                .collect::<Vec<_>>(),
            [
                ("start", 0, 0),
                ("hit", 3, 3),
                ("continuation", 3, 2),
                ("end", 5, 4),
            ]
        );
        assert_eq!(chained.segments[0].chain_end_tick, 3);
        assert_eq!(chained.segments[1].source_start_tick, 2);
    }

    #[test]
    fn accepts_equivalent_tick_rates_and_preserves_first_encoding() {
        let first = tape(&[1]);
        let mut second = tape(&[2]);
        second.tick_rate_numerator = 60;
        second.tick_rate_denominator = 2;
        let chained =
            concatenate(vec![ChainSegment::all(first), ChainSegment::all(second)]).unwrap();
        assert_eq!(chained.tape.tick_rate_numerator, 30);
        assert_eq!(chained.tape.tick_rate_denominator, 1);
    }

    #[test]
    fn preserves_one_boot_origin_and_rejects_conflicting_stage_segments() {
        let origin = TapeBoot::Stage {
            stage: "F_SP103".into(),
            room: 1,
            point: 1,
            layer: 3,
            save_slot: Some(2),
            fixture: None,
        };
        let mut first = tape(&[1]);
        first.boot = origin.clone();
        let mut matching = tape(&[2]);
        matching.boot = origin.clone();
        let chained = concatenate(vec![
            ChainSegment::all(first.clone()),
            ChainSegment::all(matching),
            ChainSegment::all(tape(&[3])),
        ])
        .unwrap();
        assert_eq!(chained.tape.boot, origin);

        let mut conflict = tape(&[4]);
        conflict.boot = TapeBoot::Stage {
            stage: "F_SP104".into(),
            room: 0,
            point: 0,
            layer: -1,
            save_slot: None,
            fixture: None,
        };
        assert!(matches!(
            concatenate(vec![ChainSegment::all(first), ChainSegment::all(conflict)]),
            Err(ChainError::BootMismatch {
                segment_index: 1,
                ..
            })
        ));
    }

    #[test]
    fn rejects_rate_port_and_reactive_mismatches() {
        let mut wrong_rate = tape(&[1]);
        wrong_rate.tick_rate_numerator = 60;
        assert!(matches!(
            concatenate(vec![
                ChainSegment::all(tape(&[0])),
                ChainSegment::all(wrong_rate)
            ]),
            Err(ChainError::TickRateMismatch {
                segment_index: 1,
                ..
            })
        ));

        let wrong_ports = InputTape {
            frames: vec![frame(0, 1)],
            ..InputTape::default()
        };
        assert!(matches!(
            concatenate_with_options(
                vec![ChainSegment::all(wrong_ports)],
                ComposeOptions {
                    required_owned_ports: Some(0x0f)
                }
            ),
            Err(ChainError::PortMismatch { tape_frame: 0, .. })
        ));

        let reactive = InputTape {
            frames: vec![InputFrame {
                wait_condition: WaitCondition::NameEntryActive,
                wait_timeout_ticks: 10,
                ..frame(0, 0x0f)
            }],
            ..InputTape::default()
        };
        assert!(matches!(
            concatenate(vec![ChainSegment::all(reactive)]),
            Err(ChainError::ReactiveFrame { tape_frame: 0, .. })
        ));
    }

    #[test]
    fn rejects_out_of_range_observation_and_marker() {
        assert!(matches!(
            concatenate(vec![ChainSegment::through_milestone(tape(&[1]), 1)]),
            Err(ChainError::MilestoneFrameOutOfRange { .. })
        ));
        let segment = ChainSegment::all(tape(&[1])).with_markers(vec![Marker {
            name: "bad".into(),
            tick: 2,
        }]);
        assert!(matches!(
            concatenate(vec![segment]),
            Err(ChainError::InvalidMarker { .. })
        ));
    }
}
