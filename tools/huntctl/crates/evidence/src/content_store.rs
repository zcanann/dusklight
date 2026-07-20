//! Bounded, verified content-addressed storage for large immutable artifacts.

use crate::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const CONTENT_BLOB_SCHEMA_V1: &str = "dusklight-content-blob/v1";
pub const MAX_CONTENT_BLOB_BYTES: u64 = 16 * 1024 * 1024 * 1024;
static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    InputTape,
    GameplayTrace,
    TransitionCorpus,
    TransitionEvidence,
    EpisodeManifest,
    ActorProfileCatalog,
    WorldContext,
    WorldInventory,
    WorldSpatialIndex,
    NativeGeometryView,
    Screenshot,
    Model,
    DatasetManifest,
    CrashArtifact,
}

impl ContentKind {
    pub fn media_type(self) -> &'static str {
        match self {
            Self::InputTape => "application/x-dusktape",
            Self::GameplayTrace => "application/x-dusktrace",
            Self::TransitionCorpus => "application/x-dusklight-transition-corpus",
            Self::TransitionEvidence => "application/vnd.dusklight.transition-evidence+json",
            Self::EpisodeManifest => "application/vnd.dusklight.episode-manifest+json",
            Self::ActorProfileCatalog => "application/vnd.dusklight.actor-profile-catalog+json",
            Self::WorldContext => "application/vnd.dusklight.world-context+json",
            Self::WorldInventory => "application/vnd.dusklight.world-inventory+json",
            Self::WorldSpatialIndex => "application/vnd.dusklight.world-spatial-index+json",
            Self::NativeGeometryView => "application/vnd.dusklight.native-geometry-view+json",
            Self::Screenshot => "image/png",
            Self::Model => "application/vnd.dusklight.model+json",
            Self::DatasetManifest => "application/vnd.dusklight.dataset-manifest+json",
            Self::CrashArtifact => "application/octet-stream",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContentBlob {
    pub schema: String,
    pub kind: ContentKind,
    pub sha256: Digest,
    pub size: u64,
    pub media_type: String,
    pub relative_path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ContentStore {
    root: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContentGcEntry {
    pub sha256: Digest,
    pub size: u64,
    pub source: PathBuf,
    pub trash_destination: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContentGcReport {
    pub schema: String,
    pub dry_run: bool,
    pub store_root: PathBuf,
    pub trash_root: PathBuf,
    pub reachable: Vec<Digest>,
    pub referenced_missing: Vec<Digest>,
    pub unreachable: Vec<ContentGcEntry>,
    pub moved: usize,
    pub reclaimed_bytes: u64,
}

impl ContentStore {
    pub fn initialize(root: impl Into<PathBuf>) -> Result<Self, ContentStoreError> {
        let store = Self { root: root.into() };
        fs::create_dir_all(store.root.join("blobs").join("sha256"))?;
        fs::create_dir_all(store.root.join("tmp"))?;
        Ok(store)
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self, ContentStoreError> {
        let store = Self { root: root.into() };
        if !store.root.join("blobs").join("sha256").is_dir() || !store.root.join("tmp").is_dir() {
            return Err(ContentStoreError::NotInitialized(store.root));
        }
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn put_bytes(
        &self,
        bytes: &[u8],
        kind: ContentKind,
    ) -> Result<ContentBlob, ContentStoreError> {
        let size = u64::try_from(bytes.len()).map_err(|_| ContentStoreError::TooLarge(u64::MAX))?;
        if size > MAX_CONTENT_BLOB_BYTES {
            return Err(ContentStoreError::TooLarge(size));
        }
        let digest = Digest(Sha256::digest(bytes).into());
        let destination = self.blob_path(digest);
        install_bytes(&destination, bytes)?;
        Ok(self.reference(kind, digest, size))
    }

    pub fn put_file(
        &self,
        source: &Path,
        kind: ContentKind,
    ) -> Result<ContentBlob, ContentStoreError> {
        let size = fs::metadata(source)?.len();
        if size > MAX_CONTENT_BLOB_BYTES {
            return Err(ContentStoreError::TooLarge(size));
        }
        let counter = TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = self
            .root
            .join("tmp")
            .join(format!("blob-{}-{counter}.tmp", std::process::id()));
        let mut input = File::open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let mut hasher = Sha256::new();
        let mut observed = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            observed = observed
                .checked_add(count as u64)
                .ok_or(ContentStoreError::TooLarge(u64::MAX))?;
            if observed > MAX_CONTENT_BLOB_BYTES {
                let _ = fs::remove_file(&temporary);
                return Err(ContentStoreError::TooLarge(observed));
            }
            hasher.update(&buffer[..count]);
            output.write_all(&buffer[..count])?;
        }
        output.sync_all()?;
        drop(output);
        if observed != size {
            let _ = fs::remove_file(&temporary);
            return Err(ContentStoreError::SourceChanged {
                expected: size,
                observed,
            });
        }
        let digest = Digest(hasher.finalize().into());
        let destination = self.blob_path(digest);
        install_temporary(&temporary, &destination, digest, size)?;
        Ok(self.reference(kind, digest, size))
    }

    pub fn verify(&self, blob: &ContentBlob) -> Result<(), ContentStoreError> {
        if blob.schema != CONTENT_BLOB_SCHEMA_V1
            || blob.media_type != blob.kind.media_type()
            || blob.relative_path != self.relative_blob_path(blob.sha256)
        {
            return Err(ContentStoreError::InvalidReference);
        }
        verify_file(&self.root.join(&blob.relative_path), blob.sha256, blob.size)
    }

    pub fn blob_path(&self, digest: Digest) -> PathBuf {
        self.root.join(self.relative_blob_path(digest))
    }

    /// Finds unreferenced blobs and, when `dry_run` is false, moves them to an
    /// explicit trash tree. Blobs are verified before they are reported or
    /// moved; this operation never permanently deletes content.
    pub fn garbage_collect(
        &self,
        referenced: &BTreeSet<Digest>,
        trash_root: &Path,
        dry_run: bool,
    ) -> Result<ContentGcReport, ContentStoreError> {
        if trash_root.as_os_str().is_empty() {
            return Err(ContentStoreError::UnsafeTrashRoot);
        }
        let store_root = fs::canonicalize(&self.root)?;
        let live_blob_tree = fs::canonicalize(self.root.join("blobs"))?;
        let blob_root = fs::canonicalize(self.root.join("blobs").join("sha256"))?;
        let resolved_trash = resolve_path_even_if_missing(trash_root)?;
        if resolved_trash == store_root
            || resolved_trash == live_blob_tree
            || resolved_trash.starts_with(&live_blob_tree)
        {
            return Err(ContentStoreError::UnsafeTrashRoot);
        }

        let mut present_reachable = Vec::new();
        let mut present = BTreeSet::new();
        let mut unreachable = Vec::new();
        for prefix in fs::read_dir(&blob_root)? {
            let prefix = prefix?;
            let prefix_name = prefix.file_name().to_string_lossy().into_owned();
            if !prefix.file_type()?.is_dir() || !is_lower_hex(&prefix_name, 2) {
                return Err(ContentStoreError::InvalidBlobPath(prefix.path()));
            }
            for entry in fs::read_dir(prefix.path())? {
                let entry = entry?;
                let suffix = entry.file_name().to_string_lossy().into_owned();
                if !entry.file_type()?.is_file() || !is_lower_hex(&suffix, 62) {
                    return Err(ContentStoreError::InvalidBlobPath(entry.path()));
                }
                let digest: Digest = format!("{prefix_name}{suffix}")
                    .parse()
                    .map_err(|_| ContentStoreError::InvalidBlobPath(entry.path()))?;
                let size = entry.metadata()?.len();
                verify_file(&entry.path(), digest, size)?;
                present.insert(digest);
                if referenced.contains(&digest) {
                    present_reachable.push(digest);
                } else {
                    let digest_text = digest.to_string();
                    unreachable.push(ContentGcEntry {
                        sha256: digest,
                        size,
                        source: entry.path(),
                        trash_destination: resolved_trash
                            .join("blobs")
                            .join("sha256")
                            .join(&digest_text[..2])
                            .join(&digest_text[2..]),
                    });
                }
            }
        }
        present_reachable.sort_unstable();
        unreachable.sort_by_key(|entry| entry.sha256);
        let referenced_missing = referenced.difference(&present).copied().collect();
        let reclaimed_bytes = unreachable.iter().map(|entry| entry.size).sum();
        if !dry_run {
            for entry in &unreachable {
                if entry.trash_destination.exists() {
                    return Err(ContentStoreError::TrashDestinationExists(
                        entry.trash_destination.clone(),
                    ));
                }
            }
            for entry in &unreachable {
                fs::create_dir_all(
                    entry
                        .trash_destination
                        .parent()
                        .expect("content trash path has a parent"),
                )?;
                fs::rename(&entry.source, &entry.trash_destination)?;
            }
        }
        Ok(ContentGcReport {
            schema: "dusklight-content-gc/v1".into(),
            dry_run,
            store_root,
            trash_root: resolved_trash,
            reachable: present_reachable,
            referenced_missing,
            moved: if dry_run { 0 } else { unreachable.len() },
            reclaimed_bytes,
            unreachable,
        })
    }

    fn relative_blob_path(&self, digest: Digest) -> PathBuf {
        let digest = digest.to_string();
        PathBuf::from("blobs")
            .join("sha256")
            .join(&digest[..2])
            .join(&digest[2..])
    }

    fn reference(&self, kind: ContentKind, sha256: Digest, size: u64) -> ContentBlob {
        ContentBlob {
            schema: CONTENT_BLOB_SCHEMA_V1.into(),
            kind,
            sha256,
            size,
            media_type: kind.media_type().into(),
            relative_path: self.relative_blob_path(sha256),
        }
    }
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn resolve_path_even_if_missing(path: &Path) -> Result<PathBuf, ContentStoreError> {
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
        let name = existing
            .file_name()
            .ok_or(ContentStoreError::UnsafeTrashRoot)?;
        missing.push(name.to_os_string());
        existing = existing
            .parent()
            .ok_or(ContentStoreError::UnsafeTrashRoot)?;
    }
    let mut resolved = fs::canonicalize(existing)?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

fn install_bytes(destination: &Path, bytes: &[u8]) -> Result<(), ContentStoreError> {
    if destination.exists() {
        return verify_file(
            destination,
            Digest(Sha256::digest(bytes).into()),
            bytes.len() as u64,
        );
    }
    fs::create_dir_all(destination.parent().expect("blob path has a parent"))?;
    let counter = TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = destination.with_extension(format!("tmp-{}-{counter}", std::process::id()));
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    output.write_all(bytes)?;
    output.sync_all()?;
    drop(output);
    install_temporary(
        &temporary,
        destination,
        Digest(Sha256::digest(bytes).into()),
        bytes.len() as u64,
    )
}

fn install_temporary(
    temporary: &Path,
    destination: &Path,
    digest: Digest,
    size: u64,
) -> Result<(), ContentStoreError> {
    fs::create_dir_all(destination.parent().expect("blob path has a parent"))?;
    match fs::hard_link(temporary, destination) {
        Ok(()) => {
            fs::remove_file(temporary)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let result = verify_file(destination, digest, size);
            let _ = fs::remove_file(temporary);
            result
        }
        Err(error) => {
            let _ = fs::remove_file(temporary);
            Err(error.into())
        }
    }
}

fn verify_file(path: &Path, expected: Digest, size: u64) -> Result<(), ContentStoreError> {
    let metadata = fs::metadata(path)?;
    if metadata.len() != size {
        return Err(ContentStoreError::SizeMismatch {
            expected: size,
            observed: metadata.len(),
        });
    }
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let observed = Digest(hasher.finalize().into());
    if observed != expected {
        return Err(ContentStoreError::HashMismatch { expected, observed });
    }
    Ok(())
}

#[derive(Debug)]
pub enum ContentStoreError {
    Io(std::io::Error),
    NotInitialized(PathBuf),
    TooLarge(u64),
    SourceChanged { expected: u64, observed: u64 },
    InvalidReference,
    SizeMismatch { expected: u64, observed: u64 },
    HashMismatch { expected: Digest, observed: Digest },
    InvalidBlobPath(PathBuf),
    UnsafeTrashRoot,
    TrashDestinationExists(PathBuf),
}

impl fmt::Display for ContentStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "content store I/O failed: {error}"),
            Self::NotInitialized(path) => write!(
                formatter,
                "directory is not an initialized content store: {}",
                path.display()
            ),
            Self::TooLarge(size) => write!(formatter, "content blob size {size} exceeds the bound"),
            Self::SourceChanged { expected, observed } => write!(
                formatter,
                "content source changed while hashing: expected {expected} bytes, read {observed}"
            ),
            Self::InvalidReference => formatter.write_str("invalid content blob reference"),
            Self::SizeMismatch { expected, observed } => write!(
                formatter,
                "content blob size mismatch: expected {expected}, observed {observed}"
            ),
            Self::HashMismatch { expected, observed } => write!(
                formatter,
                "content blob SHA-256 mismatch: expected {expected}, observed {observed}"
            ),
            Self::InvalidBlobPath(path) => write!(
                formatter,
                "unexpected entry in content blob tree: {}",
                path.display()
            ),
            Self::UnsafeTrashRoot => formatter
                .write_str("content trash root must be explicit and outside the live blob tree"),
            Self::TrashDestinationExists(path) => write!(
                formatter,
                "content trash destination already exists: {}",
                path.display()
            ),
        }
    }
}

impl Error for ContentStoreError {}

impl From<std::io::Error> for ContentStoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn every_large_artifact_kind_is_deduplicated_and_verified() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("huntctl-content-store-{nonce}"));
        let store = ContentStore::initialize(&root).unwrap();
        let bytes = b"immutable artifact";
        for kind in [
            ContentKind::InputTape,
            ContentKind::GameplayTrace,
            ContentKind::TransitionCorpus,
            ContentKind::TransitionEvidence,
            ContentKind::EpisodeManifest,
            ContentKind::ActorProfileCatalog,
            ContentKind::WorldContext,
            ContentKind::WorldInventory,
            ContentKind::WorldSpatialIndex,
            ContentKind::NativeGeometryView,
            ContentKind::Screenshot,
            ContentKind::Model,
            ContentKind::DatasetManifest,
            ContentKind::CrashArtifact,
        ] {
            let first = store.put_bytes(bytes, kind).unwrap();
            let second = store.put_bytes(bytes, kind).unwrap();
            assert_eq!(first.sha256, second.sha256);
            assert_eq!(first.relative_path, second.relative_path);
            store.verify(&first).unwrap();
        }
        fs::write(
            store.blob_path(Digest(Sha256::digest(bytes).into())),
            b"tampered",
        )
        .unwrap();
        let reference = store.put_bytes(b"different", ContentKind::Model).unwrap();
        assert!(store.verify(&reference).is_ok());
        assert!(
            store
                .verify(&ContentBlob {
                    sha256: Digest(Sha256::digest(bytes).into()),
                    size: bytes.len() as u64,
                    kind: ContentKind::Model,
                    media_type: ContentKind::Model.media_type().into(),
                    schema: CONTENT_BLOB_SCHEMA_V1.into(),
                    relative_path: store.relative_blob_path(Digest(Sha256::digest(bytes).into())),
                })
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn garbage_collection_is_dry_run_by_default_and_moves_to_recoverable_trash() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("huntctl-content-gc-{nonce}"));
        let trash = std::env::temp_dir().join(format!("huntctl-content-trash-{nonce}"));
        let store = ContentStore::initialize(&root).unwrap();
        let kept = store.put_bytes(b"kept", ContentKind::Model).unwrap();
        let orphan = store.put_bytes(b"orphan", ContentKind::Screenshot).unwrap();
        let missing = Digest([9; 32]);
        let referenced = BTreeSet::from([kept.sha256, missing]);

        let preview = store.garbage_collect(&referenced, &trash, true).unwrap();
        assert_eq!(preview.reachable, vec![kept.sha256]);
        assert_eq!(preview.referenced_missing, vec![missing]);
        assert_eq!(preview.unreachable.len(), 1);
        assert_eq!(preview.unreachable[0].sha256, orphan.sha256);
        assert_eq!(preview.moved, 0);
        assert!(store.blob_path(orphan.sha256).exists());

        let applied = store.garbage_collect(&referenced, &trash, false).unwrap();
        assert_eq!(applied.moved, 1);
        assert!(!store.blob_path(orphan.sha256).exists());
        assert!(applied.unreachable[0].trash_destination.exists());
        assert!(store.blob_path(kept.sha256).exists());
        assert!(
            store
                .garbage_collect(&referenced, &root.join("blobs"), true)
                .is_err()
        );

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(trash).unwrap();
    }
}
