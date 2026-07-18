//! Exploratory offline-RL bridge for absolute F_SP103 movement traces.
//!
//! This module deliberately makes a narrow claim: it converts an already
//! executed, absolute 30 Hz input tape and its gameplay trace into observed
//! one-tick transitions.  It does not restore snapshots, infer milestones,
//! expose counterfactual actions, or claim that the trace is a complete Markov
//! state.  In particular, gameplay trace v1 omits per-tick RNG, collision,
//! camera, animation, and most Link procedure internals.
//!
//! Trace records are post-tick observations.  For action tape frame `i`, the
//! transition is therefore `trace[i - 1] --tape[i]--> trace[i]`.  Keeping this
//! relationship explicit prevents the easy one-frame label shift that would
//! otherwise poison a fitted-Q batch.

use crate::artifact::Digest;
use crate::tape::{InputFrame, InputTape, RawPadState, WaitCondition};
use crate::trace::{self, DecodedTrace, TraceChannel, TraceChannelStatus, TracePhase, TraceRecord};
use crate::transition_corpus::{
    MacroAction, StateReference, StateReferenceKind, Transition, TransitionCorpus,
    TransitionCorpusError,
};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::f32::consts::PI;
use std::fmt;

const TRACE_HEADER_SIZE: usize = 36;
const BUTTON_A: u16 = 0x0100;
const BUTTON_B: u16 = 0x0200;
const BUTTON_A_B: u16 = BUTTON_A | BUTTON_B;
const ACTION_MACRO_KIND_PAD_FRAME_V2: u16 = 2;

/// Stable descriptor hashed into every corpus produced by this bridge.
///
/// The ordered vector has 49 entries:
///
/// 0..8 stage bytes / 255; 8 room; 9 layer; 10 point; 11 player-present;
/// 12 player-is-Link; 13 event-running; 14 tape-playing; 15 player actor;
/// 16 player procedure (-1 when absent); 17..20 position / 8192;
/// 20..23 velocity / 64; 23 forward speed / 64; 24..30 sine/cosine of current
/// yaw, shape yaw, and their wrapped delta; 30..32 prior applied button bytes
/// / 255; 32..34 prior applied stick / 127; 34 pad error; 35 event ID;
/// 36 event mode / 255; 37 event status / 255; 38 map-tool ID / 255;
/// 39..41 event-name hash halves / 65535; 41 nearest-exit-present;
/// 42 nearest-exit actor (-1 when absent); 43..46 its player-relative position
/// / 8192; 46 its distance / 8192 (-1/8192 when absent); 47 elapsed ticks from
/// the configured action start / 1024; 48 remaining configured ticks / 1024.
///
/// Integer-valued categorical entries are serialized as exact f32 values; a
/// learner should use equality/category handling rather than assume ordering.
pub const MOVEMENT_FEATURE_COUNT_V1: u32 = 49;
/// Feature slots which the native trees must split as categories rather than
/// ordered continuous quantities. This list is valid only with the exact v1
/// feature-schema digest.
pub const MOVEMENT_CATEGORICAL_FEATURES_V1: &[usize] = &[
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 30, 31, 34, 35, 36, 37, 38, 39, 40,
    41, 42,
];
pub const MOVEMENT_FEATURE_SCHEMA_V1: &str = concat!(
    "dusklight.offline-rl.movement-state/v1;",
    "stage_u8x8_div255,room_i8,layer_i8,point_i16,",
    "player_present_bool,player_is_link_bool,event_running_bool,tape_playing_bool,",
    "player_actor_i16,player_proc_i32_missing_minus1,",
    "position_xyz_div8192,velocity_xyz_div64,forward_speed_div64,",
    "current_yaw_sin_cos,shape_yaw_sin_cos,current_minus_shape_yaw_sin_cos,",
    "prior_buttons_le_bytes_div255,prior_stick_xy_div127,pad_error_i8,",
    "event_id_i16,event_mode_u8_div255,event_status_u8_div255,",
    "event_map_tool_u8_div255,event_hash_le_u16x2_div65535,",
    "nearest_exit_present_bool,nearest_exit_actor_i16_missing_minus1,",
    "nearest_exit_relative_xyz_div8192,nearest_exit_distance_div8192_missing_minus1,",
    "elapsed_div1024,remaining_div1024"
);

/// The 68 discrete actions are the Cartesian product of four button states and
/// 17 direction states. Direction zero is neutral; directions 1..=16 are the
/// nearest post-PADClamp heading in clockwise 22.5-degree increments starting
/// at forward. Exact raw stick coordinates and buttons remain in the action
/// parameters so a proposal layer can recover or perturb the observed input.
pub const MOVEMENT_ACTION_SCHEMA_V2: &str = concat!(
    "dusklight.offline-rl.pad-frame/v2;duration=1;",
    "action_id=button_mode*17+direction;",
    "button_mode=0:none,1:a,2:b,3:a+b;",
    "direction=0:clamped_neutral,1+k:nearest_heading_k;",
    "k=0..15;heading_x=round(sin(k*pi/8)*127);",
    "heading_y=round(cos(k*pi/8)*127);",
    "parameters=raw_stick_x_i16,raw_stick_y_i16,buttons_i16"
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExploratoryExtractConfig {
    /// Content identity of the build/scenario/run which produced the pair.
    pub episode_digest: Digest,
    /// First commanded tape frame, inclusive. Must be at least one because its
    /// pre-state is the preceding trace record.
    pub start_tape_frame: u64,
    /// Last commanded tape frame, inclusive.
    pub end_tape_frame: u64,
    /// Optional authoritative reference for trace[start - 1].
    pub start_reference: Option<StateReference>,
    /// Optional authoritative reference for trace[end].
    pub terminal_reference: Option<StateReference>,
    /// Whether the final extracted transition is a semantic terminal. An
    /// arbitrary crop or tape end must leave this false.
    pub end_is_terminal: bool,
}

#[derive(Debug)]
pub enum OfflineRlError {
    Trace(trace::TraceError),
    Tape(crate::tape::TapeError),
    Corpus(TransitionCorpusError),
    TraceHeader(&'static str),
    TraceRate {
        numerator: u32,
        denominator: u32,
    },
    TapeRate {
        numerator: u32,
        denominator: u32,
    },
    CapacityExhausted,
    MissingEpisodeDigest,
    InvalidRange {
        start: u64,
        end: u64,
    },
    TapeFrameOutOfRange(u64),
    MissingTraceFrame(u64),
    DuplicateTraceFrame(u64),
    DiscontinuousTrace {
        prior_tape_frame: u64,
        next_tape_frame: u64,
        prior_sim_tick: u64,
        next_sim_tick: u64,
    },
    InvalidObservationPhase {
        frame: u64,
        phase: TracePhase,
    },
    InvalidBoundary {
        frame: u64,
        boundary_index: u64,
        simulation_tick: u64,
    },
    MissingObservationChannel {
        frame: u64,
        channel: &'static str,
        status: Option<TraceChannelStatus>,
    },
    InputProvenance {
        frame: u64,
        input_source: u8,
    },
    ReactiveFrame(u64),
    PortZeroNotOwned(u64),
    UnsupportedSecondaryPad {
        frame: u64,
        port: usize,
    },
    UnsupportedPrimaryPad(u64),
    UnsupportedAction {
        frame: u64,
        buttons: u16,
        stick_x: i8,
        stick_y: i8,
    },
    AppliedInputMismatch {
        frame: u64,
        expected_buttons: u16,
        actual_buttons: u16,
        expected_stick_x: i8,
        expected_stick_y: i8,
        actual_stick_x: i8,
        actual_stick_y: i8,
    },
    NonFiniteFeature {
        frame: u64,
        index: usize,
    },
    FrameIndexOverflow,
}

impl fmt::Display for OfflineRlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Trace(error) => write!(formatter, "gameplay trace error: {error}"),
            Self::Tape(error) => write!(formatter, "input tape error: {error}"),
            Self::Corpus(error) => write!(formatter, "transition corpus error: {error}"),
            Self::TraceHeader(message) => {
                write!(formatter, "invalid gameplay trace header: {message}")
            }
            Self::TraceRate {
                numerator,
                denominator,
            } => write!(
                formatter,
                "gameplay trace rate is {numerator}/{denominator}; expected 30/1"
            ),
            Self::TapeRate {
                numerator,
                denominator,
            } => write!(
                formatter,
                "input tape rate is {numerator}/{denominator}; expected 30/1"
            ),
            Self::CapacityExhausted => formatter.write_str("gameplay trace capacity was exhausted"),
            Self::MissingEpisodeDigest => formatter.write_str("episode digest must not be zero"),
            Self::InvalidRange { start, end } => write!(
                formatter,
                "invalid inclusive action range {start}..={end}; start must be nonzero and <= end"
            ),
            Self::TapeFrameOutOfRange(frame) => {
                write!(formatter, "tape frame {frame} is out of range")
            }
            Self::MissingTraceFrame(frame) => {
                write!(formatter, "gameplay trace has no tape frame {frame}")
            }
            Self::DuplicateTraceFrame(frame) => {
                write!(formatter, "gameplay trace repeats tape frame {frame}")
            }
            Self::DiscontinuousTrace {
                prior_tape_frame,
                next_tape_frame,
                prior_sim_tick,
                next_sim_tick,
            } => write!(
                formatter,
                "trace is discontinuous from tape/sim {prior_tape_frame}/{prior_sim_tick} to {next_tape_frame}/{next_sim_tick}"
            ),
            Self::InvalidObservationPhase { frame, phase } => write!(
                formatter,
                "trace frame {frame} has {phase:?} observations; movement extraction requires post-simulation"
            ),
            Self::InvalidBoundary {
                frame,
                boundary_index,
                simulation_tick,
            } => write!(
                formatter,
                "trace frame {frame} has boundary {boundary_index} for completed simulation tick {simulation_tick}"
            ),
            Self::MissingObservationChannel {
                frame,
                channel,
                status,
            } => write!(
                formatter,
                "trace frame {frame} requires present {channel} observations, found {status:?}"
            ),
            Self::InputProvenance {
                frame,
                input_source,
            } => write!(
                formatter,
                "trace frame {frame} has input source 0x{input_source:02x}; absolute tape extraction requires tape-only input"
            ),
            Self::ReactiveFrame(frame) => write!(formatter, "tape frame {frame} is reactive"),
            Self::PortZeroNotOwned(frame) => {
                write!(formatter, "tape frame {frame} does not own port zero")
            }
            Self::UnsupportedSecondaryPad { frame, port } => write!(
                formatter,
                "tape frame {frame} has non-neutral secondary port {port}"
            ),
            Self::UnsupportedPrimaryPad(frame) => write!(
                formatter,
                "tape frame {frame} uses unsupported primary-pad fields"
            ),
            Self::UnsupportedAction {
                frame,
                buttons,
                stick_x,
                stick_y,
            } => write!(
                formatter,
                "tape frame {frame} action buttons=0x{buttons:04x} stick=({stick_x},{stick_y}) is outside the v2 catalog"
            ),
            Self::AppliedInputMismatch {
                frame,
                expected_buttons,
                actual_buttons,
                expected_stick_x,
                expected_stick_y,
                actual_stick_x,
                actual_stick_y,
            } => write!(
                formatter,
                "tape frame {frame} expected applied buttons/stick 0x{expected_buttons:04x}/({expected_stick_x},{expected_stick_y}), trace has 0x{actual_buttons:04x}/({actual_stick_x},{actual_stick_y})"
            ),
            Self::NonFiniteFeature { frame, index } => write!(
                formatter,
                "trace frame {frame} produced non-finite feature {index}"
            ),
            Self::FrameIndexOverflow => {
                formatter.write_str("tape frame cannot be represented on this host")
            }
        }
    }
}

impl Error for OfflineRlError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Trace(error) => Some(error),
            Self::Tape(error) => Some(error),
            Self::Corpus(error) => Some(error),
            _ => None,
        }
    }
}

impl From<trace::TraceError> for OfflineRlError {
    fn from(value: trace::TraceError) -> Self {
        Self::Trace(value)
    }
}

impl From<crate::tape::TapeError> for OfflineRlError {
    fn from(value: crate::tape::TapeError) -> Self {
        Self::Tape(value)
    }
}

impl From<TransitionCorpusError> for OfflineRlError {
    fn from(value: TransitionCorpusError) -> Self {
        Self::Corpus(value)
    }
}

pub fn movement_feature_schema_digest_v1() -> Digest {
    digest(MOVEMENT_FEATURE_SCHEMA_V1.as_bytes())
}

pub fn movement_action_schema_digest_v2() -> Digest {
    digest(MOVEMENT_ACTION_SCHEMA_V2.as_bytes())
}

/// Decodes and validates both artifacts before extracting exploratory data.
/// Unlike `trace::decode`, this also validates the trace header tick rate.
pub fn extract_exploratory_from_bytes(
    trace_bytes: &[u8],
    tape_bytes: &[u8],
    config: ExploratoryExtractConfig,
) -> Result<TransitionCorpus, OfflineRlError> {
    validate_trace_rate(trace_bytes)?;
    let decoded_trace = trace::decode(trace_bytes)?;
    let decoded_tape = InputTape::decode(tape_bytes)?;
    extract_exploratory(&decoded_trace, &decoded_tape.tape, config)
}

/// Lower-level bridge for callers which already decoded and rate-validated a
/// trace. Prefer `extract_exploratory_from_bytes` for untrusted artifacts.
pub fn extract_exploratory(
    decoded_trace: &DecodedTrace,
    tape: &InputTape,
    config: ExploratoryExtractConfig,
) -> Result<TransitionCorpus, OfflineRlError> {
    if decoded_trace.capacity_exhausted {
        return Err(OfflineRlError::CapacityExhausted);
    }
    if config.episode_digest == Digest::ZERO {
        return Err(OfflineRlError::MissingEpisodeDigest);
    }
    tape.validate()?;
    if tape.tick_rate_numerator != 30 || tape.tick_rate_denominator != 1 {
        return Err(OfflineRlError::TapeRate {
            numerator: tape.tick_rate_numerator,
            denominator: tape.tick_rate_denominator,
        });
    }
    if config.start_tape_frame == 0 || config.start_tape_frame > config.end_tape_frame {
        return Err(OfflineRlError::InvalidRange {
            start: config.start_tape_frame,
            end: config.end_tape_frame,
        });
    }
    let end_index =
        usize::try_from(config.end_tape_frame).map_err(|_| OfflineRlError::FrameIndexOverflow)?;
    if end_index >= tape.frames.len() {
        return Err(OfflineRlError::TapeFrameOutOfRange(config.end_tape_frame));
    }

    let records = records_by_tape_frame(decoded_trace)?;
    let first_state_frame = config.start_tape_frame - 1;
    let mut prior = record_at(&records, first_state_frame)?;
    validate_movement_observation(decoded_trace, prior, first_state_frame)?;
    let mut transitions = Vec::with_capacity(
        usize::try_from(config.end_tape_frame - config.start_tape_frame + 1)
            .map_err(|_| OfflineRlError::FrameIndexOverflow)?,
    );

    for action_frame in config.start_tape_frame..=config.end_tape_frame {
        let next = record_at(&records, action_frame)?;
        validate_movement_observation(decoded_trace, next, action_frame)?;
        if next.simulation_tick != prior.simulation_tick + 1 {
            return Err(OfflineRlError::DiscontinuousTrace {
                prior_tape_frame: action_frame - 1,
                next_tape_frame: action_frame,
                prior_sim_tick: prior.simulation_tick,
                next_sim_tick: next.simulation_tick,
            });
        }
        let action_index =
            usize::try_from(action_frame).map_err(|_| OfflineRlError::FrameIndexOverflow)?;
        let input_frame = &tape.frames[action_index];
        let action = classify_action(input_frame, action_frame)?;
        verify_applied_input(input_frame, next, action_frame)?;
        let state = movement_features(prior, first_state_frame, config.end_tape_frame)?;
        let next_state = movement_features(next, first_state_frame, config.end_tape_frame)?;
        let source = if action_frame == config.start_tape_frame {
            config.start_reference.unwrap_or_else(|| {
                derived_reference(
                    config.episode_digest,
                    first_state_frame,
                    prior.simulation_tick,
                    &state,
                )
            })
        } else {
            derived_reference(
                config.episode_digest,
                action_frame - 1,
                prior.simulation_tick,
                &state,
            )
        };
        let next_reference = if action_frame == config.end_tape_frame {
            config.terminal_reference.unwrap_or_else(|| {
                derived_reference(
                    config.episode_digest,
                    action_frame,
                    next.simulation_tick,
                    &next_state,
                )
            })
        } else {
            derived_reference(
                config.episode_digest,
                action_frame,
                next.simulation_tick,
                &next_state,
            )
        };
        transitions.push(Transition {
            source,
            state,
            action,
            duration_ticks: 1,
            reward: -1.0,
            next: next_reference,
            next_state,
            terminal: config.end_is_terminal && action_frame == config.end_tape_frame,
        });
        prior = next;
    }

    Ok(TransitionCorpus::new(
        movement_feature_schema_digest_v1(),
        movement_action_schema_digest_v2(),
        MOVEMENT_FEATURE_COUNT_V1,
        transitions,
    )?)
}

fn validate_movement_observation(
    trace: &DecodedTrace,
    record: &TraceRecord,
    frame: u64,
) -> Result<(), OfflineRlError> {
    if record.observation_phase != TracePhase::PostSimulation {
        return Err(OfflineRlError::InvalidObservationPhase {
            frame,
            phase: record.observation_phase,
        });
    }
    if record.simulation_tick == u64::MAX || record.boundary_index != record.simulation_tick + 1 {
        return Err(OfflineRlError::InvalidBoundary {
            frame,
            boundary_index: record.boundary_index,
            simulation_tick: record.simulation_tick,
        });
    }
    if !record.tape_input_applied() || record.controller_input_applied() {
        return Err(OfflineRlError::InputProvenance {
            frame,
            input_source: record.input_source,
        });
    }
    for channel in [
        TraceChannel::Core,
        TraceChannel::Stage,
        TraceChannel::AppliedPads,
        TraceChannel::PlayerMotion,
        TraceChannel::Event,
    ] {
        let status = record.channel_status.get(&channel).copied();
        if status != Some(TraceChannelStatus::Present) {
            return Err(OfflineRlError::MissingObservationChannel {
                frame,
                channel: channel.name(),
                status,
            });
        }
    }
    let exit_status = record.channel_status.get(&TraceChannel::SceneExit).copied();
    if !matches!(
        exit_status,
        Some(TraceChannelStatus::Present) | Some(TraceChannelStatus::Absent)
    ) {
        return Err(OfflineRlError::MissingObservationChannel {
            frame,
            channel: TraceChannel::SceneExit.name(),
            status: exit_status,
        });
    }
    if !record.player_is_link() {
        return Err(OfflineRlError::MissingObservationChannel {
            frame,
            channel: "player_motion(link)",
            status: record
                .channel_status
                .get(&TraceChannel::PlayerMotion)
                .copied(),
        });
    }
    if !record.event_name_hash_present {
        return Err(OfflineRlError::MissingObservationChannel {
            frame,
            channel: "event.name_hash(movement-state/v1)",
            status: record.channel_status.get(&TraceChannel::Event).copied(),
        });
    }
    if trace.version == 2 {
        let pads =
            record
                .applied_pads
                .as_ref()
                .ok_or(OfflineRlError::MissingObservationChannel {
                    frame,
                    channel: TraceChannel::AppliedPads.name(),
                    status: record
                        .channel_status
                        .get(&TraceChannel::AppliedPads)
                        .copied(),
                })?;
        if pads.owned_ports & 1 == 0 || pads.valid_ports & 1 == 0 {
            return Err(OfflineRlError::InputProvenance {
                frame,
                input_source: record.input_source,
            });
        }
    }
    Ok(())
}

fn validate_trace_rate(bytes: &[u8]) -> Result<(), OfflineRlError> {
    if bytes.len() < TRACE_HEADER_SIZE {
        return Err(OfflineRlError::TraceHeader("truncated"));
    }
    let numerator = u32::from_le_bytes(bytes[12..16].try_into().expect("fixed slice"));
    let denominator = u32::from_le_bytes(bytes[16..20].try_into().expect("fixed slice"));
    if numerator != 30 || denominator != 1 {
        return Err(OfflineRlError::TraceRate {
            numerator,
            denominator,
        });
    }
    Ok(())
}

fn records_by_tape_frame(
    trace: &DecodedTrace,
) -> Result<BTreeMap<u64, &TraceRecord>, OfflineRlError> {
    let mut records = BTreeMap::new();
    for record in &trace.records {
        let Some(frame) = record.tape_frame else {
            continue;
        };
        if records.insert(frame, record).is_some() {
            return Err(OfflineRlError::DuplicateTraceFrame(frame));
        }
    }
    Ok(records)
}

fn record_at<'a>(
    records: &'a BTreeMap<u64, &'a TraceRecord>,
    frame: u64,
) -> Result<&'a TraceRecord, OfflineRlError> {
    records
        .get(&frame)
        .copied()
        .ok_or(OfflineRlError::MissingTraceFrame(frame))
}

fn classify_action(frame: &InputFrame, frame_index: u64) -> Result<MacroAction, OfflineRlError> {
    if frame.wait_condition != WaitCondition::None || frame.wait_timeout_ticks != 0 {
        return Err(OfflineRlError::ReactiveFrame(frame_index));
    }
    if frame.owned_ports & 1 == 0 {
        return Err(OfflineRlError::PortZeroNotOwned(frame_index));
    }
    for (port, pad) in frame.pads.iter().enumerate().skip(1) {
        let owned = frame.owned_ports & (1 << port) != 0;
        let active = pad.buttons != 0
            || pad.stick_x != 0
            || pad.stick_y != 0
            || pad.substick_x != 0
            || pad.substick_y != 0
            || pad.trigger_left != 0
            || pad.trigger_right != 0
            || pad.analog_a != 0
            || pad.analog_b != 0;
        if owned && active {
            return Err(OfflineRlError::UnsupportedSecondaryPad {
                frame: frame_index,
                port,
            });
        }
    }
    let pad = frame.pads[0];
    if pad.substick_x != 0
        || pad.substick_y != 0
        || pad.trigger_left != 0
        || pad.trigger_right != 0
        || pad.analog_a != 0
        || pad.analog_b != 0
        || !pad.connected
        || pad.error != 0
    {
        return Err(OfflineRlError::UnsupportedPrimaryPad(frame_index));
    }

    let action_id = movement_action_id_v2(pad).ok_or(OfflineRlError::UnsupportedAction {
        frame: frame_index,
        buttons: pad.buttons,
        stick_x: pad.stick_x,
        stick_y: pad.stick_y,
    })?;

    Ok(MacroAction {
        action_id,
        macro_kind: ACTION_MACRO_KIND_PAD_FRAME_V2,
        parameters: vec![
            i16::from(pad.stick_x),
            i16::from(pad.stick_y),
            pad.buttons as i16,
        ],
    })
}

/// Maps an exact raw primary pad state into the learned v2 class while keeping
/// quantization semantics shared by extraction and proposal validation.
pub fn movement_action_id_v2(pad: RawPadState) -> Option<u32> {
    if pad.substick_x != 0
        || pad.substick_y != 0
        || pad.trigger_left != 0
        || pad.trigger_right != 0
        || pad.analog_a != 0
        || pad.analog_b != 0
        || !pad.connected
        || pad.error != 0
    {
        return None;
    }
    let button_mode = match pad.buttons {
        0 => 0,
        BUTTON_A => 1,
        BUTTON_B => 2,
        BUTTON_A_B => 3,
        _ => return None,
    };
    let clamped = pad_clamp_main_stick(pad.stick_x, pad.stick_y);
    let direction = if clamped == (0, 0) {
        0
    } else {
        1 + nearest_heading(clamped.0, clamped.1)
    };
    Some(button_mode * 17 + direction)
}

/// Predicts the exact primary buttons/stick which `JUTGamePad::read()` records
/// after Aurora's GameCube `PADClamp`. Buttons are unchanged; stick commands
/// pass through the SDK rectangular clamp before gameplay trace sampling.
fn verify_applied_input(
    frame: &InputFrame,
    record: &TraceRecord,
    frame_index: u64,
) -> Result<(), OfflineRlError> {
    let commanded = frame.pads[0];
    let (expected_stick_x, expected_stick_y) =
        pad_clamp_main_stick(commanded.stick_x, commanded.stick_y);
    if record.buttons != commanded.buttons
        || record.stick_x != expected_stick_x
        || record.stick_y != expected_stick_y
    {
        return Err(OfflineRlError::AppliedInputMismatch {
            frame: frame_index,
            expected_buttons: commanded.buttons,
            actual_buttons: record.buttons,
            expected_stick_x,
            expected_stick_y,
            actual_stick_x: record.stick_x,
            actual_stick_y: record.stick_y,
        });
    }
    Ok(())
}

/// Integer-for-integer port of Aurora's `ClampStick` for the main stick:
/// min=15, max=72, xy=40. Rust integer division and the C++ implementation
/// both truncate toward zero; all division here happens after absolute-value
/// conversion and is therefore non-negative.
fn pad_clamp_main_stick(stick_x: i8, stick_y: i8) -> (i8, i8) {
    const MIN: i32 = 15;
    const MAX: i32 = 72;
    const XY: i32 = 40;
    const RAW_MAX: i32 = i8::MAX as i32;

    let sign_x = if stick_x >= 0 { 1 } else { -1 };
    let sign_y = if stick_y >= 0 { 1 } else { -1 };
    let mut x = i32::from(stick_x).abs();
    let mut y = i32::from(stick_y).abs();
    x = if x <= MIN { 0 } else { x - MIN };
    y = if y <= MIN { 0 } else { y - MIN };
    if x == 0 && y == 0 {
        return (0, 0);
    }

    x = x * MAX / (RAW_MAX - MIN);
    y = y * MAX / (RAW_MAX - MIN);
    if XY * y <= XY * x {
        let diagonal = XY * x + (MAX - XY) * y;
        if XY * MAX < diagonal {
            x = XY * MAX * x / diagonal;
            y = XY * MAX * y / diagonal;
        }
    } else {
        let diagonal = XY * y + (MAX - XY) * x;
        if XY * MAX < diagonal {
            x = XY * MAX * x / diagonal;
            y = XY * MAX * y / diagonal;
        }
    }
    ((sign_x * x) as i8, (sign_y * y) as i8)
}

fn heading_stick(heading: u32) -> (i8, i8) {
    let radians = f64::from(heading) * std::f64::consts::PI / 8.0;
    (
        (radians.sin() * 127.0).round() as i8,
        (radians.cos() * 127.0).round() as i8,
    )
}

/// Canonical executable pad state for one v2 learned action class. Observed
/// transitions retain their exact raw parameters; this representative is used
/// only when a policy proposes a class which was not copied from an episode.
pub fn canonical_movement_pad_v2(action_id: u32) -> Option<RawPadState> {
    if action_id >= 4 * 17 {
        return None;
    }
    let button_mode = action_id / 17;
    let direction = action_id % 17;
    let buttons = match button_mode {
        0 => 0,
        1 => BUTTON_A,
        2 => BUTTON_B,
        3 => BUTTON_A_B,
        _ => unreachable!("button mode was range checked"),
    };
    let (stick_x, stick_y) = if direction == 0 {
        (0, 0)
    } else {
        heading_stick(direction - 1)
    };
    Some(RawPadState {
        buttons,
        stick_x,
        stick_y,
        ..RawPadState::default()
    })
}

fn nearest_heading(stick_x: i8, stick_y: i8) -> u32 {
    (0_u32..16)
        .max_by_key(|heading| {
            let candidate = heading_stick(*heading);
            i32::from(stick_x) * i32::from(candidate.0)
                + i32::from(stick_y) * i32::from(candidate.1)
        })
        .expect("the fixed heading catalog is non-empty")
}

fn movement_features(
    record: &TraceRecord,
    first_state_frame: u64,
    end_tape_frame: u64,
) -> Result<Vec<f32>, OfflineRlError> {
    let frame = record
        .tape_frame
        .ok_or(OfflineRlError::MissingTraceFrame(first_state_frame))?;
    let mut features = Vec::with_capacity(MOVEMENT_FEATURE_COUNT_V1 as usize);
    let mut stage = [0_u8; 8];
    for (destination, source) in stage.iter_mut().zip(record.stage_name.as_bytes()) {
        *destination = *source;
    }
    features.extend(stage.map(|byte| f32::from(byte) / 255.0));
    features.extend([
        f32::from(record.room),
        f32::from(record.layer),
        f32::from(record.point),
        bool_feature(record.player_present()),
        bool_feature(record.player_is_link()),
        bool_feature(record.event_running()),
        bool_feature(record.flags & (1 << 3) != 0),
        f32::from(record.player_actor_name),
        record.player_proc_id.map_or(-1.0, f32::from),
    ]);
    features.extend(record.position.map(|value| value / 8192.0));
    features.extend(record.velocity.map(|value| value / 64.0));
    features.push(record.forward_speed / 64.0);
    let current = yaw_radians(record.current_angle_y);
    let shape = yaw_radians(record.shape_angle_y);
    let delta = yaw_radians(record.current_angle_y.wrapping_sub(record.shape_angle_y));
    features.extend([
        current.sin(),
        current.cos(),
        shape.sin(),
        shape.cos(),
        delta.sin(),
        delta.cos(),
        f32::from(record.buttons as u8) / 255.0,
        f32::from((record.buttons >> 8) as u8) / 255.0,
        f32::from(record.stick_x) / 127.0,
        f32::from(record.stick_y) / 127.0,
        f32::from(record.pad_error),
        f32::from(record.event_id),
        f32::from(record.event_mode) / 255.0,
        f32::from(record.event_status) / 255.0,
        f32::from(record.event_map_tool_id) / 255.0,
        f32::from(record.event_name_hash as u16) / 65535.0,
        f32::from((record.event_name_hash >> 16) as u16) / 65535.0,
    ]);
    if let (Some(actor), Some(distance)) = (
        record.nearest_scene_exit_actor_name,
        record.nearest_scene_exit_distance,
    ) {
        features.push(1.0);
        features.push(f32::from(actor));
        features.extend(
            record
                .nearest_scene_exit_position
                .iter()
                .zip(record.position)
                .map(|(exit, player)| (exit - player) / 8192.0),
        );
        features.push(distance / 8192.0);
    } else {
        features.extend([0.0, -1.0, 0.0, 0.0, 0.0, -1.0 / 8192.0]);
    }
    features.push((frame - first_state_frame) as f32 / 1024.0);
    features.push(end_tape_frame.saturating_sub(frame) as f32 / 1024.0);

    debug_assert_eq!(features.len(), MOVEMENT_FEATURE_COUNT_V1 as usize);
    if let Some((index, _)) = features
        .iter()
        .enumerate()
        .find(|(_, value)| !value.is_finite())
    {
        return Err(OfflineRlError::NonFiniteFeature { frame, index });
    }
    Ok(features)
}

fn yaw_radians(value: i16) -> f32 {
    f32::from(value) * PI / 32768.0
}

fn bool_feature(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}

fn derived_reference(
    episode_digest: Digest,
    tape_frame: u64,
    simulation_tick: u64,
    features: &[f32],
) -> StateReference {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.offline-rl.observed-boundary/v1\0");
    hasher.update(episode_digest.as_bytes());
    hasher.update(movement_feature_schema_digest_v1().as_bytes());
    hasher.update(tape_frame.to_le_bytes());
    hasher.update(simulation_tick.to_le_bytes());
    for feature in features {
        hasher.update(feature.to_bits().to_le_bytes());
    }
    StateReference {
        kind: StateReferenceKind::Boundary,
        digest: Digest(hasher.finalize().into()),
    }
}

fn digest(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(frame: u64, x: f32) -> TraceRecord {
        let channel_status = [
            (TraceChannel::Core, TraceChannelStatus::Present),
            (TraceChannel::Stage, TraceChannelStatus::Present),
            (TraceChannel::AppliedPads, TraceChannelStatus::Present),
            (TraceChannel::PlayerMotion, TraceChannelStatus::Present),
            (TraceChannel::Event, TraceChannelStatus::Present),
            (TraceChannel::SceneExit, TraceChannelStatus::Absent),
        ]
        .into_iter()
        .collect();
        TraceRecord {
            boundary_index: 101 + frame,
            simulation_tick: 100 + frame,
            tape_frame: Some(frame),
            input_source: 1,
            channel_status,
            stage_name: "F_SP103".into(),
            room: 1,
            layer: 3,
            point: 1,
            flags: (1 << 0) | (1 << 1) | (1 << 3),
            player_actor_name: 253,
            current_angle_y: 0,
            shape_angle_y: 0,
            buttons: 0,
            stick_x: 0,
            stick_y: 0,
            position: [x, 800.0, -2300.0],
            velocity: [x, 0.0, 0.0],
            forward_speed: x,
            player_proc_id: Some(4),
            event_id: -1,
            event_mode: 0,
            event_status: 0,
            event_map_tool_id: 0xff,
            pad_error: 0,
            event_name_hash: 0,
            event_name_hash_present: true,
            nearest_scene_exit_actor_name: None,
            nearest_scene_exit_position: [0.0; 3],
            nearest_scene_exit_distance: None,
            ..TraceRecord::default()
        }
    }

    fn frame(stick_x: i8, stick_y: i8, buttons: u16) -> InputFrame {
        let mut frame = InputFrame {
            owned_ports: 1,
            ..InputFrame::default()
        };
        frame.pads[0] = RawPadState {
            stick_x,
            stick_y,
            buttons,
            ..RawPadState::default()
        };
        frame
    }

    fn fixture() -> (DecodedTrace, InputTape) {
        let mut first_applied = record(1, 20.0);
        first_applied.stick_y = 72;
        let mut second_applied = record(2, 30.0);
        second_applied.buttons = BUTTON_B;
        second_applied.stick_x = 72;
        (
            DecodedTrace {
                version: 1,
                tick_rate_numerator: 30,
                tick_rate_denominator: 1,
                requested_channels: 0,
                capacity_exhausted: false,
                records: vec![record(0, 10.0), first_applied, second_applied],
            },
            InputTape {
                frames: vec![frame(0, 0, 0), frame(0, 127, 0), frame(127, 0, BUTTON_B)],
                ..InputTape::default()
            },
        )
    }

    fn config(start_tape_frame: u64, end_tape_frame: u64) -> ExploratoryExtractConfig {
        ExploratoryExtractConfig {
            episode_digest: Digest([0x55; 32]),
            start_tape_frame,
            end_tape_frame,
            start_reference: None,
            terminal_reference: None,
            end_is_terminal: true,
        }
    }

    #[test]
    fn aligns_action_with_prior_and_post_tick_records() {
        let (trace, tape) = fixture();
        let corpus = extract_exploratory(&trace, &tape, config(1, 2)).unwrap();
        assert_eq!(corpus.transitions.len(), 2);
        assert_eq!(corpus.transitions[0].state[17], 10.0 / 8192.0);
        assert_eq!(corpus.transitions[0].next_state[17], 20.0 / 8192.0);
        assert_eq!(corpus.transitions[0].action.action_id, 1);
        assert_eq!(corpus.transitions[1].state[17], 20.0 / 8192.0);
        assert_eq!(corpus.transitions[1].next_state[17], 30.0 / 8192.0);
        assert_eq!(corpus.transitions[1].action.action_id, 39);
        assert!(!corpus.transitions[0].terminal);
        assert!(corpus.transitions[1].terminal);
    }

    #[test]
    fn preserves_supplied_endpoint_references() {
        let (trace, tape) = fixture();
        let start = StateReference {
            kind: StateReferenceKind::Snapshot,
            digest: Digest([0x11; 32]),
        };
        let end = StateReference {
            kind: StateReferenceKind::Boundary,
            digest: Digest([0x22; 32]),
        };
        let corpus = extract_exploratory(
            &trace,
            &tape,
            ExploratoryExtractConfig {
                episode_digest: Digest([0x55; 32]),
                start_reference: Some(start),
                terminal_reference: Some(end),
                ..config(1, 2)
            },
        )
        .unwrap();
        assert_eq!(corpus.transitions[0].source, start);
        assert_eq!(corpus.transitions[1].next, end);
    }

    #[test]
    fn crop_terminal_is_explicit_and_references_are_episode_scoped() {
        let (trace, tape) = fixture();
        let mut first_config = config(1, 1);
        first_config.end_is_terminal = false;
        let first = extract_exploratory(&trace, &tape, first_config).unwrap();
        assert!(!first.transitions[0].terminal);

        let mut second_config = first_config;
        second_config.episode_digest = Digest([0x66; 32]);
        let second = extract_exploratory(&trace, &tape, second_config).unwrap();
        assert_ne!(first.transitions[0].source, second.transitions[0].source);

        let mut missing = first_config;
        missing.episode_digest = Digest::ZERO;
        assert!(matches!(
            extract_exploratory(&trace, &tape, missing),
            Err(OfflineRlError::MissingEpisodeDigest)
        ));
    }

    #[test]
    fn held_b_remains_a_valid_controller_state() {
        let (mut trace, tape) = fixture();
        trace.records[1].buttons = BUTTON_B;
        let corpus = extract_exploratory(&trace, &tape, config(2, 2)).unwrap();
        assert_eq!(corpus.transitions[0].action.action_id, 39);
    }

    #[test]
    fn rejects_wrong_tape_even_when_both_actions_are_catalogued() {
        let (trace, mut tape) = fixture();
        tape.frames[1] = frame(127, 0, 0);
        assert!(matches!(
            extract_exploratory(&trace, &tape, config(1, 1)),
            Err(OfflineRlError::AppliedInputMismatch {
                frame: 1,
                expected_stick_x: 72,
                expected_stick_y: 0,
                actual_stick_x: 0,
                actual_stick_y: 72,
                ..
            })
        ));
    }

    #[test]
    fn rejects_post_tick_button_mismatch() {
        let (mut trace, tape) = fixture();
        trace.records[2].buttons = 0;
        assert!(matches!(
            extract_exploratory(&trace, &tape, config(2, 2)),
            Err(OfflineRlError::AppliedInputMismatch {
                frame: 2,
                expected_buttons: BUTTON_B,
                actual_buttons: 0,
                ..
            })
        ));
    }

    #[test]
    fn categorical_feature_indices_are_unique_and_in_range() {
        let mut indices = MOVEMENT_CATEGORICAL_FEATURES_V1.to_vec();
        assert!(
            indices
                .iter()
                .all(|index| *index < MOVEMENT_FEATURE_COUNT_V1 as usize)
        );
        indices.sort_unstable();
        indices.dedup();
        assert_eq!(indices.len(), MOVEMENT_CATEGORICAL_FEATURES_V1.len());
    }

    #[test]
    fn quantizes_arbitrary_stick_and_a_b_combinations_without_losing_raw_parameters() {
        let (mut trace, mut tape) = fixture();
        tape.frames[1].pads[0].stick_x = 70;
        tape.frames[1].pads[0].stick_y = 127;
        tape.frames[1].pads[0].buttons = BUTTON_A | BUTTON_B;
        let clamped = pad_clamp_main_stick(70, 127);
        trace.records[1].stick_x = clamped.0;
        trace.records[1].stick_y = clamped.1;
        trace.records[1].buttons = BUTTON_A | BUTTON_B;
        let corpus = extract_exploratory(&trace, &tape, config(1, 1)).unwrap();
        let action = &corpus.transitions[0].action;
        assert_eq!(action.action_id, 3 * 17 + 2);
        assert_eq!(action.parameters, [70, 127, (BUTTON_A | BUTTON_B) as i16]);
    }

    #[test]
    fn rejects_buttons_outside_the_movement_catalog() {
        let (trace, mut tape) = fixture();
        tape.frames[1].pads[0].buttons = 0x1000;
        assert!(matches!(
            extract_exploratory(&trace, &tape, config(1, 1)),
            Err(OfflineRlError::UnsupportedAction { frame: 1, .. })
        ));
    }

    #[test]
    fn every_v2_action_has_a_canonical_pad_in_its_own_class() {
        for action_id in 0..68 {
            let pad = canonical_movement_pad_v2(action_id).unwrap();
            let frame = InputFrame {
                owned_ports: 1,
                pads: [
                    pad,
                    RawPadState {
                        connected: false,
                        error: -1,
                        ..RawPadState::default()
                    },
                    RawPadState {
                        connected: false,
                        error: -1,
                        ..RawPadState::default()
                    },
                    RawPadState {
                        connected: false,
                        error: -1,
                        ..RawPadState::default()
                    },
                ],
                ..InputFrame::default()
            };
            assert_eq!(classify_action(&frame, 1).unwrap().action_id, action_id);
        }
        assert!(canonical_movement_pad_v2(68).is_none());
    }

    #[test]
    fn rejects_reactive_frame_and_trace_gap() {
        let (trace, mut tape) = fixture();
        tape.frames[1].wait_condition = WaitCondition::NameEntryActive;
        tape.frames[1].wait_timeout_ticks = 10;
        let config = config(1, 1);
        assert!(matches!(
            extract_exploratory(&trace, &tape, config),
            Err(OfflineRlError::ReactiveFrame(1))
        ));

        tape.frames[1].wait_condition = WaitCondition::None;
        tape.frames[1].wait_timeout_ticks = 0;
        let mut trace = trace;
        trace.records[1].simulation_tick += 1;
        trace.records[1].boundary_index += 1;
        assert!(matches!(
            extract_exploratory(&trace, &tape, config),
            Err(OfflineRlError::DiscontinuousTrace { .. })
        ));
    }

    #[test]
    fn rejects_capacity_exhaustion_and_zero_start() {
        let (mut trace, tape) = fixture();
        trace.capacity_exhausted = true;
        let valid_config = config(1, 1);
        assert!(matches!(
            extract_exploratory(&trace, &tape, valid_config),
            Err(OfflineRlError::CapacityExhausted)
        ));
        trace.capacity_exhausted = false;
        assert!(matches!(
            extract_exploratory(
                &trace,
                &tape,
                ExploratoryExtractConfig {
                    start_tape_frame: 0,
                    ..config(1, 1)
                }
            ),
            Err(OfflineRlError::InvalidRange { .. })
        ));
    }
}
