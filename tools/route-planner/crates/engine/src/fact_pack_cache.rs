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
    use crate::identity::{
        ContentFingerprint, ContentIdentity, ContextSelector, GamePlatform, GameRegion,
        RuntimeConfiguration,
    };
    use crate::logic::{
        ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA,
        FactCatalog, PredicateExpression, RuleEvidence, TruthStatus, ValueReference,
    };
    use crate::refinement::{COMPOSED_CATALOG_SCHEMA, ComposedPlannerCatalog};
    use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
    use crate::solver::{ForwardSolver, SearchStatus, SolverOptions};
    use crate::state::{
        BackingAttachment, EXECUTION_ENVIRONMENT_SCHEMA, ExecutionEnvironment, PlayerForm,
        PlayerState, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation,
        StateValue,
    };
    use crate::transition::{
        ActivationContract, CandidateTransition, MECHANICS_CATALOG_SCHEMA, MechanicsCatalog,
        StateOperation, TransitionKind,
    };
    use crate::{execution::PlannerExecutionState, identity::RUNTIME_CONFIGURATION_SCHEMA};
    use sha2::{Digest as _, Sha256};
    use std::collections::BTreeMap;
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

    #[test]
    fn cache_never_reuses_a_pack_across_input_schema_or_extractor_drift() {
        let fixture = Fixture::new();
        let payload = b"derived facts\n";
        let baseline = manifest(payload);
        let baseline_receipt = store_fact_pack(&fixture.0, &baseline, payload).unwrap();

        let mut changed_input = baseline.clone();
        changed_input.sources[0].sha256 = Digest([7; 32]);
        changed_input.content.fingerprint.resource_manifest_sha256 = Digest([8; 32]);
        changed_input.validate().unwrap();
        let changed_input_receipt = store_fact_pack(&fixture.0, &changed_input, payload).unwrap();
        assert!(!changed_input_receipt.reused);
        assert_ne!(
            changed_input_receipt.manifest_sha256,
            baseline_receipt.manifest_sha256
        );
        assert_ne!(
            changed_input_receipt.manifest_relative_path,
            baseline_receipt.manifest_relative_path
        );

        let mut changed_extractor = baseline.clone();
        changed_extractor.extractor.version = "2".into();
        changed_extractor.validate().unwrap();
        let changed_extractor_receipt =
            store_fact_pack(&fixture.0, &changed_extractor, payload).unwrap();
        assert!(!changed_extractor_receipt.reused);
        assert_ne!(
            changed_extractor_receipt.manifest_sha256,
            baseline_receipt.manifest_sha256
        );
        assert_ne!(
            changed_extractor_receipt.manifest_sha256,
            changed_input_receipt.manifest_sha256
        );

        let mut unsupported_schema = baseline.clone();
        unsupported_schema.schema = "dusklight.route-planner.fact-pack/v2".into();
        let error = store_fact_pack(&fixture.0, &unsupported_schema, payload).unwrap_err();
        assert_eq!(error.field(), "schema");
        assert!(
            FactPackManifest::decode_canonical(&serde_json::to_vec(&unsupported_schema).unwrap())
                .is_err()
        );

        let baseline_again = store_fact_pack(&fixture.0, &baseline, payload).unwrap();
        assert!(baseline_again.reused);
        assert_eq!(
            baseline_again.manifest_sha256,
            baseline_receipt.manifest_sha256
        );
        assert_eq!(
            load_fact_pack(&fixture.0, baseline_receipt.manifest_sha256)
                .unwrap()
                .manifest,
            baseline
        );
        assert_eq!(
            load_fact_pack(&fixture.0, changed_input_receipt.manifest_sha256)
                .unwrap()
                .manifest,
            changed_input
        );
        assert_eq!(
            load_fact_pack(&fixture.0, changed_extractor_receipt.manifest_sha256)
                .unwrap()
                .manifest,
            changed_extractor
        );
    }

    #[test]
    fn cached_catalog_replays_the_same_query_after_fixture_orig_is_removed() {
        let fixture = Fixture::new();
        let orig = fixture.0.join("orig");
        fs::create_dir_all(&orig).unwrap();
        let source_bytes = b"fixture retail input bytes\n";
        fs::write(orig.join("main.dol"), source_bytes).unwrap();
        let source_sha256 = Digest(Sha256::digest(source_bytes).into());

        let content = manifest(b"placeholder").content;
        let runtime = RuntimeConfiguration::new(&content, "en", BTreeMap::new()).unwrap();
        let start = StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.cache-replay-start".into(),
            sequence: 1,
            environment: ExecutionEnvironment {
                schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
                runtime_configuration: RuntimeConfiguration {
                    schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
                    ..runtime.clone()
                },
                active_runtime_file: RuntimeFile {
                    id: "file-0".into(),
                    origin: RuntimeFileOrigin::TitleFile0,
                    backing: BackingAttachment::MemoryOnly,
                    allowed_serialization_targets: Vec::new(),
                    lifecycle: RuntimeFileLifecycle::Active,
                },
                inactive_runtime_files: Vec::new(),
                physical_slots: Vec::new(),
                physical_slot_observations: Vec::new(),
                execution_context: crate::state::ExecutionContext::World,
                location: SceneLocation {
                    stage: "STAGE_A".into(),
                    room: 0,
                    layer: 0,
                    spawn: 0,
                },
                player: PlayerState {
                    form: PlayerForm::Human,
                    mount: None,
                    position: [0.0; 3],
                    attention_position: None,
                    rotation: [0; 3],
                    has_control: Some(true),
                    action: "idle".into(),
                },
                components: Vec::new(),
                static_world_objects: Vec::new(),
                spatial_volumes: Vec::new(),
                spatial_connections: Vec::new(),
                spatial_planes: Vec::new(),
                persisted_object_controls: Vec::new(),
                live_world_objects: Vec::new(),
            },
            semantic_observations: Vec::new(),
        };
        start.validate().unwrap();
        let scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: runtime.exact_context().unwrap(),
            }],
        };
        let stage_is = |stage: &str| PredicateExpression::Compare {
            left: ValueReference::LocationStage,
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Text(stage.into()),
            },
        };
        let evidence = RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![EvidenceRecord {
                id: "fixture.orig-main-dol".into(),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(source_sha256),
                note: "Fixture source digest retained by the derived pack.".into(),
            }],
        };
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
        };
        let mechanics = MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions: vec![CandidateTransition {
                id: "transition.cache-replay-a-to-b".into(),
                label: "Replay cached transition from A to B".into(),
                scope,
                transition_kind: TransitionKind::Other,
                approach_id: "approach.cache-replay".into(),
                activation: ActivationContract {
                    hard_guards: stage_is("STAGE_A"),
                    physical_obligation_ids: Vec::new(),
                    effects: vec![StateOperation::SetLocation {
                        location: SceneLocation {
                            stage: "STAGE_B".into(),
                            room: 0,
                            layer: 0,
                            spawn: 0,
                        },
                    }],
                    unknown_requirements: Vec::new(),
                },
                evidence,
            }],
            obligations: Vec::new(),
            writers: Vec::new(),
            gates: Vec::new(),
            readers: Vec::new(),
            reconstruction_rules: Vec::new(),
            obstructions: Vec::new(),
            resolvers: Vec::new(),
            techniques: Vec::new(),
            microtraces: Vec::new(),
            goals: Vec::new(),
        };
        let catalog = ComposedPlannerCatalog::compose(&facts, &mechanics, &[]).unwrap();
        let payload = catalog.canonical_bytes().unwrap();
        let manifest = FactPackManifest::build(
            "fixture.query-replay",
            content,
            ExtractorIdentity {
                name: "fixture-query-extractor".into(),
                version: "1".into(),
                executable_sha256: Digest([4; 32]),
                schema_sha256: Digest([5; 32]),
            },
            vec![FactPackSource {
                kind: SourceArtifactKind::Executable,
                id: "orig/main-dol".into(),
                sha256: source_sha256,
            }],
            vec![FactPackCoverage {
                domain: CoverageDomain::Topology,
                scope: "fixture-query".into(),
                status: CoverageStatus::Complete,
                detail: "The fixture transition is fully represented.".into(),
            }],
            COMPOSED_CATALOG_SCHEMA,
            Digest(Sha256::digest(&payload).into()),
        )
        .unwrap();
        let receipt = store_fact_pack(&fixture.0.join("cache"), &manifest, &payload).unwrap();
        let goal = stage_is("STAGE_B");
        let before = ForwardSolver::new(
            &catalog.facts,
            &catalog.mechanics,
            &[],
            SolverOptions::default(),
        )
        .unwrap()
        .solve(PlannerExecutionState::new(start.clone()).unwrap(), &goal)
        .unwrap();
        assert_eq!(before.status, SearchStatus::Reached);

        fs::remove_dir_all(&orig).unwrap();
        assert!(!orig.exists());
        let cached = load_fact_pack(&fixture.0.join("cache"), receipt.manifest_sha256).unwrap();
        assert_eq!(cached.manifest.sources[0].sha256, source_sha256);
        let replayed = ComposedPlannerCatalog::decode_canonical(&cached.payload_bytes).unwrap();
        let after = ForwardSolver::new(
            &replayed.facts,
            &replayed.mechanics,
            &[],
            SolverOptions::default(),
        )
        .unwrap()
        .solve(PlannerExecutionState::new(start).unwrap(), &goal)
        .unwrap();
        assert_eq!(after, before);
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
