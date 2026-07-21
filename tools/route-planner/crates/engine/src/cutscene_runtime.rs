//! Exact-content resolution of PACKAGE/JStudio missing-resource control flow.
//!
//! This layer does not claim that a corruption setup produces a failed load and
//! does not choose the outer event's normal or skip exit. It proves what the
//! audited runtime does once archive/STB availability has the stated result.

use crate::artifact::Digest;
use crate::cutscene_import::{CutsceneWrapperCut, CutsceneWrapperTopology};
use crate::identity::ContentIdentity;
use crate::jstudio_semantics::{
    JstudioAdaptorHandler, JstudioSemanticPayload, JstudioSemanticProgram,
    JstudioSemanticResolution,
};
use crate::orig_extraction::ExtractedEventDataValue;
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const CUTSCENE_PACKAGE_RUNTIME_PROFILE_SCHEMA: &str =
    "dusklight.route-planner.cutscene-package-runtime-profile/v1";
pub const RESOLVED_CUTSCENE_PACKAGE_SCHEMA: &str =
    "dusklight.route-planner.resolved-cutscene-package/v1";
const BUNDLED_GZ2E01_PROFILE: &[u8] =
    include_bytes!("../data/cutscene-runtime-profiles/gz2e01-demo07_02.json");

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutscenePackageRuntimeProfile {
    pub schema: String,
    pub id: String,
    pub content_sha256: Digest,
    pub executable_sha256: Digest,
    pub wrapper_topology_sha256: Digest,
    pub nominal_semantic_program_sha256: Digest,
    pub archive_request_failure: ArchiveRequestFailureBehavior,
    pub negative_archive_sync: NegativeArchiveSyncBehavior,
    pub stb_lookup_order: Vec<StbLookupSource>,
    pub missing_stb_parse: MissingStbParseBehavior,
    pub package_mode_zero: PackageModeZeroBehavior,
    pub evidence: Vec<CutsceneRuntimeEvidence>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveRequestFailureBehavior {
    ClearDemoArchiveNameAndContinue,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NegativeArchiveSyncBehavior {
    ContinueRoomInitialization,
    EscapeRestart,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StbLookupSource {
    SelectedDemoArchive,
    CurrentRoomArchive,
    StageArchive,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingStbParseBehavior {
    ReturnFalseBeforeDemoModeWrite,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageModeZeroBehavior {
    CompletePlayCut,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneRuntimeEvidence {
    pub source_sha256: Digest,
    pub note: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedCutscenePackage {
    pub schema: String,
    pub content_sha256: Digest,
    pub wrapper_topology_sha256: Digest,
    pub nominal_semantic_program_sha256: Digest,
    pub runtime_profile_sha256: Digest,
    pub event_name: String,
    pub demo_archive_name: String,
    pub stb_file: String,
    pub package_event_flag_parameter: Option<i32>,
    pub nominal_actor_id_writes: Vec<NominalActorIdSummary>,
    pub failure_control_flow: CutsceneFailureControlFlow,
    pub coverage: CutsceneFailureCoverage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NominalActorIdSummary {
    pub semantic: String,
    pub occurrence_count: u32,
    pub direct_ids: Vec<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneFailureControlFlow {
    pub archive_request_failure: ArchiveRequestFailureBehavior,
    pub negative_archive_sync: NegativeArchiveSyncBehavior,
    pub stb_lookup_order: Vec<StbLookupSource>,
    pub all_stb_lookups_missing: MissingStbOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MissingStbOutcome {
    pub parse_behavior: MissingStbParseBehavior,
    pub semantic_paragraphs_executed: u32,
    pub demo_mode_write_executed: bool,
    pub package_event_flag_check_after_start_attempt: bool,
    pub package_event_flag_write_executed: bool,
    pub package_cut_guard: String,
    pub package_cut_behavior: PackageModeZeroBehavior,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CutsceneRuntimeCoverageStatus {
    Resolved,
    Unresolved,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneFailureCoverage {
    pub archive_failure_behavior: CutsceneRuntimeCoverageStatus,
    pub missing_stb_lookup_and_parse: CutsceneRuntimeCoverageStatus,
    pub package_play_cut_behavior: CutsceneRuntimeCoverageStatus,
    pub actor_corruption_producer: CutsceneRuntimeCoverageStatus,
    pub final_outer_event_exit: CutsceneRuntimeCoverageStatus,
    pub other_return_place_writers: CutsceneRuntimeCoverageStatus,
}

impl CutscenePackageRuntimeProfile {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != CUTSCENE_PACKAGE_RUNTIME_PROFILE_SCHEMA {
            return Err(PlannerContractError::new(
                "cutscene_runtime_profile.schema",
                "is unsupported",
            ));
        }
        validate_stable_id("cutscene_runtime_profile.id", &self.id)?;
        if [
            self.content_sha256,
            self.executable_sha256,
            self.wrapper_topology_sha256,
            self.nominal_semantic_program_sha256,
        ]
        .contains(&Digest::ZERO)
        {
            return Err(PlannerContractError::new(
                "cutscene_runtime_profile.identity",
                "must pin nonzero exact-content and input digests",
            ));
        }
        if self.stb_lookup_order
            != [
                StbLookupSource::SelectedDemoArchive,
                StbLookupSource::CurrentRoomArchive,
                StbLookupSource::StageArchive,
            ]
        {
            return Err(PlannerContractError::new(
                "cutscene_runtime_profile.stb_lookup_order",
                "must preserve the audited ordered fallback chain",
            ));
        }
        if self.evidence.is_empty() {
            return Err(PlannerContractError::new(
                "cutscene_runtime_profile.evidence",
                "must not be empty",
            ));
        }
        let mut prior = None;
        for evidence in &self.evidence {
            if evidence.source_sha256 == Digest::ZERO {
                return Err(PlannerContractError::new(
                    "cutscene_runtime_profile.evidence.source_sha256",
                    "must be nonzero",
                ));
            }
            validate_label("cutscene_runtime_profile.evidence.note", &evidence.note)?;
            let key = (evidence.source_sha256, evidence.note.as_str());
            if prior.is_some_and(|prior| prior >= key) {
                return Err(PlannerContractError::new(
                    "cutscene_runtime_profile.evidence",
                    "must be unique and sorted by source digest then note",
                ));
            }
            prior = Some(key);
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
        let profile: Self = serde_json::from_slice(bytes)?;
        profile.validate()?;
        if profile.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "cutscene_runtime_profile",
                "is not canonical JSON",
            ));
        }
        Ok(profile)
    }
}

impl ResolvedCutscenePackage {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != RESOLVED_CUTSCENE_PACKAGE_SCHEMA {
            return Err(PlannerContractError::new(
                "resolved_cutscene_package.schema",
                "is unsupported",
            ));
        }
        if [
            self.content_sha256,
            self.wrapper_topology_sha256,
            self.nominal_semantic_program_sha256,
            self.runtime_profile_sha256,
        ]
        .contains(&Digest::ZERO)
        {
            return Err(PlannerContractError::new(
                "resolved_cutscene_package.identity",
                "must retain nonzero provenance digests",
            ));
        }
        for value in [&self.event_name, &self.demo_archive_name, &self.stb_file] {
            validate_label("resolved_cutscene_package.label", value)?;
        }
        let mut prior = None;
        for summary in &self.nominal_actor_id_writes {
            validate_stable_id(
                "resolved_cutscene_package.nominal.semantic",
                &summary.semantic,
            )?;
            if !matches!(summary.semantic.as_str(), "actor.animation" | "actor.shape")
                || summary.occurrence_count == 0
                || summary.direct_ids.is_empty()
                || summary.direct_ids.len() > summary.occurrence_count as usize
                || prior.is_some_and(|prior: &str| prior >= summary.semantic.as_str())
                || !strictly_sorted(&summary.direct_ids)
            {
                return Err(PlannerContractError::new(
                    "resolved_cutscene_package.nominal_actor_id_writes",
                    "must be nonempty, unique, and canonically sorted",
                ));
            }
            prior = Some(summary.semantic.as_str());
        }
        let missing = &self.failure_control_flow.all_stb_lookups_missing;
        let expected_failure_boundary = self.failure_control_flow.archive_request_failure
            == ArchiveRequestFailureBehavior::ClearDemoArchiveNameAndContinue
            && self.failure_control_flow.negative_archive_sync
                == NegativeArchiveSyncBehavior::ContinueRoomInitialization
            && self.failure_control_flow.stb_lookup_order
                == [
                    StbLookupSource::SelectedDemoArchive,
                    StbLookupSource::CurrentRoomArchive,
                    StbLookupSource::StageArchive,
                ]
            && missing.parse_behavior == MissingStbParseBehavior::ReturnFalseBeforeDemoModeWrite
            && missing.semantic_paragraphs_executed == 0
            && !missing.demo_mode_write_executed
            && missing.package_event_flag_check_after_start_attempt
            && missing.package_event_flag_write_executed
                == self.package_event_flag_parameter.is_some()
            && missing.package_cut_guard == "demo_mode == 0"
            && missing.package_cut_behavior == PackageModeZeroBehavior::CompletePlayCut;
        if !expected_failure_boundary {
            return Err(PlannerContractError::new(
                "resolved_cutscene_package.failure_control_flow",
                "does not preserve the audited missing-STB boundary",
            ));
        }
        let expected_coverage = CutsceneFailureCoverage {
            archive_failure_behavior: CutsceneRuntimeCoverageStatus::Resolved,
            missing_stb_lookup_and_parse: CutsceneRuntimeCoverageStatus::Resolved,
            package_play_cut_behavior: CutsceneRuntimeCoverageStatus::Resolved,
            actor_corruption_producer: CutsceneRuntimeCoverageStatus::Unresolved,
            final_outer_event_exit: CutsceneRuntimeCoverageStatus::Unresolved,
            other_return_place_writers: CutsceneRuntimeCoverageStatus::Unresolved,
        };
        if self.coverage != expected_coverage {
            return Err(PlannerContractError::new(
                "resolved_cutscene_package.coverage",
                "must not overstate the source-audited boundary",
            ));
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
        let package: Self = serde_json::from_slice(bytes)?;
        package.validate()?;
        if package.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "resolved_cutscene_package",
                "is not canonical JSON",
            ));
        }
        Ok(package)
    }
}

pub fn bundled_gz2e01_cutscene_runtime_profile()
-> Result<CutscenePackageRuntimeProfile, PlannerContractError> {
    CutscenePackageRuntimeProfile::decode_canonical(BUNDLED_GZ2E01_PROFILE)
}

pub fn resolve_cutscene_package(
    content: &ContentIdentity,
    topology: &CutsceneWrapperTopology,
    nominal_semantics: &JstudioSemanticProgram,
    profile: &CutscenePackageRuntimeProfile,
) -> Result<ResolvedCutscenePackage, PlannerContractError> {
    content.validate()?;
    topology.validate()?;
    nominal_semantics.validate()?;
    profile.validate()?;
    let content_sha256 = content.digest()?;
    let topology_sha256 = topology.digest()?;
    let semantics_sha256 = nominal_semantics.digest()?;
    if content_sha256 != profile.content_sha256
        || content.fingerprint.executable_sha256 != profile.executable_sha256
        || topology_sha256 != profile.wrapper_topology_sha256
        || semantics_sha256 != profile.nominal_semantic_program_sha256
        || nominal_semantics.content_sha256 != content_sha256
    {
        return Err(PlannerContractError::new(
            "cutscene_runtime_profile.identity",
            "does not match the exact content, wrapper, and nominal STB semantics",
        ));
    }
    if profile.negative_archive_sync != NegativeArchiveSyncBehavior::ContinueRoomInitialization {
        return Err(PlannerContractError::new(
            "cutscene_runtime_profile.negative_archive_sync",
            "does not reach the PACKAGE missing-STB branch",
        ));
    }
    let package_play = exactly_one(
        topology
            .staff_paths
            .iter()
            .filter(|staff| staff.name == "PACKAGE")
            .flat_map(|staff| staff.cuts.iter())
            .filter(|cut| cut.name == "PLAY"),
        "resolved_cutscene_package.package_play",
    )?;
    let event_flag = optional_integer_parameter(package_play, "EventFlag")?;
    let package = ResolvedCutscenePackage {
        schema: RESOLVED_CUTSCENE_PACKAGE_SCHEMA.into(),
        content_sha256,
        wrapper_topology_sha256: topology_sha256,
        nominal_semantic_program_sha256: semantics_sha256,
        runtime_profile_sha256: profile.digest()?,
        event_name: topology.event_name.clone(),
        demo_archive_name: topology.demo_archive_name.clone(),
        stb_file: topology.package_stb_file.clone(),
        package_event_flag_parameter: event_flag,
        nominal_actor_id_writes: nominal_actor_ids(nominal_semantics)?,
        failure_control_flow: CutsceneFailureControlFlow {
            archive_request_failure: profile.archive_request_failure,
            negative_archive_sync: profile.negative_archive_sync,
            stb_lookup_order: profile.stb_lookup_order.clone(),
            all_stb_lookups_missing: MissingStbOutcome {
                parse_behavior: profile.missing_stb_parse,
                semantic_paragraphs_executed: 0,
                demo_mode_write_executed: false,
                package_event_flag_check_after_start_attempt: true,
                package_event_flag_write_executed: event_flag.is_some(),
                package_cut_guard: "demo_mode == 0".into(),
                package_cut_behavior: profile.package_mode_zero,
            },
        },
        coverage: CutsceneFailureCoverage {
            archive_failure_behavior: CutsceneRuntimeCoverageStatus::Resolved,
            missing_stb_lookup_and_parse: CutsceneRuntimeCoverageStatus::Resolved,
            package_play_cut_behavior: CutsceneRuntimeCoverageStatus::Resolved,
            actor_corruption_producer: CutsceneRuntimeCoverageStatus::Unresolved,
            final_outer_event_exit: CutsceneRuntimeCoverageStatus::Unresolved,
            other_return_place_writers: CutsceneRuntimeCoverageStatus::Unresolved,
        },
    };
    package.validate()?;
    Ok(package)
}

fn nominal_actor_ids(
    semantics: &JstudioSemanticProgram,
) -> Result<Vec<NominalActorIdSummary>, PlannerContractError> {
    let mut summaries = BTreeMap::<String, (u32, BTreeSet<u32>)>::new();
    for record in &semantics.records {
        let JstudioSemanticResolution::AdaptorCall {
            semantic,
            handler: JstudioAdaptorHandler::ActorShape | JstudioAdaptorHandler::ActorAnimation,
            payload,
            ..
        } = &record.resolution
        else {
            continue;
        };
        let entry = summaries.entry(semantic.clone()).or_default();
        entry.0 = entry.0.checked_add(1).ok_or_else(|| {
            PlannerContractError::new("resolved_cutscene_package.nominal", "count overflows")
        })?;
        match payload {
            JstudioSemanticPayload::Unsigned32 { values } if values.len() == 1 => {
                entry.1.insert(values[0]);
            }
            _ => {
                return Err(PlannerContractError::new(
                    "resolved_cutscene_package.nominal",
                    "actor ID write has an unsupported resolved payload",
                ));
            }
        }
    }
    Ok(summaries
        .into_iter()
        .map(
            |(semantic, (occurrence_count, direct_ids))| NominalActorIdSummary {
                semantic,
                occurrence_count,
                direct_ids: direct_ids.into_iter().collect(),
            },
        )
        .collect())
}

fn optional_integer_parameter(
    cut: &CutsceneWrapperCut,
    name: &str,
) -> Result<Option<i32>, PlannerContractError> {
    let matches = cut
        .parameters
        .iter()
        .filter(|parameter| parameter.name == name)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [parameter] => match &parameter.value {
            ExtractedEventDataValue::Integers { values } if values.len() == 1 => {
                Ok(Some(values[0]))
            }
            _ => Err(PlannerContractError::new(
                "resolved_cutscene_package.package_event_flag",
                "is not exactly one integer",
            )),
        },
        _ => Err(PlannerContractError::new(
            "resolved_cutscene_package.package_event_flag",
            "is ambiguous",
        )),
    }
}

fn exactly_one<'a, T>(
    mut values: impl Iterator<Item = &'a T>,
    field: &'static str,
) -> Result<&'a T, PlannerContractError> {
    let value = values
        .next()
        .ok_or_else(|| PlannerContractError::new(field, "is missing"))?;
    if values.next().is_some() {
        return Err(PlannerContractError::new(field, "is ambiguous"));
    }
    Ok(value)
}

fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_profile_is_canonical_and_keeps_unknown_boundaries_explicit() {
        let profile = bundled_gz2e01_cutscene_runtime_profile().unwrap();
        assert_eq!(profile.id, "gz2e01-demo07-02-package-runtime");
        assert_eq!(
            profile.negative_archive_sync,
            NegativeArchiveSyncBehavior::ContinueRoomInitialization
        );
        assert_eq!(profile.stb_lookup_order.len(), 3);
        let audited_sources = [
            include_bytes!("../../../../../src/d/d_s_room.cpp").as_slice(),
            include_bytes!("../../../../../src/d/d_demo.cpp").as_slice(),
            include_bytes!("../../../../../src/d/d_event.cpp").as_slice(),
            include_bytes!("../../../../../libs/JSystem/src/JGadget/binary.cpp").as_slice(),
            include_bytes!("../../../../../src/d/d_event_data.cpp").as_slice(),
        ];
        assert_eq!(
            profile
                .evidence
                .iter()
                .map(|evidence| evidence.source_sha256)
                .collect::<Vec<_>>(),
            audited_sources
                .iter()
                .map(|source| Digest(Sha256::digest(source).into()))
                .collect::<Vec<_>>()
        );

        let tower_profile = CutscenePackageRuntimeProfile::decode_canonical(include_bytes!(
            "../data/cutscene-runtime-profiles/gz2e01-demo07_01.json"
        ))
        .unwrap();
        assert_eq!(tower_profile.id, "gz2e01-demo07-01-package-runtime");
        assert_eq!(
            tower_profile.archive_request_failure,
            ArchiveRequestFailureBehavior::ClearDemoArchiveNameAndContinue
        );
    }

    #[test]
    fn canonical_result_rejects_promoting_the_actor_corruption_producer() {
        let mut package = ResolvedCutscenePackage {
            schema: RESOLVED_CUTSCENE_PACKAGE_SCHEMA.into(),
            content_sha256: Digest([1; 32]),
            wrapper_topology_sha256: Digest([2; 32]),
            nominal_semantic_program_sha256: Digest([3; 32]),
            runtime_profile_sha256: Digest([4; 32]),
            event_name: "event".into(),
            demo_archive_name: "Demo00_00".into(),
            stb_file: "event.stb".into(),
            package_event_flag_parameter: None,
            nominal_actor_id_writes: vec![NominalActorIdSummary {
                semantic: "actor.shape".into(),
                occurrence_count: 1,
                direct_ids: vec![7],
            }],
            failure_control_flow: CutsceneFailureControlFlow {
                archive_request_failure:
                    ArchiveRequestFailureBehavior::ClearDemoArchiveNameAndContinue,
                negative_archive_sync: NegativeArchiveSyncBehavior::ContinueRoomInitialization,
                stb_lookup_order: vec![
                    StbLookupSource::SelectedDemoArchive,
                    StbLookupSource::CurrentRoomArchive,
                    StbLookupSource::StageArchive,
                ],
                all_stb_lookups_missing: MissingStbOutcome {
                    parse_behavior: MissingStbParseBehavior::ReturnFalseBeforeDemoModeWrite,
                    semantic_paragraphs_executed: 0,
                    demo_mode_write_executed: false,
                    package_event_flag_check_after_start_attempt: true,
                    package_event_flag_write_executed: false,
                    package_cut_guard: "demo_mode == 0".into(),
                    package_cut_behavior: PackageModeZeroBehavior::CompletePlayCut,
                },
            },
            coverage: CutsceneFailureCoverage {
                archive_failure_behavior: CutsceneRuntimeCoverageStatus::Resolved,
                missing_stb_lookup_and_parse: CutsceneRuntimeCoverageStatus::Resolved,
                package_play_cut_behavior: CutsceneRuntimeCoverageStatus::Resolved,
                actor_corruption_producer: CutsceneRuntimeCoverageStatus::Unresolved,
                final_outer_event_exit: CutsceneRuntimeCoverageStatus::Unresolved,
                other_return_place_writers: CutsceneRuntimeCoverageStatus::Unresolved,
            },
        };
        assert!(package.validate().is_ok());
        package.coverage.actor_corruption_producer = CutsceneRuntimeCoverageStatus::Resolved;
        assert_eq!(
            package.validate().unwrap_err().field(),
            "resolved_cutscene_package.coverage"
        );

        package.coverage.actor_corruption_producer = CutsceneRuntimeCoverageStatus::Unresolved;
        package.failure_control_flow.negative_archive_sync =
            NegativeArchiveSyncBehavior::EscapeRestart;
        assert_eq!(
            package.validate().unwrap_err().field(),
            "resolved_cutscene_package.failure_control_flow"
        );
    }
}
