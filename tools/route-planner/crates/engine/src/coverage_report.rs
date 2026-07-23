//! Conservative extraction-coverage aggregation across sealed fact-pack manifests.

use crate::PlannerContractError;
use crate::artifact::Digest;
use crate::fact_pack::{CoverageDomain, CoverageStatus, FactPackCoverage, FactPackManifest};
use crate::{canonical_json, require_canonical_json_bytes};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

pub const EXTRACTION_COVERAGE_REPORT_SCHEMA: &str =
    "dusklight.route-planner.extraction-coverage-report/v1";

const ALL_DOMAINS: [CoverageDomain; 9] = [
    CoverageDomain::Topology,
    CoverageDomain::ActorPlacements,
    CoverageDomain::Collision,
    CoverageDomain::HardGuards,
    CoverageDomain::StorageBindings,
    CoverageDomain::MessageFlows,
    CoverageDomain::ActorLifecycle,
    CoverageDomain::PhysicalFeasibility,
    CoverageDomain::Techniques,
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractionCoverageReport {
    pub schema: String,
    pub contexts: Vec<ContextExtractionCoverage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextExtractionCoverage {
    pub content_id: String,
    pub content_sha256: Digest,
    pub manifests: Vec<CoverageManifestIdentity>,
    pub domains: Vec<DomainExtractionCoverage>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageManifestIdentity {
    pub id: String,
    pub sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DomainExtractionCoverage {
    pub domain: CoverageDomain,
    pub reported: bool,
    pub complete_scope_count: usize,
    pub partial_scope_count: usize,
    pub unavailable_scope_count: usize,
    pub contributions: Vec<CoverageContribution>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageContribution {
    pub manifest_id: String,
    pub scope: String,
    pub status: CoverageStatus,
    pub detail: String,
}

impl ExtractionCoverageReport {
    pub fn build(manifests: &[FactPackManifest]) -> Result<Self, PlannerContractError> {
        if manifests.is_empty() {
            return Err(PlannerContractError::new(
                "extraction_coverage_report.manifests",
                "requires at least one fact-pack manifest",
            ));
        }
        let mut grouped = BTreeMap::<(String, Digest), Vec<&FactPackManifest>>::new();
        for manifest in manifests {
            manifest.validate()?;
            grouped
                .entry((manifest.content.id.clone(), manifest.content.digest()?))
                .or_default()
                .push(manifest);
        }
        let mut contexts = Vec::with_capacity(grouped.len());
        for ((content_id, content_sha256), mut group) in grouped {
            group.sort_by(|left, right| left.id.cmp(&right.id));
            if group.windows(2).any(|pair| pair[0].id == pair[1].id) {
                return Err(PlannerContractError::new(
                    "extraction_coverage_report.manifests",
                    format!("duplicates manifest ID {}", group[0].id),
                ));
            }
            let manifests = group
                .iter()
                .map(|manifest| {
                    Ok(CoverageManifestIdentity {
                        id: manifest.id.clone(),
                        sha256: manifest.digest()?,
                    })
                })
                .collect::<Result<Vec<_>, PlannerContractError>>()?;
            let domains = ALL_DOMAINS
                .into_iter()
                .map(|domain| domain_coverage(domain, &group))
                .collect();
            contexts.push(ContextExtractionCoverage {
                content_id,
                content_sha256,
                manifests,
                domains,
            });
        }
        Ok(Self {
            schema: EXTRACTION_COVERAGE_REPORT_SCHEMA.into(),
            contexts,
        })
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != EXTRACTION_COVERAGE_REPORT_SCHEMA || self.contexts.is_empty() {
            return Err(PlannerContractError::new(
                "extraction_coverage_report",
                "has unsupported schema or no contexts",
            ));
        }
        if self.contexts.windows(2).any(|pair| {
            (&pair[0].content_id, pair[0].content_sha256)
                >= (&pair[1].content_id, pair[1].content_sha256)
        }) {
            return Err(PlannerContractError::new(
                "extraction_coverage_report.contexts",
                "must be unique and canonically sorted",
            ));
        }
        for context in &self.contexts {
            if context.content_sha256 == Digest::ZERO
                || context.manifests.is_empty()
                || context.manifests.windows(2).any(|pair| pair[0] >= pair[1])
                || context.domains.len() != ALL_DOMAINS.len()
            {
                return Err(PlannerContractError::new(
                    "extraction_coverage_report.contexts",
                    "contains invalid identity, manifest ordering, or domain census",
                ));
            }
            for (expected, domain) in ALL_DOMAINS.iter().zip(&context.domains) {
                if expected != &domain.domain
                    || domain.reported != !domain.contributions.is_empty()
                    || domain.complete_scope_count
                        + domain.partial_scope_count
                        + domain.unavailable_scope_count
                        != domain.contributions.len()
                    || domain
                        .contributions
                        .windows(2)
                        .any(|pair| pair[0] >= pair[1])
                {
                    return Err(PlannerContractError::new(
                        "extraction_coverage_report.domains",
                        "domain order, counts, reported state, or contributions drifted",
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let report: Self = serde_json::from_slice(bytes)?;
        report.validate()?;
        require_canonical_json_bytes(
            "extraction_coverage_report",
            bytes,
            &report.canonical_bytes()?,
        )?;
        Ok(report)
    }
}

fn domain_coverage(
    domain: CoverageDomain,
    manifests: &[&FactPackManifest],
) -> DomainExtractionCoverage {
    let mut contributions = manifests
        .iter()
        .flat_map(|manifest| {
            manifest
                .coverage
                .iter()
                .filter(move |coverage| coverage.domain == domain)
                .map(move |coverage| contribution(manifest, coverage))
        })
        .collect::<Vec<_>>();
    contributions.sort();
    let count = |status| {
        contributions
            .iter()
            .filter(|row| row.status == status)
            .count()
    };
    DomainExtractionCoverage {
        domain,
        reported: !contributions.is_empty(),
        complete_scope_count: count(CoverageStatus::Complete),
        partial_scope_count: count(CoverageStatus::Partial),
        unavailable_scope_count: count(CoverageStatus::Unavailable),
        contributions,
    }
}

fn contribution(manifest: &FactPackManifest, coverage: &FactPackCoverage) -> CoverageContribution {
    CoverageContribution {
        manifest_id: manifest.id.clone(),
        scope: coverage.scope.clone(),
        status: coverage.status,
        detail: coverage.detail.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fact_pack::{ExtractorIdentity, FactPackSource, SourceArtifactKind};
    use crate::identity::{ContentFingerprint, ContentIdentity, GamePlatform, GameRegion};

    fn manifest(id: &str, coverage: Vec<FactPackCoverage>) -> FactPackManifest {
        FactPackManifest::build(
            id,
            ContentIdentity::new(
                "gcn-us-1.0-test",
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
            .unwrap(),
            ExtractorIdentity {
                name: "coverage-test".into(),
                version: "1".into(),
                executable_sha256: Digest([4; 32]),
                schema_sha256: Digest([5; 32]),
            },
            vec![FactPackSource {
                kind: SourceArtifactKind::Executable,
                id: format!("source.{id}"),
                sha256: Digest([6; 32]),
            }],
            coverage,
            "test.payload-v1",
            Digest([7; 32]),
        )
        .unwrap()
    }

    #[test]
    fn report_keeps_required_domains_and_unreported_coverage_separate() {
        let report = ExtractionCoverageReport::build(&[
            manifest(
                "pack.guards",
                vec![FactPackCoverage {
                    domain: CoverageDomain::HardGuards,
                    scope: "door-family".into(),
                    status: CoverageStatus::Partial,
                    detail: "One audited family".into(),
                }],
            ),
            manifest(
                "pack.topology",
                vec![FactPackCoverage {
                    domain: CoverageDomain::Topology,
                    scope: "world".into(),
                    status: CoverageStatus::Complete,
                    detail: "All decoded exits".into(),
                }],
            ),
        ])
        .unwrap();
        assert_eq!(report.contexts.len(), 1);
        let context = &report.contexts[0];
        assert_eq!(context.domains.len(), ALL_DOMAINS.len());
        assert!(context.domains[0].reported);
        assert_eq!(context.domains[3].partial_scope_count, 1);
        assert!(!context.domains[4].reported);
        assert!(!context.domains[6].reported);
        assert!(!context.domains[7].reported);
        let bytes = report.canonical_bytes().unwrap();
        assert_eq!(
            ExtractionCoverageReport::decode_canonical(&bytes).unwrap(),
            report
        );
    }
}
