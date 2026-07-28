//! Tactic-learning codecs over the shared content-addressed evidence store.

use crate::tactic_q_campaign::{
    TACTIC_Q_CHECKPOINT_EXTENSION, TACTIC_Q_CHECKPOINT_SCHEMA_V2, TACTIC_Q_CHECKPOINT_SCHEMA_V3,
    TACTIC_Q_CHECKPOINT_SERIALIZATION_BENCHMARK_SCHEMA_V1, TacticQCampaignCheckpoint,
    TacticQCampaignError, TacticQCheckpointSerializationBenchmark, TacticQFinalResult,
    TacticQTrainingCorpus, checkpoint_digest, validate_checkpoint, validate_final_result,
    validate_training_corpus,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_control::option_execution::{
    OptionCondition, OptionDuration, OptionEndReason, OptionExecution, TapeRange,
};
use dusklight_evidence::content_store::{
    ContentBlob, ContentKind, ContentStore, ContentStoreError,
};
use dusklight_learning::fact_snapshot::{ActorFactSnapshot, FactSnapshot};
use dusklight_learning::learner_state::{LearnerActionMaskEntry, LearnerState};
use dusklight_learning::option_transition::OptionTransitionSample;
use dusklight_learning::option_values::{
    OptionActionDescriptor, OptionValueConfig, OptionValueSample,
};
use dusklight_learning::tactic_exploration::TacticExplorationConfig;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

const FACT_OBJECT_SCHEMA_V1: &str = "dusklight-tactic-q-fact-object/v1";
const CHECKPOINT_MANIFEST_SCHEMA_V1: &str = "dusklight-tactic-q-checkpoint-manifest/v1";
const CHECKPOINT_MANIFEST_SCHEMA_V2: &str = "dusklight-tactic-q-checkpoint-manifest/v2";
const TRAINING_CORPUS_MANIFEST_SCHEMA_V1: &str = "dusklight-tactic-q-training-corpus-manifest/v1";
const TRAINING_CORPUS_MANIFEST_SCHEMA_V2: &str = "dusklight-tactic-q-training-corpus-manifest/v2";
const CHECKPOINT_MAGIC: &[u8; 8] = b"DSKTQZ01";
const TRAINING_CORPUS_MAGIC: &[u8; 8] = b"DSKTQC01";
const FINAL_RESULT_MAGIC: &[u8; 8] = b"DSKTQF01";
const CHECKPOINT_FORMAT_VERSION: u16 = 2;
const TRAINING_CORPUS_FORMAT_VERSION_V1: u16 = 1;
const TRAINING_CORPUS_FORMAT_VERSION_V2: u16 = 2;
const FINAL_RESULT_FORMAT_VERSION: u16 = 1;
const CHECKPOINT_HEADER_SIZE: usize = 8 + 2 + 2 + 8 + 32;
const MAXIMUM_CHECKPOINT_MANIFEST_BYTES: u64 = 256 * 1024 * 1024;
const MAXIMUM_INLINE_TRAINING_CORPUS_ROWS: usize = 256;
const CHECKPOINT_COMPRESSION_LEVEL: i32 = 1;
const CONTENT_DIRECTORY: &str = "objects";
const LEGACY_CHECKPOINT_SCHEMA_V1: &str = "dusklight-tactic-q-checkpoint/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredContentRef {
    pub kind: ContentKind,
    pub sha256: Digest,
}

impl From<&ContentBlob> for StoredContentRef {
    fn from(blob: &ContentBlob) -> Self {
        Self {
            kind: blob.kind,
            sha256: blob.sha256,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredFactSnapshot {
    schema: String,
    snapshot_sha256: Digest,
    actors: Vec<StoredContentRef>,
    snapshot_without_actors: FactSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredTransitionObjects {
    pub before: StoredContentRef,
    pub after: StoredContentRef,
    pub tactic: StoredContentRef,
    pub emitted_tape: StoredContentRef,
}

#[derive(Clone, Debug)]
pub(crate) struct TacticQContentStore {
    store: ContentStore,
}

impl TacticQContentStore {
    pub fn initialize(root: impl Into<PathBuf>) -> Result<Self, TacticQContentStoreError> {
        Ok(Self {
            store: ContentStore::initialize(root).map_err(TacticQContentStoreError::Store)?,
        })
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self, TacticQContentStoreError> {
        Ok(Self {
            store: ContentStore::open(root).map_err(TacticQContentStoreError::Store)?,
        })
    }

    pub fn store_transition(
        &self,
        before: &FactSnapshot,
        after: &FactSnapshot,
        tactic: &OptionActionDescriptor,
        emitted_tape: &InputTape,
    ) -> Result<StoredTransitionObjects, TacticQContentStoreError> {
        Ok(StoredTransitionObjects {
            before: self.store_fact(before)?,
            after: self.store_fact(after)?,
            tactic: self.store_tactic(tactic)?,
            emitted_tape: self.store_tape(emitted_tape)?,
        })
    }

    pub fn store_option_transition(
        &self,
        transition: &OptionTransitionSample,
        route: &InputTape,
    ) -> Result<StoredContentRef, TacticQContentStoreError> {
        let stored =
            encode_transition(transition, route, self).map_err(TacticQContentStoreError::domain)?;
        let raw = serde_cbor::to_vec(&stored).map_err(TacticQContentStoreError::domain)?;
        Ok(StoredContentRef::from(
            &self
                .store
                .put_bytes(&raw, ContentKind::TacticTransition)
                .map_err(TacticQContentStoreError::Store)?,
        ))
    }

    pub fn load_option_transition(
        &self,
        reference: StoredContentRef,
    ) -> Result<OptionTransitionSample, TacticQContentStoreError> {
        require_kind(reference, ContentKind::TacticTransition)?;
        let stored: StoredOptionTransition = self.read_cbor(reference)?;
        load_transition(&stored, self).map_err(TacticQContentStoreError::domain)
    }

    pub fn store_fact(
        &self,
        snapshot: &FactSnapshot,
    ) -> Result<StoredContentRef, TacticQContentStoreError> {
        snapshot
            .validate()
            .map_err(TacticQContentStoreError::domain)?;
        // One durable object per fact is substantially cheaper than syncing
        // one file per actor for every proposal. The canonical snapshot still
        // retains the complete typed actor population and its authenticated
        // identity; `load_fact` continues to accept the legacy split layout.
        let raw = serde_cbor::to_vec(snapshot).map_err(TacticQContentStoreError::domain)?;
        Ok(StoredContentRef::from(
            &self
                .store
                .put_bytes(&raw, ContentKind::FactSnapshot)
                .map_err(TacticQContentStoreError::Store)?,
        ))
    }

    pub fn load_fact(
        &self,
        reference: StoredContentRef,
    ) -> Result<FactSnapshot, TacticQContentStoreError> {
        require_kind(reference, ContentKind::FactSnapshot)?;
        let raw = self.read_bytes(reference)?;
        if let Ok(snapshot) = decode_cbor::<FactSnapshot>(&raw) {
            snapshot
                .validate()
                .map_err(TacticQContentStoreError::domain)?;
            return Ok(snapshot);
        }
        let stored: StoredFactSnapshot = decode_cbor(&raw)?;
        if stored.schema != FACT_OBJECT_SCHEMA_V1 || stored.snapshot_sha256 == Digest::ZERO {
            return Err(TacticQContentStoreError::Invalid(
                "stored fact identity is invalid",
            ));
        }
        let actors = stored
            .actors
            .into_iter()
            .map(|actor| self.load_actor(actor))
            .collect::<Result<Vec<_>, _>>()?;
        let mut snapshot = stored.snapshot_without_actors;
        snapshot.actors = actors;
        snapshot
            .validate()
            .map_err(TacticQContentStoreError::domain)?;
        if snapshot
            .content_sha256()
            .map_err(TacticQContentStoreError::domain)?
            != stored.snapshot_sha256
        {
            return Err(TacticQContentStoreError::Invalid(
                "stored fact does not reconstruct its authenticated snapshot",
            ));
        }
        Ok(snapshot)
    }

    #[cfg(test)]
    pub fn store_actor(
        &self,
        actor: &ActorFactSnapshot,
    ) -> Result<StoredContentRef, TacticQContentStoreError> {
        let raw = serde_cbor::to_vec(actor).map_err(TacticQContentStoreError::domain)?;
        Ok(StoredContentRef::from(
            &self
                .store
                .put_bytes(&raw, ContentKind::ActorSnapshot)
                .map_err(TacticQContentStoreError::Store)?,
        ))
    }

    pub fn load_actor(
        &self,
        reference: StoredContentRef,
    ) -> Result<ActorFactSnapshot, TacticQContentStoreError> {
        require_kind(reference, ContentKind::ActorSnapshot)?;
        self.read_cbor(reference)
    }

    pub fn store_tactic(
        &self,
        tactic: &OptionActionDescriptor,
    ) -> Result<StoredContentRef, TacticQContentStoreError> {
        tactic
            .validate()
            .map_err(TacticQContentStoreError::domain)?;
        let raw = serde_cbor::to_vec(tactic).map_err(TacticQContentStoreError::domain)?;
        Ok(StoredContentRef::from(
            &self
                .store
                .put_bytes(&raw, ContentKind::TacticDefinition)
                .map_err(TacticQContentStoreError::Store)?,
        ))
    }

    pub fn load_tactic(
        &self,
        reference: StoredContentRef,
    ) -> Result<OptionActionDescriptor, TacticQContentStoreError> {
        require_kind(reference, ContentKind::TacticDefinition)?;
        let tactic: OptionActionDescriptor = self.read_cbor(reference)?;
        tactic
            .validate()
            .map_err(TacticQContentStoreError::domain)?;
        Ok(tactic)
    }

    pub fn store_tape(
        &self,
        tape: &InputTape,
    ) -> Result<StoredContentRef, TacticQContentStoreError> {
        tape.validate().map_err(TacticQContentStoreError::domain)?;
        let raw = tape.encode().map_err(TacticQContentStoreError::domain)?;
        Ok(StoredContentRef::from(
            &self
                .store
                .put_bytes(&raw, ContentKind::InputTape)
                .map_err(TacticQContentStoreError::Store)?,
        ))
    }

    pub fn load_tape(
        &self,
        reference: StoredContentRef,
    ) -> Result<InputTape, TacticQContentStoreError> {
        require_kind(reference, ContentKind::InputTape)?;
        let raw = self.read_bytes(reference)?;
        let tape = InputTape::decode(&raw)
            .map_err(TacticQContentStoreError::domain)?
            .tape;
        tape.validate().map_err(TacticQContentStoreError::domain)?;
        Ok(tape)
    }

    fn read_cbor<T: for<'de> Deserialize<'de>>(
        &self,
        reference: StoredContentRef,
    ) -> Result<T, TacticQContentStoreError> {
        let raw = self.read_bytes(reference)?;
        let mut deserializer = serde_cbor::Deserializer::from_slice(&raw);
        let value = T::deserialize(&mut deserializer).map_err(TacticQContentStoreError::domain)?;
        deserializer
            .end()
            .map_err(TacticQContentStoreError::domain)?;
        Ok(value)
    }

    fn read_bytes(&self, reference: StoredContentRef) -> Result<Vec<u8>, TacticQContentStoreError> {
        let blob = self
            .store
            .reference_for_digest(reference.kind, reference.sha256)
            .map_err(TacticQContentStoreError::Store)?;
        self.store
            .read_bytes(&blob)
            .map_err(TacticQContentStoreError::Store)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredLearnerState {
    schema: String,
    snapshot_sha256: Digest,
    fact_registry_sha256: Digest,
    action_universe_sha256: Digest,
    applicable_choice_schema_sha256: Digest,
    snapshot: StoredContentRef,
    action_mask: Vec<LearnerActionMaskEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredOptionExecution {
    schema: String,
    tactic: StoredContentRef,
    duration: OptionDuration,
    termination_condition: OptionCondition,
    cancellation_conditions: Vec<OptionCondition>,
    end_reason: OptionEndReason,
    emitted_tape: StoredContentRef,
    realized_tape_range: TapeRange,
    tape_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredOptionValueSample {
    tactic: StoredContentRef,
    state: Vec<f32>,
    duration_ticks: u32,
    reward: f32,
    next_state: Vec<f32>,
    terminal: bool,
    before_state_sha256: Digest,
    after_state_sha256: Digest,
    source_checkpoint_sha256: Digest,
    next_checkpoint_sha256: Digest,
    realized_tape_range: TapeRange,
    realized_tape_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredOptionTransition {
    schema: String,
    feature_schema_sha256: Digest,
    before_state_sha256: Digest,
    after_state_sha256: Digest,
    source_checkpoint_sha256: Digest,
    next_checkpoint_sha256: Digest,
    before: StoredContentRef,
    after: StoredContentRef,
    execution: StoredOptionExecution,
    value_sample: StoredOptionValueSample,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredCheckpointManifest {
    schema: String,
    content_sha256: Digest,
    feature_schema_sha256: Digest,
    objective_sha256: Digest,
    root_checkpoint_sha256: Digest,
    episode_group: u64,
    decision_index: u64,
    current: StoredLearnerState,
    route_tape: StoredContentRef,
    replay: Vec<StoredContentRef>,
    replay_routes: Vec<StoredContentRef>,
    episode_groups: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    training_replay: Vec<StoredContentRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    training_replay_routes: Vec<StoredContentRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    training_episode_groups: Vec<u64>,
    model_config: OptionValueConfig,
    exploration: TacticExplorationConfig,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredTrainingCorpusManifest {
    schema: String,
    feature_schema_sha256: Digest,
    objective_sha256: Digest,
    root_checkpoint_sha256: Digest,
    transitions: Vec<StoredContentRef>,
    routes: Vec<StoredContentRef>,
    episode_groups: Vec<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct InlineTrainingCorpusManifest {
    schema: String,
    feature_schema_sha256: Digest,
    objective_sha256: Digest,
    root_checkpoint_sha256: Digest,
    transitions: Vec<OptionTransitionSample>,
    routes: Vec<InputTape>,
    episode_groups: Vec<u64>,
}

pub(crate) fn write_checkpoint_with_local_store(
    checkpoint: &TacticQCampaignCheckpoint,
    directory: &Path,
) -> Result<PathBuf, TacticQCampaignError> {
    write_checkpoint(checkpoint, directory, &directory.join(CONTENT_DIRECTORY))
}

pub(crate) fn write_checkpoint(
    checkpoint: &TacticQCampaignCheckpoint,
    directory: &Path,
    content_root: &Path,
) -> Result<PathBuf, TacticQCampaignError> {
    validate_checkpoint(checkpoint)?;
    if content_root.file_name().and_then(|name| name.to_str()) != Some(CONTENT_DIRECTORY) {
        return Err(TacticQCampaignError::InvalidState(
            "checkpoint content root must use the discoverable objects directory",
        ));
    }
    fs::create_dir_all(directory).map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
    let store = TacticQContentStore::initialize(content_root).map_err(checkpoint_store_error)?;
    let manifest = store_checkpoint_manifest(checkpoint, &store)?;
    let raw = serde_cbor::to_vec(&manifest)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    let envelope = encode_checkpoint_envelope(&raw)?;
    let final_path = directory.join(format!(
        "tactic-q-{}.{}",
        checkpoint.content_sha256, TACTIC_Q_CHECKPOINT_EXTENSION
    ));
    install_binary_artifact(&final_path, &envelope)?;
    Ok(final_path)
}

pub(crate) fn write_training_corpus(
    corpus: &TacticQTrainingCorpus,
    path: &Path,
    content_root: &Path,
) -> Result<(), TacticQCampaignError> {
    validate_training_corpus(corpus)?;
    if content_root.file_name().and_then(|name| name.to_str()) != Some(CONTENT_DIRECTORY) {
        return Err(TacticQCampaignError::InvalidState(
            "training corpus content root must use the discoverable objects directory",
        ));
    }
    let parent = path.parent().ok_or(TacticQCampaignError::InvalidState(
        "training corpus path has no parent",
    ))?;
    fs::create_dir_all(parent).map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
    // Completed-episode handoff is one bounded, compressed, authenticated
    // binary artifact. Splitting four hot proposal rows into dozens of tiny
    // content files forced one durable filesystem flush per field and
    // dominated native-search wall time. Checkpoints retain content-addressed
    // objects for cross-checkpoint deduplication; a generated corpus contains
    // only this episode's newly acquired rows, so an inline envelope is both
    // smaller operationally and independently recoverable after a crash.
    let store = TacticQContentStore::initialize(content_root).map_err(checkpoint_store_error)?;
    let (raw, version) = if corpus.transitions.len() <= MAXIMUM_INLINE_TRAINING_CORPUS_ROWS {
        let manifest = InlineTrainingCorpusManifest {
            schema: TRAINING_CORPUS_MANIFEST_SCHEMA_V2.into(),
            feature_schema_sha256: corpus.feature_schema_sha256,
            objective_sha256: corpus.objective_sha256,
            root_checkpoint_sha256: corpus.root_checkpoint_sha256,
            transitions: corpus.transitions.clone(),
            routes: corpus.routes.clone(),
            episode_groups: corpus.episode_groups.clone(),
        };
        (
            serde_cbor::to_vec(&manifest)
                .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?,
            TRAINING_CORPUS_FORMAT_VERSION_V2,
        )
    } else {
        // Wide or long campaigns cannot duplicate every full route in one
        // inline manifest. Reuse the content-addressed transition and tape
        // objects already produced by decision evidence, leaving the envelope
        // as a bounded vector of typed digests.
        let mut transitions = Vec::with_capacity(corpus.transitions.len());
        let mut routes = Vec::with_capacity(corpus.routes.len());
        for (transition, route) in corpus.transitions.iter().zip(&corpus.routes) {
            transitions.push(
                store
                    .store_option_transition(transition, route)
                    .map_err(checkpoint_store_error)?,
            );
            routes.push(store.store_tape(route).map_err(checkpoint_store_error)?);
        }
        let manifest = StoredTrainingCorpusManifest {
            schema: TRAINING_CORPUS_MANIFEST_SCHEMA_V1.into(),
            feature_schema_sha256: corpus.feature_schema_sha256,
            objective_sha256: corpus.objective_sha256,
            root_checkpoint_sha256: corpus.root_checkpoint_sha256,
            transitions,
            routes,
            episode_groups: corpus.episode_groups.clone(),
        };
        (
            serde_cbor::to_vec(&manifest)
                .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?,
            TRAINING_CORPUS_FORMAT_VERSION_V1,
        )
    };
    let envelope = encode_binary_envelope(&raw, TRAINING_CORPUS_MAGIC, version)?;
    install_binary_artifact(path, &envelope)
}

pub(crate) fn read_training_corpus(
    path: &Path,
) -> Result<TacticQTrainingCorpus, TacticQCampaignError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() < CHECKPOINT_HEADER_SIZE as u64
        || metadata.len() > MAXIMUM_CHECKPOINT_MANIFEST_BYTES + CHECKPOINT_HEADER_SIZE as u64
    {
        return Err(TacticQCampaignError::InvalidState(
            "training corpus path is not a bounded physical binary envelope",
        ));
    }
    let envelope = fs::read(path).map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
    if envelope.len() < CHECKPOINT_HEADER_SIZE || &envelope[..8] != TRAINING_CORPUS_MAGIC {
        return Err(TacticQCampaignError::InvalidState(
            "training corpus envelope is invalid",
        ));
    }
    let version = u16::from_le_bytes(envelope[8..10].try_into().expect("fixed slice"));
    let raw = decode_binary_envelope(&envelope, TRAINING_CORPUS_MAGIC, version)?;
    if version == TRAINING_CORPUS_FORMAT_VERSION_V2 {
        let manifest: InlineTrainingCorpusManifest =
            decode_cbor(&raw).map_err(checkpoint_store_error)?;
        let corpus = TacticQTrainingCorpus {
            feature_schema_sha256: manifest.feature_schema_sha256,
            objective_sha256: manifest.objective_sha256,
            root_checkpoint_sha256: manifest.root_checkpoint_sha256,
            transitions: manifest.transitions,
            routes: manifest.routes,
            episode_groups: manifest.episode_groups,
        };
        if manifest.schema != TRAINING_CORPUS_MANIFEST_SCHEMA_V2 {
            return Err(TacticQCampaignError::InvalidState(
                "training corpus manifest identity is invalid",
            ));
        }
        validate_training_corpus(&corpus)?;
        return Ok(corpus);
    }
    if version != TRAINING_CORPUS_FORMAT_VERSION_V1 {
        return Err(TacticQCampaignError::InvalidState(
            "training corpus envelope version is unsupported",
        ));
    }
    let manifest: StoredTrainingCorpusManifest =
        decode_cbor(&raw).map_err(checkpoint_store_error)?;
    if manifest.schema != TRAINING_CORPUS_MANIFEST_SCHEMA_V1
        || manifest.feature_schema_sha256 == Digest::ZERO
        || manifest.objective_sha256 == Digest::ZERO
        || manifest.root_checkpoint_sha256 == Digest::ZERO
        || manifest.transitions.len() != manifest.routes.len()
        || manifest.transitions.len() != manifest.episode_groups.len()
    {
        return Err(TacticQCampaignError::InvalidState(
            "training corpus manifest identity or shape is invalid",
        ));
    }
    let parent = path.parent().ok_or(TacticQCampaignError::InvalidState(
        "training corpus path has no parent",
    ))?;
    for ancestor in parent.ancestors() {
        let content_root = ancestor.join(CONTENT_DIRECTORY);
        let Ok(store) = TacticQContentStore::open(&content_root) else {
            continue;
        };
        let transitions = manifest
            .transitions
            .iter()
            .map(|transition| store.load_option_transition(*transition))
            .collect::<Result<Vec<_>, _>>();
        let routes = manifest
            .routes
            .iter()
            .map(|route| store.load_tape(*route))
            .collect::<Result<Vec<_>, _>>();
        let (Ok(transitions), Ok(routes)) = (transitions, routes) else {
            continue;
        };
        let corpus = TacticQTrainingCorpus {
            feature_schema_sha256: manifest.feature_schema_sha256,
            objective_sha256: manifest.objective_sha256,
            root_checkpoint_sha256: manifest.root_checkpoint_sha256,
            transitions,
            routes,
            episode_groups: manifest.episode_groups.clone(),
        };
        if validate_training_corpus(&corpus).is_ok() {
            return Ok(corpus);
        }
    }
    Err(TacticQCampaignError::InvalidState(
        "training corpus content objects are unavailable or invalid",
    ))
}

pub(crate) fn write_final_result(
    result: &TacticQFinalResult,
    path: &Path,
) -> Result<(), TacticQCampaignError> {
    validate_final_result(result)?;
    let raw = serde_cbor::to_vec(result)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    let envelope = encode_binary_envelope(&raw, FINAL_RESULT_MAGIC, FINAL_RESULT_FORMAT_VERSION)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
    }
    install_binary_artifact(path, &envelope)
}

pub(crate) fn read_final_result(path: &Path) -> Result<TacticQFinalResult, TacticQCampaignError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() < CHECKPOINT_HEADER_SIZE as u64
        || metadata.len() > MAXIMUM_CHECKPOINT_MANIFEST_BYTES + CHECKPOINT_HEADER_SIZE as u64
    {
        return Err(TacticQCampaignError::InvalidState(
            "final result path is not a bounded physical binary envelope",
        ));
    }
    let raw = decode_binary_envelope(
        &fs::read(path).map_err(|error| TacticQCampaignError::Io(error.to_string()))?,
        FINAL_RESULT_MAGIC,
        FINAL_RESULT_FORMAT_VERSION,
    )?;
    let result: TacticQFinalResult = decode_cbor(&raw).map_err(checkpoint_store_error)?;
    validate_final_result(&result)?;
    Ok(result)
}

pub(crate) fn read_checkpoint(
    path: &Path,
) -> Result<TacticQCampaignCheckpoint, TacticQCampaignError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() < CHECKPOINT_HEADER_SIZE as u64
        || metadata.len() > MAXIMUM_CHECKPOINT_MANIFEST_BYTES + CHECKPOINT_HEADER_SIZE as u64
    {
        return Err(TacticQCampaignError::InvalidState(
            "checkpoint path is not a bounded physical binary envelope",
        ));
    }
    let raw = decode_checkpoint_envelope(
        &fs::read(path).map_err(|error| TacticQCampaignError::Io(error.to_string()))?,
    )?;
    let manifest: StoredCheckpointManifest = decode_cbor(&raw).map_err(checkpoint_store_error)?;
    let parent = path.parent().ok_or(TacticQCampaignError::InvalidState(
        "checkpoint path has no parent",
    ))?;
    for ancestor in parent.ancestors() {
        let content_root = ancestor.join(CONTENT_DIRECTORY);
        let Ok(store) = TacticQContentStore::open(&content_root) else {
            continue;
        };
        if let Ok(checkpoint) = load_checkpoint_manifest(&manifest, &store) {
            validate_checkpoint(&checkpoint)?;
            return Ok(checkpoint);
        }
    }
    Err(TacticQCampaignError::InvalidState(
        "checkpoint content objects are unavailable or invalid",
    ))
}

pub(crate) fn benchmark_checkpoint_serialization(
    legacy_json_path: &Path,
    current_checkpoint_path: &Path,
    iterations: u64,
) -> Result<TacticQCheckpointSerializationBenchmark, TacticQCampaignError> {
    if iterations == 0 {
        return Err(TacticQCampaignError::InvalidState(
            "checkpoint serialization benchmark requires at least one iteration",
        ));
    }
    let legacy_bytes =
        fs::read(legacy_json_path).map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
    let legacy: TacticQCampaignCheckpoint = serde_json::from_slice(&legacy_bytes)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    if legacy.schema != LEGACY_CHECKPOINT_SCHEMA_V1 {
        return Err(TacticQCampaignError::InvalidState(
            "legacy checkpoint does not use the v1 JSON schema",
        ));
    }

    let current_envelope = fs::read(current_checkpoint_path)
        .map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
    let current_raw = decode_checkpoint_envelope(&current_envelope)?;
    let current_manifest: StoredCheckpointManifest =
        decode_cbor(&current_raw).map_err(checkpoint_store_error)?;
    let current = read_checkpoint(current_checkpoint_path)?;

    let mut normalized_legacy = legacy.clone();
    normalized_legacy.schema.clear();
    normalized_legacy.content_sha256 = Digest::ZERO;
    if normalized_legacy.training_replay.is_empty() {
        normalized_legacy.training_replay = normalized_legacy.replay.clone();
        normalized_legacy.training_replay_routes = normalized_legacy.replay_routes.clone();
        normalized_legacy.training_episode_groups = normalized_legacy.episode_groups.clone();
    }
    let mut normalized_current = current.clone();
    normalized_current.schema.clear();
    normalized_current.content_sha256 = Digest::ZERO;
    if normalized_legacy != normalized_current {
        return Err(TacticQCampaignError::InvalidState(
            "legacy and current checkpoints do not contain the same logical campaign state",
        ));
    }

    let legacy_round_trip = serde_json::to_vec(&legacy)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    if legacy_round_trip != legacy_bytes {
        return Err(TacticQCampaignError::InvalidState(
            "legacy JSON checkpoint is not canonically encoded",
        ));
    }
    let current_round_trip = encode_checkpoint_envelope(
        &serde_cbor::to_vec(&current_manifest)
            .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?,
    )?;
    if current_round_trip != current_envelope {
        return Err(TacticQCampaignError::InvalidState(
            "current checkpoint manifest is not canonically encoded",
        ));
    }

    let legacy_started = Instant::now();
    for _ in 0..iterations {
        let encoded = serde_json::to_vec(&legacy)
            .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
        std::hint::black_box(encoded);
    }
    let legacy_nanos = u64::try_from(legacy_started.elapsed().as_nanos()).unwrap_or(u64::MAX);

    let current_started = Instant::now();
    for _ in 0..iterations {
        let raw = serde_cbor::to_vec(&current_manifest)
            .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
        let envelope = encode_checkpoint_envelope(&raw)?;
        std::hint::black_box(envelope);
    }
    let current_nanos = u64::try_from(current_started.elapsed().as_nanos()).unwrap_or(u64::MAX);

    Ok(TacticQCheckpointSerializationBenchmark {
        schema: TACTIC_Q_CHECKPOINT_SERIALIZATION_BENCHMARK_SCHEMA_V1.into(),
        iterations,
        decision_index: current.decision_index,
        replay_transitions: current.replay.len() as u64,
        legacy_json_bytes_per_iteration: legacy_bytes.len() as u64,
        current_manifest_envelope_bytes_per_iteration: current_envelope.len() as u64,
        legacy_json_serialization_total_nanos: legacy_nanos,
        current_manifest_serialization_total_nanos: current_nanos,
    })
}

fn store_checkpoint_manifest(
    checkpoint: &TacticQCampaignCheckpoint,
    store: &TacticQContentStore,
) -> Result<StoredCheckpointManifest, TacticQCampaignError> {
    let current_snapshot = store
        .store_fact(&checkpoint.current.snapshot)
        .map_err(checkpoint_store_error)?;
    let current = StoredLearnerState {
        schema: checkpoint.current.schema.clone(),
        snapshot_sha256: checkpoint.current.snapshot_sha256,
        fact_registry_sha256: checkpoint.current.fact_registry_sha256,
        action_universe_sha256: checkpoint.current.action_universe_sha256,
        applicable_choice_schema_sha256: checkpoint.current.applicable_choice_schema_sha256,
        snapshot: current_snapshot,
        action_mask: checkpoint.current.action_mask.clone(),
    };
    let route_tape = store
        .store_tape(&checkpoint.route_tape)
        .map_err(checkpoint_store_error)?;
    let replay = checkpoint
        .replay
        .iter()
        .zip(&checkpoint.replay_routes)
        .map(|(transition, route)| {
            store
                .store_option_transition(transition, route)
                .map_err(checkpoint_store_error)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let replay_routes = checkpoint
        .replay_routes
        .iter()
        .map(|route| store.store_tape(route).map_err(checkpoint_store_error))
        .collect::<Result<Vec<_>, _>>()?;
    let training_replay = checkpoint
        .training_replay
        .iter()
        .zip(&checkpoint.training_replay_routes)
        .map(|(transition, route)| {
            store
                .store_option_transition(transition, route)
                .map_err(checkpoint_store_error)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let training_replay_routes = checkpoint
        .training_replay_routes
        .iter()
        .map(|route| store.store_tape(route).map_err(checkpoint_store_error))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(StoredCheckpointManifest {
        schema: CHECKPOINT_MANIFEST_SCHEMA_V2.into(),
        content_sha256: checkpoint.content_sha256,
        feature_schema_sha256: checkpoint.feature_schema_sha256,
        objective_sha256: checkpoint.objective_sha256,
        root_checkpoint_sha256: checkpoint.root_checkpoint_sha256,
        episode_group: checkpoint.episode_group,
        decision_index: checkpoint.decision_index,
        current,
        route_tape,
        replay,
        replay_routes,
        episode_groups: checkpoint.episode_groups.clone(),
        training_replay,
        training_replay_routes,
        training_episode_groups: checkpoint.training_episode_groups.clone(),
        model_config: checkpoint.model_config.clone(),
        exploration: checkpoint.exploration,
    })
}

fn encode_transition(
    transition: &OptionTransitionSample,
    route: &InputTape,
    store: &TacticQContentStore,
) -> Result<StoredOptionTransition, TacticQCampaignError> {
    transition.validate()?;
    let emitted_tape = InputTape {
        boot: route.boot.clone(),
        tick_rate_numerator: route.tick_rate_numerator,
        tick_rate_denominator: route.tick_rate_denominator,
        frames: transition.execution.emitted_raw_actions.clone(),
    };
    let objects = store
        .store_transition(
            &transition.before,
            &transition.after,
            &transition.value_sample.action,
            &emitted_tape,
        )
        .map_err(checkpoint_store_error)?;
    Ok(StoredOptionTransition {
        schema: transition.schema.clone(),
        feature_schema_sha256: transition.feature_schema_sha256,
        before_state_sha256: transition.before_state_sha256,
        after_state_sha256: transition.after_state_sha256,
        source_checkpoint_sha256: transition.source_checkpoint_sha256,
        next_checkpoint_sha256: transition.next_checkpoint_sha256,
        before: objects.before,
        after: objects.after,
        execution: StoredOptionExecution {
            schema: transition.execution.schema.clone(),
            tactic: objects.tactic,
            duration: transition.execution.duration,
            termination_condition: transition.execution.termination_condition.clone(),
            cancellation_conditions: transition.execution.cancellation_conditions.clone(),
            end_reason: transition.execution.end_reason,
            emitted_tape: objects.emitted_tape,
            realized_tape_range: transition.execution.realized_tape_range,
            tape_sha256: transition.execution.tape_sha256,
        },
        value_sample: StoredOptionValueSample {
            tactic: objects.tactic,
            state: transition.value_sample.state.clone(),
            duration_ticks: transition.value_sample.duration_ticks,
            reward: transition.value_sample.reward,
            next_state: transition.value_sample.next_state.clone(),
            terminal: transition.value_sample.terminal,
            before_state_sha256: transition.value_sample.before_state_sha256,
            after_state_sha256: transition.value_sample.after_state_sha256,
            source_checkpoint_sha256: transition.value_sample.source_checkpoint_sha256,
            next_checkpoint_sha256: transition.value_sample.next_checkpoint_sha256,
            realized_tape_range: transition.value_sample.realized_tape_range,
            realized_tape_sha256: transition.value_sample.realized_tape_sha256,
        },
    })
}

fn load_checkpoint_manifest(
    manifest: &StoredCheckpointManifest,
    store: &TacticQContentStore,
) -> Result<TacticQCampaignCheckpoint, TacticQCampaignError> {
    let legacy = manifest.schema == CHECKPOINT_MANIFEST_SCHEMA_V1;
    let current = manifest.schema == CHECKPOINT_MANIFEST_SCHEMA_V2;
    if (!legacy && !current)
        || manifest.content_sha256 == Digest::ZERO
        || manifest.replay.len() != manifest.replay_routes.len()
        || manifest.replay.len() != manifest.episode_groups.len()
        || (legacy
            && (!manifest.training_replay.is_empty()
                || !manifest.training_replay_routes.is_empty()
                || !manifest.training_episode_groups.is_empty()))
        || (current
            && (manifest.training_replay.len() != manifest.training_replay_routes.len()
                || manifest.training_replay.len() != manifest.training_episode_groups.len()))
    {
        return Err(TacticQCampaignError::InvalidState(
            "checkpoint manifest identity or shape is invalid",
        ));
    }
    let current = LearnerState {
        schema: manifest.current.schema.clone(),
        snapshot_sha256: manifest.current.snapshot_sha256,
        fact_registry_sha256: manifest.current.fact_registry_sha256,
        action_universe_sha256: manifest.current.action_universe_sha256,
        applicable_choice_schema_sha256: manifest.current.applicable_choice_schema_sha256,
        snapshot: store
            .load_fact(manifest.current.snapshot)
            .map_err(checkpoint_store_error)?,
        action_mask: manifest.current.action_mask.clone(),
    };
    let route_tape = store
        .load_tape(manifest.route_tape)
        .map_err(checkpoint_store_error)?;
    let replay = manifest
        .replay
        .iter()
        .map(|transition| {
            store
                .load_option_transition(*transition)
                .map_err(checkpoint_store_error)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let replay_routes = manifest
        .replay_routes
        .iter()
        .map(|route| store.load_tape(*route).map_err(checkpoint_store_error))
        .collect::<Result<Vec<_>, _>>()?;
    let training_replay = manifest
        .training_replay
        .iter()
        .map(|transition| {
            store
                .load_option_transition(*transition)
                .map_err(checkpoint_store_error)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let training_replay_routes = manifest
        .training_replay_routes
        .iter()
        .map(|route| store.load_tape(*route).map_err(checkpoint_store_error))
        .collect::<Result<Vec<_>, _>>()?;
    let checkpoint = TacticQCampaignCheckpoint {
        schema: if legacy {
            TACTIC_Q_CHECKPOINT_SCHEMA_V2
        } else {
            TACTIC_Q_CHECKPOINT_SCHEMA_V3
        }
        .into(),
        content_sha256: manifest.content_sha256,
        feature_schema_sha256: manifest.feature_schema_sha256,
        objective_sha256: manifest.objective_sha256,
        root_checkpoint_sha256: manifest.root_checkpoint_sha256,
        episode_group: manifest.episode_group,
        decision_index: manifest.decision_index,
        current,
        route_tape,
        replay,
        replay_routes,
        episode_groups: manifest.episode_groups.clone(),
        training_replay,
        training_replay_routes,
        training_episode_groups: manifest.training_episode_groups.clone(),
        model_config: manifest.model_config.clone(),
        exploration: manifest.exploration,
    };
    if checkpoint_digest(&checkpoint)? != manifest.content_sha256 {
        return Err(TacticQCampaignError::InvalidState(
            "checkpoint manifest does not reconstruct its content identity",
        ));
    }
    Ok(checkpoint)
}

fn load_transition(
    stored: &StoredOptionTransition,
    store: &TacticQContentStore,
) -> Result<OptionTransitionSample, TacticQCampaignError> {
    if stored.execution.tactic != stored.value_sample.tactic {
        return Err(TacticQCampaignError::InvalidState(
            "stored transition tactic references disagree",
        ));
    }
    let before = store
        .load_fact(stored.before)
        .map_err(checkpoint_store_error)?;
    let after = store
        .load_fact(stored.after)
        .map_err(checkpoint_store_error)?;
    let tactic = store
        .load_tactic(stored.execution.tactic)
        .map_err(checkpoint_store_error)?;
    let emitted_tape = store
        .load_tape(stored.execution.emitted_tape)
        .map_err(checkpoint_store_error)?;
    let transition = OptionTransitionSample {
        schema: stored.schema.clone(),
        feature_schema_sha256: stored.feature_schema_sha256,
        before_state_sha256: stored.before_state_sha256,
        after_state_sha256: stored.after_state_sha256,
        source_checkpoint_sha256: stored.source_checkpoint_sha256,
        next_checkpoint_sha256: stored.next_checkpoint_sha256,
        before,
        after,
        execution: OptionExecution {
            schema: stored.execution.schema.clone(),
            option_id: tactic.option_id.clone(),
            option_type: tactic.option_type.clone(),
            parameters: tactic.parameters.clone(),
            duration: stored.execution.duration,
            termination_condition: stored.execution.termination_condition.clone(),
            cancellation_conditions: stored.execution.cancellation_conditions.clone(),
            end_reason: stored.execution.end_reason,
            emitted_raw_actions: emitted_tape.frames,
            realized_tape_range: stored.execution.realized_tape_range,
            tape_sha256: stored.execution.tape_sha256,
        },
        value_sample: OptionValueSample {
            action: tactic,
            state: stored.value_sample.state.clone(),
            duration_ticks: stored.value_sample.duration_ticks,
            reward: stored.value_sample.reward,
            next_state: stored.value_sample.next_state.clone(),
            terminal: stored.value_sample.terminal,
            before_state_sha256: stored.value_sample.before_state_sha256,
            after_state_sha256: stored.value_sample.after_state_sha256,
            source_checkpoint_sha256: stored.value_sample.source_checkpoint_sha256,
            next_checkpoint_sha256: stored.value_sample.next_checkpoint_sha256,
            realized_tape_range: stored.value_sample.realized_tape_range,
            realized_tape_sha256: stored.value_sample.realized_tape_sha256,
        },
    };
    transition.validate()?;
    Ok(transition)
}

fn install_binary_artifact(path: &Path, bytes: &[u8]) -> Result<(), TacticQCampaignError> {
    if path.exists() {
        if fs::read(path).map_err(|error| TacticQCampaignError::Io(error.to_string()))? == bytes {
            return Ok(());
        }
        return Err(TacticQCampaignError::InvalidState(
            "immutable binary artifact path contains different bytes",
        ));
    }
    let parent = path.parent().ok_or(TacticQCampaignError::InvalidState(
        "checkpoint path has no parent",
    ))?;
    let partial = parent.join(format!(
        ".{}.{}.partial",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("tactic-q"),
        std::process::id()
    ));
    if partial.exists() {
        fs::remove_file(&partial).map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
    drop(file);
    fs::rename(partial, path).map_err(|error| TacticQCampaignError::Io(error.to_string()))
}

fn encode_checkpoint_envelope(raw: &[u8]) -> Result<Vec<u8>, TacticQCampaignError> {
    encode_binary_envelope(raw, CHECKPOINT_MAGIC, CHECKPOINT_FORMAT_VERSION)
}

fn encode_binary_envelope(
    raw: &[u8],
    magic: &[u8; 8],
    version: u16,
) -> Result<Vec<u8>, TacticQCampaignError> {
    let raw_len = u64::try_from(raw.len())
        .map_err(|_| TacticQCampaignError::InvalidState("binary manifest length overflows"))?;
    if raw_len > MAXIMUM_CHECKPOINT_MANIFEST_BYTES {
        return Err(TacticQCampaignError::InvalidState(
            "binary manifest exceeds its size bound",
        ));
    }
    let raw_sha256 = Digest(Sha256::digest(raw).into());
    let compressed = zstd::bulk::compress(raw, CHECKPOINT_COMPRESSION_LEVEL)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    let mut envelope = Vec::with_capacity(CHECKPOINT_HEADER_SIZE + compressed.len());
    envelope.extend_from_slice(magic);
    envelope.extend_from_slice(&version.to_le_bytes());
    envelope.extend_from_slice(&0_u16.to_le_bytes());
    envelope.extend_from_slice(&raw_len.to_le_bytes());
    envelope.extend_from_slice(&raw_sha256.0);
    debug_assert_eq!(envelope.len(), CHECKPOINT_HEADER_SIZE);
    envelope.extend_from_slice(&compressed);
    Ok(envelope)
}

fn decode_checkpoint_envelope(envelope: &[u8]) -> Result<Vec<u8>, TacticQCampaignError> {
    decode_binary_envelope(envelope, CHECKPOINT_MAGIC, CHECKPOINT_FORMAT_VERSION)
}

fn decode_binary_envelope(
    envelope: &[u8],
    magic: &[u8; 8],
    expected_version: u16,
) -> Result<Vec<u8>, TacticQCampaignError> {
    if envelope.len() < CHECKPOINT_HEADER_SIZE || &envelope[..8] != magic {
        return Err(TacticQCampaignError::InvalidState(
            "binary artifact envelope is invalid",
        ));
    }
    let version = u16::from_le_bytes(envelope[8..10].try_into().expect("fixed slice"));
    let flags = u16::from_le_bytes(envelope[10..12].try_into().expect("fixed slice"));
    let raw_len = u64::from_le_bytes(envelope[12..20].try_into().expect("fixed slice"));
    let raw_sha256 = Digest(envelope[20..52].try_into().expect("fixed slice"));
    if version != expected_version || flags != 0 || raw_len > MAXIMUM_CHECKPOINT_MANIFEST_BYTES {
        return Err(TacticQCampaignError::InvalidState(
            "binary artifact envelope identity is invalid",
        ));
    }
    let raw_len = usize::try_from(raw_len)
        .map_err(|_| TacticQCampaignError::InvalidState("binary manifest length overflows"))?;
    let raw = zstd::bulk::decompress(&envelope[CHECKPOINT_HEADER_SIZE..], raw_len)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    if raw.len() != raw_len || Digest(Sha256::digest(&raw).into()) != raw_sha256 {
        return Err(TacticQCampaignError::InvalidState(
            "binary artifact payload identity is invalid",
        ));
    }
    Ok(raw)
}

fn decode_cbor<T: for<'de> Deserialize<'de>>(raw: &[u8]) -> Result<T, TacticQContentStoreError> {
    let mut deserializer = serde_cbor::Deserializer::from_slice(raw);
    let value = T::deserialize(&mut deserializer).map_err(TacticQContentStoreError::domain)?;
    deserializer
        .end()
        .map_err(TacticQContentStoreError::domain)?;
    Ok(value)
}

fn checkpoint_store_error(error: impl fmt::Display) -> TacticQCampaignError {
    TacticQCampaignError::Serialization(error.to_string())
}

fn require_kind(
    reference: StoredContentRef,
    expected: ContentKind,
) -> Result<(), TacticQContentStoreError> {
    if reference.kind != expected || reference.sha256 == Digest::ZERO {
        return Err(TacticQContentStoreError::Invalid(
            "stored content reference kind is detached",
        ));
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) enum TacticQContentStoreError {
    Invalid(&'static str),
    Store(ContentStoreError),
    Domain(String),
}

impl TacticQContentStoreError {
    fn domain(error: impl fmt::Display) -> Self {
        Self::Domain(error.to_string())
    }
}

impl fmt::Display for TacticQContentStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "tactic-Q content is invalid: {message}"),
            Self::Store(error) => write!(formatter, "tactic-Q content store failed: {error}"),
            Self::Domain(message) => write!(formatter, "tactic-Q content codec failed: {message}"),
        }
    }
}

impl Error for TacticQContentStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
    use dusklight_learning::fact_snapshot::FactSnapshot;
    use dusklight_learning::option_values::OptionActionDescriptor;
    use std::collections::BTreeMap;
    use std::fs;

    fn fact() -> FactSnapshot {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap()
    }

    #[test]
    fn shared_store_round_trips_whole_facts_and_reads_legacy_split_objects() {
        let root = std::env::temp_dir().join(format!(
            "dusklight-tactic-shared-content-store-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let store = TacticQContentStore::initialize(&root).unwrap();
        let first = fact();
        let mut second = first.clone();
        second.boundary_index += 1;
        second.simulation_tick += 1;
        second.tape_frame += 1;
        let first_ref = store.store_fact(&first).unwrap();
        let second_ref = store.store_fact(&second).unwrap();
        assert_ne!(first_ref, second_ref);
        assert_eq!(
            first_ref.sha256,
            Digest(Sha256::digest(serde_cbor::to_vec(&first).unwrap()).into())
        );
        assert_eq!(store.load_fact(first_ref).unwrap(), first);
        assert_eq!(store.load_fact(second_ref).unwrap(), second);

        let tactic = OptionActionDescriptor {
            option_id: "move.east".into(),
            option_type: dusklight_control::option_execution::OptionType::Move,
            parameters: BTreeMap::new(),
        };
        let tactic_ref = store.store_tactic(&tactic).unwrap();
        assert_eq!(store.load_tactic(tactic_ref).unwrap(), tactic);
        let tape = InputTape::default();
        let tape_ref = store.store_tape(&tape).unwrap();
        assert_eq!(store.load_tape(tape_ref).unwrap(), tape);

        let actors = first
            .actors
            .iter()
            .map(|actor| store.store_actor(actor).unwrap())
            .collect::<Vec<_>>();
        let mut snapshot_without_actors = first.clone();
        snapshot_without_actors.actors.clear();
        let legacy_raw = serde_cbor::to_vec(&StoredFactSnapshot {
            schema: FACT_OBJECT_SCHEMA_V1.into(),
            snapshot_sha256: first.content_sha256().unwrap(),
            actors,
            snapshot_without_actors,
        })
        .unwrap();
        let legacy_ref = StoredContentRef::from(
            &ContentStore::open(&root)
                .unwrap()
                .put_bytes(&legacy_raw, ContentKind::FactSnapshot)
                .unwrap(),
        );
        assert_eq!(store.load_fact(legacy_ref).unwrap(), first);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inline_training_corpus_reader_accepts_legacy_reference_envelopes() {
        let root = std::env::temp_dir().join(format!(
            "dusklight-tactic-training-corpus-legacy-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let content_root = root.join(CONTENT_DIRECTORY);
        TacticQContentStore::initialize(&content_root).unwrap();
        let expected = TacticQTrainingCorpus {
            feature_schema_sha256: Digest([1; 32]),
            objective_sha256: Digest([2; 32]),
            root_checkpoint_sha256: Digest([3; 32]),
            transitions: Vec::new(),
            routes: Vec::new(),
            episode_groups: Vec::new(),
        };
        let legacy = StoredTrainingCorpusManifest {
            schema: TRAINING_CORPUS_MANIFEST_SCHEMA_V1.into(),
            feature_schema_sha256: expected.feature_schema_sha256,
            objective_sha256: expected.objective_sha256,
            root_checkpoint_sha256: expected.root_checkpoint_sha256,
            transitions: Vec::new(),
            routes: Vec::new(),
            episode_groups: Vec::new(),
        };
        let envelope = encode_binary_envelope(
            &serde_cbor::to_vec(&legacy).unwrap(),
            TRAINING_CORPUS_MAGIC,
            TRAINING_CORPUS_FORMAT_VERSION_V1,
        )
        .unwrap();
        let path = root.join("legacy.dtqc");
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, envelope).unwrap();
        assert_eq!(read_training_corpus(&path).unwrap(), expected);
        fs::remove_dir_all(root).unwrap();
    }
}
