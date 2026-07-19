//! Immutable, content-addressed publication of complete learning episodes.

use crate::artifact::Digest;
use crate::content_store::{ContentBlob, ContentKind, ContentStore, ContentStoreError};
use crate::episode::EpisodeManifest;
use crate::tape::InputTape;
use crate::transition_corpus::TransitionCorpus;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const EPISODE_STORE_ENTRY_SCHEMA_V1: &str = "dusklight-episode-store-entry/v1";
static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct EpisodeStore {
    root: PathBuf,
    content: ContentStore,
}

#[derive(Clone, Debug)]
pub struct EpisodeBundleSources<'a> {
    pub manifest: &'a Path,
    pub absolute_tape: &'a Path,
    pub gameplay_trace: &'a Path,
    pub transition_corpus: &'a Path,
    pub transition_evidence: &'a Path,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeStoreEntry {
    pub schema: String,
    pub episode_sha256: Digest,
    pub input_identity_sha256: Digest,
    pub manifest: ContentBlob,
    pub absolute_tape: ContentBlob,
    pub gameplay_trace: ContentBlob,
    pub transition_corpus: ContentBlob,
    pub transition_evidence: ContentBlob,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpisodeStoreIngestResult {
    pub episode_sha256: Digest,
    pub entry_path: PathBuf,
    pub created: bool,
}

impl EpisodeStore {
    pub fn initialize(root: impl Into<PathBuf>) -> Result<Self, EpisodeStoreError> {
        let root = root.into();
        let content = ContentStore::initialize(root.clone())?;
        fs::create_dir_all(root.join("episodes").join("sha256"))?;
        Ok(Self { root, content })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ingest(
        &self,
        sources: EpisodeBundleSources<'_>,
    ) -> Result<EpisodeStoreIngestResult, EpisodeStoreError> {
        let manifest: EpisodeManifest = serde_json::from_slice(&fs::read(sources.manifest)?)?;
        let corpus = TransitionCorpus::read_zstd_file(sources.transition_corpus)
            .map_err(|error| EpisodeStoreError::InvalidBundle(error.to_string()))?;
        manifest
            .validate(&corpus)
            .map_err(|error| EpisodeStoreError::InvalidBundle(error.to_string()))?;

        let tape_bytes = fs::read(sources.absolute_tape)?;
        InputTape::decode(&tape_bytes)
            .map_err(|error| EpisodeStoreError::InvalidBundle(error.to_string()))?;
        require_digest(
            "absolute tape",
            &tape_bytes,
            manifest.artifacts.absolute_tape_sha256,
        )?;
        let trace_bytes = fs::read(sources.gameplay_trace)?;
        require_digest(
            "gameplay trace",
            &trace_bytes,
            manifest.artifacts.gameplay_trace_sha256,
        )?;
        let evidence_bytes = fs::read(sources.transition_evidence)?;
        require_digest(
            "transition evidence",
            &evidence_bytes,
            manifest.artifacts.transition_evidence_sha256,
        )?;

        let entry = EpisodeStoreEntry {
            schema: EPISODE_STORE_ENTRY_SCHEMA_V1.into(),
            episode_sha256: manifest.episode_sha256,
            input_identity_sha256: manifest.input_identity_sha256,
            // Store canonical JSON rather than caller formatting so semantically
            // identical manifests converge across import and evaluator paths.
            manifest: self.content.put_bytes(
                &serde_json::to_vec(&manifest)?,
                ContentKind::EpisodeManifest,
            )?,
            absolute_tape: self
                .content
                .put_file(sources.absolute_tape, ContentKind::InputTape)?,
            gameplay_trace: self
                .content
                .put_file(sources.gameplay_trace, ContentKind::GameplayTrace)?,
            transition_corpus: self
                .content
                .put_file(sources.transition_corpus, ContentKind::TransitionCorpus)?,
            transition_evidence: self
                .content
                .put_file(sources.transition_evidence, ContentKind::TransitionEvidence)?,
        };
        self.validate_entry(&entry)?;
        let entry_path = self.entry_path(entry.episode_sha256);
        let created = install_immutable(&entry_path, &serde_json::to_vec(&entry)?)?;
        Ok(EpisodeStoreIngestResult {
            episode_sha256: entry.episode_sha256,
            entry_path,
            created,
        })
    }

    pub fn verify(&self, episode_sha256: Digest) -> Result<EpisodeStoreEntry, EpisodeStoreError> {
        let entry: EpisodeStoreEntry =
            serde_json::from_slice(&fs::read(self.entry_path(episode_sha256))?)?;
        if entry.episode_sha256 != episode_sha256 {
            return Err(EpisodeStoreError::InvalidBundle(
                "episode entry filename does not match its identity".into(),
            ));
        }
        self.validate_entry(&entry)?;
        Ok(entry)
    }

    pub fn entry_path(&self, episode_sha256: Digest) -> PathBuf {
        let digest = episode_sha256.to_string();
        self.root
            .join("episodes")
            .join("sha256")
            .join(&digest[..2])
            .join(format!("{}.json", &digest[2..]))
    }

    fn validate_entry(&self, entry: &EpisodeStoreEntry) -> Result<(), EpisodeStoreError> {
        if entry.schema != EPISODE_STORE_ENTRY_SCHEMA_V1
            || entry.episode_sha256 == Digest::ZERO
            || entry.input_identity_sha256 == Digest::ZERO
            || entry.manifest.kind != ContentKind::EpisodeManifest
            || entry.absolute_tape.kind != ContentKind::InputTape
            || entry.gameplay_trace.kind != ContentKind::GameplayTrace
            || entry.transition_corpus.kind != ContentKind::TransitionCorpus
            || entry.transition_evidence.kind != ContentKind::TransitionEvidence
        {
            return Err(EpisodeStoreError::InvalidBundle(
                "episode store entry identity or artifact kinds are invalid".into(),
            ));
        }
        for blob in [
            &entry.manifest,
            &entry.absolute_tape,
            &entry.gameplay_trace,
            &entry.transition_corpus,
            &entry.transition_evidence,
        ] {
            self.content.verify(blob)?;
        }
        let manifest: EpisodeManifest =
            serde_json::from_slice(&fs::read(self.content.blob_path(entry.manifest.sha256))?)?;
        let corpus = TransitionCorpus::read_zstd_file(
            self.content.blob_path(entry.transition_corpus.sha256),
        )
        .map_err(|error| EpisodeStoreError::InvalidBundle(error.to_string()))?;
        manifest
            .validate(&corpus)
            .map_err(|error| EpisodeStoreError::InvalidBundle(error.to_string()))?;
        if manifest.episode_sha256 != entry.episode_sha256
            || manifest.input_identity_sha256 != entry.input_identity_sha256
            || manifest.artifacts.absolute_tape_sha256 != entry.absolute_tape.sha256
            || manifest.artifacts.gameplay_trace_sha256 != entry.gameplay_trace.sha256
            || manifest.artifacts.transition_evidence_sha256 != entry.transition_evidence.sha256
        {
            return Err(EpisodeStoreError::InvalidBundle(
                "episode manifest does not authenticate stored artifacts".into(),
            ));
        }
        Ok(())
    }
}

fn require_digest(
    label: &'static str,
    bytes: &[u8],
    expected: Digest,
) -> Result<(), EpisodeStoreError> {
    let observed = Digest(Sha256::digest(bytes).into());
    if observed != expected {
        return Err(EpisodeStoreError::DigestMismatch {
            label,
            expected,
            observed,
        });
    }
    Ok(())
}

fn install_immutable(path: &Path, bytes: &[u8]) -> Result<bool, EpisodeStoreError> {
    if path.exists() {
        if fs::read(path)? == bytes {
            return Ok(false);
        }
        return Err(EpisodeStoreError::ImmutableCollision(path.to_path_buf()));
    }
    fs::create_dir_all(path.parent().expect("episode entry path has a parent"))?;
    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    match fs::hard_link(&temporary, path) {
        Ok(()) => {
            fs::remove_file(temporary)?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = fs::read(path);
            let _ = fs::remove_file(temporary);
            if existing? == bytes {
                Ok(false)
            } else {
                Err(EpisodeStoreError::ImmutableCollision(path.to_path_buf()))
            }
        }
        Err(error) => {
            let _ = fs::remove_file(temporary);
            Err(error.into())
        }
    }
}

#[derive(Debug)]
pub enum EpisodeStoreError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Content(ContentStoreError),
    InvalidBundle(String),
    DigestMismatch {
        label: &'static str,
        expected: Digest,
        observed: Digest,
    },
    ImmutableCollision(PathBuf),
}

impl fmt::Display for EpisodeStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "episode store I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "invalid episode store JSON: {error}"),
            Self::Content(error) => write!(formatter, "episode content storage failed: {error}"),
            Self::InvalidBundle(message) => write!(formatter, "invalid episode bundle: {message}"),
            Self::DigestMismatch {
                label,
                expected,
                observed,
            } => write!(
                formatter,
                "{label} SHA-256 mismatch: expected {expected}, observed {observed}"
            ),
            Self::ImmutableCollision(path) => {
                write!(
                    formatter,
                    "immutable episode collision at {}",
                    path.display()
                )
            }
        }
    }
}

impl Error for EpisodeStoreError {}

impl From<std::io::Error> for EpisodeStoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for EpisodeStoreError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<ContentStoreError> for EpisodeStoreError {
    fn from(value: ContentStoreError) -> Self {
        Self::Content(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode::{
        EPISODE_CONTEXT_SCHEMA_V1, EpisodeContext, EpisodeLineage, EpisodeManifestBuild,
        EpisodeObjectiveIdentity, EpisodeOutcome, EpisodeOutcomeClass, EpisodeProducerIdentity,
        EpisodeProducerKind, EpisodeSeed, RunBuildIdentity,
    };
    use crate::tape::{InputFrame, TapeBoot};
    use crate::transition_corpus::{MacroAction, StateReference, StateReferenceKind, Transition};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture_corpus() -> TransitionCorpus {
        TransitionCorpus::new(
            Digest([1; 32]),
            Digest([2; 32]),
            1,
            vec![Transition {
                source: StateReference {
                    kind: StateReferenceKind::Boundary,
                    digest: Digest([3; 32]),
                },
                state: vec![0.0],
                action: MacroAction {
                    action_id: 1,
                    macro_kind: 1,
                    parameters: Vec::new(),
                },
                duration_ticks: 1,
                reward: 1.0,
                next: StateReference {
                    kind: StateReferenceKind::Boundary,
                    digest: Digest([4; 32]),
                },
                next_state: vec![1.0],
                terminal: true,
            }],
        )
        .unwrap()
    }

    fn context() -> EpisodeContext {
        EpisodeContext {
            schema: EPISODE_CONTEXT_SCHEMA_V1.into(),
            run_identity: None,
            run_build: RunBuildIdentity {
                executable_sha256: Digest([5; 32]),
                dusklight_commit: Some("abc123".into()),
                aurora_commit: None,
                target: Some("aarch64-apple-darwin".into()),
                profile: Some("debug".into()),
                feature_digest: None,
            },
            objective: EpisodeObjectiveIdentity {
                id: "store-test".into(),
                digest: Digest([6; 32]),
            },
            producer: EpisodeProducerIdentity {
                kind: EpisodeProducerKind::Evolution,
                name: "huntctl".into(),
                version: "1".into(),
            },
            seed: EpisodeSeed::Deterministic { value: 7 },
            worker_id: "worker-0".into(),
            lineage: EpisodeLineage {
                candidate_id: Some("candidate-a".into()),
                parent_candidate_id: None,
                generation: 0,
                intervention: None,
            },
            outcome: EpisodeOutcome {
                class: EpisodeOutcomeClass::Successful,
                reason: "objective reached".into(),
            },
        }
    }

    #[test]
    fn independent_roots_deduplicate_and_tampering_is_rejected() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("huntctl-episode-store-{nonce}"));
        let first_root = root.join("run-a");
        let second_root = root.join("run-b");
        fs::create_dir_all(&first_root).unwrap();
        fs::create_dir_all(&second_root).unwrap();

        let tape_bytes = InputTape {
            frames: vec![InputFrame::default()],
            ..InputTape::default()
        }
        .encode()
        .unwrap();
        let trace_bytes = b"authenticated trace";
        let evidence_bytes = b"authenticated transition evidence";
        let corpus = fixture_corpus();
        let manifest = EpisodeManifest::build(EpisodeManifestBuild {
            context: &context(),
            boot: &TapeBoot::Process,
            corpus: &corpus,
            query_view_id: "movement-state/v2",
            tape_sha256: Digest(Sha256::digest(&tape_bytes).into()),
            trace_sha256: Digest(Sha256::digest(trace_bytes).into()),
            transition_evidence_sha256: Digest(Sha256::digest(evidence_bytes).into()),
        })
        .unwrap();

        for run_root in [&first_root, &second_root] {
            fs::write(
                run_root.join("episode.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            fs::write(run_root.join("run.tape"), &tape_bytes).unwrap();
            fs::write(run_root.join("gameplay.trace"), trace_bytes).unwrap();
            corpus
                .write_zstd_file(run_root.join("transitions.dtcz"), 3)
                .unwrap();
            fs::write(run_root.join("transitions.evidence.json"), evidence_bytes).unwrap();
        }

        let store = EpisodeStore::initialize(root.join("shared")).unwrap();
        let ingest = |run_root: &Path| {
            store
                .ingest(EpisodeBundleSources {
                    manifest: &run_root.join("episode.json"),
                    absolute_tape: &run_root.join("run.tape"),
                    gameplay_trace: &run_root.join("gameplay.trace"),
                    transition_corpus: &run_root.join("transitions.dtcz"),
                    transition_evidence: &run_root.join("transitions.evidence.json"),
                })
                .unwrap()
        };
        let first = ingest(&first_root);
        let second = ingest(&second_root);
        assert!(first.created);
        assert!(!second.created);
        assert_eq!(first.entry_path, second.entry_path);
        assert_eq!(
            store.verify(first.episode_sha256).unwrap().episode_sha256,
            manifest.episode_sha256
        );

        fs::write(second_root.join("gameplay.trace"), b"tampered").unwrap();
        assert!(
            store
                .ingest(EpisodeBundleSources {
                    manifest: &second_root.join("episode.json"),
                    absolute_tape: &second_root.join("run.tape"),
                    gameplay_trace: &second_root.join("gameplay.trace"),
                    transition_corpus: &second_root.join("transitions.dtcz"),
                    transition_evidence: &second_root.join("transitions.evidence.json"),
                })
                .unwrap_err()
                .to_string()
                .contains("gameplay trace SHA-256 mismatch")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
