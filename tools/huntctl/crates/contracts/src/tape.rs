use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

use crate::scenario_fixture::ScenarioFixture;

pub const MAGIC: [u8; 8] = *b"DUSKTAPE";
pub const MAJOR_VERSION: u16 = 3;
pub const MINOR_VERSION: u16 = 2;
pub const HEADER_SIZE: usize = 64;
pub const PAD_SIZE: usize = 12;
pub const FRAME_SIZE: usize = 52;
pub const PORT_COUNT: usize = 4;
const LEGACY_MAJOR_VERSION: u16 = 1;
const LEGACY_MAX_MINOR_VERSION: u16 = 2;
const LEGACY_HEADER_SIZE: usize = 32;
const COMPRESSED_V2_MAJOR_VERSION: u16 = 2;
const COMPRESSED_V2_HEADER_SIZE: usize = 40;
const STAGE_NAME_CAPACITY: usize = 16;
const ALL_PORTS: u8 = (1 << PORT_COUNT) - 1;
const CONNECTED_FLAG: u8 = 1;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitCondition {
    #[default]
    None,
    NameEntryActive,
    NameEntryCharacterSelect,
    NameEntryInputReady,
    FileSelectNoSaveReady,
    FileSelectDataSelectReady,
    FileSelectAcceptReady,
}

impl WaitCondition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::NameEntryActive => "name_entry_active",
            Self::NameEntryCharacterSelect => "name_entry_character_select",
            Self::NameEntryInputReady => "name_entry_input_ready",
            Self::FileSelectNoSaveReady => "file_select_no_save_ready",
            Self::FileSelectDataSelectReady => "file_select_data_select_ready",
            Self::FileSelectAcceptReady => "file_select_accept_ready",
        }
    }

    const fn encode(self) -> u8 {
        match self {
            Self::None => 0,
            Self::NameEntryActive => 1,
            Self::NameEntryCharacterSelect => 2,
            Self::NameEntryInputReady => 3,
            Self::FileSelectNoSaveReady => 4,
            Self::FileSelectDataSelectReady => 5,
            Self::FileSelectAcceptReady => 6,
        }
    }

    fn decode(value: u8) -> Result<Self, TapeError> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::NameEntryActive),
            2 => Ok(Self::NameEntryCharacterSelect),
            3 => Ok(Self::NameEntryInputReady),
            4 => Ok(Self::FileSelectNoSaveReady),
            5 => Ok(Self::FileSelectDataSelectReady),
            6 => Ok(Self::FileSelectAcceptReady),
            _ => Err(TapeError::InvalidWaitCondition),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TapeVersion {
    pub major: u16,
    pub minor: u16,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[expect(
    clippy::large_enum_variant,
    reason = "the canonical stage fixture stays inline so tape values have no hidden heap indirection"
)]
pub enum TapeBoot {
    #[default]
    Process,
    Stage {
        stage: String,
        room: i8,
        point: i16,
        layer: i8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        save_slot: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fixture: Option<ScenarioFixture>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
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

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputFrame {
    pub owned_ports: u8,
    pub wait_condition: WaitCondition,
    pub wait_timeout_ticks: u16,
    pub pads: [RawPadState; PORT_COUNT],
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputTape {
    pub boot: TapeBoot,
    pub tick_rate_numerator: u32,
    pub tick_rate_denominator: u32,
    pub frames: Vec<InputFrame>,
}

impl Default for InputTape {
    fn default() -> Self {
        Self {
            boot: TapeBoot::Process,
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
    InvalidBoot,
    InvalidOwnedPorts,
    InvalidPadFlags,
    InvalidWaitCondition,
    InvalidWaitTimeout,
    InvalidCompressedPayload,
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
            Self::InvalidBoot => "input tape boot origin is invalid",
            Self::InvalidOwnedPorts => "input tape owns an invalid controller port",
            Self::InvalidPadFlags => "input tape contains unknown controller flags",
            Self::InvalidWaitCondition => "input tape contains an unknown wait condition",
            Self::InvalidWaitTimeout => "input tape wait timeout is invalid",
            Self::InvalidCompressedPayload => "input tape contains an invalid compressed payload",
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
        if let TapeBoot::Stage {
            stage,
            save_slot,
            fixture,
            ..
        } = &self.boot
        {
            if stage.is_empty()
                || stage.len() > STAGE_NAME_CAPACITY
                || !stage
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic() && byte != b',')
            {
                return Err(TapeError::InvalidBoot);
            }
            if save_slot.is_some_and(|slot| !(1..=3).contains(&slot)) {
                return Err(TapeError::InvalidBoot);
            }
            if let Some(fixture) = fixture {
                fixture.validate().map_err(|_| TapeError::InvalidBoot)?;
            }
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
        }
        Ok(())
    }

    /// Encodes canonical DUSKTAPE v3 bytes. The canonical v1.2 frame stream is
    /// compressed as one zstd frame and prefixed by the boot-origin header.
    pub fn encode(&self) -> Result<Vec<u8>, TapeError> {
        self.validate()?;
        let payload_size = self
            .frames
            .len()
            .checked_mul(FRAME_SIZE)
            .ok_or(TapeError::TooManyFrames)?;
        let mut expanded = vec![0_u8; payload_size];
        encode_frame_stream(&self.frames, &mut expanded);
        let compressed =
            zstd::bulk::compress(&expanded, 0).map_err(|_| TapeError::InvalidCompressedPayload)?;
        let fixture_bytes = match &self.boot {
            TapeBoot::Stage {
                fixture: Some(fixture),
                ..
            } => fixture.encode().map_err(|_| TapeError::InvalidBoot)?,
            _ => Vec::new(),
        };
        let total_size = HEADER_SIZE
            .checked_add(compressed.len())
            .and_then(|size| size.checked_add(fixture_bytes.len()))
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
        put_u64(&mut output[32..40], compressed.len() as u64);
        match &self.boot {
            TapeBoot::Process => {}
            TapeBoot::Stage {
                stage,
                room,
                point,
                layer,
                save_slot,
                fixture,
            } => {
                output[40] = 1;
                output[41] = stage.len() as u8;
                output[42] = *room as u8;
                output[43] = *layer as u8;
                output[44..46].copy_from_slice(&point.to_le_bytes());
                output[46] = save_slot.unwrap_or(0);
                output[47] = u8::from(fixture.is_some());
                output[48..48 + stage.len()].copy_from_slice(stage.as_bytes());
            }
        }
        output[HEADER_SIZE..HEADER_SIZE + compressed.len()].copy_from_slice(&compressed);
        output[HEADER_SIZE + compressed.len()..].copy_from_slice(&fixture_bytes);
        Ok(output)
    }

    pub fn decode(bytes: &[u8]) -> Result<DecodedInputTape, TapeError> {
        if bytes.len() < LEGACY_HEADER_SIZE {
            return Err(TapeError::Truncated);
        }
        if bytes[..8] != MAGIC {
            return Err(TapeError::BadMagic);
        }
        let version = TapeVersion {
            major: get_u16(&bytes[8..10]),
            minor: get_u16(&bytes[10..12]),
        };
        let legacy =
            version.major == LEGACY_MAJOR_VERSION && version.minor <= LEGACY_MAX_MINOR_VERSION;
        let compressed_v2 = version.major == COMPRESSED_V2_MAJOR_VERSION && version.minor == 0;
        let compressed_v3 = version.major == MAJOR_VERSION && version.minor <= MINOR_VERSION;
        let compressed = compressed_v2 || compressed_v3;
        if !legacy && !compressed {
            return Err(TapeError::UnsupportedVersion);
        }
        let header_size = if legacy {
            LEGACY_HEADER_SIZE
        } else if compressed_v2 {
            COMPRESSED_V2_HEADER_SIZE
        } else {
            HEADER_SIZE
        };
        if bytes.len() < header_size {
            return Err(TapeError::Truncated);
        }
        if get_u16(&bytes[12..14]) as usize != header_size {
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
        if frame_count_u64 > (usize::MAX / FRAME_SIZE) as u64 {
            return Err(TapeError::TooManyFrames);
        }
        let frame_count = frame_count_u64 as usize;
        let expanded_size = frame_count
            .checked_mul(FRAME_SIZE)
            .ok_or(TapeError::TooManyFrames)?;
        let mut fixture_present = false;
        let mut boot = if compressed_v3 {
            match bytes[40] {
                0 => {
                    if bytes[41..HEADER_SIZE].iter().any(|byte| *byte != 0) {
                        return Err(TapeError::InvalidBoot);
                    }
                    TapeBoot::Process
                }
                1 => {
                    let stage_len = bytes[41] as usize;
                    if stage_len == 0
                        || stage_len > STAGE_NAME_CAPACITY
                        || bytes[47] > 1
                        || (version.minor < 2 && bytes[47] != 0)
                        || (version.minor == 0 && bytes[46] != 0)
                        || bytes[48 + stage_len..HEADER_SIZE]
                            .iter()
                            .any(|byte| *byte != 0)
                    {
                        return Err(TapeError::InvalidBoot);
                    }
                    let stage = std::str::from_utf8(&bytes[48..48 + stage_len])
                        .map_err(|_| TapeError::InvalidBoot)?
                        .to_owned();
                    let boot = TapeBoot::Stage {
                        stage,
                        room: bytes[42] as i8,
                        point: i16::from_le_bytes(bytes[44..46].try_into().unwrap()),
                        layer: bytes[43] as i8,
                        save_slot: (bytes[46] != 0).then_some(bytes[46]),
                        fixture: None,
                    };
                    fixture_present = bytes[47] != 0;
                    let candidate = InputTape {
                        boot: boot.clone(),
                        ..InputTape::default()
                    };
                    candidate.validate()?;
                    boot
                }
                _ => return Err(TapeError::InvalidBoot),
            }
        } else {
            TapeBoot::Process
        };
        let (frame_bytes, frame_minor_version) = if legacy {
            let expected = LEGACY_HEADER_SIZE
                .checked_add(expanded_size)
                .ok_or(TapeError::TooManyFrames)?;
            if bytes.len() < expected {
                return Err(TapeError::Truncated);
            }
            if bytes.len() != expected {
                return Err(TapeError::TrailingData);
            }
            (bytes[LEGACY_HEADER_SIZE..].to_vec(), version.minor)
        } else {
            let payload_size_u64 = get_u64(&bytes[32..40]);
            if payload_size_u64 > usize::MAX as u64 {
                return Err(TapeError::TooManyFrames);
            }
            let payload_size = payload_size_u64 as usize;
            let available = bytes.len() - header_size;
            if payload_size > available {
                return Err(TapeError::Truncated);
            }
            if payload_size < available && !fixture_present {
                return Err(TapeError::TrailingData);
            }
            let payload = &bytes[header_size..header_size + payload_size];
            let expanded = zstd::bulk::decompress(payload, expanded_size)
                .map_err(|_| TapeError::InvalidCompressedPayload)?;
            if expanded.len() != expanded_size {
                return Err(TapeError::InvalidCompressedPayload);
            }
            (expanded, LEGACY_MAX_MINOR_VERSION)
        };
        if fixture_present {
            let payload_size = get_u64(&bytes[32..40]) as usize;
            let fixture_start = header_size
                .checked_add(payload_size)
                .ok_or(TapeError::TooManyFrames)?;
            if fixture_start == bytes.len() {
                return Err(TapeError::Truncated);
            }
            let fixture = ScenarioFixture::decode(&bytes[fixture_start..])
                .map_err(|_| TapeError::InvalidBoot)?;
            match &mut boot {
                TapeBoot::Stage {
                    fixture: target, ..
                } => *target = Some(fixture),
                TapeBoot::Process => return Err(TapeError::InvalidBoot),
            }
        }
        let frames = decode_frame_stream(&frame_bytes, frame_count, frame_minor_version)?;
        Ok(DecodedInputTape {
            source_version: version,
            tape: InputTape {
                boot,
                tick_rate_numerator: numerator,
                tick_rate_denominator: denominator,
                frames,
            },
        })
    }
}

fn encode_frame_stream(frames: &[InputFrame], output: &mut [u8]) {
    for (frame_index, frame) in frames.iter().enumerate() {
        let frame_start = frame_index * FRAME_SIZE;
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
}

fn decode_frame_stream(
    bytes: &[u8],
    frame_count: usize,
    minor_version: u16,
) -> Result<Vec<InputFrame>, TapeError> {
    let mut frames = Vec::with_capacity(frame_count);
    for source in bytes.chunks_exact(FRAME_SIZE) {
        if source[0] & !ALL_PORTS != 0 {
            return Err(TapeError::InvalidOwnedPorts);
        }
        if minor_version < 2 && source[1..4] != [0, 0, 0] {
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
            let start = 4 + port * PAD_SIZE;
            frame.pads[port] = decode_pad(&source[start..start + PAD_SIZE], minor_version)?;
        }
        frames.push(frame);
    }
    if frames.len() != frame_count {
        return Err(TapeError::InvalidCompressedPayload);
    }
    Ok(frames)
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
    use crate::scenario_fixture::{HealthFixture, PlayerForm, SCENARIO_FIXTURE_SCHEMA};

    fn legacy_bytes(tape: &InputTape, minor: u16) -> Vec<u8> {
        let mut frames = vec![0; tape.frames.len() * FRAME_SIZE];
        encode_frame_stream(&tape.frames, &mut frames);
        if minor < 2 {
            for frame in frames.chunks_exact_mut(FRAME_SIZE) {
                frame[1..4].fill(0);
            }
        }
        let mut bytes = vec![0; LEGACY_HEADER_SIZE + frames.len()];
        bytes[..8].copy_from_slice(&MAGIC);
        put_u16(&mut bytes[8..10], 1);
        put_u16(&mut bytes[10..12], minor);
        put_u16(&mut bytes[12..14], LEGACY_HEADER_SIZE as u16);
        put_u16(&mut bytes[14..16], FRAME_SIZE as u16);
        put_u32(&mut bytes[16..20], tape.tick_rate_numerator);
        put_u32(&mut bytes[20..24], tape.tick_rate_denominator);
        put_u64(&mut bytes[24..32], tape.frames.len() as u64);
        bytes[LEGACY_HEADER_SIZE..].copy_from_slice(&frames);
        bytes
    }

    fn mutate_v2_frame(mut bytes: Vec<u8>, mutate: impl FnOnce(&mut [u8])) -> Vec<u8> {
        let expanded_size = get_u64(&bytes[24..32]) as usize * FRAME_SIZE;
        let mut frames = zstd::bulk::decompress(&bytes[HEADER_SIZE..], expanded_size).unwrap();
        mutate(&mut frames);
        let compressed = zstd::bulk::compress(&frames, 0).unwrap();
        bytes.truncate(HEADER_SIZE);
        put_u64(&mut bytes[32..40], compressed.len() as u64);
        bytes.extend_from_slice(&compressed);
        bytes
    }

    #[test]
    fn v3_round_trip_is_compact() {
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
            ..InputTape::default()
        }
        .encode()
        .unwrap();
        assert_eq!(&bytes[..8], &MAGIC);
        assert_eq!(get_u16(&bytes[8..10]), 3);
        assert_eq!(get_u16(&bytes[12..14]) as usize, HEADER_SIZE);
        assert!(bytes.len() < HEADER_SIZE + FRAME_SIZE);
        let decoded = InputTape::decode(&bytes).unwrap();
        assert_eq!(decoded.source_version, TapeVersion { major: 3, minor: 2 });
        assert_eq!(decoded.tape.frames[0].pads[0].error, -3);
    }

    #[test]
    fn full_pad_surface_round_trips_on_all_four_ports() {
        let mut frame = InputFrame {
            owned_ports: 0x0f,
            ..InputFrame::default()
        };
        for (port, pad) in frame.pads.iter_mut().enumerate() {
            *pad = RawPadState {
                buttons: u16::MAX - port as u16,
                stick_x: i8::MIN + port as i8,
                stick_y: i8::MAX - port as i8,
                substick_x: -64 + port as i8,
                substick_y: 64 - port as i8,
                trigger_left: u8::MAX - port as u8,
                trigger_right: port as u8,
                analog_a: 17 + port as u8,
                analog_b: 239 - port as u8,
                connected: port != 3,
                error: if port == 3 { -1 } else { port as i8 },
            };
        }
        let tape = InputTape {
            frames: vec![frame],
            ..InputTape::default()
        };
        let decoded = InputTape::decode(&tape.encode().unwrap()).unwrap();
        assert_eq!(decoded.tape, tape);
        assert_eq!(decoded.tape.frames[0].owned_ports, 0x0f);
    }

    #[test]
    fn stage_boot_round_trips_and_v2_defaults_to_process_boot() {
        let tape = InputTape {
            boot: TapeBoot::Stage {
                stage: "F_SP103".into(),
                room: 1,
                point: 257,
                layer: -1,
                save_slot: Some(2),
                fixture: None,
            },
            frames: vec![InputFrame::default()],
            ..InputTape::default()
        };
        let bytes = tape.encode().unwrap();
        assert_eq!(InputTape::decode(&bytes).unwrap().tape.boot, tape.boot);
        let mut v3_0 = bytes.clone();
        put_u16(&mut v3_0[10..12], 0);
        v3_0[46] = 0;
        let decoded_v3_0 = InputTape::decode(&v3_0).unwrap();
        assert_eq!(
            decoded_v3_0.source_version,
            TapeVersion { major: 3, minor: 0 }
        );
        assert!(matches!(
            decoded_v3_0.tape.boot,
            TapeBoot::Stage {
                save_slot: None,
                ..
            }
        ));

        let process = InputTape {
            frames: vec![InputFrame::default()],
            ..InputTape::default()
        };
        let v3 = process.encode().unwrap();
        let mut v2 = Vec::with_capacity(v3.len() - (HEADER_SIZE - COMPRESSED_V2_HEADER_SIZE));
        v2.extend_from_slice(&v3[..COMPRESSED_V2_HEADER_SIZE]);
        v2.extend_from_slice(&v3[HEADER_SIZE..]);
        put_u16(&mut v2[8..10], COMPRESSED_V2_MAJOR_VERSION);
        put_u16(&mut v2[10..12], 0);
        put_u16(&mut v2[12..14], COMPRESSED_V2_HEADER_SIZE as u16);
        let decoded = InputTape::decode(&v2).unwrap();
        assert_eq!(decoded.source_version, TapeVersion { major: 2, minor: 0 });
        assert_eq!(decoded.tape.boot, TapeBoot::Process);
    }

    #[test]
    fn stage_boot_embeds_one_canonical_scenario_fixture() {
        let fixture = ScenarioFixture {
            schema: SCENARIO_FIXTURE_SCHEMA.into(),
            name: "low-health wolf".into(),
            form: Some(PlayerForm::Wolf),
            health: Some(HealthFixture {
                current: 4,
                maximum: 20,
            }),
            rng: Vec::new(),
            video_mode: None,
            inventory: Vec::new(),
            equipment: Vec::new(),
            flags: Vec::new(),
            settings: Vec::new(),
        };
        let tape = InputTape {
            boot: TapeBoot::Stage {
                stage: "F_SP103".into(),
                room: 1,
                point: 1,
                layer: 3,
                save_slot: None,
                fixture: Some(fixture),
            },
            frames: vec![InputFrame::default()],
            ..InputTape::default()
        };
        let encoded = tape.encode().unwrap();
        assert_eq!(encoded[47], 1);
        assert_eq!(get_u16(&encoded[10..12]), 2);
        assert_eq!(InputTape::decode(&encoded).unwrap().tape, tape);

        let mut missing = encoded.clone();
        missing.truncate(HEADER_SIZE + get_u64(&missing[32..40]) as usize);
        assert_eq!(InputTape::decode(&missing), Err(TapeError::Truncated));

        let mut process = tape;
        process.boot = TapeBoot::Process;
        assert!(process.encode().is_ok());
    }

    #[test]
    fn rejects_noncanonical_stage_boot_headers() {
        let invalid = InputTape {
            boot: TapeBoot::Stage {
                stage: "".into(),
                room: 0,
                point: 0,
                layer: -1,
                save_slot: None,
                fixture: None,
            },
            ..InputTape::default()
        };
        assert_eq!(invalid.validate(), Err(TapeError::InvalidBoot));

        let invalid_save = InputTape {
            boot: TapeBoot::Stage {
                stage: "F_SP103".into(),
                room: 0,
                point: 0,
                layer: -1,
                save_slot: Some(4),
                fixture: None,
            },
            ..InputTape::default()
        };
        assert_eq!(invalid_save.validate(), Err(TapeError::InvalidBoot));

        let mut bytes = InputTape::default().encode().unwrap();
        bytes[41] = 1;
        assert_eq!(InputTape::decode(&bytes), Err(TapeError::InvalidBoot));
    }

    #[test]
    fn legacy_v1_0_derives_error_from_connected_flag() {
        let tape = InputTape {
            frames: vec![InputFrame::default()],
            ..InputTape::default()
        };
        let mut bytes = legacy_bytes(&tape, 0);
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
        bytes = mutate_v2_frame(bytes, |frames| frames[1] = 1);
        assert_eq!(
            InputTape::decode(&bytes).unwrap_err(),
            TapeError::InvalidWaitTimeout
        );

        bytes = InputTape {
            frames: vec![InputFrame::default()],
            ..InputTape::default()
        }
        .encode()
        .unwrap();
        bytes = mutate_v2_frame(bytes, |frames| frames[1] = 7);
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
        let tape = InputTape {
            frames: vec![InputFrame::default()],
            ..InputTape::default()
        };
        let mut bytes = legacy_bytes(&tape, 1);
        bytes[33] = 1;
        bytes[34] = 1;
        assert_eq!(
            InputTape::decode(&bytes).unwrap_err(),
            TapeError::InvalidFrameSize
        );
    }

    #[test]
    fn legacy_v1_1_tape_decodes() {
        let tape = InputTape {
            frames: vec![InputFrame::default()],
            ..InputTape::default()
        };
        let bytes = legacy_bytes(&tape, 1);
        let decoded = InputTape::decode(&bytes).unwrap();
        assert_eq!(decoded.source_version.minor, 1);
        assert_eq!(decoded.tape.frames[0].wait_condition, WaitCondition::None);
    }

    #[test]
    fn conditioned_pulse_frames_round_trip() {
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
        assert_eq!(tape.validate(), Ok(()));

        let bytes = InputTape {
            frames: vec![InputFrame {
                wait_condition: WaitCondition::NameEntryActive,
                wait_timeout_ticks: 1,
                ..InputFrame::default()
            }],
            ..InputTape::default()
        }
        .encode()
        .unwrap();
        let bytes = mutate_v2_frame(bytes, |frames| frames[4] = 1);
        let decoded = InputTape::decode(&bytes).unwrap();
        assert_eq!(decoded.tape.frames[0].pads[0].buttons, 1);
    }
}
