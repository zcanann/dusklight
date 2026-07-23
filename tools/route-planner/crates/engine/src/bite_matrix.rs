//! Exact-context preservation matrix for standard selected-file BiTE.
//!
//! The matrix deliberately separates bytes restored from the selected file,
//! runtime metadata carried across the title/BiT lifetime cut, and state that
//! survives because it is outside that runtime lifetime. A community-witnessed
//! trick name never turns those distinct mechanisms into an arbitrary carry.

use crate::PlannerContractError;
use crate::artifact::Digest;
use crate::identity::{ContentIdentity, ContextSelector, ExactContext, RuntimeConfiguration};
use crate::logic::{ContextScope, EvidenceKind, EvidenceRecord, RuleEvidence, TruthStatus};
use crate::return_place::{GZ2E01_CONTENT_SHA256, GZ2E01_EN_RUNTIME_SHA256};
use crate::title_boundary::gz2e01_reset_to_opening_mechanics;
use crate::transition::{MechanicsCatalog, StateOperation};
use crate::{canonical_json, require_canonical_json_bytes, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

pub const BITE_PRESERVATION_MATRIX_SCHEMA: &str =
    "dusklight.route-planner.bite-preservation-matrix/v1";
pub const GZ2E01_STANDARD_BITE_MATRIX_ID: &str = "gz2e01.standard-selected-file-bite";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BiteVariant {
    StandardSelectedFileLoad,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BitePreservationDisposition {
    RestoredFromSelectedFile,
    CarriedFromBitRuntime,
    PreservedOutsideRuntimeLifetime,
    RemovedWithBitRuntime,
    UnchangedSealedBacking,
    RequiresCompatibility,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BitePreservationEntry {
    pub subject_id: String,
    pub disposition: BitePreservationDisposition,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BitePreservationMatrix {
    pub schema: String,
    pub id: String,
    pub scope: ContextScope,
    pub variant: BiteVariant,
    pub entries: Vec<BitePreservationEntry>,
    pub content_sha256: Digest,
}

impl BitePreservationMatrix {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != BITE_PRESERVATION_MATRIX_SCHEMA
            || self.id != GZ2E01_STANDARD_BITE_MATRIX_ID
            || self.variant != BiteVariant::StandardSelectedFileLoad
            || self.content_sha256 == Digest::ZERO
        {
            return Err(PlannerContractError::new(
                "bite_preservation_matrix",
                "has an unsupported schema, identity, variant, or zero content seal",
            ));
        }
        self.scope.validate("bite_preservation_matrix.scope")?;
        if self.scope != exact_gz2e01_scope() {
            return Err(PlannerContractError::new(
                "bite_preservation_matrix.scope",
                "must select only the exact GZ2E01/English context",
            ));
        }
        if self.entries.len() != expected_dispositions().len() {
            return Err(PlannerContractError::new(
                "bite_preservation_matrix.entries",
                "must contain the complete exact subject set",
            ));
        }
        let expected = expected_dispositions();
        let mut previous = None;
        for entry in &self.entries {
            validate_stable_id(
                "bite_preservation_matrix.entries.subject_id",
                &entry.subject_id,
            )?;
            entry
                .evidence
                .validate("bite_preservation_matrix.entries.evidence")?;
            if previous.is_some_and(|prior: &str| prior >= entry.subject_id.as_str()) {
                return Err(PlannerContractError::new(
                    "bite_preservation_matrix.entries",
                    "must be unique and sorted by subject ID",
                ));
            }
            if expected.get(entry.subject_id.as_str()) != Some(&entry.disposition) {
                return Err(PlannerContractError::new(
                    "bite_preservation_matrix.entries",
                    "contains an unsupported subject or disposition",
                ));
            }
            let should_be_unknown = entry.disposition == BitePreservationDisposition::Unknown;
            if should_be_unknown != (entry.evidence.truth == TruthStatus::Unknown) {
                return Err(PlannerContractError::new(
                    "bite_preservation_matrix.entries.evidence",
                    "unknown dispositions must remain unknown and all exact dispositions must be established",
                ));
            }
            if !should_be_unknown && entry.evidence.truth != TruthStatus::Established {
                return Err(PlannerContractError::new(
                    "bite_preservation_matrix.entries.evidence",
                    "non-unknown matrix dispositions require established evidence",
                ));
            }
            previous = Some(entry.subject_id.as_str());
        }
        if self.content_sha256 != self.identity()? {
            return Err(PlannerContractError::new(
                "bite_preservation_matrix.content_sha256",
                "does not reproduce the canonical matrix",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let matrix: Self = serde_json::from_slice(bytes)?;
        matrix.validate()?;
        require_canonical_json_bytes(
            "bite_preservation_matrix",
            bytes,
            &matrix.canonical_bytes()?,
        )?;
        Ok(matrix)
    }

    fn identity(&self) -> Result<Digest, PlannerContractError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.route-planner.bite-preservation-matrix/v1\0");
        hasher.update(canonical_json(&canonical)?);
        Ok(Digest(hasher.finalize().into()))
    }
}

/// Builds the standard selected-file BiTE matrix from the exact GZ2E01 title
/// mechanics. The selected image and explicit carry manifests are therefore
/// mechanically tied to the executable load/save contracts rather than copied
/// into a second handwritten component list.
pub fn gz2e01_bite_preservation_matrix(
    content: &ContentIdentity,
    runtime: &RuntimeConfiguration,
) -> Result<BitePreservationMatrix, PlannerContractError> {
    let mechanics = gz2e01_reset_to_opening_mechanics(content, runtime)?;
    let (restored, restored_evidence) = unique_save_manifest(&mechanics)?;
    let (carried, carried_evidence) = unique_load_carry_manifest(&mechanics)?;
    let community = community_evidence(TruthStatus::Established);
    let unknown_community = community_evidence(TruthStatus::Unknown);
    let mut entries = BTreeMap::new();
    for subject_id in restored {
        insert_entry(
            &mut entries,
            subject_id,
            BitePreservationDisposition::RestoredFromSelectedFile,
            restored_evidence.clone(),
        )?;
    }
    for subject_id in carried {
        insert_entry(
            &mut entries,
            subject_id,
            BitePreservationDisposition::CarriedFromBitRuntime,
            carried_evidence.clone(),
        )?;
    }
    for (subject_id, disposition, evidence) in [
        (
            "outside-runtime.session-components",
            BitePreservationDisposition::PreservedOutsideRuntimeLifetime,
            carried_evidence.clone(),
        ),
        (
            "physical-slot.unselected-images",
            BitePreservationDisposition::UnchangedSealedBacking,
            carried_evidence.clone(),
        ),
        (
            "source-runtime.omitted-components",
            BitePreservationDisposition::RemovedWithBitRuntime,
            carried_evidence.clone(),
        ),
        (
            "visible-link.items-equipment-and-progress",
            BitePreservationDisposition::RestoredFromSelectedFile,
            community.clone(),
        ),
        (
            "compatibility.eldin-field-room-zero-spawn",
            BitePreservationDisposition::RequiresCompatibility,
            community.clone(),
        ),
        (
            "activation.standard-bite-overlap",
            BitePreservationDisposition::Unknown,
            unknown_community.clone(),
        ),
        (
            "destination.king-bulblin-fight",
            BitePreservationDisposition::Unknown,
            unknown_community,
        ),
    ] {
        insert_entry(&mut entries, subject_id.into(), disposition, evidence)?;
    }
    let mut matrix = BitePreservationMatrix {
        schema: BITE_PRESERVATION_MATRIX_SCHEMA.into(),
        id: GZ2E01_STANDARD_BITE_MATRIX_ID.into(),
        scope: exact_gz2e01_scope(),
        variant: BiteVariant::StandardSelectedFileLoad,
        entries: entries.into_values().collect(),
        content_sha256: Digest::ZERO,
    };
    matrix.content_sha256 = matrix.identity()?;
    matrix.validate()?;
    Ok(matrix)
}

fn unique_save_manifest(
    mechanics: &MechanicsCatalog,
) -> Result<(Vec<String>, RuleEvidence), PlannerContractError> {
    unique_manifest(mechanics, true)
}

fn unique_load_carry_manifest(
    mechanics: &MechanicsCatalog,
) -> Result<(Vec<String>, RuleEvidence), PlannerContractError> {
    unique_manifest(mechanics, false)
}

fn unique_manifest(
    mechanics: &MechanicsCatalog,
    save: bool,
) -> Result<(Vec<String>, RuleEvidence), PlannerContractError> {
    let mut found: Option<(Vec<String>, RuleEvidence)> = None;
    for transition in &mechanics.transitions {
        for operation in &transition.activation.effects {
            let manifest = match (save, operation) {
                (
                    true,
                    StateOperation::SaveActiveRuntimeToSlot {
                        runtime_component_ids,
                        ..
                    },
                ) => Some(runtime_component_ids),
                (
                    false,
                    StateOperation::LoadActiveRuntimeFromSlot {
                        carried_runtime_component_ids,
                        ..
                    },
                ) => Some(carried_runtime_component_ids),
                _ => None,
            };
            if let Some(manifest) = manifest {
                if let Some((expected, _)) = &found {
                    if expected != manifest {
                        return Err(PlannerContractError::new(
                            "bite_preservation_matrix.mechanics",
                            "title mechanics contain inconsistent selected-file manifests",
                        ));
                    }
                } else {
                    found = Some((manifest.clone(), transition.evidence.clone()));
                }
            }
        }
    }
    found.ok_or_else(|| {
        PlannerContractError::new(
            "bite_preservation_matrix.mechanics",
            "title mechanics contain no selected-file manifest",
        )
    })
}

fn insert_entry(
    entries: &mut BTreeMap<String, BitePreservationEntry>,
    subject_id: String,
    disposition: BitePreservationDisposition,
    evidence: RuleEvidence,
) -> Result<(), PlannerContractError> {
    if entries
        .insert(
            subject_id.clone(),
            BitePreservationEntry {
                subject_id,
                disposition,
                evidence,
            },
        )
        .is_some()
    {
        return Err(PlannerContractError::new(
            "bite_preservation_matrix.entries",
            "mechanically derived manifests overlap",
        ));
    }
    Ok(())
}

fn community_evidence(truth: TruthStatus) -> RuleEvidence {
    RuleEvidence {
        truth,
        records: vec![EvidenceRecord {
            id: "community.zsr.tp.standard-bite".into(),
            kind: EvidenceKind::CommunityReported,
            source_sha256: None,
            note: "ZeldaSpeedRuns BiTE documentation reports selected-save items/equipment/progress, replacement of title-Link properties, Eldin field room-0 spawn compatibility, and a King Bulblin outcome: https://www.zeldaspeedruns.com/tp/bit/back-in-time-equipped".into(),
        }],
    }
}

fn exact_gz2e01_scope() -> ContextScope {
    ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: ExactContext {
                content_sha256: GZ2E01_CONTENT_SHA256,
                runtime_configuration_sha256: GZ2E01_EN_RUNTIME_SHA256,
            },
        }],
    }
}

fn expected_dispositions() -> BTreeMap<&'static str, BitePreservationDisposition> {
    use BitePreservationDisposition::*;
    BTreeMap::from([
        ("activation.standard-bite-overlap", Unknown),
        (
            "compatibility.eldin-field-room-zero-spawn",
            RequiresCompatibility,
        ),
        ("destination.king-bulblin-fight", Unknown),
        ("flags.persistent-event-registers", RestoredFromSelectedFile),
        ("flags.temporary-event-registers", CarriedFromBitRuntime),
        ("inventory-and-resources", RestoredFromSelectedFile),
        (
            "outside-runtime.session-components",
            PreservedOutsideRuntimeLifetime,
        ),
        ("physical-slot.unselected-images", UnchangedSealedBacking),
        ("restart", CarriedFromBitRuntime),
        ("return-place", RestoredFromSelectedFile),
        ("runtime-file.header", CarriedFromBitRuntime),
        ("save.dungeon-memory.index-6", RestoredFromSelectedFile),
        ("save.player-info", RestoredFromSelectedFile),
        ("save.player-light-drop", RestoredFromSelectedFile),
        ("source-runtime.omitted-components", RemovedWithBitRuntime),
        (
            "visible-link.items-equipment-and-progress",
            RestoredFromSelectedFile,
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{
        CONTENT_IDENTITY_SCHEMA, ContentFingerprint, GamePlatform, GameRegion,
        RUNTIME_CONFIGURATION_SCHEMA,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn context() -> (ContentIdentity, RuntimeConfiguration) {
        let content = ContentIdentity {
            schema: CONTENT_IDENTITY_SCHEMA.into(),
            id: "gcn-us-1.0-gz2e01".into(),
            fingerprint: ContentFingerprint {
                platform: GamePlatform::GameCube,
                region: GameRegion::Usa,
                revision: "1.0".into(),
                product_id: "GZ2E01".into(),
                executable_sha256:
                    "e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8"
                        .parse()
                        .unwrap(),
                game_data_sha256:
                    "0bc3bb229279d4b8a8c7cbe962b0bffdfecd35ff21e2d6761ad42e90a070f772"
                        .parse()
                        .unwrap(),
                resource_manifest_sha256:
                    "2ab36f6c1d9d551c1397e1cf59e13288d2684c973cb7bd0ad6878f5a3b3a2ab1"
                        .parse()
                        .unwrap(),
            },
        };
        let runtime = RuntimeConfiguration {
            schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
            content_sha256: GZ2E01_CONTENT_SHA256,
            language: "en".into(),
            settings: BTreeMap::new(),
        };
        (content, runtime)
    }

    #[test]
    fn matrix_derives_exact_save_and_carry_sets() {
        let (content, runtime) = context();
        let matrix = gz2e01_bite_preservation_matrix(&content, &runtime).unwrap();
        let subjects = |disposition| {
            matrix
                .entries
                .iter()
                .filter(|entry| entry.disposition == disposition)
                .map(|entry| entry.subject_id.as_str())
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(
            subjects(BitePreservationDisposition::CarriedFromBitRuntime),
            BTreeSet::from([
                "flags.temporary-event-registers",
                "restart",
                "runtime-file.header",
            ])
        );
        assert_eq!(
            subjects(BitePreservationDisposition::RestoredFromSelectedFile),
            BTreeSet::from([
                "flags.persistent-event-registers",
                "inventory-and-resources",
                "return-place",
                "save.dungeon-memory.index-6",
                "save.player-info",
                "save.player-light-drop",
                "visible-link.items-equipment-and-progress",
            ])
        );
        assert!(
            !subjects(BitePreservationDisposition::CarriedFromBitRuntime)
                .contains("visible-link.items-equipment-and-progress")
        );
    }

    #[test]
    fn activation_and_destination_remain_unknown() {
        let (content, runtime) = context();
        let matrix = gz2e01_bite_preservation_matrix(&content, &runtime).unwrap();
        for subject in [
            "activation.standard-bite-overlap",
            "destination.king-bulblin-fight",
        ] {
            let entry = matrix
                .entries
                .iter()
                .find(|entry| entry.subject_id == subject)
                .unwrap();
            assert_eq!(entry.disposition, BitePreservationDisposition::Unknown);
            assert_eq!(entry.evidence.truth, TruthStatus::Unknown);
        }
    }

    #[test]
    fn canonical_matrix_round_trips_and_rejects_tampering() {
        let (content, runtime) = context();
        let matrix = gz2e01_bite_preservation_matrix(&content, &runtime).unwrap();
        let bytes = matrix.canonical_bytes().unwrap();
        assert_eq!(
            BitePreservationMatrix::decode_canonical(&bytes).unwrap(),
            matrix
        );
        let mut tampered = matrix.clone();
        tampered.entries[0].disposition = BitePreservationDisposition::CarriedFromBitRuntime;
        assert!(tampered.validate().is_err());
        let mut noncanonical = bytes.clone();
        noncanonical.pop();
        assert!(BitePreservationMatrix::decode_canonical(&noncanonical).is_err());
    }

    #[test]
    fn builder_rejects_other_contexts() {
        let (mut content, runtime) = context();
        content.id = "gcn-us-1.0-gz2e01-mutant".into();
        assert!(gz2e01_bite_preservation_matrix(&content, &runtime).is_err());
    }
}
