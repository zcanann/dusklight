//! Immutable content-addressed storage for redistributable derived fact packs.

use crate::artifact::Digest;
use crate::fact_pack::FactPackManifest;
use crate::{PlannerContractError, canonical_json};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const FACT_PACK_CACHE_RECEIPT_SCHEMA: &str =
    "dusklight.route-planner.fact-pack-cache-receipt/v1";

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactPackCacheReceipt {
    pub schema: String,
    pub manifest_sha256: Digest,
    pub payload_sha256: Digest,
    pub manifest_relative_path: String,
    pub payload_relative_path: String,
    pub reused: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedFactPack {
    pub manifest: FactPackManifest,
    pub manifest_bytes: Vec<u8>,
    pub payload_bytes: Vec<u8>,
}

pub fn store_fact_pack(
    cache_root: &Path,
    manifest: &FactPackManifest,
    payload: &[u8],
) -> Result<FactPackCacheReceipt, PlannerContractError> {
    manifest.validate()?;
    manifest.verify_payload(payload)?;
    let manifest_bytes = manifest.canonical_bytes()?;
    let manifest_sha256 = manifest.digest()?;
    let (entry_relative, entry) = entry_path(cache_root, manifest_sha256);
    reject_symlink_ancestors(cache_root, &entry)?;
    fs::create_dir_all(&entry).map_err(|error| io_error("fact_pack_cache.directory", error))?;
    reject_symlink(&entry, "fact_pack_cache.directory")?;
    let manifest_path = entry.join("manifest.json");
    let payload_path = entry.join("payload.json");
    let manifest_reused = install_immutable(&manifest_path, &manifest_bytes)?;
    let payload_reused = install_immutable(&payload_path, payload)?;
    let receipt = FactPackCacheReceipt {
        schema: FACT_PACK_CACHE_RECEIPT_SCHEMA.into(),
        manifest_sha256,
        payload_sha256: manifest.payload_sha256,
        manifest_relative_path: format!("{entry_relative}/manifest.json"),
        payload_relative_path: format!("{entry_relative}/payload.json"),
        reused: manifest_reused && payload_reused,
    };
    receipt.validate()?;
    Ok(receipt)
}

pub fn load_fact_pack(
    cache_root: &Path,
    manifest_sha256: Digest,
) -> Result<CachedFactPack, PlannerContractError> {
    if manifest_sha256 == Digest::ZERO {
        return Err(PlannerContractError::new(
            "fact_pack_cache.manifest_sha256",
            "must be nonzero",
        ));
    }
    let (_, entry) = entry_path(cache_root, manifest_sha256);
    reject_symlink_ancestors(cache_root, &entry)?;
    let manifest_path = entry.join("manifest.json");
    let payload_path = entry.join("payload.json");
    let manifest_bytes = read_regular_file(&manifest_path)?;
    let payload_bytes = read_regular_file(&payload_path)?;
    let manifest = FactPackManifest::decode_canonical(&manifest_bytes)?;
    if manifest.digest()? != manifest_sha256 {
        return Err(PlannerContractError::new(
            "fact_pack_cache.manifest",
            "digest does not match the requested cache key",
        ));
    }
    manifest.verify_payload(&payload_bytes)?;
    Ok(CachedFactPack {
        manifest,
        manifest_bytes,
        payload_bytes,
    })
}

impl FactPackCacheReceipt {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != FACT_PACK_CACHE_RECEIPT_SCHEMA
            || self.manifest_sha256 == Digest::ZERO
            || self.payload_sha256 == Digest::ZERO
        {
            return Err(PlannerContractError::new(
                "fact_pack_cache_receipt",
                "has an unsupported schema or zero digest",
            ));
        }
        let expected = cache_relative_path(self.manifest_sha256);
        if self.manifest_relative_path != format!("{expected}/manifest.json")
            || self.payload_relative_path != format!("{expected}/payload.json")
        {
            return Err(PlannerContractError::new(
                "fact_pack_cache_receipt",
                "paths do not derive from the manifest digest",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let receipt: Self = serde_json::from_slice(bytes)?;
        receipt.validate()?;
        if receipt.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "fact_pack_cache_receipt",
                "is not canonical JSON",
            ));
        }
        Ok(receipt)
    }
}

fn entry_path(cache_root: &Path, digest: Digest) -> (String, PathBuf) {
    let relative = cache_relative_path(digest);
    (relative.clone(), cache_root.join(relative))
}

fn cache_relative_path(digest: Digest) -> String {
    let digest = digest.to_string();
    format!("sha256/{}/{}", &digest[..2], digest)
}

fn install_immutable(path: &Path, bytes: &[u8]) -> Result<bool, PlannerContractError> {
    if path.exists() {
        if read_regular_file(path)? != bytes {
            return Err(PlannerContractError::new(
                "fact_pack_cache.entry",
                "an immutable cache key already contains different bytes",
            ));
        }
        return Ok(true);
    }
    let parent = path.parent().ok_or_else(|| {
        PlannerContractError::new("fact_pack_cache.entry", "has no parent directory")
    })?;
    let temporary = parent.join(format!(
        ".install-{}-{}",
        std::process::id(),
        NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| io_error("fact_pack_cache.temporary", error))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| io_error("fact_pack_cache.temporary", error))?;
        match fs::hard_link(&temporary, path) {
            Ok(()) => Ok(false),
            Err(_) if path.exists() => {
                if read_regular_file(path)? == bytes {
                    Ok(true)
                } else {
                    Err(PlannerContractError::new(
                        "fact_pack_cache.entry",
                        "a concurrent immutable install wrote different bytes",
                    ))
                }
            }
            Err(error) => Err(io_error("fact_pack_cache.install", error)),
        }
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn read_regular_file(path: &Path) -> Result<Vec<u8>, PlannerContractError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| io_error("fact_pack_cache.entry", error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PlannerContractError::new(
            "fact_pack_cache.entry",
            "must be a regular non-symlink file",
        ));
    }
    fs::read(path).map_err(|error| io_error("fact_pack_cache.entry", error))
}

fn reject_symlink(path: &Path, field: &str) -> Result<(), PlannerContractError> {
    if path.exists()
        && fs::symlink_metadata(path)
            .map_err(|error| io_error(field, error))?
            .file_type()
            .is_symlink()
    {
        return Err(PlannerContractError::new(field, "must not be a symlink"));
    }
    Ok(())
}

fn reject_symlink_ancestors(root: &Path, entry: &Path) -> Result<(), PlannerContractError> {
    reject_symlink(root, "fact_pack_cache.root")?;
    let relative = entry.strip_prefix(root).map_err(|_| {
        PlannerContractError::new("fact_pack_cache.entry", "escaped the cache root")
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        reject_symlink(&current, "fact_pack_cache.entry")?;
    }
    Ok(())
}

fn io_error(field: &str, error: std::io::Error) -> PlannerContractError {
    PlannerContractError::new(field, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fact_pack::{
        CoverageDomain, CoverageStatus, ExtractorIdentity, FactPackCoverage, FactPackSource,
        SourceArtifactKind,
    };
    use crate::identity::{ContentFingerprint, ContentIdentity, GamePlatform, GameRegion};
    use sha2::{Digest as _, Sha256};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "dusklight-route-planner-cache-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn manifest(payload: &[u8]) -> FactPackManifest {
        let content = ContentIdentity::new(
            "fixture",
            ContentFingerprint {
                platform: GamePlatform::GameCube,
                region: GameRegion::Usa,
                revision: "1.0".into(),
                product_id: "GZ2E01".into(),
                executable_sha256: Digest([1; 32]),
                game_data_sha256: Digest([2; 32]),
                resource_manifest_sha256: Digest([3; 32]),
            },
        )
        .unwrap();
        FactPackManifest::build(
            "fixture.pack",
            content,
            ExtractorIdentity {
                name: "fixture".into(),
                version: "1".into(),
                executable_sha256: Digest([4; 32]),
                schema_sha256: Digest([5; 32]),
            },
            vec![FactPackSource {
                kind: SourceArtifactKind::Executable,
                id: "fixture/source".into(),
                sha256: Digest([6; 32]),
            }],
            vec![FactPackCoverage {
                domain: CoverageDomain::Topology,
                scope: "fixture".into(),
                status: CoverageStatus::Partial,
                detail: "fixture coverage".into(),
            }],
            "fixture.payload/v1",
            Digest(Sha256::digest(payload).into()),
        )
        .unwrap()
    }

    #[test]
    fn cache_is_content_addressed_reusable_and_loadable_without_orig() {
        let fixture = Fixture::new();
        let payload = b"derived facts\n";
        let manifest = manifest(payload);
        let first = store_fact_pack(&fixture.0, &manifest, payload).unwrap();
        assert!(!first.reused);
        let second = store_fact_pack(&fixture.0, &manifest, payload).unwrap();
        assert!(second.reused);
        assert_eq!(first.manifest_sha256, second.manifest_sha256);
        assert_eq!(
            FactPackCacheReceipt::decode_canonical(&first.canonical_bytes().unwrap()).unwrap(),
            first
        );
        let loaded = load_fact_pack(&fixture.0, first.manifest_sha256).unwrap();
        assert_eq!(loaded.manifest, manifest);
        assert_eq!(loaded.payload_bytes, payload);
    }

    #[test]
    fn cache_rejects_payload_mismatch_and_tampering() {
        let fixture = Fixture::new();
        let payload = b"derived facts\n";
        let manifest = manifest(payload);
        assert!(store_fact_pack(&fixture.0, &manifest, b"wrong\n").is_err());
        let receipt = store_fact_pack(&fixture.0, &manifest, payload).unwrap();
        fs::write(
            fixture.0.join(&receipt.payload_relative_path),
            b"tampered\n",
        )
        .unwrap();
        assert!(load_fact_pack(&fixture.0, receipt.manifest_sha256).is_err());
        assert!(store_fact_pack(&fixture.0, &manifest, payload).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn cache_rejects_a_symlinked_content_directory() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        let cache = fixture.0.join("cache");
        let outside = fixture.0.join("outside");
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, cache.join("sha256")).unwrap();
        let payload = b"derived facts\n";
        assert!(store_fact_pack(&cache, &manifest(payload), payload).is_err());
    }
}
