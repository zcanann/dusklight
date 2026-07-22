//! Lossless raw-PAD residuals over an incumbent input tape.

use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::{InputTape, PORT_COUNT, RawPadState, WaitCondition};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const RESIDUAL_CANDIDATE_SCHEMA_V1: &str = "dusklight-residual-candidate/v1";
pub const RESIDUAL_COMPILATION_SCHEMA_V1: &str = "dusklight-residual-compilation/v1";
pub const RESIDUAL_PROPOSAL_SCHEMA_ID_V1: &str = "dusklight-residual-action/v1";
pub const RESIDUAL_PROPOSAL_SCHEMA_DESCRIPTOR_V1: &str =
    "dusklight-residual-action/v1:raw-pad-additive-bounded-exact-2-4-8-16-32-ramp-curve";
const MAX_ANALOG_RESIDUALS: usize = 128;
const MAX_BUTTON_RESIDUALS: usize = 128;
const MAX_ABSOLUTE_DELTA: i16 = 255;
pub const AUTHENTIC_BUTTON_MASK: u16 = 0x1f7f;

pub fn residual_proposal_schema_sha256() -> Digest {
    sha256(RESIDUAL_PROPOSAL_SCHEMA_DESCRIPTOR_V1.as_bytes())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCandidate {
    pub schema: String,
    pub content_sha256: Digest,
    pub parent_tape_sha256: Digest,
    pub analog: Vec<AnalogResidual>,
    pub buttons: Vec<ButtonResidual>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalogChannel {
    MainX,
    MainY,
    CameraX,
    CameraY,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidualWindow {
    Frames2,
    Frames4,
    Frames8,
    Frames16,
    Frames32,
}

impl ResidualWindow {
    pub const fn frames(self) -> u64 {
        match self {
            Self::Frames2 => 2,
            Self::Frames4 => 4,
            Self::Frames8 => 8,
            Self::Frames16 => 16,
            Self::Frames32 => 32,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TemporalBasis {
    ExactFrame {
        frame: u64,
        delta: i16,
    },
    Window {
        start_frame: u64,
        window: ResidualWindow,
        delta: i16,
    },
    PiecewiseLinearRamp {
        start_frame: u64,
        end_frame: u64,
        start_delta: i16,
        end_delta: i16,
    },
    CubicControlCurve {
        start_frame: u64,
        end_frame: u64,
        control_deltas: [i16; 4],
    },
}

impl TemporalBasis {
    fn span(&self) -> Result<InterventionSpan, ResidualActionError> {
        let (start_frame, end_exclusive) = match self {
            Self::ExactFrame { frame, .. } => (*frame, frame.checked_add(1)),
            Self::Window {
                start_frame,
                window,
                ..
            } => (*start_frame, start_frame.checked_add(window.frames())),
            Self::PiecewiseLinearRamp {
                start_frame,
                end_frame,
                ..
            }
            | Self::CubicControlCurve {
                start_frame,
                end_frame,
                ..
            } if end_frame > start_frame => (*start_frame, end_frame.checked_add(1)),
            _ => return Err(action_error("temporal ramp or curve span is invalid")),
        };
        Ok(InterventionSpan {
            start_frame,
            end_frame_exclusive: end_exclusive
                .ok_or_else(|| action_error("temporal basis span overflowed"))?,
        })
    }

    fn validate(&self) -> Result<(), ResidualActionError> {
        let deltas = match self {
            Self::ExactFrame { delta, .. } | Self::Window { delta, .. } => vec![*delta],
            Self::PiecewiseLinearRamp {
                start_delta,
                end_delta,
                ..
            } => vec![*start_delta, *end_delta],
            Self::CubicControlCurve { control_deltas, .. } => control_deltas.to_vec(),
        };
        self.span()?;
        if deltas
            .iter()
            .any(|delta| *delta < -MAX_ABSOLUTE_DELTA || *delta > MAX_ABSOLUTE_DELTA)
            || deltas.iter().all(|delta| *delta == 0)
        {
            return Err(action_error(
                "temporal basis deltas must be bounded and not identically zero",
            ));
        }
        Ok(())
    }

    fn value_at(&self, frame: u64) -> i32 {
        match self {
            Self::ExactFrame {
                frame: target,
                delta,
            } => {
                if frame == *target {
                    i32::from(*delta)
                } else {
                    0
                }
            }
            Self::Window {
                start_frame,
                window,
                delta,
            } => {
                if frame >= *start_frame && frame < *start_frame + window.frames() {
                    i32::from(*delta)
                } else {
                    0
                }
            }
            Self::PiecewiseLinearRamp {
                start_frame,
                end_frame,
                start_delta,
                end_delta,
            } if frame >= *start_frame && frame <= *end_frame => interpolate_linear(
                i64::from(*start_delta),
                i64::from(*end_delta),
                frame - *start_frame,
                *end_frame - *start_frame,
            ) as i32,
            Self::CubicControlCurve {
                start_frame,
                end_frame,
                control_deltas,
            } if frame >= *start_frame && frame <= *end_frame => cubic_control_value(
                *control_deltas,
                frame - *start_frame,
                *end_frame - *start_frame,
            ),
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalogResidual {
    pub port: u8,
    pub channel: AnalogChannel,
    pub basis: TemporalBasis,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonResidualMode {
    Press,
    Release,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ButtonResidual {
    pub port: u8,
    pub buttons: u16,
    pub start_frame: u64,
    pub duration_frames: u64,
    pub mode: ButtonResidualMode,
}

impl ButtonResidual {
    fn span(&self) -> Result<InterventionSpan, ResidualActionError> {
        if self.buttons == 0
            || self.buttons & !AUTHENTIC_BUTTON_MASK != 0
            || self.duration_frames == 0
        {
            return Err(action_error(
                "button residual requires authentic nonzero buttons and duration",
            ));
        }
        Ok(InterventionSpan {
            start_frame: self.start_frame,
            end_frame_exclusive: self
                .start_frame
                .checked_add(self.duration_frames)
                .ok_or_else(|| action_error("button residual span overflowed"))?,
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InterventionSpan {
    pub start_frame: u64,
    pub end_frame_exclusive: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCompilationReport {
    pub schema: String,
    pub candidate_sha256: Digest,
    pub parent_tape_sha256: Digest,
    pub realized_tape_sha256: Digest,
    pub frame_count: u64,
    pub declared_intervention_span: InterventionSpan,
    pub intervention_span: InterventionSpan,
    pub analog_residuals: u64,
    pub button_residuals: u64,
    pub clamp_boundary: String,
    pub realized_tape_authoritative: bool,
}

#[derive(Clone, Debug)]
pub struct CompiledResidualCandidate {
    pub report: ResidualCompilationReport,
    pub tape: InputTape,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCompilationAlias {
    pub candidate_sha256: Digest,
    pub realized_tape_sha256: Digest,
    pub retained_candidate_sha256: Digest,
}

#[derive(Clone, Debug)]
pub struct DeduplicatedResidualCompilations {
    pub unique: Vec<CompiledResidualCandidate>,
    pub aliases: Vec<ResidualCompilationAlias>,
}

impl ResidualCandidate {
    pub fn seal(
        parent_tape_bytes: &[u8],
        analog: Vec<AnalogResidual>,
        buttons: Vec<ButtonResidual>,
    ) -> Result<Self, ResidualActionError> {
        let mut candidate = Self {
            schema: RESIDUAL_CANDIDATE_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            parent_tape_sha256: sha256(parent_tape_bytes),
            analog,
            buttons,
        };
        candidate.content_sha256 = candidate.compute_identity()?;
        candidate.validate()?;
        Ok(candidate)
    }

    pub fn validate(&self) -> Result<(), ResidualActionError> {
        if self.schema != RESIDUAL_CANDIDATE_SCHEMA_V1
            || self.parent_tape_sha256 == Digest::ZERO
            || self.analog.len() > MAX_ANALOG_RESIDUALS
            || self.buttons.len() > MAX_BUTTON_RESIDUALS
            || (self.analog.is_empty() && self.buttons.is_empty())
            || !self.analog.windows(2).all(|pair| pair[0] < pair[1])
            || !self.buttons.windows(2).all(|pair| pair[0] < pair[1])
        {
            return Err(action_error(
                "residual candidate envelope, bounds, or canonical ordering is invalid",
            ));
        }
        for residual in &self.analog {
            validate_port(residual.port)?;
            residual.basis.validate()?;
        }
        for residual in &self.buttons {
            validate_port(residual.port)?;
            residual.span()?;
        }
        if self.content_sha256 == Digest::ZERO || self.content_sha256 != self.compute_identity()? {
            return Err(action_error("residual candidate content seal is invalid"));
        }
        Ok(())
    }

    pub fn intervention_span(&self) -> Result<InterventionSpan, ResidualActionError> {
        self.analog
            .iter()
            .map(|residual| residual.basis.span())
            .chain(self.buttons.iter().map(ButtonResidual::span))
            .try_fold(None, |span: Option<InterventionSpan>, next| {
                let next = next?;
                Ok::<_, ResidualActionError>(Some(match span {
                    None => next,
                    Some(span) => InterventionSpan {
                        start_frame: span.start_frame.min(next.start_frame),
                        end_frame_exclusive: span.end_frame_exclusive.max(next.end_frame_exclusive),
                    },
                }))
            })?
            .ok_or_else(|| action_error("residual candidate has no intervention span"))
    }

    fn compute_identity(&self) -> Result<Digest, ResidualActionError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.residual-candidate/v1\0", &canonical)
    }
}

pub fn compile_residual_candidate(
    parent: &InputTape,
    parent_bytes: &[u8],
    candidate: &ResidualCandidate,
) -> Result<CompiledResidualCandidate, ResidualActionError> {
    compile_residual_candidate_to_horizon(
        parent,
        parent_bytes,
        candidate,
        parent.frames.len() as u64,
    )
}

/// Compiles a residual and extends the authoritative raw-PAD tape with released
/// input through the exploration horizon. The extension is part of the sealed
/// realized tape, so a harness cannot silently stop at the shorter incumbent.
pub fn compile_residual_candidate_to_horizon(
    parent: &InputTape,
    parent_bytes: &[u8],
    candidate: &ResidualCandidate,
    exploration_horizon_ticks: u64,
) -> Result<CompiledResidualCandidate, ResidualActionError> {
    candidate.validate()?;
    let canonical_parent = parent
        .encode()
        .map_err(|source| action_error(source.to_string()))?;
    if canonical_parent != parent_bytes || sha256(parent_bytes) != candidate.parent_tape_sha256 {
        return Err(action_error(
            "residual parent tape bytes differ from the sealed candidate",
        ));
    }
    let declared_span = candidate.intervention_span()?;
    if declared_span.end_frame_exclusive > parent.frames.len() as u64 {
        return Err(action_error(
            "residual intervention exceeds the incumbent tape",
        ));
    }
    validate_touched_frames(parent, candidate)?;

    let mut realized = parent.clone();
    for frame_index in declared_span.start_frame..declared_span.end_frame_exclusive {
        let frame = &mut realized.frames[frame_index as usize];
        for port in 0..PORT_COUNT {
            for channel in [
                AnalogChannel::MainX,
                AnalogChannel::MainY,
                AnalogChannel::CameraX,
                AnalogChannel::CameraY,
            ] {
                let delta = candidate
                    .analog
                    .iter()
                    .filter(|residual| {
                        usize::from(residual.port) == port && residual.channel == channel
                    })
                    .map(|residual| residual.basis.value_at(frame_index))
                    .sum::<i32>();
                if delta != 0 {
                    apply_analog_delta(&mut frame.pads[port], channel, delta);
                }
            }
        }
    }
    // Releases compose before presses, so two bounded edits can shift an
    // overlapping held interval earlier or later without replacing the rest
    // of the button schedule.
    for mode in [ButtonResidualMode::Release, ButtonResidualMode::Press] {
        for residual in candidate
            .buttons
            .iter()
            .filter(|residual| residual.mode == mode)
        {
            let end = residual.start_frame + residual.duration_frames;
            for frame_index in residual.start_frame..end {
                let pad =
                    &mut realized.frames[frame_index as usize].pads[usize::from(residual.port)];
                match mode {
                    ButtonResidualMode::Press => pad.buttons |= residual.buttons,
                    ButtonResidualMode::Release => pad.buttons &= !residual.buttons,
                }
            }
        }
    }
    let span = exact_intervention_span(parent, &realized)
        .ok_or_else(|| action_error("residual candidate compiles to the incumbent tape"))?;
    extend_tape_with_released_input(&mut realized, exploration_horizon_ticks)?;
    let bytes = realized
        .encode()
        .map_err(|source| action_error(source.to_string()))?;
    let decoded = InputTape::decode(&bytes).map_err(|source| action_error(source.to_string()))?;
    if decoded.tape != realized {
        return Err(action_error(
            "compiled residual tape does not losslessly round-trip",
        ));
    }
    Ok(CompiledResidualCandidate {
        report: ResidualCompilationReport {
            schema: RESIDUAL_COMPILATION_SCHEMA_V1.into(),
            candidate_sha256: candidate.content_sha256,
            parent_tape_sha256: candidate.parent_tape_sha256,
            realized_tape_sha256: sha256(&bytes),
            frame_count: realized.frames.len() as u64,
            declared_intervention_span: declared_span,
            intervention_span: span,
            analog_residuals: candidate.analog.len() as u64,
            button_residuals: candidate.buttons.len() as u64,
            clamp_boundary: "raw_pad_i8_after_sum".into(),
            realized_tape_authoritative: true,
        },
        tape: realized,
        bytes,
    })
}

/// Extends a tape with released input while preserving controller connection
/// and error state. The native checkpoint source and every compiled residual
/// use this one authoritative horizon rule.
pub fn extend_tape_with_released_input(
    tape: &mut InputTape,
    exploration_horizon_ticks: u64,
) -> Result<(), ResidualActionError> {
    let horizon = usize::try_from(exploration_horizon_ticks)
        .map_err(|_| action_error("residual exploration horizon does not fit memory"))?;
    if horizon < tape.frames.len() {
        return Err(action_error(
            "residual exploration horizon cannot truncate the incumbent tape",
        ));
    }
    if horizon > tape.frames.len() {
        let mut released = tape
            .frames
            .last()
            .cloned()
            .ok_or_else(|| action_error("residual parent tape has no frames"))?;
        released.wait_condition = WaitCondition::None;
        released.wait_timeout_ticks = 0;
        for pad in &mut released.pads {
            let connected = pad.connected;
            let error = pad.error;
            *pad = RawPadState {
                connected,
                error,
                ..RawPadState::default()
            };
        }
        tape.frames.resize(horizon, released);
    }
    Ok(())
}

pub fn compile_unique_residual_candidates(
    parent: &InputTape,
    parent_bytes: &[u8],
    candidates: &[ResidualCandidate],
) -> Result<DeduplicatedResidualCompilations, ResidualActionError> {
    let mut unique = Vec::<CompiledResidualCandidate>::new();
    let mut by_tape = BTreeMap::<Digest, Digest>::new();
    let mut aliases = Vec::new();
    for candidate in candidates {
        let compiled = compile_residual_candidate(parent, parent_bytes, candidate)?;
        if let Some(retained) = by_tape.get(&compiled.report.realized_tape_sha256) {
            aliases.push(ResidualCompilationAlias {
                candidate_sha256: candidate.content_sha256,
                realized_tape_sha256: compiled.report.realized_tape_sha256,
                retained_candidate_sha256: *retained,
            });
        } else {
            by_tape.insert(
                compiled.report.realized_tape_sha256,
                candidate.content_sha256,
            );
            unique.push(compiled);
        }
    }
    Ok(DeduplicatedResidualCompilations { unique, aliases })
}

fn validate_touched_frames(
    parent: &InputTape,
    candidate: &ResidualCandidate,
) -> Result<(), ResidualActionError> {
    for residual in &candidate.analog {
        let span = residual.basis.span()?;
        validate_touch(parent, residual.port, span)?;
    }
    for residual in &candidate.buttons {
        validate_touch(parent, residual.port, residual.span()?)?;
    }
    Ok(())
}

fn validate_touch(
    parent: &InputTape,
    port: u8,
    span: InterventionSpan,
) -> Result<(), ResidualActionError> {
    if span.end_frame_exclusive > parent.frames.len() as u64 {
        return Err(action_error("residual span exceeds parent tape"));
    }
    let mask = 1_u8 << port;
    for frame in &parent.frames[span.start_frame as usize..span.end_frame_exclusive as usize] {
        if frame.owned_ports & mask == 0 || frame.wait_condition != WaitCondition::None {
            return Err(action_error(
                "residuals require an owned port on ordinary non-wait frames",
            ));
        }
    }
    Ok(())
}

fn exact_intervention_span(parent: &InputTape, realized: &InputTape) -> Option<InterventionSpan> {
    let start = parent
        .frames
        .iter()
        .zip(&realized.frames)
        .position(|(left, right)| left != right)?;
    let end = parent
        .frames
        .iter()
        .zip(&realized.frames)
        .rposition(|(left, right)| left != right)?
        + 1;
    Some(InterventionSpan {
        start_frame: start as u64,
        end_frame_exclusive: end as u64,
    })
}

fn apply_analog_delta(pad: &mut RawPadState, channel: AnalogChannel, delta: i32) {
    let value = match channel {
        AnalogChannel::MainX => &mut pad.stick_x,
        AnalogChannel::MainY => &mut pad.stick_y,
        AnalogChannel::CameraX => &mut pad.substick_x,
        AnalogChannel::CameraY => &mut pad.substick_y,
    };
    *value = (i32::from(*value) + delta).clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8;
}

fn interpolate_linear(start: i64, end: i64, offset: u64, span: u64) -> i64 {
    start + div_round_nearest((end - start) * offset as i64, span as i64)
}

fn cubic_control_value(points: [i16; 4], offset: u64, span: u64) -> i32 {
    let t = offset as i64;
    let inverse = span as i64 - t;
    let numerator = i64::from(points[0]) * inverse * inverse * inverse
        + 3 * i64::from(points[1]) * inverse * inverse * t
        + 3 * i64::from(points[2]) * inverse * t * t
        + i64::from(points[3]) * t * t * t;
    div_round_nearest(numerator, (span as i64).pow(3)) as i32
}

fn div_round_nearest(numerator: i64, denominator: i64) -> i64 {
    let magnitude = numerator.unsigned_abs();
    let rounded = (magnitude + denominator as u64 / 2) / denominator as u64;
    if numerator < 0 {
        -(rounded as i64)
    } else {
        rounded as i64
    }
}

fn validate_port(port: u8) -> Result<(), ResidualActionError> {
    if usize::from(port) >= PORT_COUNT {
        return Err(action_error("residual controller port is invalid"));
    }
    Ok(())
}

fn canonical_digest(domain: &[u8], value: &impl Serialize) -> Result<Digest, ResidualActionError> {
    let bytes = serde_json::to_vec(value).map_err(|source| action_error(source.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResidualActionError(String);

fn action_error(message: impl Into<String>) -> ResidualActionError {
    ResidualActionError(message.into())
}

impl fmt::Display for ResidualActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ResidualActionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_automation_contracts::tape::InputFrame;
    use std::fs;
    use std::path::PathBuf;

    fn parent_tape(frame_count: usize) -> (InputTape, Vec<u8>) {
        let tape = InputTape {
            frames: (0..frame_count)
                .map(|_| InputFrame {
                    owned_ports: 1,
                    ..InputFrame::default()
                })
                .collect(),
            ..InputTape::default()
        };
        let bytes = tape.encode().unwrap();
        (tape, bytes)
    }

    fn sealed(
        parent_bytes: &[u8],
        mut analog: Vec<AnalogResidual>,
        mut buttons: Vec<ButtonResidual>,
    ) -> ResidualCandidate {
        analog.sort();
        buttons.sort();
        ResidualCandidate::seal(parent_bytes, analog, buttons).unwrap()
    }

    fn analog(channel: AnalogChannel, basis: TemporalBasis) -> AnalogResidual {
        AnalogResidual {
            port: 0,
            channel,
            basis,
        }
    }

    #[test]
    fn overlapping_analog_residuals_sum_before_the_authentic_boundary_clamp() {
        let (mut parent, _) = parent_tape(40);
        parent.frames[5].pads[0].stick_x = 120;
        parent.frames[5].pads[0].substick_y = -120;
        parent.frames[5].pads[0].trigger_left = 77;
        let parent_bytes = parent.encode().unwrap();
        let candidate = sealed(
            &parent_bytes,
            vec![
                analog(
                    AnalogChannel::MainX,
                    TemporalBasis::ExactFrame {
                        frame: 5,
                        delta: 100,
                    },
                ),
                analog(
                    AnalogChannel::MainX,
                    TemporalBasis::ExactFrame {
                        frame: 5,
                        delta: -20,
                    },
                ),
                analog(
                    AnalogChannel::CameraY,
                    TemporalBasis::ExactFrame {
                        frame: 5,
                        delta: -100,
                    },
                ),
                analog(
                    AnalogChannel::CameraY,
                    TemporalBasis::ExactFrame {
                        frame: 5,
                        delta: 20,
                    },
                ),
            ],
            vec![],
        );
        let compiled = compile_residual_candidate(&parent, &parent_bytes, &candidate).unwrap();
        assert_eq!(compiled.tape.frames[5].pads[0].stick_x, 127);
        assert_eq!(compiled.tape.frames[5].pads[0].substick_y, -128);
        assert_eq!(compiled.tape.frames[5].pads[0].trigger_left, 77);
        assert_eq!(compiled.report.clamp_boundary, "raw_pad_i8_after_sum");
        assert!(compiled.report.realized_tape_authoritative);
        assert_eq!(
            InputTape::decode(&compiled.bytes).unwrap().tape,
            compiled.tape
        );
    }

    #[test]
    fn every_declared_temporal_basis_is_exact_and_deterministic() {
        for (window, frames) in [
            (ResidualWindow::Frames2, 2),
            (ResidualWindow::Frames4, 4),
            (ResidualWindow::Frames8, 8),
            (ResidualWindow::Frames16, 16),
            (ResidualWindow::Frames32, 32),
        ] {
            let basis = TemporalBasis::Window {
                start_frame: 3,
                window,
                delta: 9,
            };
            assert_eq!(basis.span().unwrap().end_frame_exclusive, 3 + frames);
            assert_eq!(basis.value_at(2), 0);
            assert_eq!(basis.value_at(3), 9);
            assert_eq!(basis.value_at(3 + frames - 1), 9);
            assert_eq!(basis.value_at(3 + frames), 0);
        }
        let ramp = TemporalBasis::PiecewiseLinearRamp {
            start_frame: 10,
            end_frame: 14,
            start_delta: -20,
            end_delta: 20,
        };
        assert_eq!(
            (10..=14)
                .map(|frame| ramp.value_at(frame))
                .collect::<Vec<_>>(),
            vec![-20, -10, 0, 10, 20]
        );
        let curve = TemporalBasis::CubicControlCurve {
            start_frame: 20,
            end_frame: 24,
            control_deltas: [0, 40, -20, 10],
        };
        assert_eq!(curve.value_at(20), 0);
        assert_eq!(curve.value_at(24), 10);
        assert_eq!(curve.value_at(22), 9);
        assert_eq!(curve.value_at(22), curve.value_at(22));
    }

    #[test]
    fn button_edges_shift_appear_and_disappear_without_replacing_the_schedule() {
        let (mut parent, _) = parent_tape(24);
        for frame in 10..12 {
            parent.frames[frame].pads[0].buttons |= 0x0100;
        }
        parent.frames[7].pads[0].buttons |= 0x0200;
        let parent_bytes = parent.encode().unwrap();
        let candidate = sealed(
            &parent_bytes,
            vec![],
            vec![
                ButtonResidual {
                    port: 0,
                    buttons: 0x0100,
                    start_frame: 10,
                    duration_frames: 2,
                    mode: ButtonResidualMode::Release,
                },
                ButtonResidual {
                    port: 0,
                    buttons: 0x0100,
                    start_frame: 11,
                    duration_frames: 2,
                    mode: ButtonResidualMode::Press,
                },
                ButtonResidual {
                    port: 0,
                    buttons: 0x0200,
                    start_frame: 7,
                    duration_frames: 1,
                    mode: ButtonResidualMode::Release,
                },
                ButtonResidual {
                    port: 0,
                    buttons: 0x1000,
                    start_frame: 18,
                    duration_frames: 1,
                    mode: ButtonResidualMode::Press,
                },
            ],
        );
        let compiled = compile_residual_candidate(&parent, &parent_bytes, &candidate).unwrap();
        assert_eq!(compiled.tape.frames[7].pads[0].buttons & 0x0200, 0);
        assert_eq!(compiled.tape.frames[10].pads[0].buttons & 0x0100, 0);
        assert_eq!(compiled.tape.frames[11].pads[0].buttons & 0x0100, 0x0100);
        assert_eq!(compiled.tape.frames[12].pads[0].buttons & 0x0100, 0x0100);
        assert_eq!(compiled.tape.frames[13].pads[0].buttons & 0x0100, 0);
        assert_eq!(compiled.tape.frames[18].pads[0].buttons & 0x1000, 0x1000);
        assert_eq!(
            compiled.report.intervention_span,
            InterventionSpan {
                start_frame: 7,
                end_frame_exclusive: 19
            }
        );
    }

    #[test]
    fn several_early_and_downstream_repairs_share_one_candidate_and_exact_span() {
        let (parent, parent_bytes) = parent_tape(80);
        let candidate = sealed(
            &parent_bytes,
            vec![
                analog(
                    AnalogChannel::MainX,
                    TemporalBasis::Window {
                        start_frame: 2,
                        window: ResidualWindow::Frames4,
                        delta: 11,
                    },
                ),
                analog(
                    AnalogChannel::MainY,
                    TemporalBasis::PiecewiseLinearRamp {
                        start_frame: 30,
                        end_frame: 38,
                        start_delta: 0,
                        end_delta: 40,
                    },
                ),
                analog(
                    AnalogChannel::CameraX,
                    TemporalBasis::CubicControlCurve {
                        start_frame: 60,
                        end_frame: 70,
                        control_deltas: [10, -20, 20, -10],
                    },
                ),
            ],
            vec![ButtonResidual {
                port: 0,
                buttons: 0x0100,
                start_frame: 45,
                duration_frames: 2,
                mode: ButtonResidualMode::Press,
            }],
        );
        let compiled = compile_residual_candidate(&parent, &parent_bytes, &candidate).unwrap();
        assert_eq!(compiled.report.analog_residuals, 3);
        assert_eq!(compiled.report.button_residuals, 1);
        assert_eq!(
            compiled.report.intervention_span,
            InterventionSpan {
                start_frame: 2,
                end_frame_exclusive: 71
            }
        );
        assert_eq!(compiled.report.parent_tape_sha256, sha256(&parent_bytes));
    }

    #[test]
    fn exploration_horizon_is_an_authoritative_released_pad_tail() {
        let (mut parent, _) = parent_tape(40);
        parent.frames[39].pads[0].buttons = 0x0100;
        parent.frames[39].pads[0].stick_x = 72;
        parent.frames[39].pads[0].connected = true;
        let parent_bytes = parent.encode().unwrap();
        let candidate = sealed(
            &parent_bytes,
            vec![analog(
                AnalogChannel::MainY,
                TemporalBasis::ExactFrame { frame: 3, delta: 8 },
            )],
            vec![],
        );
        let compiled =
            compile_residual_candidate_to_horizon(&parent, &parent_bytes, &candidate, 64).unwrap();
        assert_eq!(compiled.report.frame_count, 64);
        assert_eq!(compiled.tape.frames.len(), 64);
        assert_eq!(compiled.tape.frames[39].pads[0].buttons, 0x0100);
        for frame in &compiled.tape.frames[40..] {
            assert_eq!(frame.owned_ports, 1);
            assert_eq!(frame.wait_condition, WaitCondition::None);
            assert_eq!(frame.pads[0].buttons, 0);
            assert_eq!(frame.pads[0].stick_x, 0);
            assert!(frame.pads[0].connected);
        }
        assert_eq!(
            InputTape::decode(&compiled.bytes).unwrap().tape,
            compiled.tape
        );
        assert_eq!(
            compiled.report.realized_tape_sha256,
            sha256(&compiled.bytes)
        );
        assert!(
            compile_residual_candidate_to_horizon(&parent, &parent_bytes, &candidate, 39)
                .unwrap_err()
                .to_string()
                .contains("cannot truncate")
        );
    }

    #[test]
    fn deduplication_uses_compiled_raw_tape_not_residual_parameters() {
        let (mut parent, _) = parent_tape(8);
        parent.frames[3].pads[0].stick_x = 100;
        let parent_bytes = parent.encode().unwrap();
        let first = sealed(
            &parent_bytes,
            vec![analog(
                AnalogChannel::MainX,
                TemporalBasis::ExactFrame {
                    frame: 3,
                    delta: 100,
                },
            )],
            vec![],
        );
        let second = sealed(
            &parent_bytes,
            vec![analog(
                AnalogChannel::MainX,
                TemporalBasis::ExactFrame {
                    frame: 3,
                    delta: 200,
                },
            )],
            vec![],
        );
        assert_ne!(first.content_sha256, second.content_sha256);
        let deduplicated =
            compile_unique_residual_candidates(&parent, &parent_bytes, &[first.clone(), second])
                .unwrap();
        assert_eq!(deduplicated.unique.len(), 1);
        assert_eq!(deduplicated.aliases.len(), 1);
        assert_eq!(
            deduplicated.aliases[0].retained_candidate_sha256,
            first.content_sha256
        );
    }

    #[test]
    fn button_precedence_and_detached_unowned_wait_or_out_of_bounds_edits_are_exact() {
        let (mut parent, _) = parent_tape(8);
        let parent_bytes = parent.encode().unwrap();
        let overlapping = sealed(
            &parent_bytes,
            vec![],
            vec![
                ButtonResidual {
                    port: 0,
                    buttons: 0x0100,
                    start_frame: 2,
                    duration_frames: 3,
                    mode: ButtonResidualMode::Press,
                },
                ButtonResidual {
                    port: 0,
                    buttons: 0x0100,
                    start_frame: 3,
                    duration_frames: 1,
                    mode: ButtonResidualMode::Release,
                },
            ],
        );
        let overlap_compiled =
            compile_residual_candidate(&parent, &parent_bytes, &overlapping).unwrap();
        assert_eq!(
            overlap_compiled.tape.frames[3].pads[0].buttons & 0x0100,
            0x0100
        );

        let invalid_button = ResidualCandidate::seal(
            &parent_bytes,
            vec![],
            vec![ButtonResidual {
                port: 0,
                buttons: 0x8000,
                start_frame: 2,
                duration_frames: 1,
                mode: ButtonResidualMode::Press,
            }],
        )
        .unwrap_err();
        assert!(invalid_button.to_string().contains("authentic"));

        let candidate = sealed(
            &parent_bytes,
            vec![analog(
                AnalogChannel::MainX,
                TemporalBasis::ExactFrame { frame: 2, delta: 1 },
            )],
            vec![],
        );
        let mut tampered = candidate.clone();
        tampered.parent_tape_sha256 = Digest([9; 32]);
        assert!(compile_residual_candidate(&parent, &parent_bytes, &tampered).is_err());

        parent.frames[2].owned_ports = 0;
        let unowned_bytes = parent.encode().unwrap();
        let unowned = sealed(
            &unowned_bytes,
            vec![analog(
                AnalogChannel::MainX,
                TemporalBasis::ExactFrame { frame: 2, delta: 1 },
            )],
            vec![],
        );
        assert!(compile_residual_candidate(&parent, &unowned_bytes, &unowned).is_err());

        parent.frames[2].owned_ports = 1;
        parent.frames[2].wait_condition = WaitCondition::NameEntryActive;
        parent.frames[2].wait_timeout_ticks = 1;
        let wait_bytes = parent.encode().unwrap();
        let wait = sealed(
            &wait_bytes,
            vec![analog(
                AnalogChannel::MainX,
                TemporalBasis::ExactFrame { frame: 2, delta: 1 },
            )],
            vec![],
        );
        assert!(compile_residual_candidate(&parent, &wait_bytes, &wait).is_err());

        let (parent, parent_bytes) = parent_tape(8);
        let outside = sealed(
            &parent_bytes,
            vec![analog(
                AnalogChannel::MainX,
                TemporalBasis::ExactFrame { frame: 8, delta: 1 },
            )],
            vec![],
        );
        assert!(compile_residual_candidate(&parent, &parent_bytes, &outside).is_err());
    }

    #[test]
    fn checked_optimization_request_schema_identity_matches_the_implementation() {
        assert_eq!(
            residual_proposal_schema_sha256().to_string(),
            "8b11beebb1528ebb225f0366c997cd32171b10d596c0f1c4b6ffa740879e2313"
        );
    }

    #[test]
    fn checked_ordon_incumbent_is_a_lossless_residual_parent() {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap();
        let parent_bytes = fs::read(
            repository.join("routes/Glitch Exhibition/intro/segments/to_ordon_spring_q125.tape"),
        )
        .unwrap();
        let parent = InputTape::decode(&parent_bytes).unwrap().tape;
        assert_eq!(parent.encode().unwrap(), parent_bytes);
        let frame = parent
            .frames
            .iter()
            .position(|frame| {
                frame.owned_ports & 1 != 0 && frame.wait_condition == WaitCondition::None
            })
            .unwrap();
        let delta = if parent.frames[frame].pads[0].stick_x == i8::MAX {
            -1
        } else {
            1
        };
        let candidate = sealed(
            &parent_bytes,
            vec![analog(
                AnalogChannel::MainX,
                TemporalBasis::ExactFrame {
                    frame: frame as u64,
                    delta,
                },
            )],
            vec![],
        );
        let compiled = compile_residual_candidate(&parent, &parent_bytes, &candidate).unwrap();
        assert_eq!(compiled.report.frame_count, parent.frames.len() as u64);
        assert_eq!(
            compiled.report.intervention_span,
            InterventionSpan {
                start_frame: frame as u64,
                end_frame_exclusive: frame as u64 + 1,
            }
        );
        assert_eq!(
            InputTape::decode(&compiled.bytes).unwrap().tape,
            compiled.tape
        );
    }
}
