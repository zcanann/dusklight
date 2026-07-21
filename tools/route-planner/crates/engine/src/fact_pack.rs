//! Deterministic manifests for generated, exact-content planner facts.

use crate::artifact::Digest;
use crate::identity::{ContentIdentity, require_digest};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub const FACT_PACK_SCHEMA: &str = "dusklight.route-planner.fact-pack/v1";
pub const MAX_FACT_PACK_SOURCES: usize = 16_384;
pub const MAX_FACT_PACK_COVERAGE: usize = 1_024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceArtifactKind {
    Executable,
    StageArchive,
    MessageArchive,
    ResourceArchive,
    SourceAudit,
    TraceEvidence,
    ExternalEvidence,
    WorldContext,
    WorldInventory,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactPackSource {
    pub kind: SourceArtifactKind,
    pub id: String,
    pub sha256: Digest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageDomain {
    Topology,
    ActorPlacements,
    Collision,
    HardGuards,
    StorageBindings,
    MessageFlows,
    ActorLifecycle,
    PhysicalFeasibility,
    Techniques,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Complete,
    Partial,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactPackCoverage {
    pub domain: CoverageDomain,
    pub scope: String,
    pub status: CoverageStatus,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractorIdentity {
    pub name: String,
    pub version: String,
    pub executable_sha256: Digest,
    pub schema_sha256: Digest,
}

/// The manifest names only derived payloads and source digests, never host paths
/// or original game bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactPackManifest {
    pub schema: String,
    pub id: String,
    pub content: ContentIdentity,
    pub extractor: ExtractorIdentity,
    pub sources: Vec<FactPackSource>,
    pub coverage: Vec<FactPackCoverage>,
    pub payload_schema: String,
    pub payload_sha256: Digest,
}

impl ExtractorIdentity {
    fn validate(&self) -> Result<(), PlannerContractError> {
        validate_stable_id("extractor.name", &self.name)?;
        validate_label("extractor.version", &self.version)?;
        require_digest("extractor.executable_sha256", self.executable_sha256)?;
        require_digest("extractor.schema_sha256", self.schema_sha256)
    }
}

impl FactPackManifest {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        id: impl Into<String>,
        content: ContentIdentity,
        extractor: ExtractorIdentity,
        mut sources: Vec<FactPackSource>,
        mut coverage: Vec<FactPackCoverage>,
        payload_schema: impl Into<String>,
        payload_sha256: Digest,
    ) -> Result<Self, PlannerContractError> {
        sources.sort();
        coverage.sort();
        let manifest = Self {
            schema: FACT_PACK_SCHEMA.into(),
            id: id.into(),
            content,
            extractor,
            sources,
            coverage,
            payload_schema: payload_schema.into(),
            payload_sha256,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != FACT_PACK_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        validate_stable_id("id", &self.id)?;
        self.content.validate()?;
        self.extractor.validate()?;
        validate_stable_id("payload_schema", &self.payload_schema)?;
        require_digest("payload_sha256", self.payload_sha256)?;
        validate_sources(&self.sources)?;
        validate_coverage(&self.coverage)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let manifest: Self = serde_json::from_slice(bytes)?;
        manifest.validate()?;
        if manifest.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "fact_pack",
                "is not canonical JSON",
            ));
        }
        Ok(manifest)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn verify_payload(&self, payload: &[u8]) -> Result<(), PlannerContractError> {
        let actual = Digest(Sha256::digest(payload).into());
        if actual != self.payload_sha256 {
            return Err(PlannerContractError::new(
                "payload_sha256",
                "does not match the supplied derived payload",
            ));
        }
        Ok(())
    }
}

fn validate_sources(sources: &[FactPackSource]) -> Result<(), PlannerContractError> {
    if sources.is_empty() || sources.len() > MAX_FACT_PACK_SOURCES {
        return Err(PlannerContractError::new(
            "sources",
            "must contain between 1 and 16384 records",
        ));
    }
    let mut unique = BTreeSet::new();
    let mut previous = None;
    for source in sources {
        validate_stable_id("sources.id", &source.id)?;
        require_digest("sources.sha256", source.sha256)?;
        if !unique.insert((source.kind, source.id.as_str()))
            || previous.is_some_and(|prior: &FactPackSource| prior >= source)
        {
            return Err(PlannerContractError::new(
                "sources",
                "must be unique and sorted canonically",
            ));
        }
        previous = Some(source);
    }
    Ok(())
}

fn validate_coverage(coverage: &[FactPackCoverage]) -> Result<(), PlannerContractError> {
    if coverage.is_empty() || coverage.len() > MAX_FACT_PACK_COVERAGE {
        return Err(PlannerContractError::new(
            "coverage",
            "must contain between 1 and 1024 records",
        ));
    }
    let mut unique = BTreeSet::new();
    let mut previous = None;
    for record in coverage {
        validate_stable_id("coverage.scope", &record.scope)?;
        validate_label("coverage.detail", &record.detail)?;
        if !unique.insert((record.domain, record.scope.as_str()))
            || previous.is_some_and(|prior: &FactPackCoverage| prior >= record)
        {
            return Err(PlannerContractError::new(
                "coverage",
                "must be unique and sorted canonically",
            ));
        }
        previous = Some(record);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{
        CONTENT_GROUP_SCHEMA, ConfigurationValue, ContentFingerprint, ContentGroup,
        ContextSelector, EQUIVALENCE_SET_SCHEMA, EquivalenceEvidence, EquivalenceEvidenceKind,
        EquivalenceSet, ExactContext, GamePlatform, GameRegion, RuntimeConfiguration,
    };
    use std::collections::BTreeMap;

    fn content() -> ContentIdentity {
        ContentIdentity::new(
            "gcn-us-1.0",
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
        .unwrap()
    }

    fn extractor() -> ExtractorIdentity {
        ExtractorIdentity {
            name: "huntctl-route-facts".into(),
            version: "0.1.0".into(),
            executable_sha256: Digest([4; 32]),
            schema_sha256: Digest([5; 32]),
        }
    }

    fn manifest(payload: &[u8]) -> FactPackManifest {
        FactPackManifest::build(
            "gcn-us-1.0.base",
            content(),
            extractor(),
            vec![
                FactPackSource {
                    kind: SourceArtifactKind::StageArchive,
                    id: "stage/f_sp103".into(),
                    sha256: Digest([7; 32]),
                },
                FactPackSource {
                    kind: SourceArtifactKind::Executable,
                    id: "main-dol".into(),
                    sha256: Digest([6; 32]),
                },
            ],
            vec![
                FactPackCoverage {
                    domain: CoverageDomain::PhysicalFeasibility,
                    scope: "world".into(),
                    status: CoverageStatus::Unavailable,
                    detail: "No geometric feasibility audit is included.".into(),
                },
                FactPackCoverage {
                    domain: CoverageDomain::Topology,
                    scope: "world".into(),
                    status: CoverageStatus::Partial,
                    detail: "SCLS records are extracted; actor exits remain unaudited.".into(),
                },
            ],
            "route-facts/v1",
            Digest(Sha256::digest(payload).into()),
        )
        .unwrap()
    }

    #[test]
    fn exact_content_rejects_label_override_and_noncanonical_json() {
        let content = content();
        assert_eq!(
            ContentIdentity::decode_canonical(&content.canonical_bytes().unwrap()).unwrap(),
            content
        );
        let mut detected = content.fingerprint.clone();
        detected.game_data_sha256 = Digest([9; 32]);
        assert_eq!(
            content.verify_detected(&detected).unwrap_err().field(),
            "fingerprint"
        );
        assert!(
            ContentIdentity::decode_canonical(&serde_json::to_vec_pretty(&content).unwrap())
                .is_err()
        );
    }

    #[test]
    fn language_is_runtime_state_not_content_identity() {
        let content = content();
        let mut settings = BTreeMap::new();
        settings.insert("subtitles".into(), ConfigurationValue::Boolean(true));
        let english = RuntimeConfiguration::new(&content, "en", settings.clone()).unwrap();
        let french = RuntimeConfiguration::new(&content, "fr", settings).unwrap();
        assert_eq!(english.content_sha256, french.content_sha256);
        assert_ne!(english.digest().unwrap(), french.digest().unwrap());
        assert_ne!(
            english.exact_context().unwrap(),
            french.exact_context().unwrap()
        );
    }

    #[test]
    fn generated_manifest_is_sorted_sealed_and_replayable() {
        let payload = b"derived facts only\n";
        let manifest = manifest(payload);
        assert_eq!(manifest.sources[0].id, "main-dol");
        assert_eq!(manifest.coverage[0].domain, CoverageDomain::Topology);
        manifest.verify_payload(payload).unwrap();
        assert!(manifest.verify_payload(b"stale").is_err());
        let bytes = manifest.canonical_bytes().unwrap();
        let decoded = FactPackManifest::decode_canonical(&bytes).unwrap();
        assert_eq!(decoded, manifest);
        assert_ne!(decoded.digest().unwrap(), Digest::ZERO);
    }

    #[test]
    fn universality_requires_sorted_exact_contexts_and_evidence() {
        let content = content();
        let english = RuntimeConfiguration::new(&content, "en", BTreeMap::new())
            .unwrap()
            .exact_context()
            .unwrap();
        let french = RuntimeConfiguration::new(&content, "fr", BTreeMap::new())
            .unwrap()
            .exact_context()
            .unwrap();
        let mut contexts = vec![english, french];
        contexts.sort();
        let set = EquivalenceSet {
            schema: EQUIVALENCE_SET_SCHEMA.into(),
            id: "pal.shared.cannon-base".into(),
            semantic_scope: "event.cannon.base".into(),
            contexts,
            evidence: vec![EquivalenceEvidence {
                kind: EquivalenceEvidenceKind::StaticDiff,
                source_id: "audit/pal-cannon".into(),
                source_sha256: Digest([8; 32]),
            }],
        };
        set.validate().unwrap();
        assert!(set.proves(&set.contexts[0]));

        let mut unsupported = set.clone();
        unsupported.contexts = vec![ExactContext {
            content_sha256: Digest([1; 32]),
            runtime_configuration_sha256: Digest([2; 32]),
        }];
        assert_eq!(unsupported.validate().unwrap_err().field(), "contexts");

        let mut unevidenced = set;
        unevidenced.evidence.clear();
        assert_eq!(unevidenced.validate().unwrap_err().field(), "evidence");
    }

    #[test]
    fn friendly_groups_and_selectors_never_imply_wildcards() {
        let mut members = vec![Digest([3; 32]), Digest([1; 32])];
        members.sort();
        let group = ContentGroup {
            schema: CONTENT_GROUP_SCHEMA.into(),
            id: "gcn".into(),
            members,
        };
        group.validate().unwrap();

        let exact = ContextSelector::Exact {
            context: ExactContext {
                content_sha256: Digest([1; 32]),
                runtime_configuration_sha256: Digest([2; 32]),
            },
        };
        exact.validate().unwrap();
        ContextSelector::Equivalent {
            equivalence_set_id: "pal.shared.cannon-base".into(),
        }
        .validate()
        .unwrap();

        let mut duplicate = group;
        duplicate.members = vec![Digest([1; 32]), Digest([1; 32])];
        assert_eq!(duplicate.validate().unwrap_err().field(), "members");
    }

    #[test]
    fn unknown_fields_and_zero_digests_fail_closed() {
        let manifest = manifest(b"facts");
        let mut value = serde_json::to_value(&manifest).unwrap();
        value["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<FactPackManifest>(value).is_err());

        let mut zero = manifest;
        zero.payload_sha256 = Digest::ZERO;
        assert_eq!(zero.validate().unwrap_err().field(), "payload_sha256");
    }
}
