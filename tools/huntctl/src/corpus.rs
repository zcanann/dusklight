use crate::artifact::{BuildIdentity, Digest};
use crate::tape::{InputTape, TapeError, TapeVersion};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as ShaDigest, Sha256};
use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const CORPUS_SCHEMA: &str = "dusklight-corpus/v1";
pub const RUN_SCHEMA: &str = "dusklight-run/v1";
pub const TAPE_MEDIA_TYPE: &str = "application/x-dusktape";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CorpusManifest {
    schema: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioManifest {
    pub id: String,
    pub metadata: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlobManifest {
    pub algorithm: String,
    pub digest: Digest,
    pub size: u64,
    pub media_type: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunManifest {
    pub schema: String,
    pub build: BuildIdentity,
    pub scenario: ScenarioManifest,
    pub tape: BlobManifest,
    pub tape_version: TapeVersion,
    pub tick_rate_numerator: u32,
    pub tick_rate_denominator: u32,
    pub frame_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestResult {
    pub artifact_id: Digest,
    pub tape_digest: Digest,
    pub created: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListedArtifact {
    pub artifact_id: Digest,
    pub manifest: RunManifest,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct VerifyReport {
    pub artifacts: usize,
    pub blobs: usize,
}

#[derive(Debug)]
pub enum CorpusError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Tape(TapeError),
    NotInitialized,
    SchemaMismatch {
        expected: &'static str,
        received: String,
    },
    InvalidScenario,
    InvalidDigestName(String),
    HashMismatch {
        expected: Digest,
        received: Digest,
    },
    BlobSizeMismatch {
        expected: u64,
        received: u64,
    },
    ImmutableCollision(PathBuf),
}

impl fmt::Display for CorpusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "corpus I/O error: {error}"),
            Self::Json(error) => write!(f, "invalid corpus JSON: {error}"),
            Self::Tape(error) => write!(f, "invalid input tape: {error}"),
            Self::NotInitialized => f.write_str("directory is not an initialized Dusklight corpus"),
            Self::SchemaMismatch { expected, received } => {
                write!(f, "expected schema {expected:?}, received {received:?}")
            }
            Self::InvalidScenario => {
                f.write_str("scenario id must be nonempty and metadata must be a JSON object")
            }
            Self::InvalidDigestName(name) => {
                write!(f, "invalid content-addressed filename {name:?}")
            }
            Self::HashMismatch { expected, received } => write!(
                f,
                "SHA-256 mismatch: expected {expected}, received {received}"
            ),
            Self::BlobSizeMismatch { expected, received } => write!(
                f,
                "blob size mismatch: expected {expected}, received {received}"
            ),
            Self::ImmutableCollision(path) => {
                write!(f, "immutable content collision at {}", path.display())
            }
        }
    }
}

impl Error for CorpusError {}
impl From<std::io::Error> for CorpusError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<serde_json::Error> for CorpusError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
impl From<TapeError> for CorpusError {
    fn from(value: TapeError) -> Self {
        Self::Tape(value)
    }
}

pub struct Corpus {
    root: PathBuf,
}

impl Corpus {
    pub fn initialize(root: impl AsRef<Path>) -> Result<Self, CorpusError> {
        let root = root.as_ref();
        if root.join("corpus.json").exists() {
            return Self::open(root);
        }
        fs::create_dir_all(root.join("runs"))?;
        fs::create_dir_all(root.join("blobs").join("sha256"))?;
        let manifest = serde_json::to_vec_pretty(&CorpusManifest {
            schema: CORPUS_SCHEMA.into(),
        })?;
        install_immutable(&root.join("corpus.json"), &manifest)?;
        Self::open(root)
    }

    pub fn open(root: impl AsRef<Path>) -> Result<Self, CorpusError> {
        let root = root.as_ref().to_path_buf();
        let path = root.join("corpus.json");
        if !path.is_file() {
            return Err(CorpusError::NotInitialized);
        }
        let manifest: CorpusManifest = serde_json::from_slice(&fs::read(path)?)?;
        require_schema(&manifest.schema, CORPUS_SCHEMA)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ingest(
        &self,
        tape_bytes: &[u8],
        build: BuildIdentity,
        scenario_id: String,
        scenario_metadata: Value,
    ) -> Result<IngestResult, CorpusError> {
        if scenario_id.is_empty() || !scenario_metadata.is_object() {
            return Err(CorpusError::InvalidScenario);
        }
        let decoded = InputTape::decode(tape_bytes)?;
        let tape_digest = sha256(tape_bytes);
        install_immutable(&self.blob_path(tape_digest), tape_bytes)?;
        let manifest = RunManifest {
            schema: RUN_SCHEMA.into(),
            build,
            scenario: ScenarioManifest {
                id: scenario_id,
                metadata: scenario_metadata,
            },
            tape: BlobManifest {
                algorithm: "sha256".into(),
                digest: tape_digest,
                size: tape_bytes.len() as u64,
                media_type: TAPE_MEDIA_TYPE.into(),
            },
            tape_version: decoded.source_version,
            tick_rate_numerator: decoded.tape.tick_rate_numerator,
            tick_rate_denominator: decoded.tape.tick_rate_denominator,
            frame_count: decoded.tape.frames.len() as u64,
        };
        let manifest_bytes = serde_json::to_vec(&manifest)?;
        let artifact_id = sha256(&manifest_bytes);
        let created = install_immutable(&self.run_path(artifact_id), &manifest_bytes)?;
        Ok(IngestResult {
            artifact_id,
            tape_digest,
            created,
        })
    }

    pub fn show(&self, artifact_id: Digest) -> Result<RunManifest, CorpusError> {
        self.read_run(artifact_id)
    }

    pub fn list(&self) -> Result<Vec<ListedArtifact>, CorpusError> {
        let mut artifacts = Vec::new();
        for entry in fs::read_dir(self.root.join("runs"))? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };
            let artifact_id = stem
                .parse()
                .map_err(|_| CorpusError::InvalidDigestName(name))?;
            artifacts.push(ListedArtifact {
                artifact_id,
                manifest: self.read_run(artifact_id)?,
            });
        }
        artifacts.sort_by_key(|artifact| artifact.artifact_id.to_string());
        Ok(artifacts)
    }

    pub fn verify(&self) -> Result<VerifyReport, CorpusError> {
        let artifacts = self.list()?;
        let mut referenced = HashSet::new();
        for artifact in &artifacts {
            let blob = &artifact.manifest.tape;
            if blob.algorithm != "sha256" {
                return Err(CorpusError::SchemaMismatch {
                    expected: "sha256",
                    received: blob.algorithm.clone(),
                });
            }
            if blob.media_type != TAPE_MEDIA_TYPE {
                return Err(CorpusError::SchemaMismatch {
                    expected: TAPE_MEDIA_TYPE,
                    received: blob.media_type.clone(),
                });
            }
            let bytes = fs::read(self.blob_path(blob.digest))?;
            if bytes.len() as u64 != blob.size {
                return Err(CorpusError::BlobSizeMismatch {
                    expected: blob.size,
                    received: bytes.len() as u64,
                });
            }
            verify_digest(blob.digest, &bytes)?;
            InputTape::decode(&bytes)?;
            referenced.insert(blob.digest);
        }
        for first_byte in fs::read_dir(self.root.join("blobs").join("sha256"))? {
            let first_byte = first_byte?;
            if !first_byte.file_type()?.is_dir() {
                continue;
            }
            let prefix = first_byte.file_name().to_string_lossy().into_owned();
            for entry in fs::read_dir(first_byte.path())? {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let name = format!("{}{}", prefix, entry.file_name().to_string_lossy());
                let digest: Digest = name
                    .parse()
                    .map_err(|_| CorpusError::InvalidDigestName(name))?;
                verify_digest(digest, &fs::read(entry.path())?)?;
                referenced.insert(digest);
            }
        }
        Ok(VerifyReport {
            artifacts: artifacts.len(),
            blobs: referenced.len(),
        })
    }

    pub fn blob_path(&self, digest: Digest) -> PathBuf {
        let digest = digest.to_string();
        self.root
            .join("blobs")
            .join("sha256")
            .join(&digest[..2])
            .join(&digest[2..])
    }

    fn run_path(&self, digest: Digest) -> PathBuf {
        self.root.join("runs").join(format!("{digest}.json"))
    }

    fn read_run(&self, artifact_id: Digest) -> Result<RunManifest, CorpusError> {
        let bytes = fs::read(self.run_path(artifact_id))?;
        verify_digest(artifact_id, &bytes)?;
        let manifest: RunManifest = serde_json::from_slice(&bytes)?;
        require_schema(&manifest.schema, RUN_SCHEMA)?;
        Ok(manifest)
    }
}

pub fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn require_schema(received: &str, expected: &'static str) -> Result<(), CorpusError> {
    if received == expected {
        Ok(())
    } else {
        Err(CorpusError::SchemaMismatch {
            expected,
            received: received.into(),
        })
    }
}

fn verify_digest(expected: Digest, bytes: &[u8]) -> Result<(), CorpusError> {
    let received = sha256(bytes);
    if received == expected {
        Ok(())
    } else {
        Err(CorpusError::HashMismatch { expected, received })
    }
}

fn install_immutable(path: &Path, bytes: &[u8]) -> Result<bool, CorpusError> {
    if path.exists() {
        return if fs::read(path)? == bytes {
            Ok(false)
        } else {
            Err(CorpusError::ImmutableCollision(path.into()))
        };
    }
    fs::create_dir_all(path.parent().expect("content path has a parent"))?;
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .expect("content path has a filename")
        .to_string_lossy();
    let temporary =
        path.with_file_name(format!(".{file_name}.tmp-{}-{counter}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    drop(file);
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(true),
        Err(_error) if path.exists() => {
            let existing = fs::read(path)?;
            let _ = fs::remove_file(&temporary);
            if existing == bytes {
                Ok(false)
            } else {
                Err(CorpusError::ImmutableCollision(path.into()))
            }
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error.into())
        }
    }
}
