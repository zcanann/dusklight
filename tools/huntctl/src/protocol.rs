use crate::artifact::{ArtifactIdentity, BuildIdentity, Digest};
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Write};

pub const PROTOCOL_VERSION: u16 = 1;
pub const FRAME_MAGIC: [u8; 4] = *b"DSKH";
pub const MAX_PAYLOAD_LEN: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum MessageKind {
    HelloRequest = 1,
    HelloResponse = 2,
    RunBatchRequest = 3,
    RunBatchResponse = 4,
    ReplayRequest = 5,
    ReplayResponse = 6,
    ShutdownRequest = 7,
    Ack = 8,
    Error = 9,
}

impl MessageKind {
    fn from_u16(value: u16) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::HelloRequest),
            2 => Ok(Self::HelloResponse),
            3 => Ok(Self::RunBatchRequest),
            4 => Ok(Self::RunBatchResponse),
            5 => Ok(Self::ReplayRequest),
            6 => Ok(Self::ReplayResponse),
            7 => Ok(Self::ShutdownRequest),
            8 => Ok(Self::Ack),
            9 => Ok(Self::Error),
            _ => Err(ProtocolError::InvalidEnum("message kind", value as u64)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub protocol_version: u16,
    pub kind: MessageKind,
    pub request_id: u64,
    pub flags: u32,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(kind: MessageKind, request_id: u64, payload: Vec<u8>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            kind,
            request_id,
            flags: 0,
            payload,
        }
    }

    pub fn write_to(&self, mut writer: impl Write) -> Result<(), ProtocolError> {
        let len = u32::try_from(self.payload.len())
            .map_err(|_| ProtocolError::LengthOverflow(self.payload.len()))?;
        writer.write_all(&FRAME_MAGIC)?;
        writer.write_all(&self.protocol_version.to_le_bytes())?;
        writer.write_all(&(self.kind as u16).to_le_bytes())?;
        writer.write_all(&self.request_id.to_le_bytes())?;
        writer.write_all(&len.to_le_bytes())?;
        writer.write_all(&self.flags.to_le_bytes())?;
        writer.write_all(&self.payload)?;
        writer.flush()?;
        Ok(())
    }

    pub fn read_from(mut reader: impl Read) -> Result<Option<Self>, ProtocolError> {
        let mut magic = [0_u8; 4];
        loop {
            match reader.read(&mut magic[..1]) {
                Ok(0) => return Ok(None),
                Ok(1) => break,
                Ok(_) => unreachable!(),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
        reader.read_exact(&mut magic[1..])?;
        if magic != FRAME_MAGIC {
            return Err(ProtocolError::BadMagic(magic));
        }
        let protocol_version = read_u16(&mut reader)?;
        let kind = MessageKind::from_u16(read_u16(&mut reader)?)?;
        let request_id = read_u64(&mut reader)?;
        let payload_len = read_u32(&mut reader)? as usize;
        let flags = read_u32(&mut reader)?;
        if payload_len > MAX_PAYLOAD_LEN {
            return Err(ProtocolError::PayloadTooLarge(payload_len));
        }
        let mut payload = vec![0_u8; payload_len];
        reader.read_exact(&mut payload)?;
        Ok(Some(Self {
            protocol_version,
            kind,
            request_id,
            flags,
            payload,
        }))
    }
}

#[derive(Debug)]
pub enum ProtocolError {
    Io(io::Error),
    BadMagic([u8; 4]),
    UnsupportedVersion(u16),
    InvalidEnum(&'static str, u64),
    InvalidBool(u8),
    InvalidUtf8,
    LengthOverflow(usize),
    PayloadTooLarge(usize),
    Truncated,
    TrailingBytes(usize),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "protocol I/O error: {error}"),
            Self::BadMagic(magic) => write!(f, "bad frame magic: {magic:?}"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported protocol version {version}")
            }
            Self::InvalidEnum(name, value) => write!(f, "invalid {name} value {value}"),
            Self::InvalidBool(value) => write!(f, "invalid boolean value {value}"),
            Self::InvalidUtf8 => f.write_str("invalid UTF-8 string"),
            Self::LengthOverflow(len) => write!(f, "value length {len} exceeds wire format"),
            Self::PayloadTooLarge(len) => write!(f, "payload length {len} exceeds safety limit"),
            Self::Truncated => f.write_str("truncated protocol payload"),
            Self::TrailingBytes(len) => write!(f, "protocol payload has {len} trailing bytes"),
        }
    }
}

impl Error for ProtocolError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ProtocolError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub mod capability {
    pub const HEADLESS: u64 = 1 << 0;
    pub const HEADFUL: u64 = 1 << 1;
    pub const EXCLUSIVE_INPUT: u64 = 1 << 2;
    pub const BATCH_RUN: u64 = 1 << 3;
    pub const STATE_HASH: u64 = 1 << 4;
    pub const CHECKPOINT: u64 = 1 << 5;
    pub const RENDER_CAPTURE: u64 = 1 << 6;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Capabilities {
    pub flags: u64,
    pub max_controllers: u8,
    pub checkpoint_tier: u8,
    pub max_batch_size: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelloRequest {
    pub min_protocol: u16,
    pub max_protocol: u16,
    pub client_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelloResponse {
    pub selected_protocol: u16,
    pub worker_name: String,
    pub build: BuildIdentity,
    pub capabilities: Capabilities,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunCandidate {
    pub candidate_id: u64,
    /// Canonical tape bytes, uploaded once per coarse candidate operation.
    pub tape: Vec<u8>,
    pub start_tick: u64,
    pub max_ticks: u64,
    pub observation_mask: u64,
    pub oracle_mask: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunBatchRequest {
    pub scenario_id: String,
    pub candidates: Vec<RunCandidate>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TerminalReason {
    Completed = 0,
    OracleMatched = 1,
    Timeout = 2,
    Crashed = 3,
    Diverged = 4,
    Rejected = 5,
}

impl TerminalReason {
    fn decode(value: u8) -> Result<Self, ProtocolError> {
        match value {
            0 => Ok(Self::Completed),
            1 => Ok(Self::OracleMatched),
            2 => Ok(Self::Timeout),
            3 => Ok(Self::Crashed),
            4 => Ok(Self::Diverged),
            5 => Ok(Self::Rejected),
            _ => Err(ProtocolError::InvalidEnum("terminal reason", value as u64)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunOutcome {
    pub candidate_id: u64,
    pub reason: TerminalReason,
    pub completed_ticks: u64,
    pub terminal_state: Digest,
    pub event_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunBatchResponse {
    pub outcomes: Vec<RunOutcome>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PresentationMode {
    Headless = 0,
    UnpacedHeadful = 1,
    RealtimeHeadful = 2,
}

impl PresentationMode {
    fn decode(value: u8) -> Result<Self, ProtocolError> {
        match value {
            0 => Ok(Self::Headless),
            1 => Ok(Self::UnpacedHeadful),
            2 => Ok(Self::RealtimeHeadful),
            _ => Err(ProtocolError::InvalidEnum(
                "presentation mode",
                value as u64,
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayRequest {
    pub artifact: ArtifactIdentity,
    pub tape: Vec<u8>,
    pub presentation: PresentationMode,
    pub present_from_tick: u64,
    pub max_ticks: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayResponse {
    pub completed_ticks: u64,
    pub verified: bool,
    pub divergence_tick: Option<u64>,
    pub terminal_state: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ack {
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerError {
    pub code: u32,
    pub message: String,
}

macro_rules! wire_type {
    ($ty:ty, |$this:ident, $writer_ref:ident| $encode:block, |$reader_ref:ident| $decode:block) => {
        impl $ty {
            pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
                let $this = self;
                let mut writer = WireWriter::default();
                let $writer_ref = &mut writer;
                $encode
                Ok(writer.finish())
            }

            pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
                let mut reader = WireReader::new(bytes);
                let $reader_ref = &mut reader;
                let value: Self = $decode;
                reader.finish()?;
                Ok(value)
            }
        }
    };
}

wire_type!(
    HelloRequest,
    |this, writer_ref| {
        writer_ref.u16(this.min_protocol);
        writer_ref.u16(this.max_protocol);
        writer_ref.string(&this.client_name)?;
    },
    |reader_ref| {
        Self {
            min_protocol: reader_ref.u16()?,
            max_protocol: reader_ref.u16()?,
            client_name: reader_ref.string()?,
        }
    }
);

wire_type!(
    HelloResponse,
    |this, writer_ref| {
        writer_ref.u16(this.selected_protocol);
        writer_ref.string(&this.worker_name)?;
        encode_build(writer_ref, &this.build)?;
        encode_capabilities(writer_ref, &this.capabilities);
    },
    |reader_ref| {
        Self {
            selected_protocol: reader_ref.u16()?,
            worker_name: reader_ref.string()?,
            build: decode_build(reader_ref)?,
            capabilities: decode_capabilities(reader_ref)?,
        }
    }
);

wire_type!(
    RunBatchRequest,
    |this, writer_ref| {
        writer_ref.string(&this.scenario_id)?;
        writer_ref.len(this.candidates.len())?;
        for candidate in &this.candidates {
            writer_ref.u64(candidate.candidate_id);
            writer_ref.bytes(&candidate.tape)?;
            writer_ref.u64(candidate.start_tick);
            writer_ref.u64(candidate.max_ticks);
            writer_ref.u64(candidate.observation_mask);
            writer_ref.u64(candidate.oracle_mask);
        }
    },
    |reader_ref| {
        let scenario_id = reader_ref.string()?;
        let len = reader_ref.len()?;
        let mut candidates = Vec::with_capacity(len);
        for _ in 0..len {
            candidates.push(RunCandidate {
                candidate_id: reader_ref.u64()?,
                tape: reader_ref.bytes()?,
                start_tick: reader_ref.u64()?,
                max_ticks: reader_ref.u64()?,
                observation_mask: reader_ref.u64()?,
                oracle_mask: reader_ref.u64()?,
            });
        }
        Self {
            scenario_id,
            candidates,
        }
    }
);

wire_type!(
    RunBatchResponse,
    |this, writer_ref| {
        writer_ref.len(this.outcomes.len())?;
        for outcome in &this.outcomes {
            writer_ref.u64(outcome.candidate_id);
            writer_ref.u8(outcome.reason as u8);
            writer_ref.u64(outcome.completed_ticks);
            writer_ref.digest(outcome.terminal_state);
            writer_ref.u32(outcome.event_count);
        }
    },
    |reader_ref| {
        let len = reader_ref.len()?;
        let mut outcomes = Vec::with_capacity(len);
        for _ in 0..len {
            outcomes.push(RunOutcome {
                candidate_id: reader_ref.u64()?,
                reason: TerminalReason::decode(reader_ref.u8()?)?,
                completed_ticks: reader_ref.u64()?,
                terminal_state: reader_ref.digest()?,
                event_count: reader_ref.u32()?,
            });
        }
        Self { outcomes }
    }
);

wire_type!(
    ReplayRequest,
    |this, writer_ref| {
        encode_artifact(writer_ref, &this.artifact)?;
        writer_ref.bytes(&this.tape)?;
        writer_ref.u8(this.presentation as u8);
        writer_ref.u64(this.present_from_tick);
        writer_ref.u64(this.max_ticks);
    },
    |reader_ref| {
        Self {
            artifact: decode_artifact(reader_ref)?,
            tape: reader_ref.bytes()?,
            presentation: PresentationMode::decode(reader_ref.u8()?)?,
            present_from_tick: reader_ref.u64()?,
            max_ticks: reader_ref.u64()?,
        }
    }
);

wire_type!(
    ReplayResponse,
    |this, writer_ref| {
        writer_ref.u64(this.completed_ticks);
        writer_ref.bool(this.verified);
        writer_ref.option_u64(this.divergence_tick);
        writer_ref.digest(this.terminal_state);
    },
    |reader_ref| {
        Self {
            completed_ticks: reader_ref.u64()?,
            verified: reader_ref.bool()?,
            divergence_tick: reader_ref.option_u64()?,
            terminal_state: reader_ref.digest()?,
        }
    }
);

wire_type!(
    Ack,
    |this, writer_ref| {
        writer_ref.string(&this.message)?;
    },
    |reader_ref| {
        Self {
            message: reader_ref.string()?,
        }
    }
);

wire_type!(
    WorkerError,
    |this, writer_ref| {
        writer_ref.u32(this.code);
        writer_ref.string(&this.message)?;
    },
    |reader_ref| {
        Self {
            code: reader_ref.u32()?,
            message: reader_ref.string()?,
        }
    }
);

#[derive(Default)]
struct WireWriter(Vec<u8>);

impl WireWriter {
    fn finish(self) -> Vec<u8> {
        self.0
    }
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }
    fn u16(&mut self, value: u16) {
        self.0.extend(value.to_le_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.0.extend(value.to_le_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.0.extend(value.to_le_bytes());
    }
    fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }
    fn digest(&mut self, value: Digest) {
        self.0.extend(value.0);
    }
    fn len(&mut self, len: usize) -> Result<(), ProtocolError> {
        self.u32(u32::try_from(len).map_err(|_| ProtocolError::LengthOverflow(len))?);
        Ok(())
    }
    fn bytes(&mut self, value: &[u8]) -> Result<(), ProtocolError> {
        self.len(value.len())?;
        self.0.extend(value);
        Ok(())
    }
    fn string(&mut self, value: &str) -> Result<(), ProtocolError> {
        self.bytes(value.as_bytes())
    }
    fn option_digest(&mut self, value: Option<Digest>) {
        self.bool(value.is_some());
        if let Some(value) = value {
            self.digest(value);
        }
    }
    fn option_u64(&mut self, value: Option<u64>) {
        self.bool(value.is_some());
        if let Some(value) = value {
            self.u64(value);
        }
    }
}

struct WireReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> WireReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }
    fn take(&mut self, len: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self
            .cursor
            .checked_add(len)
            .ok_or(ProtocolError::Truncated)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(ProtocolError::Truncated)?;
        self.cursor = end;
        Ok(value)
    }
    fn finish(self) -> Result<(), ProtocolError> {
        let remaining = self.bytes.len() - self.cursor;
        if remaining == 0 {
            Ok(())
        } else {
            Err(ProtocolError::TrailingBytes(remaining))
        }
    }
    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ProtocolError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, ProtocolError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, ProtocolError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bool(&mut self) -> Result<bool, ProtocolError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(ProtocolError::InvalidBool(value)),
        }
    }
    fn len(&mut self) -> Result<usize, ProtocolError> {
        Ok(self.u32()? as usize)
    }
    fn bytes(&mut self) -> Result<Vec<u8>, ProtocolError> {
        let len = self.len()?;
        if len > MAX_PAYLOAD_LEN {
            return Err(ProtocolError::PayloadTooLarge(len));
        }
        Ok(self.take(len)?.to_vec())
    }
    fn string(&mut self) -> Result<String, ProtocolError> {
        String::from_utf8(self.bytes()?).map_err(|_| ProtocolError::InvalidUtf8)
    }
    fn digest(&mut self) -> Result<Digest, ProtocolError> {
        Ok(Digest(self.take(32)?.try_into().unwrap()))
    }
    fn option_digest(&mut self) -> Result<Option<Digest>, ProtocolError> {
        if self.bool()? {
            Ok(Some(self.digest()?))
        } else {
            Ok(None)
        }
    }
    fn option_u64(&mut self) -> Result<Option<u64>, ProtocolError> {
        if self.bool()? {
            Ok(Some(self.u64()?))
        } else {
            Ok(None)
        }
    }
}

fn encode_build(writer: &mut WireWriter, build: &BuildIdentity) -> Result<(), ProtocolError> {
    writer.string(&build.dusklight_commit)?;
    writer.string(&build.aurora_commit)?;
    writer.string(&build.compiler)?;
    writer.string(&build.target)?;
    writer.string(&build.profile)?;
    writer.digest(build.feature_digest);
    writer.digest(build.game_digest);
    writer.option_digest(build.dirty_digest);
    writer.string(&build.fidelity_profile)?;
    Ok(())
}

fn decode_build(reader: &mut WireReader<'_>) -> Result<BuildIdentity, ProtocolError> {
    Ok(BuildIdentity {
        dusklight_commit: reader.string()?,
        aurora_commit: reader.string()?,
        compiler: reader.string()?,
        target: reader.string()?,
        profile: reader.string()?,
        feature_digest: reader.digest()?,
        game_digest: reader.digest()?,
        dirty_digest: reader.option_digest()?,
        fidelity_profile: reader.string()?,
    })
}

fn encode_artifact(
    writer: &mut WireWriter,
    artifact: &ArtifactIdentity,
) -> Result<(), ProtocolError> {
    writer.u16(artifact.schema_version);
    writer.digest(artifact.content_digest);
    encode_build(writer, &artifact.build)?;
    writer.string(&artifact.scenario_id)?;
    Ok(())
}

fn decode_artifact(reader: &mut WireReader<'_>) -> Result<ArtifactIdentity, ProtocolError> {
    Ok(ArtifactIdentity {
        schema_version: reader.u16()?,
        content_digest: reader.digest()?,
        build: decode_build(reader)?,
        scenario_id: reader.string()?,
    })
}

fn encode_capabilities(writer: &mut WireWriter, caps: &Capabilities) {
    writer.u64(caps.flags);
    writer.u8(caps.max_controllers);
    writer.u8(caps.checkpoint_tier);
    writer.u32(caps.max_batch_size);
}

fn decode_capabilities(reader: &mut WireReader<'_>) -> Result<Capabilities, ProtocolError> {
    Ok(Capabilities {
        flags: reader.u64()?,
        max_controllers: reader.u8()?,
        checkpoint_tier: reader.u8()?,
        max_batch_size: reader.u32()?,
    })
}

fn read_u16(reader: &mut impl Read) -> io::Result<u16> {
    let mut b = [0; 2];
    reader.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
}
fn read_u32(reader: &mut impl Read) -> io::Result<u32> {
    let mut b = [0; 4];
    reader.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn read_u64(reader: &mut impl Read) -> io::Result<u64> {
    let mut b = [0; 8];
    reader.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build() -> BuildIdentity {
        BuildIdentity {
            dusklight_commit: "abc".into(),
            aurora_commit: "def".into(),
            compiler: "clang".into(),
            target: "x86_64-pc-windows-msvc".into(),
            profile: "release".into(),
            feature_digest: Digest([1; 32]),
            game_digest: Digest([2; 32]),
            dirty_digest: Some(Digest([3; 32])),
            fidelity_profile: "safe".into(),
        }
    }

    #[test]
    fn frame_round_trip() {
        let frame = Frame::new(MessageKind::RunBatchRequest, 42, vec![1, 2, 3]);
        let mut bytes = Vec::new();
        frame.write_to(&mut bytes).unwrap();
        assert_eq!(Frame::read_from(bytes.as_slice()).unwrap(), Some(frame));
    }

    #[test]
    fn hello_round_trip() {
        let hello = HelloResponse {
            selected_protocol: 1,
            worker_name: "worker".into(),
            build: build(),
            capabilities: Capabilities {
                flags: 7,
                max_controllers: 4,
                checkpoint_tier: 1,
                max_batch_size: 128,
            },
        };
        assert_eq!(
            HelloResponse::decode(&hello.encode().unwrap()).unwrap(),
            hello
        );
    }

    #[test]
    fn request_round_trips() {
        let batch = RunBatchRequest {
            scenario_id: "title".into(),
            candidates: vec![RunCandidate {
                candidate_id: 8,
                tape: vec![1, 2],
                start_tick: 3,
                max_ticks: 600,
                observation_mask: 4,
                oracle_mask: 5,
            }],
        };
        assert_eq!(
            RunBatchRequest::decode(&batch.encode().unwrap()).unwrap(),
            batch
        );

        let replay = ReplayRequest {
            artifact: ArtifactIdentity {
                schema_version: 1,
                content_digest: Digest([9; 32]),
                build: build(),
                scenario_id: "title".into(),
            },
            tape: vec![5, 6],
            presentation: PresentationMode::UnpacedHeadful,
            present_from_tick: 90,
            max_ticks: 300,
        };
        assert_eq!(
            ReplayRequest::decode(&replay.encode().unwrap()).unwrap(),
            replay
        );
    }

    #[test]
    fn rejects_trailing_payload() {
        let mut bytes = Ack {
            message: "ok".into(),
        }
        .encode()
        .unwrap();
        bytes.push(0);
        assert!(matches!(
            Ack::decode(&bytes),
            Err(ProtocolError::TrailingBytes(1))
        ));
    }
}
