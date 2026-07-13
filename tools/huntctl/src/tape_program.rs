use crate::tape::{InputFrame, InputTape, PORT_COUNT, RawPadState};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;

pub const PROGRAM_SCHEMA: &str = "dusktape-program/v1";
pub const MAX_EXPANDED_FRAMES: usize = 10_000_000;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TapeProgram {
    pub schema: String,
    #[serde(default)]
    pub tick_rate: TickRate,
    #[serde(default = "default_owned_ports")]
    pub default_owned_ports: u8,
    pub steps: Vec<Step>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickRate {
    pub numerator: u32,
    pub denominator: u32,
}

impl Default for TickRate {
    fn default() -> Self {
        Self {
            numerator: 30,
            denominator: 1,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Frame { frame: FrameSpec },
    Repeat { count: u64, frame: FrameSpec },
    Hold { count: u64 },
    Marker { name: String },
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameSpec {
    #[serde(default)]
    pub owned_ports: Option<u8>,
    #[serde(default)]
    pub pads: BTreeMap<String, PadSpec>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PadSpec {
    #[serde(deserialize_with = "deserialize_buttons")]
    pub buttons: u16,
    pub stick_x: i8,
    pub stick_y: i8,
    pub substick_x: i8,
    pub substick_y: i8,
    pub trigger_left: u8,
    pub trigger_right: u8,
    pub analog_a: u8,
    pub analog_b: u8,
    pub connected: bool,
    pub error: i8,
}

impl Default for PadSpec {
    fn default() -> Self {
        let pad = RawPadState::default();
        Self {
            buttons: pad.buttons,
            stick_x: pad.stick_x,
            stick_y: pad.stick_y,
            substick_x: pad.substick_x,
            substick_y: pad.substick_y,
            trigger_left: pad.trigger_left,
            trigger_right: pad.trigger_right,
            analog_a: pad.analog_a,
            analog_b: pad.analog_b,
            connected: pad.connected,
            error: pad.error,
        }
    }
}

impl From<PadSpec> for RawPadState {
    fn from(value: PadSpec) -> Self {
        Self {
            buttons: value.buttons,
            stick_x: value.stick_x,
            stick_y: value.stick_y,
            substick_x: value.substick_x,
            substick_y: value.substick_y,
            trigger_left: value.trigger_left,
            trigger_right: value.trigger_right,
            analog_a: value.analog_a,
            analog_b: value.analog_b,
            connected: value.connected,
            error: value.error,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Marker {
    pub name: String,
    pub tick: u64,
}

#[derive(Clone, Debug)]
pub struct CompiledProgram {
    pub tape: InputTape,
    pub markers: Vec<Marker>,
}

#[derive(Debug)]
pub enum ProgramError {
    Json(serde_json::Error),
    UnsupportedSchema(String),
    InvalidTickRate,
    InvalidOwnedPorts(u8),
    InvalidPort(String),
    ZeroCount,
    HoldBeforeFrame,
    EmptyMarker,
    DuplicateMarker(String),
    TooManyFrames,
}

impl fmt::Display for ProgramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "invalid tape program JSON: {error}"),
            Self::UnsupportedSchema(schema) => {
                write!(f, "unsupported tape program schema {schema:?}")
            }
            Self::InvalidTickRate => {
                f.write_str("tick-rate numerator and denominator must be nonzero")
            }
            Self::InvalidOwnedPorts(mask) => {
                write!(f, "owned_ports mask 0x{mask:02x} addresses ports above 3")
            }
            Self::InvalidPort(port) => write!(f, "pad port {port:?} is outside 0..=3"),
            Self::ZeroCount => f.write_str("repeat and hold counts must be nonzero"),
            Self::HoldBeforeFrame => f.write_str("hold requires a previously emitted frame"),
            Self::EmptyMarker => f.write_str("marker names must not be empty"),
            Self::DuplicateMarker(name) => write!(f, "marker name {name:?} is duplicated"),
            Self::TooManyFrames => write!(f, "program expands beyond {MAX_EXPANDED_FRAMES} frames"),
        }
    }
}

impl Error for ProgramError {}
impl From<serde_json::Error> for ProgramError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl TapeProgram {
    pub fn from_json(source: &str) -> Result<Self, ProgramError> {
        Ok(serde_json::from_str(source)?)
    }

    pub fn compile(self) -> Result<CompiledProgram, ProgramError> {
        if self.schema != PROGRAM_SCHEMA {
            return Err(ProgramError::UnsupportedSchema(self.schema));
        }
        if self.tick_rate.numerator == 0 || self.tick_rate.denominator == 0 {
            return Err(ProgramError::InvalidTickRate);
        }
        validate_owned_ports(self.default_owned_ports)?;
        let mut frames: Vec<InputFrame> = Vec::new();
        let mut markers = Vec::new();
        let mut marker_names = HashSet::new();

        for step in self.steps {
            match step {
                Step::Frame { frame } => push_repeated(
                    &mut frames,
                    materialize(frame, self.default_owned_ports)?,
                    1,
                )?,
                Step::Repeat { count, frame } => {
                    push_repeated(
                        &mut frames,
                        materialize(frame, self.default_owned_ports)?,
                        count,
                    )?;
                }
                Step::Hold { count } => {
                    if count == 0 {
                        return Err(ProgramError::ZeroCount);
                    }
                    let frame = frames
                        .last()
                        .cloned()
                        .ok_or(ProgramError::HoldBeforeFrame)?;
                    push_repeated(&mut frames, frame, count)?;
                }
                Step::Marker { name } => {
                    if name.is_empty() {
                        return Err(ProgramError::EmptyMarker);
                    }
                    if !marker_names.insert(name.clone()) {
                        return Err(ProgramError::DuplicateMarker(name));
                    }
                    markers.push(Marker {
                        name,
                        tick: frames.len() as u64,
                    });
                }
            }
        }
        Ok(CompiledProgram {
            tape: InputTape {
                tick_rate_numerator: self.tick_rate.numerator,
                tick_rate_denominator: self.tick_rate.denominator,
                frames,
            },
            markers,
        })
    }
}

fn materialize(spec: FrameSpec, default_owned_ports: u8) -> Result<InputFrame, ProgramError> {
    let owned_ports = spec.owned_ports.unwrap_or(default_owned_ports);
    validate_owned_ports(owned_ports)?;
    let mut frame = InputFrame {
        owned_ports,
        ..InputFrame::default()
    };
    for (port_text, pad) in spec.pads {
        let port = port_text
            .parse::<usize>()
            .map_err(|_| ProgramError::InvalidPort(port_text.clone()))?;
        if port >= PORT_COUNT {
            return Err(ProgramError::InvalidPort(port_text));
        }
        frame.pads[port] = pad.into();
    }
    Ok(frame)
}

fn push_repeated(
    frames: &mut Vec<InputFrame>,
    frame: InputFrame,
    count: u64,
) -> Result<(), ProgramError> {
    if count == 0 {
        return Err(ProgramError::ZeroCount);
    }
    let count = usize::try_from(count).map_err(|_| ProgramError::TooManyFrames)?;
    let target = frames
        .len()
        .checked_add(count)
        .ok_or(ProgramError::TooManyFrames)?;
    if target > MAX_EXPANDED_FRAMES {
        return Err(ProgramError::TooManyFrames);
    }
    frames.reserve(count);
    frames.extend(std::iter::repeat_n(frame, count));
    Ok(())
}

fn validate_owned_ports(mask: u8) -> Result<(), ProgramError> {
    if mask & !0x0f == 0 {
        Ok(())
    } else {
        Err(ProgramError::InvalidOwnedPorts(mask))
    }
}

fn default_owned_ports() -> u8 {
    1
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ButtonInput {
    Mask(u16),
    Names(Vec<ButtonName>),
}

#[derive(Clone, Copy, Deserialize)]
enum ButtonName {
    #[serde(rename = "LEFT")]
    Left,
    #[serde(rename = "RIGHT")]
    Right,
    #[serde(rename = "DOWN")]
    Down,
    #[serde(rename = "UP")]
    Up,
    #[serde(rename = "Z")]
    Z,
    #[serde(rename = "R")]
    R,
    #[serde(rename = "L")]
    L,
    #[serde(rename = "A")]
    A,
    #[serde(rename = "B")]
    B,
    #[serde(rename = "X")]
    X,
    #[serde(rename = "Y")]
    Y,
    #[serde(rename = "START", alias = "MENU")]
    Start,
}

impl ButtonName {
    fn mask(self) -> u16 {
        match self {
            Self::Left => 0x0001,
            Self::Right => 0x0002,
            Self::Down => 0x0004,
            Self::Up => 0x0008,
            Self::Z => 0x0010,
            Self::R => 0x0020,
            Self::L => 0x0040,
            Self::A => 0x0100,
            Self::B => 0x0200,
            Self::X => 0x0400,
            Self::Y => 0x0800,
            Self::Start => 0x1000,
        }
    }
}

fn deserialize_buttons<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u16, D::Error> {
    let input = ButtonInput::deserialize(deserializer)?;
    Ok(match input {
        ButtonInput::Mask(mask) => mask,
        ButtonInput::Names(names) => names.into_iter().fold(0, |mask, name| mask | name.mask()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_frames_repeats_holds_and_markers() {
        let program = TapeProgram::from_json(
            r#"{
          "schema":"dusktape-program/v1", "default_owned_ports":1,
          "steps":[
            {"op":"repeat","count":2,"frame":{"pads":{"0":{"buttons":["A","RIGHT"],"stick_x":-7}}}},
            {"op":"marker","name":"pressed"},
            {"op":"hold","count":3},
            {"op":"frame","frame":{}}
          ]
        }"#,
        )
        .unwrap()
        .compile()
        .unwrap();
        assert_eq!(program.tape.frames.len(), 6);
        assert_eq!(program.tape.frames[0].pads[0].buttons, 0x0102);
        assert_eq!(program.tape.frames[4].pads[0].stick_x, -7);
        assert_eq!(program.tape.frames[5].pads[0].buttons, 0);
        assert_eq!(
            program.markers,
            vec![Marker {
                name: "pressed".into(),
                tick: 2
            }]
        );
    }

    #[test]
    fn rejects_unknown_fields_and_unbounded_expansion() {
        assert!(
            TapeProgram::from_json(r#"{"schema":"dusktape-program/v1","wat":1,"steps":[]}"#)
                .is_err()
        );
        let error = TapeProgram::from_json(
            r#"{
          "schema":"dusktape-program/v1",
          "steps":[{"op":"repeat","count":10000001,"frame":{}}]
        }"#,
        )
        .unwrap()
        .compile()
        .unwrap_err();
        assert!(matches!(error, ProgramError::TooManyFrames));
    }
}
