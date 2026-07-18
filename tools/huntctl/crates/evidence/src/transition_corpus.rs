//! Compact, deterministic transition batches for native learning tools.
//!
//! The payload is deliberately not serde-based. All integers and IEEE-754 bit
//! patterns are written explicitly in little-endian order, and every variable
//! length is bounded before allocation.

use crate::artifact::Digest;
use sha2::{Digest as ShaDigest, Sha256};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::Path;

const MAGIC: &[u8; 8] = b"DUSKTRN\0";
const ZSTD_MAGIC: &[u8; 8] = b"DUSKTCZ\0";
const HEADER_PREFIX_SIZE: usize = 92;
const HEADER_SIZE: usize = 124;
const ZSTD_HEADER_SIZE: usize = 52;

pub const TRANSITION_CORPUS_VERSION: u16 = 1;
pub const TRANSITION_CORPUS_ZSTD_VERSION: u16 = 1;
pub const MAX_TRANSITIONS: usize = 1_000_000;
pub const MAX_FEATURES: usize = 4_096;
pub const MAX_ACTION_PARAMETERS: usize = 64;
pub const MAX_ENCODED_BYTES: usize = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StateReferenceKind {
    Boundary = 0,
    Snapshot = 1,
}

impl StateReferenceKind {
    fn decode(value: u8) -> Result<Self, TransitionCorpusError> {
        match value {
            0 => Ok(Self::Boundary),
            1 => Ok(Self::Snapshot),
            _ => Err(TransitionCorpusError::InvalidReferenceKind(value)),
        }
    }
}

/// Content identity of the state on either side of a transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StateReference {
    pub kind: StateReferenceKind,
    pub digest: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroAction {
    /// Learner-visible discrete action ID.
    pub action_id: u32,
    /// Environment-defined macro family/version ID.
    pub macro_kind: u16,
    /// Small signed integral metadata, such as stick coordinates or buttons.
    pub parameters: Vec<i16>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub source: StateReference,
    pub state: Vec<f32>,
    pub action: MacroAction,
    pub duration_ticks: u32,
    pub reward: f32,
    pub next: StateReference,
    pub next_state: Vec<f32>,
    pub terminal: bool,
}

/// A batch has one fixed, content-identified feature and action schema. Fixed
/// feature width keeps records compact and prevents ragged inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionCorpus {
    /// Ordered feature names, types, units, and normalization rules.
    pub feature_schema: Digest,
    /// Action IDs, macro meanings, and parameter layouts.
    pub action_schema: Digest,
    pub feature_count: u32,
    pub transitions: Vec<Transition>,
}

#[derive(Debug)]
pub enum TransitionCorpusError {
    Io(std::io::Error),
    BadMagic,
    BadZstdMagic,
    UnsupportedVersion(u16),
    UnsupportedZstdVersion(u16),
    NonzeroFlags(u16),
    InvalidFeatureCount(u32),
    MissingSchema(&'static str),
    TooManyTransitions(usize),
    TooManyActionParameters {
        transition: usize,
        count: usize,
    },
    FeatureWidthMismatch {
        transition: usize,
        expected: usize,
        state: usize,
        next_state: usize,
    },
    NonCanonicalFloat {
        transition: usize,
        field: &'static str,
        index: usize,
    },
    InvalidReferenceKind(u8),
    InvalidTerminal(u8),
    InvalidDuration {
        transition: usize,
    },
    Truncated,
    TrailingData(usize),
    LengthOverflow,
    EncodedSizeLimit {
        size: u64,
        limit: usize,
    },
    PayloadLengthMismatch {
        expected: u64,
        received: usize,
    },
    IntegrityMismatch {
        expected: Digest,
        received: Digest,
    },
    Compression(String),
}

impl fmt::Display for TransitionCorpusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "transition corpus I/O error: {error}"),
            Self::BadMagic => f.write_str("invalid transition corpus magic"),
            Self::BadZstdMagic => f.write_str("invalid compressed transition corpus magic"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported transition corpus version {version}")
            }
            Self::UnsupportedZstdVersion(version) => {
                write!(
                    f,
                    "unsupported compressed transition corpus version {version}"
                )
            }
            Self::NonzeroFlags(flags) => write!(f, "unsupported corpus flags {flags:#06x}"),
            Self::InvalidFeatureCount(count) => {
                write!(f, "feature count {count} is outside the supported range")
            }
            Self::MissingSchema(schema) => write!(f, "{schema} schema digest must not be zero"),
            Self::TooManyTransitions(count) => {
                write!(f, "transition count {count} exceeds {MAX_TRANSITIONS}")
            }
            Self::TooManyActionParameters { transition, count } => write!(
                f,
                "transition {transition} has {count} action parameters; maximum is {MAX_ACTION_PARAMETERS}"
            ),
            Self::FeatureWidthMismatch {
                transition,
                expected,
                state,
                next_state,
            } => write!(
                f,
                "transition {transition} feature widths are {state}/{next_state}; expected {expected}"
            ),
            Self::NonCanonicalFloat {
                transition,
                field,
                index,
            } => write!(
                f,
                "transition {transition} {field}[{index}] is not a finite canonical f32"
            ),
            Self::InvalidReferenceKind(kind) => {
                write!(f, "invalid state reference kind {kind}")
            }
            Self::InvalidTerminal(value) => write!(f, "invalid terminal value {value}"),
            Self::InvalidDuration { transition } => {
                write!(f, "transition {transition} has zero duration")
            }
            Self::Truncated => f.write_str("truncated transition corpus"),
            Self::TrailingData(count) => write!(f, "transition corpus has {count} trailing bytes"),
            Self::LengthOverflow => f.write_str("transition corpus length overflow"),
            Self::EncodedSizeLimit { size, limit } => {
                write!(f, "encoded size {size} exceeds limit {limit}")
            }
            Self::PayloadLengthMismatch { expected, received } => write!(
                f,
                "payload length mismatch: header says {expected}, received {received}"
            ),
            Self::IntegrityMismatch { expected, received } => write!(
                f,
                "transition corpus SHA-256 mismatch: expected {expected}, received {received}"
            ),
            Self::Compression(message) => write!(f, "zstd transition corpus error: {message}"),
        }
    }
}

impl Error for TransitionCorpusError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for TransitionCorpusError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl TransitionCorpus {
    pub fn new(
        feature_schema: Digest,
        action_schema: Digest,
        feature_count: u32,
        transitions: Vec<Transition>,
    ) -> Result<Self, TransitionCorpusError> {
        let corpus = Self {
            feature_schema,
            action_schema,
            feature_count,
            transitions,
        };
        corpus.validate()?;
        Ok(corpus)
    }

    pub fn validate(&self) -> Result<(), TransitionCorpusError> {
        if self.feature_schema == Digest::ZERO {
            return Err(TransitionCorpusError::MissingSchema("feature"));
        }
        if self.action_schema == Digest::ZERO {
            return Err(TransitionCorpusError::MissingSchema("action"));
        }
        let feature_count = usize::try_from(self.feature_count)
            .map_err(|_| TransitionCorpusError::InvalidFeatureCount(self.feature_count))?;
        if feature_count == 0 || feature_count > MAX_FEATURES {
            return Err(TransitionCorpusError::InvalidFeatureCount(
                self.feature_count,
            ));
        }
        if self.transitions.len() > MAX_TRANSITIONS {
            return Err(TransitionCorpusError::TooManyTransitions(
                self.transitions.len(),
            ));
        }
        for (transition_index, transition) in self.transitions.iter().enumerate() {
            if transition.state.len() != feature_count
                || transition.next_state.len() != feature_count
            {
                return Err(TransitionCorpusError::FeatureWidthMismatch {
                    transition: transition_index,
                    expected: feature_count,
                    state: transition.state.len(),
                    next_state: transition.next_state.len(),
                });
            }
            if transition.action.parameters.len() > MAX_ACTION_PARAMETERS {
                return Err(TransitionCorpusError::TooManyActionParameters {
                    transition: transition_index,
                    count: transition.action.parameters.len(),
                });
            }
            if transition.duration_ticks == 0 {
                return Err(TransitionCorpusError::InvalidDuration {
                    transition: transition_index,
                });
            }
            validate_float(transition.reward, transition_index, "reward", 0)?;
            for (index, value) in transition.state.iter().copied().enumerate() {
                validate_float(value, transition_index, "state", index)?;
            }
            for (index, value) in transition.next_state.iter().copied().enumerate() {
                validate_float(value, transition_index, "next_state", index)?;
            }
        }
        Ok(())
    }

    /// Canonical uncompressed representation. The integrity field
    /// authenticates all header metadata and transition records with SHA-256.
    pub fn encode(&self) -> Result<Vec<u8>, TransitionCorpusError> {
        self.validate()?;
        let payload_capacity = estimated_payload_size(self)?;
        let total_size = HEADER_SIZE
            .checked_add(payload_capacity)
            .ok_or(TransitionCorpusError::LengthOverflow)?;
        ensure_size(total_size as u64)?;

        let mut payload = Vec::with_capacity(payload_capacity);
        for transition in &self.transitions {
            encode_reference(&mut payload, transition.source);
            encode_reference(&mut payload, transition.next);
            put_u32(&mut payload, transition.action.action_id);
            put_u16(&mut payload, transition.action.macro_kind);
            put_u16(
                &mut payload,
                u16::try_from(transition.action.parameters.len())
                    .map_err(|_| TransitionCorpusError::LengthOverflow)?,
            );
            put_u32(&mut payload, transition.duration_ticks);
            put_f32(&mut payload, transition.reward);
            payload.push(u8::from(transition.terminal));
            for value in &transition.state {
                put_f32(&mut payload, *value);
            }
            for value in &transition.next_state {
                put_f32(&mut payload, *value);
            }
            for parameter in &transition.action.parameters {
                payload.extend_from_slice(&parameter.to_le_bytes());
            }
        }

        let mut bytes = Vec::with_capacity(total_size);
        bytes.extend_from_slice(MAGIC);
        put_u16(&mut bytes, TRANSITION_CORPUS_VERSION);
        put_u16(&mut bytes, 0);
        put_u32(
            &mut bytes,
            u32::try_from(self.transitions.len())
                .map_err(|_| TransitionCorpusError::LengthOverflow)?,
        );
        put_u32(&mut bytes, self.feature_count);
        put_u64(
            &mut bytes,
            u64::try_from(payload.len()).map_err(|_| TransitionCorpusError::LengthOverflow)?,
        );
        bytes.extend_from_slice(self.feature_schema.as_bytes());
        bytes.extend_from_slice(self.action_schema.as_bytes());
        let integrity_digest = corpus_integrity_digest(&bytes, &payload);
        bytes.extend_from_slice(integrity_digest.as_bytes());
        debug_assert_eq!(bytes.len(), HEADER_SIZE);
        bytes.extend_from_slice(&payload);
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TransitionCorpusError> {
        ensure_size(bytes.len() as u64)?;
        if bytes.len() < HEADER_SIZE {
            return Err(TransitionCorpusError::Truncated);
        }
        if &bytes[..8] != MAGIC {
            return Err(TransitionCorpusError::BadMagic);
        }
        let mut header = Reader::new(&bytes[8..HEADER_SIZE]);
        let version = header.u16()?;
        if version != TRANSITION_CORPUS_VERSION {
            return Err(TransitionCorpusError::UnsupportedVersion(version));
        }
        let flags = header.u16()?;
        if flags != 0 {
            return Err(TransitionCorpusError::NonzeroFlags(flags));
        }
        let transition_count = header.u32()? as usize;
        if transition_count > MAX_TRANSITIONS {
            return Err(TransitionCorpusError::TooManyTransitions(transition_count));
        }
        let feature_count = header.u32()?;
        let feature_count_usize = feature_count as usize;
        if feature_count_usize == 0 || feature_count_usize > MAX_FEATURES {
            return Err(TransitionCorpusError::InvalidFeatureCount(feature_count));
        }
        let payload_len = header.u64()?;
        ensure_size(
            (HEADER_SIZE as u64)
                .checked_add(payload_len)
                .ok_or(TransitionCorpusError::LengthOverflow)?,
        )?;
        let feature_schema = Digest(header.array_32()?);
        if feature_schema == Digest::ZERO {
            return Err(TransitionCorpusError::MissingSchema("feature"));
        }
        let action_schema = Digest(header.array_32()?);
        if action_schema == Digest::ZERO {
            return Err(TransitionCorpusError::MissingSchema("action"));
        }
        let expected_digest = Digest(header.array_32()?);
        let payload = &bytes[HEADER_SIZE..];
        if payload_len != payload.len() as u64 {
            return Err(TransitionCorpusError::PayloadLengthMismatch {
                expected: payload_len,
                received: payload.len(),
            });
        }
        verify_corpus_digest(expected_digest, &bytes[..HEADER_PREFIX_SIZE], payload)?;

        let minimum_record_size = 83_usize
            .checked_add(
                feature_count_usize
                    .checked_mul(8)
                    .ok_or(TransitionCorpusError::LengthOverflow)?,
            )
            .ok_or(TransitionCorpusError::LengthOverflow)?;
        let minimum_payload = transition_count
            .checked_mul(minimum_record_size)
            .ok_or(TransitionCorpusError::LengthOverflow)?;
        if minimum_payload > payload.len() {
            return Err(TransitionCorpusError::Truncated);
        }

        let mut reader = Reader::new(payload);
        let mut transitions = Vec::with_capacity(transition_count);
        for transition_index in 0..transition_count {
            let source = decode_reference(&mut reader)?;
            let next = decode_reference(&mut reader)?;
            let action_id = reader.u32()?;
            let macro_kind = reader.u16()?;
            let parameter_count = reader.u16()? as usize;
            if parameter_count > MAX_ACTION_PARAMETERS {
                return Err(TransitionCorpusError::TooManyActionParameters {
                    transition: transition_index,
                    count: parameter_count,
                });
            }
            let duration_ticks = reader.u32()?;
            if duration_ticks == 0 {
                return Err(TransitionCorpusError::InvalidDuration {
                    transition: transition_index,
                });
            }
            let reward = reader.f32()?;
            validate_float(reward, transition_index, "reward", 0)?;
            let terminal = match reader.u8()? {
                0 => false,
                1 => true,
                value => return Err(TransitionCorpusError::InvalidTerminal(value)),
            };
            let state =
                decode_features(&mut reader, feature_count_usize, transition_index, "state")?;
            let next_state = decode_features(
                &mut reader,
                feature_count_usize,
                transition_index,
                "next_state",
            )?;
            let mut parameters = Vec::with_capacity(parameter_count);
            for _ in 0..parameter_count {
                parameters.push(reader.i16()?);
            }
            transitions.push(Transition {
                source,
                state,
                action: MacroAction {
                    action_id,
                    macro_kind,
                    parameters,
                },
                duration_ticks,
                reward,
                next,
                next_state,
                terminal,
            });
        }
        if reader.remaining() != 0 {
            return Err(TransitionCorpusError::TrailingData(reader.remaining()));
        }
        Self::new(feature_schema, action_schema, feature_count, transitions)
    }

    /// SHA-256 of the canonical, uncompressed file bytes.
    pub fn content_digest(&self) -> Result<Digest, TransitionCorpusError> {
        Ok(sha256(&self.encode()?))
    }

    pub fn write_zstd_file(
        &self,
        path: impl AsRef<Path>,
        compression_level: i32,
    ) -> Result<Digest, TransitionCorpusError> {
        let raw = self.encode()?;
        let raw_digest = sha256(&raw);
        let compressed = zstd::bulk::compress(&raw, compression_level)
            .map_err(|error| TransitionCorpusError::Compression(error.to_string()))?;
        let total_size = ZSTD_HEADER_SIZE
            .checked_add(compressed.len())
            .ok_or(TransitionCorpusError::LengthOverflow)?;
        ensure_size(total_size as u64)?;
        let mut bytes = Vec::with_capacity(total_size);
        bytes.extend_from_slice(ZSTD_MAGIC);
        put_u16(&mut bytes, TRANSITION_CORPUS_ZSTD_VERSION);
        put_u16(&mut bytes, 0);
        put_u64(
            &mut bytes,
            u64::try_from(raw.len()).map_err(|_| TransitionCorpusError::LengthOverflow)?,
        );
        bytes.extend_from_slice(raw_digest.as_bytes());
        debug_assert_eq!(bytes.len(), ZSTD_HEADER_SIZE);
        bytes.extend_from_slice(&compressed);
        fs::write(path, bytes)?;
        Ok(raw_digest)
    }

    pub fn read_zstd_file(path: impl AsRef<Path>) -> Result<Self, TransitionCorpusError> {
        let bytes = read_bounded_file(path.as_ref())?;
        if bytes.len() < ZSTD_HEADER_SIZE {
            return Err(TransitionCorpusError::Truncated);
        }
        if &bytes[..8] != ZSTD_MAGIC {
            return Err(TransitionCorpusError::BadZstdMagic);
        }
        let mut header = Reader::new(&bytes[8..ZSTD_HEADER_SIZE]);
        let version = header.u16()?;
        if version != TRANSITION_CORPUS_ZSTD_VERSION {
            return Err(TransitionCorpusError::UnsupportedZstdVersion(version));
        }
        let flags = header.u16()?;
        if flags != 0 {
            return Err(TransitionCorpusError::NonzeroFlags(flags));
        }
        let raw_len = header.u64()?;
        ensure_size(raw_len)?;
        let expected_digest = Digest(header.array_32()?);
        let raw = zstd::bulk::decompress(&bytes[ZSTD_HEADER_SIZE..], raw_len as usize)
            .map_err(|error| TransitionCorpusError::Compression(error.to_string()))?;
        if raw.len() as u64 != raw_len {
            return Err(TransitionCorpusError::PayloadLengthMismatch {
                expected: raw_len,
                received: raw.len(),
            });
        }
        verify_digest(expected_digest, &raw)?;
        Self::decode(&raw)
    }
}

fn estimated_payload_size(corpus: &TransitionCorpus) -> Result<usize, TransitionCorpusError> {
    let feature_bytes = (corpus.feature_count as usize)
        .checked_mul(8)
        .ok_or(TransitionCorpusError::LengthOverflow)?;
    let mut size = 0_usize;
    for transition in &corpus.transitions {
        size = size
            .checked_add(83)
            .and_then(|value| value.checked_add(feature_bytes))
            .and_then(|value| value.checked_add(transition.action.parameters.len() * 2))
            .ok_or(TransitionCorpusError::LengthOverflow)?;
    }
    Ok(size)
}

fn validate_float(
    value: f32,
    transition: usize,
    field: &'static str,
    index: usize,
) -> Result<(), TransitionCorpusError> {
    // Negative zero has the same numerical value as positive zero and would
    // otherwise permit two byte encodings for one logical corpus.
    if !value.is_finite() || value.to_bits() == (-0.0_f32).to_bits() {
        Err(TransitionCorpusError::NonCanonicalFloat {
            transition,
            field,
            index,
        })
    } else {
        Ok(())
    }
}

fn encode_reference(bytes: &mut Vec<u8>, reference: StateReference) {
    bytes.push(reference.kind as u8);
    bytes.extend_from_slice(reference.digest.as_bytes());
}

fn decode_reference(reader: &mut Reader<'_>) -> Result<StateReference, TransitionCorpusError> {
    Ok(StateReference {
        kind: StateReferenceKind::decode(reader.u8()?)?,
        digest: Digest(reader.array_32()?),
    })
}

fn decode_features(
    reader: &mut Reader<'_>,
    count: usize,
    transition: usize,
    field: &'static str,
) -> Result<Vec<f32>, TransitionCorpusError> {
    let mut features = Vec::with_capacity(count);
    for index in 0..count {
        let value = reader.f32()?;
        validate_float(value, transition, field, index)?;
        features.push(value);
    }
    Ok(features)
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn corpus_integrity_digest(header_prefix: &[u8], payload: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(header_prefix);
    hasher.update(payload);
    Digest(hasher.finalize().into())
}

fn verify_corpus_digest(
    expected: Digest,
    header_prefix: &[u8],
    payload: &[u8],
) -> Result<(), TransitionCorpusError> {
    let received = corpus_integrity_digest(header_prefix, payload);
    if received == expected {
        Ok(())
    } else {
        Err(TransitionCorpusError::IntegrityMismatch { expected, received })
    }
}

fn verify_digest(expected: Digest, bytes: &[u8]) -> Result<(), TransitionCorpusError> {
    let received = sha256(bytes);
    if received == expected {
        Ok(())
    } else {
        Err(TransitionCorpusError::IntegrityMismatch { expected, received })
    }
}

fn ensure_size(size: u64) -> Result<(), TransitionCorpusError> {
    if size > MAX_ENCODED_BYTES as u64 {
        Err(TransitionCorpusError::EncodedSizeLimit {
            size,
            limit: MAX_ENCODED_BYTES,
        })
    } else {
        Ok(())
    }
}

fn read_bounded_file(path: &Path) -> Result<Vec<u8>, TransitionCorpusError> {
    let file = fs::File::open(path)?;
    let length = file.metadata()?.len();
    ensure_size(length)?;
    let capacity = usize::try_from(length).map_err(|_| TransitionCorpusError::LengthOverflow)?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take((MAX_ENCODED_BYTES as u64) + 1)
        .read_to_end(&mut bytes)?;
    ensure_size(bytes.len() as u64)?;
    Ok(bytes)
}

fn put_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_bits().to_le_bytes());
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], TransitionCorpusError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(TransitionCorpusError::LengthOverflow)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(TransitionCorpusError::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, TransitionCorpusError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, TransitionCorpusError> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().expect("two-byte slice"),
        ))
    }

    fn i16(&mut self) -> Result<i16, TransitionCorpusError> {
        Ok(i16::from_le_bytes(
            self.take(2)?.try_into().expect("two-byte slice"),
        ))
    }

    fn u32(&mut self) -> Result<u32, TransitionCorpusError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("four-byte slice"),
        ))
    }

    fn u64(&mut self) -> Result<u64, TransitionCorpusError> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("eight-byte slice"),
        ))
    }

    fn f32(&mut self) -> Result<f32, TransitionCorpusError> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn array_32(&mut self) -> Result<[u8; 32], TransitionCorpusError> {
        Ok(self.take(32)?.try_into().expect("32-byte slice"))
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn reference(kind: StateReferenceKind, byte: u8) -> StateReference {
        StateReference {
            kind,
            digest: Digest([byte; 32]),
        }
    }

    fn sample() -> TransitionCorpus {
        TransitionCorpus::new(
            Digest([0xf1; 32]),
            Digest([0xa1; 32]),
            3,
            vec![
                Transition {
                    source: reference(StateReferenceKind::Snapshot, 0x11),
                    state: vec![1.0, 2.5, -3.0],
                    action: MacroAction {
                        action_id: 7,
                        macro_kind: 2,
                        parameters: vec![-127, 127, 5],
                    },
                    duration_ticks: 4,
                    reward: -0.25,
                    next: reference(StateReferenceKind::Boundary, 0x22),
                    next_state: vec![1.5, 3.0, 0.0],
                    terminal: false,
                },
                Transition {
                    source: reference(StateReferenceKind::Boundary, 0x22),
                    state: vec![1.5, 3.0, 0.0],
                    action: MacroAction {
                        action_id: 9,
                        macro_kind: 3,
                        parameters: Vec::new(),
                    },
                    duration_ticks: 1,
                    reward: 100.0,
                    next: reference(StateReferenceKind::Snapshot, 0x33),
                    next_state: vec![2.0, 4.0, 1.0],
                    terminal: true,
                },
            ],
        )
        .unwrap()
    }

    #[test]
    fn canonical_round_trip() {
        let corpus = sample();
        let first = corpus.encode().unwrap();
        let decoded = TransitionCorpus::decode(&first).unwrap();
        assert_eq!(decoded, corpus);
        assert_eq!(decoded.encode().unwrap(), first);
        assert_eq!(decoded.content_digest().unwrap(), sha256(&first));
    }

    #[test]
    fn corruption_is_detected_before_record_decode() {
        let mut bytes = sample().encode().unwrap();
        *bytes.last_mut().unwrap() ^= 0x40;
        assert!(matches!(
            TransitionCorpus::decode(&bytes),
            Err(TransitionCorpusError::IntegrityMismatch { .. })
        ));
    }

    #[test]
    fn schema_identity_is_preserved_and_authenticated() {
        let corpus = sample();
        let original_digest = corpus.content_digest().unwrap();
        let mut other_schema = corpus.clone();
        other_schema.action_schema = Digest([0xa2; 32]);
        assert_ne!(other_schema.content_digest().unwrap(), original_digest);

        let mut bytes = corpus.encode().unwrap();
        // Feature schema starts after the 28-byte scalar header prefix.
        bytes[28] ^= 1;
        assert!(matches!(
            TransitionCorpus::decode(&bytes),
            Err(TransitionCorpusError::IntegrityMismatch { .. })
        ));
    }

    #[test]
    fn every_truncation_is_rejected() {
        let bytes = sample().encode().unwrap();
        for length in 0..bytes.len() {
            assert!(TransitionCorpus::decode(&bytes[..length]).is_err());
        }
    }

    #[test]
    fn noncanonical_floats_are_rejected_on_encode() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.0] {
            let mut corpus = sample();
            corpus.transitions[0].state[1] = value;
            assert!(matches!(
                corpus.encode(),
                Err(TransitionCorpusError::NonCanonicalFloat { .. })
            ));
        }
    }

    #[test]
    fn authenticated_noncanonical_float_is_rejected_on_decode() {
        let mut bytes = sample().encode().unwrap();
        // First record: 66 bytes of references, 17 bytes of action metadata,
        // then state[0]. Rewrite it to negative zero and re-authenticate.
        let offset = HEADER_SIZE + 83;
        bytes[offset..offset + 4].copy_from_slice(&(-0.0_f32).to_bits().to_le_bytes());
        let digest = corpus_integrity_digest(&bytes[..HEADER_PREFIX_SIZE], &bytes[HEADER_SIZE..]);
        bytes[HEADER_PREFIX_SIZE..HEADER_SIZE].copy_from_slice(digest.as_bytes());
        assert!(matches!(
            TransitionCorpus::decode(&bytes),
            Err(TransitionCorpusError::NonCanonicalFloat {
                field: "state",
                index: 0,
                ..
            })
        ));
    }

    #[test]
    fn limits_and_ragged_features_are_rejected() {
        assert!(matches!(
            TransitionCorpus::new(Digest([1; 32]), Digest([2; 32]), 0, Vec::new()),
            Err(TransitionCorpusError::InvalidFeatureCount(0))
        ));
        assert!(matches!(
            TransitionCorpus::new(Digest::ZERO, Digest([2; 32]), 1, Vec::new()),
            Err(TransitionCorpusError::MissingSchema("feature"))
        ));
        let mut corpus = sample();
        corpus.transitions[0].next_state.pop();
        assert!(matches!(
            corpus.validate(),
            Err(TransitionCorpusError::FeatureWidthMismatch { .. })
        ));
        let mut corpus = sample();
        corpus.transitions[0].action.parameters = vec![0; MAX_ACTION_PARAMETERS + 1];
        assert!(matches!(
            corpus.validate(),
            Err(TransitionCorpusError::TooManyActionParameters { .. })
        ));

        let mut bytes = sample().encode().unwrap();
        bytes[12..16].copy_from_slice(&((MAX_TRANSITIONS as u32) + 1).to_le_bytes());
        assert!(matches!(
            TransitionCorpus::decode(&bytes),
            Err(TransitionCorpusError::TooManyTransitions(_))
        ));

        let mut bytes = sample().encode().unwrap();
        bytes[20..28].copy_from_slice(&((MAX_ENCODED_BYTES as u64) + 1).to_le_bytes());
        assert!(matches!(
            TransitionCorpus::decode(&bytes),
            Err(TransitionCorpusError::EncodedSizeLimit { .. })
        ));
    }

    #[test]
    fn compressed_file_round_trip_and_corruption() {
        let corpus = sample();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "dusklight-transition-corpus-{}-{nonce}.dtcz",
            std::process::id()
        ));
        let expected_digest = corpus.write_zstd_file(&path, 1).unwrap();
        assert_eq!(TransitionCorpus::read_zstd_file(&path).unwrap(), corpus);
        assert_eq!(expected_digest, corpus.content_digest().unwrap());

        let mut bytes = fs::read(&path).unwrap();
        bytes[20] ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(TransitionCorpus::read_zstd_file(&path).is_err());
        fs::remove_file(path).unwrap();
    }
}
