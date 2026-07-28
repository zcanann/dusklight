//! Durable campaign-owned admission of authenticated tactic experience.
//!
//! Native lanes may execute independently, but they publish evidence through
//! this one append-only authority. The journal keeps only small typed content
//! references; full transitions and routes live in the campaign content store.

use crate::tactic_q_campaign::{TacticQTrainingCorpus, validate_training_corpus};
use crate::tactic_q_checkpoint_store::{StoredContentRef, TacticQContentStore};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_learning::option_transition::OptionTransitionSample;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const TACTIC_REPLAY_CONTROL_PLANE_SCHEMA_V1: &str = "dusklight-tactic-replay-control-plane/v1";
pub const TACTIC_REPLAY_ADMISSION_SCHEMA_V1: &str = "dusklight-tactic-replay-admission/v1";
pub const TACTIC_REPLAY_SNAPSHOT_SCHEMA_V1: &str = "dusklight-tactic-replay-snapshot/v1";

const JOURNAL_MAGIC: &[u8; 8] = b"DSKTRP01";
const JOURNAL_VERSION: u16 = 1;
const JOURNAL_HEADER_BYTES: usize = 8 + 2 + 2 + 4 + 32;
const RECORD_HEADER_BYTES: usize = 4 + 4 + 32;
const MAXIMUM_IDENTITY_BYTES: usize = 64 * 1024;
const MAXIMUM_RECORD_BYTES: usize = 256 * 1024 * 1024;
const MAXIMUM_RECORDS: usize = 10_000_000;
const RECORD_COMPRESSION_LEVEL: i32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticReplayControlPlaneIdentity {
    pub schema: String,
    pub execution_authority_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub objective_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
}

impl TacticReplayControlPlaneIdentity {
    pub fn new(
        execution_authority_sha256: Digest,
        feature_schema_sha256: Digest,
        objective_sha256: Digest,
        root_checkpoint_sha256: Digest,
    ) -> Result<Self, TacticReplayControlPlaneError> {
        let identity = Self {
            schema: TACTIC_REPLAY_CONTROL_PLANE_SCHEMA_V1.into(),
            execution_authority_sha256,
            feature_schema_sha256,
            objective_sha256,
            root_checkpoint_sha256,
        };
        identity.validate()?;
        Ok(identity)
    }

    pub fn content_sha256(&self) -> Result<Digest, TacticReplayControlPlaneError> {
        self.validate()?;
        digest_cbor(b"dusklight.tactic-replay-control-plane-identity/v1\0", self)
    }

    fn validate(&self) -> Result<(), TacticReplayControlPlaneError> {
        if self.schema != TACTIC_REPLAY_CONTROL_PLANE_SCHEMA_V1
            || self.execution_authority_sha256 == Digest::ZERO
            || self.feature_schema_sha256 == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || self.root_checkpoint_sha256 == Digest::ZERO
        {
            return Err(TacticReplayControlPlaneError::Invalid(
                "replay control-plane identity is invalid",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredTacticReplayAdmission {
    schema: String,
    sequence: u64,
    publisher_lane: u32,
    publisher_decision: u64,
    learner_snapshot_sha256: Digest,
    transition_identity_sha256: Digest,
    transition: StoredContentRef,
    route: StoredContentRef,
    episode_group: u64,
    parent_replay_snapshot_sha256: Digest,
    replay_snapshot_sha256: Digest,
}

impl StoredTacticReplayAdmission {
    fn admission_sha256(&self) -> Result<Digest, TacticReplayControlPlaneError> {
        digest_cbor(
            b"dusklight.tactic-replay-admission/v1\0",
            &(
                &self.schema,
                self.sequence,
                self.publisher_lane,
                self.publisher_decision,
                self.learner_snapshot_sha256,
                self.transition_identity_sha256,
                self.transition,
                self.route,
                self.episode_group,
                self.parent_replay_snapshot_sha256,
            ),
        )
    }

    fn expected_snapshot_sha256(&self) -> Result<Digest, TacticReplayControlPlaneError> {
        digest_cbor(
            b"dusklight.tactic-replay-snapshot-step/v1\0",
            &(self.parent_replay_snapshot_sha256, self.admission_sha256()?),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TacticReplaySnapshotVersion {
    pub revision: u64,
    pub sha256: Digest,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TacticReplaySnapshot {
    pub schema: String,
    pub version: TacticReplaySnapshotVersion,
    pub corpus: TacticQTrainingCorpus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TacticReplayAdmissionMetadata {
    pub sequence: u64,
    pub publisher_lane: u32,
    pub publisher_decision: u64,
    pub learner_snapshot_sha256: Digest,
    pub transition_identity_sha256: Digest,
    pub episode_group: u64,
    pub replay_snapshot: TacticReplaySnapshotVersion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TacticReplayAdmissionOutcome {
    Admitted {
        sequence: u64,
        transition_identity_sha256: Digest,
        replay_snapshot: TacticReplaySnapshotVersion,
    },
    Duplicate {
        existing_sequence: u64,
        transition_identity_sha256: Digest,
        replay_snapshot: TacticReplaySnapshotVersion,
    },
}

/// One durable replay/frontier authority for a native route campaign.
///
/// `publish` synchronizes the complete journal record before advancing the
/// in-memory revision. Opening the journal removes only an incomplete final
/// record; checksum failures and detached content are hard errors.
pub struct TacticReplayControlPlane {
    identity: TacticReplayControlPlaneIdentity,
    journal_path: PathBuf,
    journal: File,
    content_store: TacticQContentStore,
    entries: Vec<StoredTacticReplayAdmission>,
    transition_sequences: BTreeMap<Digest, u64>,
    replay_snapshot: TacticReplaySnapshotVersion,
}

impl TacticReplayControlPlane {
    pub fn create(
        journal_path: impl Into<PathBuf>,
        content_root: impl Into<PathBuf>,
        identity: TacticReplayControlPlaneIdentity,
    ) -> Result<Self, TacticReplayControlPlaneError> {
        identity.validate()?;
        let journal_path = journal_path.into();
        let content_root = content_root.into();
        let parent = journal_path
            .parent()
            .ok_or(TacticReplayControlPlaneError::Invalid(
                "replay journal path has no parent",
            ))?;
        fs::create_dir_all(parent).map_err(io_error)?;
        let content_store = TacticQContentStore::initialize(content_root).map_err(store_error)?;
        let mut journal = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&journal_path)
            .map_err(io_error)?;
        let identity_raw = serde_cbor::to_vec(&identity).map_err(serialization_error)?;
        if identity_raw.is_empty() || identity_raw.len() > MAXIMUM_IDENTITY_BYTES {
            return Err(TacticReplayControlPlaneError::Invalid(
                "replay control-plane identity is oversized",
            ));
        }
        let identity_sha256 = identity.content_sha256()?;
        let mut header = Vec::with_capacity(JOURNAL_HEADER_BYTES + identity_raw.len());
        header.extend_from_slice(JOURNAL_MAGIC);
        header.extend_from_slice(&JOURNAL_VERSION.to_le_bytes());
        header.extend_from_slice(&0_u16.to_le_bytes());
        header.extend_from_slice(
            &u32::try_from(identity_raw.len())
                .map_err(|_| {
                    TacticReplayControlPlaneError::Invalid(
                        "replay control-plane identity is oversized",
                    )
                })?
                .to_le_bytes(),
        );
        header.extend_from_slice(&identity_sha256.0);
        header.extend_from_slice(&identity_raw);
        journal.write_all(&header).map_err(io_error)?;
        journal.sync_all().map_err(io_error)?;
        let replay_snapshot = initial_snapshot_version(&identity)?;
        Ok(Self {
            identity,
            journal_path,
            journal,
            content_store,
            entries: Vec::new(),
            transition_sequences: BTreeMap::new(),
            replay_snapshot,
        })
    }

    pub fn open(
        journal_path: impl Into<PathBuf>,
        content_root: impl Into<PathBuf>,
        expected_identity: &TacticReplayControlPlaneIdentity,
    ) -> Result<Self, TacticReplayControlPlaneError> {
        expected_identity.validate()?;
        let journal_path = journal_path.into();
        let content_store = TacticQContentStore::open(content_root.into()).map_err(store_error)?;
        let mut journal = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&journal_path)
            .map_err(io_error)?;
        let file_len = journal.metadata().map_err(io_error)?.len();
        if file_len < JOURNAL_HEADER_BYTES as u64 {
            return Err(TacticReplayControlPlaneError::Invalid(
                "replay control-plane journal header is truncated",
            ));
        }
        let mut fixed_header = [0_u8; JOURNAL_HEADER_BYTES];
        journal.read_exact(&mut fixed_header).map_err(io_error)?;
        let version = u16::from_le_bytes(fixed_header[8..10].try_into().expect("fixed slice"));
        let flags = u16::from_le_bytes(fixed_header[10..12].try_into().expect("fixed slice"));
        let identity_len =
            u32::from_le_bytes(fixed_header[12..16].try_into().expect("fixed slice")) as usize;
        let expected_identity_sha256 =
            Digest(fixed_header[16..48].try_into().expect("fixed slice"));
        if &fixed_header[..8] != JOURNAL_MAGIC
            || version != JOURNAL_VERSION
            || flags != 0
            || identity_len == 0
            || identity_len > MAXIMUM_IDENTITY_BYTES
            || file_len < (JOURNAL_HEADER_BYTES + identity_len) as u64
        {
            return Err(TacticReplayControlPlaneError::Invalid(
                "replay control-plane journal header is invalid",
            ));
        }
        let mut identity_raw = vec![0_u8; identity_len];
        journal.read_exact(&mut identity_raw).map_err(io_error)?;
        let identity: TacticReplayControlPlaneIdentity =
            decode_cbor(&identity_raw).map_err(serialization_error)?;
        if identity != *expected_identity || identity.content_sha256()? != expected_identity_sha256
        {
            return Err(TacticReplayControlPlaneError::Invalid(
                "replay control-plane journal belongs to another campaign",
            ));
        }

        let mut entries = Vec::new();
        let mut transition_sequences = BTreeMap::new();
        let mut replay_snapshot = initial_snapshot_version(&identity)?;
        let mut valid_len = (JOURNAL_HEADER_BYTES + identity_len) as u64;
        loop {
            if entries.len() >= MAXIMUM_RECORDS {
                return Err(TacticReplayControlPlaneError::Invalid(
                    "replay control-plane journal exceeds its record bound",
                ));
            }
            let remaining = file_len.saturating_sub(valid_len);
            if remaining == 0 {
                break;
            }
            if remaining < RECORD_HEADER_BYTES as u64 {
                break;
            }
            let mut record_header = [0_u8; RECORD_HEADER_BYTES];
            journal.read_exact(&mut record_header).map_err(io_error)?;
            let compressed_len =
                u32::from_le_bytes(record_header[..4].try_into().expect("fixed slice")) as usize;
            let raw_len =
                u32::from_le_bytes(record_header[4..8].try_into().expect("fixed slice")) as usize;
            let expected_sha256 = Digest(record_header[8..40].try_into().expect("fixed slice"));
            if compressed_len == 0
                || raw_len == 0
                || compressed_len > MAXIMUM_RECORD_BYTES
                || raw_len > MAXIMUM_RECORD_BYTES
            {
                return Err(TacticReplayControlPlaneError::Invalid(
                    "replay control-plane record size is invalid",
                ));
            }
            let record_bytes = RECORD_HEADER_BYTES.checked_add(compressed_len).ok_or(
                TacticReplayControlPlaneError::Invalid(
                    "replay control-plane record size overflowed",
                ),
            )?;
            if remaining < record_bytes as u64 {
                break;
            }
            let mut compressed = vec![0_u8; compressed_len];
            journal.read_exact(&mut compressed).map_err(io_error)?;
            let raw = zstd::stream::decode_all(Cursor::new(compressed)).map_err(io_error)?;
            if raw.len() != raw_len || sha256(&raw) != expected_sha256 {
                return Err(TacticReplayControlPlaneError::Invalid(
                    "replay control-plane record checksum is invalid",
                ));
            }
            let entry: StoredTacticReplayAdmission =
                decode_cbor(&raw).map_err(serialization_error)?;
            validate_stored_entry(
                &identity,
                &content_store,
                &entry,
                entries.len() as u64,
                replay_snapshot.sha256,
            )?;
            if transition_sequences
                .insert(entry.transition_identity_sha256, entry.sequence)
                .is_some()
            {
                return Err(TacticReplayControlPlaneError::Invalid(
                    "replay control-plane journal repeats transition authority",
                ));
            }
            replay_snapshot = TacticReplaySnapshotVersion {
                revision: entry.sequence + 1,
                sha256: entry.replay_snapshot_sha256,
            };
            entries.push(entry);
            valid_len = valid_len.saturating_add(record_bytes as u64);
        }
        if valid_len != file_len {
            journal.set_len(valid_len).map_err(io_error)?;
            journal.sync_all().map_err(io_error)?;
        }
        journal.seek(SeekFrom::End(0)).map_err(io_error)?;
        Ok(Self {
            identity,
            journal_path,
            journal,
            content_store,
            entries,
            transition_sequences,
            replay_snapshot,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn publish(
        &mut self,
        publisher_lane: u32,
        publisher_decision: u64,
        learner_snapshot_sha256: Digest,
        transition: &OptionTransitionSample,
        route: &InputTape,
        episode_group: u64,
    ) -> Result<TacticReplayAdmissionOutcome, TacticReplayControlPlaneError> {
        if learner_snapshot_sha256 == Digest::ZERO {
            return Err(TacticReplayControlPlaneError::Invalid(
                "published replay has no learner snapshot authority",
            ));
        }
        let corpus = TacticQTrainingCorpus {
            execution_authority_sha256: self.identity.execution_authority_sha256,
            feature_schema_sha256: self.identity.feature_schema_sha256,
            objective_sha256: self.identity.objective_sha256,
            root_checkpoint_sha256: self.identity.root_checkpoint_sha256,
            transitions: vec![transition.clone()],
            routes: vec![route.clone()],
            episode_groups: vec![episode_group],
        };
        validate_training_corpus(&corpus).map_err(domain_error)?;
        let transition_identity_sha256 =
            transition.replay_identity_sha256().map_err(domain_error)?;
        if let Some(existing_sequence) = self
            .transition_sequences
            .get(&transition_identity_sha256)
            .copied()
        {
            return Ok(TacticReplayAdmissionOutcome::Duplicate {
                existing_sequence,
                transition_identity_sha256,
                replay_snapshot: self.replay_snapshot,
            });
        }
        if self.entries.len() >= MAXIMUM_RECORDS {
            return Err(TacticReplayControlPlaneError::Invalid(
                "replay control-plane journal exceeds its record bound",
            ));
        }
        let transition_ref = self
            .content_store
            .store_option_transition(transition, route)
            .map_err(store_error)?;
        let route_ref = self.content_store.store_tape(route).map_err(store_error)?;
        let sequence = self.entries.len() as u64;
        let mut entry = StoredTacticReplayAdmission {
            schema: TACTIC_REPLAY_ADMISSION_SCHEMA_V1.into(),
            sequence,
            publisher_lane,
            publisher_decision,
            learner_snapshot_sha256,
            transition_identity_sha256,
            transition: transition_ref,
            route: route_ref,
            episode_group,
            parent_replay_snapshot_sha256: self.replay_snapshot.sha256,
            replay_snapshot_sha256: Digest::ZERO,
        };
        entry.replay_snapshot_sha256 = entry.expected_snapshot_sha256()?;
        let raw = serde_cbor::to_vec(&entry).map_err(serialization_error)?;
        if raw.is_empty() || raw.len() > MAXIMUM_RECORD_BYTES {
            return Err(TacticReplayControlPlaneError::Invalid(
                "replay control-plane record is oversized",
            ));
        }
        let compressed = zstd::stream::encode_all(Cursor::new(&raw), RECORD_COMPRESSION_LEVEL)
            .map_err(io_error)?;
        if compressed.is_empty() || compressed.len() > MAXIMUM_RECORD_BYTES {
            return Err(TacticReplayControlPlaneError::Invalid(
                "compressed replay control-plane record is oversized",
            ));
        }
        let mut envelope = Vec::with_capacity(RECORD_HEADER_BYTES + compressed.len());
        envelope.extend_from_slice(
            &u32::try_from(compressed.len())
                .map_err(|_| {
                    TacticReplayControlPlaneError::Invalid(
                        "compressed replay control-plane record is oversized",
                    )
                })?
                .to_le_bytes(),
        );
        envelope.extend_from_slice(
            &u32::try_from(raw.len())
                .map_err(|_| {
                    TacticReplayControlPlaneError::Invalid(
                        "replay control-plane record is oversized",
                    )
                })?
                .to_le_bytes(),
        );
        envelope.extend_from_slice(&sha256(&raw).0);
        envelope.extend_from_slice(&compressed);
        self.journal.write_all(&envelope).map_err(io_error)?;
        self.journal.sync_data().map_err(io_error)?;
        self.replay_snapshot = TacticReplaySnapshotVersion {
            revision: sequence + 1,
            sha256: entry.replay_snapshot_sha256,
        };
        self.transition_sequences
            .insert(transition_identity_sha256, sequence);
        self.entries.push(entry);
        Ok(TacticReplayAdmissionOutcome::Admitted {
            sequence,
            transition_identity_sha256,
            replay_snapshot: self.replay_snapshot,
        })
    }

    pub fn identity(&self) -> &TacticReplayControlPlaneIdentity {
        &self.identity
    }

    pub fn journal_path(&self) -> &Path {
        &self.journal_path
    }

    pub fn replay_snapshot(&self) -> TacticReplaySnapshotVersion {
        self.replay_snapshot
    }

    pub fn admissions(&self) -> Vec<TacticReplayAdmissionMetadata> {
        self.entries
            .iter()
            .map(|entry| TacticReplayAdmissionMetadata {
                sequence: entry.sequence,
                publisher_lane: entry.publisher_lane,
                publisher_decision: entry.publisher_decision,
                learner_snapshot_sha256: entry.learner_snapshot_sha256,
                transition_identity_sha256: entry.transition_identity_sha256,
                episode_group: entry.episode_group,
                replay_snapshot: TacticReplaySnapshotVersion {
                    revision: entry.sequence + 1,
                    sha256: entry.replay_snapshot_sha256,
                },
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn snapshot(&self) -> Result<TacticReplaySnapshot, TacticReplayControlPlaneError> {
        self.snapshot_through(self.replay_snapshot.revision)
    }

    /// Materialize the immutable replay prefix at an exact admitted revision.
    ///
    /// Deterministic generation barriers use this to keep a valid, partially
    /// published next generation invisible after an interrupted resume.
    pub fn snapshot_through(
        &self,
        revision: u64,
    ) -> Result<TacticReplaySnapshot, TacticReplayControlPlaneError> {
        let end = usize::try_from(revision).map_err(|_| {
            TacticReplayControlPlaneError::Invalid("replay snapshot revision overflows")
        })?;
        if end > self.entries.len() {
            return Err(TacticReplayControlPlaneError::Invalid(
                "replay snapshot revision is outside the journal",
            ));
        }
        let version = if end == 0 {
            initial_snapshot_version(&self.identity)?
        } else {
            TacticReplaySnapshotVersion {
                revision,
                sha256: self.entries[end - 1].replay_snapshot_sha256,
            }
        };
        self.materialize_snapshot(&self.entries[..end], version)
    }

    pub fn snapshot_from(
        &self,
        first_sequence: u64,
    ) -> Result<TacticReplaySnapshot, TacticReplayControlPlaneError> {
        let first = usize::try_from(first_sequence).map_err(|_| {
            TacticReplayControlPlaneError::Invalid("replay snapshot offset overflows")
        })?;
        if first > self.entries.len() {
            return Err(TacticReplayControlPlaneError::Invalid(
                "replay snapshot offset is outside the journal",
            ));
        }
        self.materialize_snapshot(&self.entries[first..], self.replay_snapshot)
    }

    fn materialize_snapshot(
        &self,
        entries: &[StoredTacticReplayAdmission],
        version: TacticReplaySnapshotVersion,
    ) -> Result<TacticReplaySnapshot, TacticReplayControlPlaneError> {
        let mut transitions = Vec::with_capacity(entries.len());
        let mut routes = Vec::with_capacity(entries.len());
        let mut episode_groups = Vec::with_capacity(entries.len());
        for entry in entries {
            transitions.push(
                self.content_store
                    .load_option_transition(entry.transition)
                    .map_err(store_error)?,
            );
            routes.push(
                self.content_store
                    .load_tape(entry.route)
                    .map_err(store_error)?,
            );
            episode_groups.push(entry.episode_group);
        }
        let corpus = TacticQTrainingCorpus {
            execution_authority_sha256: self.identity.execution_authority_sha256,
            feature_schema_sha256: self.identity.feature_schema_sha256,
            objective_sha256: self.identity.objective_sha256,
            root_checkpoint_sha256: self.identity.root_checkpoint_sha256,
            transitions,
            routes,
            episode_groups,
        };
        validate_training_corpus(&corpus).map_err(domain_error)?;
        Ok(TacticReplaySnapshot {
            schema: TACTIC_REPLAY_SNAPSHOT_SCHEMA_V1.into(),
            version,
            corpus,
        })
    }
}

fn validate_stored_entry(
    identity: &TacticReplayControlPlaneIdentity,
    content_store: &TacticQContentStore,
    entry: &StoredTacticReplayAdmission,
    expected_sequence: u64,
    expected_parent_snapshot: Digest,
) -> Result<(), TacticReplayControlPlaneError> {
    if entry.schema != TACTIC_REPLAY_ADMISSION_SCHEMA_V1
        || entry.sequence != expected_sequence
        || entry.learner_snapshot_sha256 == Digest::ZERO
        || entry.transition_identity_sha256 == Digest::ZERO
        || entry.parent_replay_snapshot_sha256 != expected_parent_snapshot
        || entry.replay_snapshot_sha256 != entry.expected_snapshot_sha256()?
    {
        return Err(TacticReplayControlPlaneError::Invalid(
            "replay control-plane admission authority is invalid",
        ));
    }
    let transition = content_store
        .load_option_transition(entry.transition)
        .map_err(store_error)?;
    let route = content_store.load_tape(entry.route).map_err(store_error)?;
    let corpus = TacticQTrainingCorpus {
        execution_authority_sha256: identity.execution_authority_sha256,
        feature_schema_sha256: identity.feature_schema_sha256,
        objective_sha256: identity.objective_sha256,
        root_checkpoint_sha256: identity.root_checkpoint_sha256,
        transitions: vec![transition.clone()],
        routes: vec![route],
        episode_groups: vec![entry.episode_group],
    };
    validate_training_corpus(&corpus).map_err(domain_error)?;
    if transition.replay_identity_sha256().map_err(domain_error)?
        != entry.transition_identity_sha256
    {
        return Err(TacticReplayControlPlaneError::Invalid(
            "stored replay transition identity is detached",
        ));
    }
    Ok(())
}

fn initial_snapshot_version(
    identity: &TacticReplayControlPlaneIdentity,
) -> Result<TacticReplaySnapshotVersion, TacticReplayControlPlaneError> {
    Ok(TacticReplaySnapshotVersion {
        revision: 0,
        sha256: digest_cbor(
            b"dusklight.tactic-replay-snapshot-root/v1\0",
            &identity.content_sha256()?,
        )?,
    })
}

fn digest_cbor<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Digest, TacticReplayControlPlaneError> {
    let raw = serde_cbor::to_vec(value).map_err(serialization_error)?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((raw.len() as u64).to_le_bytes());
    hasher.update(raw);
    Ok(Digest(hasher.finalize().into()))
}

fn decode_cbor<T: for<'de> Deserialize<'de>>(raw: &[u8]) -> Result<T, serde_cbor::Error> {
    let mut deserializer = serde_cbor::Deserializer::from_slice(raw);
    let value = T::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn io_error(error: impl fmt::Display) -> TacticReplayControlPlaneError {
    TacticReplayControlPlaneError::Io(error.to_string())
}

fn serialization_error(error: impl fmt::Display) -> TacticReplayControlPlaneError {
    TacticReplayControlPlaneError::Serialization(error.to_string())
}

fn domain_error(error: impl fmt::Display) -> TacticReplayControlPlaneError {
    TacticReplayControlPlaneError::Domain(error.to_string())
}

fn store_error(error: impl fmt::Display) -> TacticReplayControlPlaneError {
    TacticReplayControlPlaneError::Store(error.to_string())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TacticReplayControlPlaneError {
    Invalid(&'static str),
    Io(String),
    Serialization(String),
    Domain(String),
    Store(String),
}

impl fmt::Display for TacticReplayControlPlaneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid tactic replay service: {message}"),
            Self::Io(message) => write!(formatter, "tactic replay service I/O failed: {message}"),
            Self::Serialization(message) => {
                write!(
                    formatter,
                    "tactic replay service serialization failed: {message}"
                )
            }
            Self::Domain(message) => {
                write!(
                    formatter,
                    "tactic replay service evidence failed: {message}"
                )
            }
            Self::Store(message) => {
                write!(
                    formatter,
                    "tactic replay service content store failed: {message}"
                )
            }
        }
    }
}

impl Error for TacticReplayControlPlaneError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tactic_q_campaign::route_checkpoint;
    use dusklight_automation_contracts::tape::{InputFrame, RawPadState};
    use dusklight_control::option_execution::{
        OptionCondition, OptionEndReason, OptionExecution, OptionParameter, OptionType, TapeRange,
    };
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
    use dusklight_learning::fact_snapshot::{FactSnapshot, FactTerminalReason};
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn identity() -> TacticReplayControlPlaneIdentity {
        TacticReplayControlPlaneIdentity::new(
            Digest([8; 32]),
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
        )
        .unwrap()
    }

    fn test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "dusklight-replay-control-plane-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn row(lineage_button: bool) -> (OptionTransitionSample, InputTape) {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let step = &shard.episodes[0].steps[0];
        let mut before =
            FactSnapshot::from_native_learning(&step.pre_input, &[], None, Vec::new()).unwrap();
        let mut after = FactSnapshot::from_native_learning(
            &step.post_simulation,
            &[step.pre_input.clone()],
            None,
            Vec::new(),
        )
        .unwrap();
        before.terminal.configured = Some(true);
        before.terminal.reached = Some(false);
        before.terminal.reason = FactTerminalReason::None;
        after.terminal.configured = Some(true);
        after.terminal.reached = Some(false);
        after.terminal.reason = FactTerminalReason::None;
        let mut route = InputTape {
            frames: vec![InputFrame::default(); after.tape_frame as usize + 1],
            ..InputTape::default()
        };
        if lineage_button {
            route.frames[0].pads[0] = RawPadState {
                buttons: 1,
                ..RawPadState::default()
            };
        }
        let range = TapeRange {
            start_frame: before.tape_frame,
            end_frame_exclusive: after.tape_frame + 1,
        };
        let execution = OptionExecution::capture(
            "wait".into(),
            OptionType::Neutral,
            BTreeMap::<String, OptionParameter>::new(),
            1,
            1,
            OptionCondition::DurationElapsed,
            Vec::new(),
            OptionEndReason::Completed,
            &route,
            range,
        )
        .unwrap();
        let root = identity().root_checkpoint_sha256;
        let source = InputTape {
            frames: route.frames[..range.start_frame as usize].to_vec(),
            ..route.clone()
        };
        let next = InputTape {
            frames: route.frames[..range.end_frame_exclusive as usize].to_vec(),
            ..route.clone()
        };
        let mut transition = OptionTransitionSample::capture(
            Digest([1; 32]),
            route_checkpoint(root, &source).unwrap(),
            route_checkpoint(root, &next).unwrap(),
            before,
            after,
            execution,
            &route,
            -0.01,
            false,
            |facts| Ok::<_, &'static str>(vec![facts.player.position_f32_bits[0] as f32]),
        )
        .unwrap();
        transition.execution_authority_sha256 = Digest([8; 32]);
        transition.validate().unwrap();
        (transition, route)
    }

    #[test]
    fn appends_deduplicates_and_reopens_exact_authenticated_replay() {
        let root = test_root("round-trip");
        let journal = root.join("replay.dtrp");
        let objects = root.join("objects");
        let expected = identity();
        let mut service =
            TacticReplayControlPlane::create(&journal, &objects, expected.clone()).unwrap();
        let root_snapshot = service.replay_snapshot();
        let (transition, route) = row(false);
        let admitted = service
            .publish(2, 7, Digest([9; 32]), &transition, &route, 11)
            .unwrap();
        let TacticReplayAdmissionOutcome::Admitted {
            sequence,
            replay_snapshot,
            ..
        } = admitted
        else {
            panic!("first publication must be admitted");
        };
        assert_eq!(sequence, 0);
        assert_eq!(replay_snapshot.revision, 1);
        assert_ne!(replay_snapshot.sha256, root_snapshot.sha256);
        assert!(matches!(
            service
                .publish(3, 8, Digest([10; 32]), &transition, &route, 12)
                .unwrap(),
            TacticReplayAdmissionOutcome::Duplicate {
                existing_sequence: 0,
                ..
            }
        ));
        drop(service);

        let reopened = TacticReplayControlPlane::open(&journal, &objects, &expected).unwrap();
        assert_eq!(reopened.len(), 1);
        assert_eq!(reopened.replay_snapshot(), replay_snapshot);
        let snapshot = reopened.snapshot().unwrap();
        assert_eq!(snapshot.version, replay_snapshot);
        assert_eq!(snapshot.corpus.transitions, vec![transition]);
        assert_eq!(snapshot.corpus.routes, vec![route]);
        assert_eq!(snapshot.corpus.episode_groups, vec![11]);
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn similar_observations_with_distinct_input_lineages_remain_distinct() {
        let root = test_root("lineages");
        let journal = root.join("replay.dtrp");
        let objects = root.join("objects");
        let expected = identity();
        let mut service = TacticReplayControlPlane::create(&journal, &objects, expected).unwrap();
        let (first, first_route) = row(false);
        let (second, second_route) = row(true);
        assert_eq!(first.before_state_sha256, second.before_state_sha256);
        assert_eq!(first.after_state_sha256, second.after_state_sha256);
        assert_ne!(
            first.replay_identity_sha256().unwrap(),
            second.replay_identity_sha256().unwrap()
        );
        service
            .publish(0, 0, Digest([4; 32]), &first, &first_route, 1)
            .unwrap();
        service
            .publish(1, 0, Digest([4; 32]), &second, &second_route, 2)
            .unwrap();
        assert_eq!(service.len(), 2);
        assert_eq!(service.snapshot().unwrap().corpus.transitions.len(), 2);
        let first_barrier = service.snapshot_through(1).unwrap();
        assert_eq!(first_barrier.version.revision, 1);
        assert_eq!(first_barrier.corpus.transitions, vec![first]);
        let admissions = service.admissions();
        assert_eq!(admissions.len(), 2);
        assert_eq!(admissions[0].publisher_lane, 0);
        assert_eq!(admissions[1].publisher_lane, 1);
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opening_discards_only_an_incomplete_tail() {
        let root = test_root("tail");
        let journal = root.join("replay.dtrp");
        let objects = root.join("objects");
        let expected = identity();
        let mut service =
            TacticReplayControlPlane::create(&journal, &objects, expected.clone()).unwrap();
        let (transition, route) = row(false);
        service
            .publish(0, 0, Digest([4; 32]), &transition, &route, 1)
            .unwrap();
        let complete_len = fs::metadata(&journal).unwrap().len();
        drop(service);
        OpenOptions::new()
            .append(true)
            .open(&journal)
            .unwrap()
            .write_all(&[1, 2, 3, 4, 5])
            .unwrap();
        assert!(fs::metadata(&journal).unwrap().len() > complete_len);

        let reopened = TacticReplayControlPlane::open(&journal, &objects, &expected).unwrap();
        assert_eq!(reopened.len(), 1);
        assert_eq!(fs::metadata(&journal).unwrap().len(), complete_len);
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
}
