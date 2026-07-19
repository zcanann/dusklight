//! Immutable, content-addressed publication of complete learning episodes.

use crate::artifact::Digest;
use crate::content_store::{
    ContentBlob, ContentGcReport, ContentKind, ContentStore, ContentStoreError,
};
use crate::episode::EpisodeManifest;
use crate::tape::InputTape;
use crate::transition_corpus::TransitionCorpus;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const EPISODE_STORE_ENTRY_SCHEMA_V1: &str = "dusklight-episode-store-entry/v1";
pub const EPISODE_STORE_GC_SCHEMA_V1: &str = "dusklight-episode-store-gc/v1";
pub const MAX_EPISODE_STORE_ENTRIES: usize = 1_000_000;
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeStoreVerifyReport {
    pub entries: usize,
    pub unique_blobs: usize,
    pub episode_sha256: Vec<Digest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeGcEntry {
    pub episode_sha256: Digest,
    pub size: u64,
    pub source: PathBuf,
    pub trash_destination: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeStoreGcReport {
    pub schema: String,
    pub dry_run: bool,
    pub store_root: PathBuf,
    pub trash_root: PathBuf,
    pub retained: Vec<Digest>,
    pub unretained: Vec<EpisodeGcEntry>,
    pub moved_entries: usize,
    pub reclaimed_bytes: u64,
    pub content: ContentGcReport,
}

impl EpisodeStore {
    pub fn initialize(root: impl Into<PathBuf>) -> Result<Self, EpisodeStoreError> {
        let root = root.into();
        let content = ContentStore::initialize(root.clone())?;
        fs::create_dir_all(root.join("episodes").join("sha256"))?;
        Ok(Self { root, content })
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self, EpisodeStoreError> {
        let root = root.into();
        let content = ContentStore::open(root.clone())?;
        if !root.join("episodes").join("sha256").is_dir() {
            return Err(EpisodeStoreError::NotInitialized(root));
        }
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

    pub fn verify_all(&self) -> Result<EpisodeStoreVerifyReport, EpisodeStoreError> {
        let ids = self.entry_ids()?;
        let mut blobs = BTreeSet::new();
        for episode_sha256 in &ids {
            let entry = self.verify(*episode_sha256)?;
            blobs.extend(entry_blob_digests(&entry));
        }
        Ok(EpisodeStoreVerifyReport {
            entries: ids.len(),
            unique_blobs: blobs.len(),
            episode_sha256: ids,
        })
    }

    /// Retains explicitly selected episode identities and moves every other
    /// entry plus blobs unreachable from the retained entries to recoverable
    /// trash. An empty retained set is never accepted.
    pub fn garbage_collect(
        &self,
        retained: &BTreeSet<Digest>,
        additionally_referenced: &BTreeSet<Digest>,
        trash_root: &Path,
        dry_run: bool,
    ) -> Result<EpisodeStoreGcReport, EpisodeStoreError> {
        if retained.is_empty() {
            return Err(EpisodeStoreError::EmptyRetentionSet);
        }
        let store_root = fs::canonicalize(&self.root)?;
        let resolved_trash = resolve_path_even_if_missing(trash_root)?;
        if resolved_trash == store_root || resolved_trash.starts_with(&store_root) {
            return Err(EpisodeStoreError::UnsafeTrashRoot);
        }

        let present = self.entry_ids()?;
        let present_set = present.iter().copied().collect::<BTreeSet<_>>();
        let missing = retained
            .difference(&present_set)
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(EpisodeStoreError::RetainedEpisodeMissing(missing));
        }

        let mut referenced_blobs = additionally_referenced.clone();
        let mut unretained = Vec::new();
        for episode_sha256 in present {
            let entry = self.verify(episode_sha256)?;
            if retained.contains(&episode_sha256) {
                referenced_blobs.extend(entry_blob_digests(&entry));
            } else {
                let source = self.entry_path(episode_sha256);
                let digest = episode_sha256.to_string();
                unretained.push(EpisodeGcEntry {
                    episode_sha256,
                    size: fs::metadata(&source)?.len(),
                    source,
                    trash_destination: resolved_trash
                        .join("episodes")
                        .join("sha256")
                        .join(&digest[..2])
                        .join(format!("{}.json", &digest[2..])),
                });
            }
        }
        unretained.sort_by_key(|entry| entry.episode_sha256);
        for entry in &unretained {
            if entry.trash_destination.exists() {
                return Err(EpisodeStoreError::TrashDestinationExists(
                    entry.trash_destination.clone(),
                ));
            }
        }

        let content = self
            .content
            .garbage_collect(&referenced_blobs, &resolved_trash, dry_run)?;
        if !dry_run {
            for entry in &unretained {
                fs::create_dir_all(
                    entry
                        .trash_destination
                        .parent()
                        .expect("episode trash path has a parent"),
                )?;
                fs::rename(&entry.source, &entry.trash_destination)?;
            }
        }
        let reclaimed_bytes =
            content.reclaimed_bytes + unretained.iter().map(|entry| entry.size).sum::<u64>();
        Ok(EpisodeStoreGcReport {
            schema: EPISODE_STORE_GC_SCHEMA_V1.into(),
            dry_run,
            store_root,
            trash_root: resolved_trash,
            retained: retained.iter().copied().collect(),
            moved_entries: if dry_run { 0 } else { unretained.len() },
            reclaimed_bytes,
            unretained,
            content,
        })
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

    fn entry_ids(&self) -> Result<Vec<Digest>, EpisodeStoreError> {
        let entry_root = self.root.join("episodes").join("sha256");
        let mut ids = Vec::new();
        for prefix in fs::read_dir(entry_root)? {
            let prefix = prefix?;
            let prefix_name = prefix.file_name().to_string_lossy().into_owned();
            if !prefix.file_type()?.is_dir() || !is_lower_hex(&prefix_name, 2) {
                return Err(EpisodeStoreError::InvalidEntryPath(prefix.path()));
            }
            for entry in fs::read_dir(prefix.path())? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let Some(suffix) = name.strip_suffix(".json") else {
                    return Err(EpisodeStoreError::InvalidEntryPath(entry.path()));
                };
                if !entry.file_type()?.is_file() || !is_lower_hex(suffix, 62) {
                    return Err(EpisodeStoreError::InvalidEntryPath(entry.path()));
                }
                ids.push(
                    format!("{prefix_name}{suffix}")
                        .parse()
                        .map_err(|_| EpisodeStoreError::InvalidEntryPath(entry.path()))?,
                );
                if ids.len() > MAX_EPISODE_STORE_ENTRIES {
                    return Err(EpisodeStoreError::TooManyEntries(ids.len()));
                }
            }
        }
        ids.sort_unstable();
        Ok(ids)
    }
}

fn entry_blob_digests(entry: &EpisodeStoreEntry) -> impl Iterator<Item = Digest> {
    [
        entry.manifest.sha256,
        entry.absolute_tape.sha256,
        entry.gameplay_trace.sha256,
        entry.transition_corpus.sha256,
        entry.transition_evidence.sha256,
    ]
    .into_iter()
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn resolve_path_even_if_missing(path: &Path) -> Result<PathBuf, EpisodeStoreError> {
    if path.as_os_str().is_empty() {
        return Err(EpisodeStoreError::UnsafeTrashRoot);
    }
    if path.exists() {
        return Ok(fs::canonicalize(path)?);
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut missing = Vec::new();
    let mut existing = absolute.as_path();
    while !existing.exists() {
        missing.push(
            existing
                .file_name()
                .ok_or(EpisodeStoreError::UnsafeTrashRoot)?
                .to_os_string(),
        );
        existing = existing
            .parent()
            .ok_or(EpisodeStoreError::UnsafeTrashRoot)?;
    }
    let mut resolved = fs::canonicalize(existing)?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
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
    NotInitialized(PathBuf),
    Json(serde_json::Error),
    Content(ContentStoreError),
    InvalidBundle(String),
    InvalidEntryPath(PathBuf),
    TooManyEntries(usize),
    EmptyRetentionSet,
    RetainedEpisodeMissing(Vec<Digest>),
    UnsafeTrashRoot,
    TrashDestinationExists(PathBuf),
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
            Self::NotInitialized(path) => write!(
                formatter,
                "directory is not an initialized episode store: {}",
                path.display()
            ),
            Self::Json(error) => write!(formatter, "invalid episode store JSON: {error}"),
            Self::Content(error) => write!(formatter, "episode content storage failed: {error}"),
            Self::InvalidBundle(message) => write!(formatter, "invalid episode bundle: {message}"),
            Self::InvalidEntryPath(path) => {
                write!(formatter, "invalid episode entry path: {}", path.display())
            }
            Self::TooManyEntries(count) => write!(
                formatter,
                "episode store contains {count} entries; limit is {MAX_EPISODE_STORE_ENTRIES}"
            ),
            Self::EmptyRetentionSet => {
                formatter.write_str("episode garbage collection requires a nonempty retention set")
            }
            Self::RetainedEpisodeMissing(missing) => write!(
                formatter,
                "retained episode identities are missing from the store: {}",
                missing
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::UnsafeTrashRoot => formatter
                .write_str("episode trash root must be explicit and outside the live store"),
            Self::TrashDestinationExists(path) => write!(
                formatter,
                "episode trash destination already exists: {}",
                path.display()
            ),
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

    fn context(worker_id: &str) -> EpisodeContext {
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
            worker_id: worker_id.into(),
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
            context: &context("worker-0"),
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

        let third_root = root.join("run-c");
        fs::create_dir_all(&third_root).unwrap();
        let second_manifest = EpisodeManifest::build(EpisodeManifestBuild {
            context: &context("worker-1"),
            boot: &TapeBoot::Process,
            corpus: &corpus,
            query_view_id: "movement-state/v2",
            tape_sha256: Digest(Sha256::digest(&tape_bytes).into()),
            trace_sha256: Digest(Sha256::digest(trace_bytes).into()),
            transition_evidence_sha256: Digest(Sha256::digest(evidence_bytes).into()),
        })
        .unwrap();
        fs::write(
            third_root.join("episode.json"),
            serde_json::to_vec_pretty(&second_manifest).unwrap(),
        )
        .unwrap();
        fs::copy(first_root.join("run.tape"), third_root.join("run.tape")).unwrap();
        fs::copy(
            first_root.join("gameplay.trace"),
            third_root.join("gameplay.trace"),
        )
        .unwrap();
        fs::copy(
            first_root.join("transitions.dtcz"),
            third_root.join("transitions.dtcz"),
        )
        .unwrap();
        fs::copy(
            first_root.join("transitions.evidence.json"),
            third_root.join("transitions.evidence.json"),
        )
        .unwrap();
        let third = ingest(&third_root);
        assert!(third.created);
        let verified = store.verify_all().unwrap();
        assert_eq!(verified.entries, 2);
        assert_eq!(verified.unique_blobs, 6);

        let malformed = store.root().join("episodes/sha256/not-hex");
        fs::create_dir_all(&malformed).unwrap();
        assert!(matches!(
            store.verify_all(),
            Err(EpisodeStoreError::InvalidEntryPath(_))
        ));
        fs::remove_dir(malformed).unwrap();

        let retained = BTreeSet::from([first.episode_sha256]);
        let trash = root.join("trash");
        assert!(matches!(
            store.garbage_collect(&BTreeSet::new(), &BTreeSet::new(), &trash, true),
            Err(EpisodeStoreError::EmptyRetentionSet)
        ));
        assert!(matches!(
            store.garbage_collect(
                &retained,
                &BTreeSet::new(),
                &store.root().join("trash"),
                true
            ),
            Err(EpisodeStoreError::UnsafeTrashRoot)
        ));
        assert!(matches!(
            store.garbage_collect(
                &BTreeSet::from([Digest([0xaa; 32])]),
                &BTreeSet::new(),
                &trash,
                true
            ),
            Err(EpisodeStoreError::RetainedEpisodeMissing(_))
        ));
        let preview = store
            .garbage_collect(&retained, &BTreeSet::new(), &trash, true)
            .unwrap();
        assert_eq!(preview.unretained.len(), 1);
        assert_eq!(preview.content.unreachable.len(), 1);
        assert_eq!(preview.moved_entries, 0);
        assert!(third.entry_path.is_file());
        let applied = store
            .garbage_collect(&retained, &BTreeSet::new(), &trash, false)
            .unwrap();
        assert_eq!(applied.moved_entries, 1);
        assert_eq!(applied.content.moved, 1);
        assert!(!third.entry_path.exists());
        assert!(applied.unretained[0].trash_destination.is_file());
        assert_eq!(store.verify_all().unwrap().entries, 1);

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
        let retained_entry = store.verify(first.episode_sha256).unwrap();
        fs::write(
            store
                .content
                .blob_path(retained_entry.gameplay_trace.sha256),
            b"corrupt",
        )
        .unwrap();
        assert!(store.verify_all().is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
