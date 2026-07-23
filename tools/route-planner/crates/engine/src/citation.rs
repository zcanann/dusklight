//! Digest-bound citations attached to composed-catalog evidence records.

use crate::artifact::Digest;
use crate::refinement::ComposedPlannerCatalog;
use crate::{
    PlannerContractError, canonical_json, require_canonical_json_bytes, validate_label,
    validate_stable_id,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub const EVIDENCE_CITATION_INDEX_SCHEMA: &str =
    "dusklight.route-planner.evidence-citation-index/v1";
const MAX_CITATIONS: usize = 65_536;
const MAX_URL_BYTES: usize = 2_048;
const MAX_PATH_BYTES: usize = 1_024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceCitationIndex {
    pub schema: String,
    pub fact_catalog_sha256: Digest,
    pub mechanics_catalog_sha256: Digest,
    pub citations: Vec<EvidenceCitation>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceCitation {
    pub id: String,
    pub evidence_record_id: String,
    pub kind: CitationKind,
    pub label: String,
    pub locator: CitationLocator,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CitationKind {
    Source,
    Extraction,
    Trace,
    Video,
    Community,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CitationLocator {
    Artifact {
        sha256: Digest,
    },
    Repository {
        repository: String,
        revision: String,
        path: String,
        line_start: Option<u32>,
        line_end: Option<u32>,
        source_sha256: Option<Digest>,
    },
    Url {
        url: String,
        archived_sha256: Option<Digest>,
    },
}

impl EvidenceCitationIndex {
    pub fn validate(&self, catalog: &ComposedPlannerCatalog) -> Result<(), PlannerContractError> {
        catalog.validate()?;
        if self.schema != EVIDENCE_CITATION_INDEX_SCHEMA {
            return Err(PlannerContractError::new(
                "evidence_citation_index.schema",
                "is unsupported",
            ));
        }
        if self.fact_catalog_sha256 != catalog.facts.digest()? {
            return Err(PlannerContractError::new(
                "evidence_citation_index.fact_catalog_sha256",
                "does not match the composed fact catalog",
            ));
        }
        if self.mechanics_catalog_sha256 != catalog.mechanics.digest()? {
            return Err(PlannerContractError::new(
                "evidence_citation_index.mechanics_catalog_sha256",
                "does not match the composed mechanics catalog",
            ));
        }
        if self.citations.is_empty() || self.citations.len() > MAX_CITATIONS {
            return Err(PlannerContractError::new(
                "evidence_citation_index.citations",
                format!("must contain between 1 and {MAX_CITATIONS} citations"),
            ));
        }
        if self
            .citations
            .windows(2)
            .any(|pair| pair[0].id >= pair[1].id)
        {
            return Err(PlannerContractError::new(
                "evidence_citation_index.citations",
                "must be unique and sorted by citation ID",
            ));
        }
        let evidence_ids = catalog_evidence_record_ids(catalog)?;
        for citation in &self.citations {
            citation.validate()?;
            if !evidence_ids.contains(citation.evidence_record_id.as_str()) {
                return Err(PlannerContractError::new(
                    "evidence_citation_index.citations.evidence_record_id",
                    format!(
                        "{} does not occur in the digest-bound composed catalog",
                        citation.evidence_record_id
                    ),
                ));
            }
        }
        Ok(())
    }

    pub fn canonical_bytes(
        &self,
        catalog: &ComposedPlannerCatalog,
    ) -> Result<Vec<u8>, PlannerContractError> {
        self.validate(catalog)?;
        canonical_json(self)
    }

    pub fn digest(&self, catalog: &ComposedPlannerCatalog) -> Result<Digest, PlannerContractError> {
        Ok(Digest(
            Sha256::digest(self.canonical_bytes(catalog)?).into(),
        ))
    }

    pub fn decode_canonical(
        bytes: &[u8],
        catalog: &ComposedPlannerCatalog,
    ) -> Result<Self, PlannerContractError> {
        let index: Self = serde_json::from_slice(bytes)?;
        index.validate(catalog)?;
        require_canonical_json_bytes(
            "evidence_citation_index",
            bytes,
            &index.canonical_bytes(catalog)?,
        )?;
        Ok(index)
    }
}

impl EvidenceCitation {
    fn validate(&self) -> Result<(), PlannerContractError> {
        validate_stable_id("citation.id", &self.id)?;
        validate_stable_id("citation.evidence_record_id", &self.evidence_record_id)?;
        validate_label("citation.label", &self.label)?;
        self.locator.validate()
    }
}

impl CitationLocator {
    fn validate(&self) -> Result<(), PlannerContractError> {
        match self {
            Self::Artifact { sha256 } => require_digest("citation.locator.sha256", *sha256),
            Self::Repository {
                repository,
                revision,
                path,
                line_start,
                line_end,
                source_sha256,
            } => {
                validate_label("citation.locator.repository", repository)?;
                validate_label("citation.locator.revision", revision)?;
                validate_relative_path(path)?;
                if line_start.is_some() != line_end.is_some()
                    || line_start
                        .zip(*line_end)
                        .is_some_and(|(start, end)| start == 0 || end == 0 || start > end)
                {
                    return Err(PlannerContractError::new(
                        "citation.locator.lines",
                        "must be absent together or a nonzero inclusive ordered range",
                    ));
                }
                if let Some(digest) = source_sha256 {
                    require_digest("citation.locator.source_sha256", *digest)?;
                }
                Ok(())
            }
            Self::Url {
                url,
                archived_sha256,
            } => {
                if url.len() > MAX_URL_BYTES
                    || !(url.starts_with("https://") || url.starts_with("http://"))
                    || url.chars().any(char::is_control)
                    || url.bytes().any(|byte| byte.is_ascii_whitespace())
                {
                    return Err(PlannerContractError::new(
                        "citation.locator.url",
                        "must be an HTTP(S) URL of at most 2048 bytes without whitespace",
                    ));
                }
                if let Some(digest) = archived_sha256 {
                    require_digest("citation.locator.archived_sha256", *digest)?;
                }
                Ok(())
            }
        }
    }
}

fn validate_relative_path(path: &str) -> Result<(), PlannerContractError> {
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || path.starts_with('/')
        || path.starts_with('\\')
        || path
            .split(['/', '\\'])
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
        || path.chars().any(char::is_control)
    {
        return Err(PlannerContractError::new(
            "citation.locator.path",
            "must be a bounded normalized relative path",
        ));
    }
    Ok(())
}

fn catalog_evidence_record_ids(
    catalog: &ComposedPlannerCatalog,
) -> Result<BTreeSet<String>, PlannerContractError> {
    let mut ids = BTreeSet::new();
    collect_evidence_record_ids(&serde_json::to_value(&catalog.facts)?, &mut ids);
    collect_evidence_record_ids(&serde_json::to_value(&catalog.mechanics)?, &mut ids);
    Ok(ids)
}

fn collect_evidence_record_ids(value: &Value, ids: &mut BTreeSet<String>) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_evidence_record_ids(value, ids);
            }
        }
        Value::Object(object) => {
            if object.contains_key("truth")
                && let Some(Value::Array(records)) = object.get("records")
            {
                for record in records {
                    if let Some(id) = record.get("id").and_then(Value::as_str) {
                        ids.insert(id.to_owned());
                    }
                }
            }
            for value in object.values() {
                collect_evidence_record_ids(value, ids);
            }
        }
        _ => {}
    }
}

fn require_digest(field: &str, digest: Digest) -> Result<(), PlannerContractError> {
    if digest == Digest::ZERO {
        return Err(PlannerContractError::new(field, "must be nonzero"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin_refinement::{GZ2E01_ORDINARY_MOVEMENT_PACK_ID, bundled_refinement_pack};
    use crate::logic::{FACT_CATALOG_SCHEMA, FactCatalog};
    use crate::transition::{MECHANICS_CATALOG_SCHEMA, MechanicsCatalog};

    fn catalog() -> ComposedPlannerCatalog {
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
        };
        let mechanics = MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions: Vec::new(),
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
        ComposedPlannerCatalog::compose(
            &facts,
            &mechanics,
            &[bundled_refinement_pack(GZ2E01_ORDINARY_MOVEMENT_PACK_ID).unwrap()],
        )
        .unwrap()
    }

    #[test]
    fn citation_index_is_catalog_bound_and_accepts_every_locator_family() {
        let catalog = catalog();
        let index = EvidenceCitationIndex {
            schema: EVIDENCE_CITATION_INDEX_SCHEMA.into(),
            fact_catalog_sha256: catalog.facts.digest().unwrap(),
            mechanics_catalog_sha256: catalog.mechanics.digest().unwrap(),
            citations: vec![
                EvidenceCitation {
                    id: "citation.extraction".into(),
                    evidence_record_id: "builtin.gz2e01.ordinary-movement".into(),
                    kind: CitationKind::Extraction,
                    label: "Canonical extracted evidence".into(),
                    locator: CitationLocator::Artifact {
                        sha256: Digest([1; 32]),
                    },
                },
                EvidenceCitation {
                    id: "citation.source".into(),
                    evidence_record_id: "builtin.gz2e01.ordinary-movement".into(),
                    kind: CitationKind::Source,
                    label: "Audited source lines".into(),
                    locator: CitationLocator::Repository {
                        repository: "dusklight".into(),
                        revision: "deadbeef".into(),
                        path: "src/example.cpp".into(),
                        line_start: Some(10),
                        line_end: Some(12),
                        source_sha256: Some(Digest([2; 32])),
                    },
                },
                EvidenceCitation {
                    id: "citation.video".into(),
                    evidence_record_id: "builtin.gz2e01.ordinary-movement".into(),
                    kind: CitationKind::Video,
                    label: "Witness video".into(),
                    locator: CitationLocator::Url {
                        url: "https://example.invalid/video".into(),
                        archived_sha256: Some(Digest([3; 32])),
                    },
                },
            ],
        };
        let bytes = index.canonical_bytes(&catalog).unwrap();
        assert_eq!(
            EvidenceCitationIndex::decode_canonical(&bytes, &catalog).unwrap(),
            index
        );

        let mut drifted = index.clone();
        drifted.mechanics_catalog_sha256 = Digest([9; 32]);
        assert!(drifted.validate(&catalog).is_err());
        let mut dangling = index;
        dangling.citations[0].evidence_record_id = "source.missing".into();
        assert!(dangling.validate(&catalog).is_err());
    }
}
