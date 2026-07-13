use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

pub const MAGIC: [u8; 8] = *b"DUSKTAPE";
pub const MAJOR_VERSION: u16 = 1;
pub const MINOR_VERSION: u16 = 2;
pub const HEADER_SIZE: usize = 32;
pub const PAD_SIZE: usize = 12;
pub const FRAME_SIZE: usize = 52;
pub const PORT_COUNT: usize = 4;
const ALL_PORTS: u8 = (1 << PORT_COUNT) - 1;
const CONNECTED_FLAG: u8 = 1;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitCondition {
    #[default]
    None,
    NameEntryActive,
}

impl WaitCondition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::NameEntryActive => "name_entry_active",
        }
    }

    const fn encode(self) -> u8 {
        match self {
            Self::None => 0,
            Self::NameEntryActive => 1,
        }
    }

    fn decode(value: u8) -> Result<Self, TapeError> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::NameEntryActive),
            _ => Err(TapeError::InvalidWaitCondition),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TapeVersion {
    pub major: u16,
    pub minor: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct RawPadState {
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

impl Default for RawPadState {
    fn default() -> Self {
        Self {
            buttons: 0,
            stick_x: 0,
            stick_y: 0,
            substick_x: 0,
            substick_y: 0,
            trigger_left: 0,
            trigger_right: 0,
            analog_a: 0,
            analog_b: 0,
            connected: true,
            error: 0,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct InputFrame {
    pub owned_ports: u8,
    pub wait_condition: WaitCondition,
    pub wait_timeout_ticks: u16,
    pub pads: [RawPadState; PORT_COUNT],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InputTape {
    pub tick_rate_numerator: u32,
    pub tick_rate_denominator: u32,
    pub frames: Vec<InputFrame>,
}

impl Default for InputTape {
    fn default() -> Self {
        Self {
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            frames: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DecodedInputTape {
    pub source_version: TapeVersion,
    pub tape: InputTape,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TapeError {
    Truncated,
    BadMagic,
    UnsupportedVersion,
    InvalidHeaderSize,
    InvalidFrameSize,
    InvalidTickRate,
    InvalidOwnedPorts,
    InvalidPadFlags,
    InvalidWaitCondition,
    InvalidWaitTimeout,
    InvalidWaitInput,
    TrailingData,
    TooManyFrames,
}

impl fmt::Display for TapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Truncated => "input tape is truncated",
            Self::BadMagic => "input tape has an invalid magic value",
            Self::UnsupportedVersion => "input tape version is unsupported",
            Self::InvalidHeaderSize => "input tape header size is invalid",
            Self::InvalidFrameSize => "input tape frame size is invalid",
            Self::InvalidTickRate => "input tape tick rate is invalid",
            Self::InvalidOwnedPorts => "input tape owns an invalid controller port",
            Self::InvalidPadFlags => "input tape contains unknown controller flags",
            Self::InvalidWaitCondition => "input tape contains an unknown wait condition",
            Self::InvalidWaitTimeout => "input tape wait timeout is invalid",
            Self::InvalidWaitInput => "input tape wait frame contains non-neutral input",
            Self::TrailingData => "input tape contains trailing data",
            Self::TooManyFrames => "input tape frame count is too large",
        })
    }
}

impl Error for TapeError {}

impl InputTape {
    pub fn validate(&self) -> Result<(), TapeError> {
        if self.tick_rate_numerator == 0 || self.tick_rate_denominator == 0 {
            return Err(TapeError::InvalidTickRate);
        }
        for frame in &self.frames {
            if frame.owned_ports & !ALL_PORTS != 0 {
                return Err(TapeError::InvalidOwnedPorts);
            }
            match (frame.wait_condition, frame.wait_timeout_ticks) {
                (WaitCondition::None, 0) => {}
                (WaitCondition::None, _) | (_, 0) => return Err(TapeError::InvalidWaitTimeout),
                _ => {}
            }
            if frame.wait_condition != WaitCondition::None
                && frame.pads.iter().any(|pad| *pad != RawPadState::default())
            {
                return Err(TapeError::InvalidWaitInput);
            }
        }
        Ok(())
    }

    /// Encodes canonical DUSKTAPE v1.2 bytes.
    pub fn encode(&self) -> Result<Vec<u8>, TapeError> {
        self.validate()?;
        let payload_size = self
            .frames
            .len()
            .checked_mul(FRAME_SIZE)
            .ok_or(TapeError::TooManyFrames)?;
        let total_size = HEADER_SIZE
            .checked_add(payload_size)
            .ok_or(TapeError::TooManyFrames)?;
        let mut output = vec![0_u8; total_size];
        output[..8].copy_from_slice(&MAGIC);
        put_u16(&mut output[8..10], MAJOR_VERSION);
        put_u16(&mut output[10..12], MINOR_VERSION);
        put_u16(&mut output[12..14], HEADER_SIZE as u16);
        put_u16(&mut output[14..16], FRAME_SIZE as u16);
        put_u32(&mut output[16..20], self.tick_rate_numerator);
        put_u32(&mut output[20..24], self.tick_rate_denominator);
        put_u64(&mut output[24..32], self.frames.len() as u64);

        for (frame_index, frame) in self.frames.iter().enumerate() {
            let frame_start = HEADER_SIZE + frame_index * FRAME_SIZE;
            output[frame_start] = frame.owned_ports;
            output[frame_start + 1] = frame.wait_condition.encode();
            put_u16(
                &mut output[frame_start + 2..frame_start + 4],
                frame.wait_timeout_ticks,
            );
            for (port, pad) in frame.pads.iter().enumerate() {
                let start = frame_start + 4 + port * PAD_SIZE;
                encode_pad(pad, &mut output[start..start + PAD_SIZE]);
            }
        }
        Ok(output)
    }

    pub fn decode(bytes: &[u8]) -> Result<DecodedInputTape, TapeError> {
        if bytes.len() < HEADER_SIZE {
            return Err(TapeError::Truncated);
        }
        if bytes[..8] != MAGIC {
            return Err(TapeError::BadMagic);
        }
        let version = TapeVersion {
            major: get_u16(&bytes[8..10]),
            minor: get_u16(&bytes[10..12]),
        };
        if version.major != MAJOR_VERSION || version.minor > MINOR_VERSION {
            return Err(TapeError::UnsupportedVersion);
        }
        if get_u16(&bytes[12..14]) as usize != HEADER_SIZE {
            return Err(TapeError::InvalidHeaderSize);
        }
        if get_u16(&bytes[14..16]) as usize != FRAME_SIZE {
            return Err(TapeError::InvalidFrameSize);
        }
        let numerator = get_u32(&bytes[16..20]);
        let denominator = get_u32(&bytes[20..24]);
        if numerator == 0 || denominator == 0 {
            return Err(TapeError::InvalidTickRate);
        }
        let frame_count_u64 = get_u64(&bytes[24..32]);
        let available = (bytes.len() - HEADER_SIZE) / FRAME_SIZE;
        if frame_count_u64 > available as u64 || frame_count_u64 > usize::MAX as u64 {
            return Err(TapeError::TooManyFrames);
        }
        let frame_count = frame_count_u64 as usize;
        let expected = HEADER_SIZE
            .checked_add(
                frame_count
                    .checked_mul(FRAME_SIZE)
                    .ok_or(TapeError::TooManyFrames)?,
            )
            .ok_or(TapeError::TooManyFrames)?;
        if bytes.len() < expected {
            return Err(TapeError::Truncated);
        }
        if bytes.len() != expected {
            return Err(TapeError::TrailingData);
        }

        let mut frames = Vec::with_capacity(frame_count);
        for frame_index in 0..frame_count {
            let start = HEADER_SIZE + frame_index * FRAME_SIZE;
            let source = &bytes[start..start + FRAME_SIZE];
            if source[0] & !ALL_PORTS != 0 {
                return Err(TapeError::InvalidOwnedPorts);
            }
            if version.minor < 2 && source[1..4] != [0, 0, 0] {
                return Err(TapeError::InvalidFrameSize);
            }
            let wait_condition = WaitCondition::decode(source[1])?;
            let wait_timeout_ticks = get_u16(&source[2..4]);
            match (wait_condition, wait_timeout_ticks) {
                (WaitCondition::None, 0) => {}
                (WaitCondition::None, _) | (_, 0) => return Err(TapeError::InvalidWaitTimeout),
                _ => {}
            }
            let mut frame = InputFrame {
                owned_ports: source[0],
                wait_condition,
                wait_timeout_ticks,
                ..InputFrame::default()
            };
            for port in 0..PORT_COUNT {
                let pad_start = 4 + port * PAD_SIZE;
                frame.pads[port] =
                    decode_pad(&source[pad_start..pad_start + PAD_SIZE], version.minor)?;
            }
            if wait_condition != WaitCondition::None
                && frame.pads.iter().any(|pad| *pad != RawPadState::default())
            {
                return Err(TapeError::InvalidWaitInput);
            }
            frames.push(frame);
        }
        Ok(DecodedInputTape {
            source_version: version,
            tape: InputTape {
                tick_rate_numerator: numerator,
                tick_rate_denominator: denominator,
                frames,
            },
        })
    }
}

fn encode_pad(pad: &RawPadState, output: &mut [u8]) {
    put_u16(&mut output[..2], pad.buttons);
    output[2] = pad.stick_x as u8;
    output[3] = pad.stick_y as u8;
    output[4] = pad.substick_x as u8;
    output[5] = pad.substick_y as u8;
    output[6] = pad.trigger_left;
    output[7] = pad.trigger_right;
    output[8] = pad.analog_a;
    output[9] = pad.analog_b;
    output[10] = u8::from(pad.connected);
    output[11] = pad.error as u8;
}

fn decode_pad(input: &[u8], minor_version: u16) -> Result<RawPadState, TapeError> {
    if input[10] & !CONNECTED_FLAG != 0 {
        return Err(TapeError::InvalidPadFlags);
    }
    let connected = input[10] & CONNECTED_FLAG != 0;
    Ok(RawPadState {
        buttons: get_u16(&input[..2]),
        stick_x: input[2] as i8,
        stick_y: input[3] as i8,
        substick_x: input[4] as i8,
        substick_y: input[5] as i8,
        trigger_left: input[6],
        trigger_right: input[7],
        analog_a: input[8],
        analog_b: input[9],
        connected,
        error: if minor_version == 0 {
            if connected { 0 } else { -1 }
        } else {
            input[11] as i8
        },
    })
}

fn get_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes.try_into().expect("two-byte field"))
}
fn get_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("four-byte field"))
}
fn get_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().expect("eight-byte field"))
}
fn put_u16(bytes: &mut [u8], value: u16) {
    bytes.copy_from_slice(&value.to_le_bytes());
}
fn put_u32(bytes: &mut [u8], value: u32) {
    bytes.copy_from_slice(&value.to_le_bytes());
}
fn put_u64(bytes: &mut [u8], value: u64) {
    bytes.copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_2_golden_bytes() {
        let mut frame = InputFrame {
            owned_ports: 1,
            ..InputFrame::default()
        };
        frame.pads[0] = RawPadState {
            buttons: 0x1102,
            stick_x: -128,
            stick_y: 127,
            substick_x: -2,
            substick_y: 3,
            trigger_left: 4,
            trigger_right: 5,
            analog_a: 6,
            analog_b: 7,
            connected: true,
            error: -3,
        };
        let bytes = InputTape {
            tick_rate_numerator: 30_000,
            tick_rate_denominator: 1_001,
            frames: vec![frame],
        }
        .encode()
        .unwrap();
        let mut expected = vec![
            0x44, 0x55, 0x53, 0x4b, 0x54, 0x41, 0x50, 0x45, 0x01, 0x00, 0x02, 0x00, 0x20, 0x00,
            0x34, 0x00, 0x30, 0x75, 0x00, 0x00, 0xe9, 0x03, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x11, 0x80, 0x7f, 0xfe, 0x03,
            0x04, 0x05, 0x06, 0x07, 0x01, 0xfd,
        ];
        expected.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0].repeat(3));
        assert_eq!(bytes, expected);
        assert_eq!(
            InputTape::decode(&bytes).unwrap().tape.frames[0].pads[0].error,
            -3
        );
    }

    #[test]
    fn legacy_v1_0_derives_error_from_connected_flag() {
        let mut bytes = InputTape {
            frames: vec![InputFrame::default()],
            ..InputTape::default()
        }
        .encode()
        .unwrap();
        bytes[10] = 0;
        bytes[11] = 0;
        bytes[32 + 4 + 11] = 99;
        bytes[32 + 4 + 12 + 10] = 0;
        bytes[32 + 4 + 12 + 11] = 99;
        let decoded = InputTape::decode(&bytes).unwrap();
        assert_eq!(decoded.source_version.minor, 0);
        assert_eq!(decoded.tape.frames[0].pads[0].error, 0);
        assert_eq!(decoded.tape.frames[0].pads[1].error, -1);
    }

    #[test]
    fn wait_frame_round_trips() {
        let frame = InputFrame {
            owned_ports: 0x0f,
            wait_condition: WaitCondition::NameEntryActive,
            wait_timeout_ticks: 900,
            ..InputFrame::default()
        };
        let bytes = InputTape {
            frames: vec![frame.clone()],
            ..InputTape::default()
        }
        .encode()
        .unwrap();
        assert_eq!(&bytes[32..36], &[0x0f, 1, 0x84, 0x03]);
        assert_eq!(InputTape::decode(&bytes).unwrap().tape.frames[0], frame);
    }

    #[test]
    fn rejects_invalid_wait_metadata() {
        let mut bytes = InputTape {
            frames: vec![InputFrame::default()],
            ..InputTape::default()
        }
        .encode()
        .unwrap();
        bytes[33] = 1;
        assert_eq!(
            InputTape::decode(&bytes).unwrap_err(),
            TapeError::InvalidWaitTimeout
        );

        bytes[33] = 2;
        assert_eq!(
            InputTape::decode(&bytes).unwrap_err(),
            TapeError::InvalidWaitCondition
        );

        let invalid = InputTape {
            frames: vec![InputFrame {
                wait_timeout_ticks: 1,
                ..InputFrame::default()
            }],
            ..InputTape::default()
        };
        assert_eq!(invalid.validate(), Err(TapeError::InvalidWaitTimeout));
    }

    #[test]
    fn legacy_versions_require_zero_reserved_frame_bytes() {
        let mut bytes = InputTape {
            frames: vec![InputFrame::default()],
            ..InputTape::default()
        }
        .encode()
        .unwrap();
        bytes[10] = 1;
        bytes[33] = 1;
        bytes[34] = 1;
        assert_eq!(
            InputTape::decode(&bytes).unwrap_err(),
            TapeError::InvalidFrameSize
        );
    }

    #[test]
    fn legacy_v1_1_tape_decodes() {
        let mut bytes = InputTape {
            frames: vec![InputFrame::default()],
            ..InputTape::default()
        }
        .encode()
        .unwrap();
        bytes[10] = 1;
        let decoded = InputTape::decode(&bytes).unwrap();
        assert_eq!(decoded.source_version.minor, 1);
        assert_eq!(decoded.tape.frames[0].wait_condition, WaitCondition::None);
    }

    #[test]
    fn rejects_input_on_wait_frames() {
        let frame = InputFrame {
            wait_condition: WaitCondition::NameEntryActive,
            wait_timeout_ticks: 1,
            pads: [
                RawPadState {
                    buttons: 1,
                    ..RawPadState::default()
                },
                RawPadState::default(),
                RawPadState::default(),
                RawPadState::default(),
            ],
            ..InputFrame::default()
        };
        let tape = InputTape {
            frames: vec![frame],
            ..InputTape::default()
        };
        assert_eq!(tape.validate(), Err(TapeError::InvalidWaitInput));

        let mut bytes = InputTape {
            frames: vec![InputFrame {
                wait_condition: WaitCondition::NameEntryActive,
                wait_timeout_ticks: 1,
                ..InputFrame::default()
            }],
            ..InputTape::default()
        }
        .encode()
        .unwrap();
        bytes[36] = 1;
        assert_eq!(
            InputTape::decode(&bytes).unwrap_err(),
            TapeError::InvalidWaitInput
        );
    }
}
