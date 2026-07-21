//! Exact-resource resolution of outer event completion and scene-change dispatch.
//!
//! The PACKAGE resolver establishes a conditional missing-STB outcome. This
//! layer joins that outcome to raw REVT and `event_list.dat` records, derives
//! the local flag sequence, and emits ordinary candidate transitions. It does
//! not claim that actor corruption produces the missing-STB predicate.

use crate::artifact::Digest;
use crate::cutscene_import::{
    CutsceneWrapperExit, CutsceneWrapperExitKind, CutsceneWrapperTopology,
};
use crate::cutscene_runtime::{
    CutsceneRuntimeCoverageStatus, PackageModeZeroBehavior, ResolvedCutscenePackage,
};
use crate::identity::{ContentIdentity, ContextSelector, ExactContext, RuntimeConfiguration};
use crate::logic::{
    ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, PredicateExpression,
    RuleEvidence, TruthStatus, ValueReference,
};
use crate::orig_extraction::{ExtractedEventCut, ExtractedEventList, ExtractedStageData};
use crate::state::{ComponentSelector, SceneLocation, StateValue};
use crate::transition::{
    ActivationContract, CandidateTransition, ComponentFieldTarget, StateOperation, TransitionKind,
};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub const CUTSCENE_OUTER_RUNTIME_PROFILE_SCHEMA: &str =
    "dusklight.route-planner.cutscene-outer-runtime-profile/v1";
pub const RESOLVED_CUTSCENE_OUTER_EVENT_SCHEMA: &str =
    "dusklight.route-planner.resolved-cutscene-outer-event/v1";
const BUNDLED_GZ2E01_PROFILE: &[u8] =
    include_bytes!("../data/cutscene-outer-runtime-profiles/gz2e01-demo07_02.json");

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneOuterRuntimeProfile {
    pub schema: String,
    pub id: String,
    pub content_sha256: Digest,
    pub executable_sha256: Digest,
    pub wrapper_topology_sha256: Digest,
    pub resolved_package_sha256: Digest,
    pub state_binding: CutsceneOuterStateBinding,
    pub evidence: Vec<CutsceneOuterRuntimeEvidence>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneOuterStateBinding {
    pub flow_component_id: String,
    pub package_play_node: String,
    pub package_wait_node: String,
    pub outer_finish_node: String,
    pub stb_lookup_result_field: String,
    pub all_stb_lookups_missing_value: String,
    pub scene_change_suppressed_field: String,
    pub skip_active_field: String,
    pub event_local_flag_field_prefix: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneOuterRuntimeEvidence {
    pub id: String,
    pub source_sha256: Digest,
    pub note: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedCutsceneOuterEvent {
    pub schema: String,
    pub content_sha256: Digest,
    pub runtime_configuration_sha256: Digest,
    pub wrapper_topology_sha256: Digest,
    pub resolved_package_sha256: Digest,
    pub runtime_profile_sha256: Digest,
    pub stage_archive_sha256: Digest,
    pub stage_resource_sha256: Digest,
    pub event_list_resource_sha256: Digest,
    pub event_name: String,
    pub map_event_record_index: u32,
    pub event_list_index: u32,
    pub event_finish_flags: [i32; 3],
    pub package_play: ResolvedOuterCut,
    pub package_wait: ResolvedOuterCut,
    pub completion_steps: Vec<OuterCompletionStep>,
    pub skip_cut_enabled: bool,
    pub skip_cut_type: u8,
    pub state_binding: CutsceneOuterStateBinding,
    pub normal_exit: CutsceneWrapperExit,
    pub skip_exit: CutsceneWrapperExit,
    pub evidence: Vec<CutsceneOuterRuntimeEvidence>,
    pub transitions: Vec<CandidateTransition>,
    pub coverage: CutsceneOuterCoverage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedOuterCut {
    pub cut_index: u32,
    pub name: String,
    pub start_flags: [i32; 3],
    pub completion_flag: u32,
    pub next_cut_index: Option<u32>,
    pub has_timer_parameter: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OuterCompletionStep {
    pub ordinal: u8,
    pub kind: OuterCompletionStepKind,
    pub flag_id: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OuterCompletionStepKind {
    PackageModeZeroCompletesPlay,
    AdvanceToWait,
    ZeroTimerCompletesWait,
    EventFinishSatisfied,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CutsceneOuterCoverageStatus {
    Resolved,
    Unresolved,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneOuterCoverage {
    pub conditional_missing_stb_completion: CutsceneOuterCoverageStatus,
    pub outer_exit_dispatch: CutsceneOuterCoverageStatus,
    pub actor_corruption_producer: CutsceneOuterCoverageStatus,
    pub witnessed_corruption_suffix: CutsceneOuterCoverageStatus,
    pub other_return_place_writers: CutsceneOuterCoverageStatus,
}

impl CutsceneOuterRuntimeProfile {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != CUTSCENE_OUTER_RUNTIME_PROFILE_SCHEMA {
            return Err(PlannerContractError::new(
                "cutscene_outer_profile.schema",
                "is unsupported",
            ));
        }
        validate_stable_id("cutscene_outer_profile.id", &self.id)?;
        if [
            self.content_sha256,
            self.executable_sha256,
            self.wrapper_topology_sha256,
            self.resolved_package_sha256,
        ]
        .contains(&Digest::ZERO)
        {
            return Err(PlannerContractError::new(
                "cutscene_outer_profile.identity",
                "must pin nonzero exact-content and input digests",
            ));
        }
        self.state_binding.validate()?;
        validate_runtime_evidence(&self.evidence)
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
                "cutscene_outer_profile",
                "is not canonical JSON",
            ));
        }
        Ok(profile)
    }
}

impl CutsceneOuterStateBinding {
    fn validate(&self) -> Result<(), PlannerContractError> {
        for (field, value) in [
            ("flow_component_id", &self.flow_component_id),
            ("package_play_node", &self.package_play_node),
            ("package_wait_node", &self.package_wait_node),
            ("outer_finish_node", &self.outer_finish_node),
            ("stb_lookup_result_field", &self.stb_lookup_result_field),
            (
                "all_stb_lookups_missing_value",
                &self.all_stb_lookups_missing_value,
            ),
            (
                "scene_change_suppressed_field",
                &self.scene_change_suppressed_field,
            ),
            ("skip_active_field", &self.skip_active_field),
            (
                "event_local_flag_field_prefix",
                &self.event_local_flag_field_prefix,
            ),
        ] {
            validate_stable_id(
                &format!("cutscene_outer_profile.state_binding.{field}"),
                value,
            )?;
        }
        Ok(())
    }
}

impl ResolvedCutsceneOuterEvent {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != RESOLVED_CUTSCENE_OUTER_EVENT_SCHEMA {
            return Err(PlannerContractError::new(
                "resolved_cutscene_outer.schema",
                "is unsupported",
            ));
        }
        if [
            self.content_sha256,
            self.runtime_configuration_sha256,
            self.wrapper_topology_sha256,
            self.resolved_package_sha256,
            self.runtime_profile_sha256,
            self.stage_archive_sha256,
            self.stage_resource_sha256,
            self.event_list_resource_sha256,
        ]
        .contains(&Digest::ZERO)
        {
            return Err(PlannerContractError::new(
                "resolved_cutscene_outer.identity",
                "must retain nonzero exact input identities",
            ));
        }
        validate_label("resolved_cutscene_outer.event_name", &self.event_name)?;
        if self.skip_cut_type > 3
            || self.package_play.name != "PLAY"
            || self.package_wait.name != "WAIT"
            || self.package_play.next_cut_index != Some(self.package_wait.cut_index)
            || self.package_wait.next_cut_index.is_some()
            || self.package_play.has_timer_parameter
            || self.package_wait.has_timer_parameter
            || self.event_finish_flags != [self.package_wait.completion_flag as i32, -1, -1]
        {
            return Err(PlannerContractError::new(
                "resolved_cutscene_outer.completion",
                "does not preserve the audited PLAY -> WAIT -> event-finish chain",
            ));
        }
        self.state_binding.validate()?;
        if self.normal_exit.kind != CutsceneWrapperExitKind::Normal
            || self.skip_exit.kind != CutsceneWrapperExitKind::Skip
        {
            return Err(PlannerContractError::new(
                "resolved_cutscene_outer.exits",
                "must retain one normal and one skip wrapper exit",
            ));
        }
        validate_runtime_evidence(&self.evidence)?;
        let expected_steps = [
            (
                OuterCompletionStepKind::PackageModeZeroCompletesPlay,
                Some(self.package_play.completion_flag),
            ),
            (OuterCompletionStepKind::AdvanceToWait, None),
            (
                OuterCompletionStepKind::ZeroTimerCompletesWait,
                Some(self.package_wait.completion_flag),
            ),
            (
                OuterCompletionStepKind::EventFinishSatisfied,
                Some(self.package_wait.completion_flag),
            ),
        ];
        if self.completion_steps.len() != expected_steps.len()
            || self
                .completion_steps
                .iter()
                .zip(expected_steps)
                .enumerate()
                .any(|(index, (step, expected))| {
                    step.ordinal != index as u8 || (step.kind, step.flag_id) != expected
                })
        {
            return Err(PlannerContractError::new(
                "resolved_cutscene_outer.completion_steps",
                "must retain the exact ordered local-flag proof",
            ));
        }
        if self.transitions.len() != 5 {
            return Err(PlannerContractError::new(
                "resolved_cutscene_outer.transitions",
                "must contain PLAY, WAIT, normal, skip, and suppressed cases",
            ));
        }
        let mut prior = None;
        let mut scene_changes = 0;
        let mut resource_failures = 0;
        for transition in &self.transitions {
            transition.validate()?;
            if prior.is_some_and(|prior: &str| prior >= transition.id.as_str()) {
                return Err(PlannerContractError::new(
                    "resolved_cutscene_outer.transitions",
                    "must be unique and sorted",
                ));
            }
            prior = Some(transition.id.as_str());
            if transition.transition_kind == TransitionKind::CutsceneSceneChange {
                scene_changes += 1;
            }
            if transition.transition_kind == TransitionKind::ResourceLoadFailure {
                resource_failures += 1;
            }
        }
        if scene_changes != 2 || resource_failures != 1 {
            return Err(PlannerContractError::new(
                "resolved_cutscene_outer.transitions",
                "must retain one failure step and exactly two scene changes",
            ));
        }
        let expected_transitions = compile_dispatch_transitions(
            &ContextScope {
                selectors: vec![ContextSelector::Exact {
                    context: ExactContext {
                        content_sha256: self.content_sha256,
                        runtime_configuration_sha256: self.runtime_configuration_sha256,
                    },
                }],
            },
            &self.event_name,
            &self.state_binding,
            self.package_play.completion_flag,
            self.package_wait.completion_flag,
            Some(&self.normal_exit),
            Some(&self.skip_exit),
            &self.evidence,
        )?;
        if self.transitions != expected_transitions {
            return Err(PlannerContractError::new(
                "resolved_cutscene_outer.transitions",
                "do not match the derived phase and dispatch graph",
            ));
        }
        let expected_coverage = CutsceneOuterCoverage {
            conditional_missing_stb_completion: CutsceneOuterCoverageStatus::Resolved,
            outer_exit_dispatch: CutsceneOuterCoverageStatus::Resolved,
            actor_corruption_producer: CutsceneOuterCoverageStatus::Unresolved,
            witnessed_corruption_suffix: CutsceneOuterCoverageStatus::Unresolved,
            other_return_place_writers: CutsceneOuterCoverageStatus::Unresolved,
        };
        if self.coverage != expected_coverage {
            return Err(PlannerContractError::new(
                "resolved_cutscene_outer.coverage",
                "must not promote unresolved corruption or return-place claims",
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
        let event: Self = serde_json::from_slice(bytes)?;
        event.validate()?;
        if event.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "resolved_cutscene_outer",
                "is not canonical JSON",
            ));
        }
        Ok(event)
    }
}

pub fn bundled_gz2e01_cutscene_outer_runtime_profile()
-> Result<CutsceneOuterRuntimeProfile, PlannerContractError> {
    CutsceneOuterRuntimeProfile::decode_canonical(BUNDLED_GZ2E01_PROFILE)
}

#[allow(clippy::too_many_arguments)]
pub fn resolve_cutscene_outer_event(
    content: &ContentIdentity,
    runtime: &RuntimeConfiguration,
    topology: &CutsceneWrapperTopology,
    package: &ResolvedCutscenePackage,
    stage_archive_sha256: Digest,
    stage_resource: &[u8],
    event_list_resource: &[u8],
    stage: &ExtractedStageData,
    event_list: &ExtractedEventList,
    profile: &CutsceneOuterRuntimeProfile,
) -> Result<ResolvedCutsceneOuterEvent, PlannerContractError> {
    content.validate()?;
    runtime.validate()?;
    topology.validate()?;
    package.validate()?;
    profile.validate()?;
    let content_sha256 = content.digest()?;
    let runtime_configuration_sha256 = runtime.digest()?;
    let topology_sha256 = topology.digest()?;
    let package_sha256 = package.digest()?;
    if runtime.content_sha256 != content_sha256
        || content_sha256 != profile.content_sha256
        || content.fingerprint.executable_sha256 != profile.executable_sha256
        || topology_sha256 != profile.wrapper_topology_sha256
        || package_sha256 != profile.resolved_package_sha256
        || package.content_sha256 != content_sha256
        || package.wrapper_topology_sha256 != topology_sha256
    {
        return Err(PlannerContractError::new(
            "cutscene_outer_profile.identity",
            "does not match the exact content, runtime, wrapper, and package",
        ));
    }
    let stage_resource_sha256 = Digest(Sha256::digest(stage_resource).into());
    let event_list_resource_sha256 = Digest(Sha256::digest(event_list_resource).into());
    if stage_archive_sha256 != topology.source.stage_archive_sha256
        || stage_resource_sha256 != topology.source.stage_resource_sha256
        || event_list_resource_sha256 != topology.source.event_list_resource_sha256
    {
        return Err(PlannerContractError::new(
            "resolved_cutscene_outer.source",
            "raw resource identities do not match the wrapper source",
        ));
    }

    let map_event = exactly_one(
        stage.map_events.iter().filter(|event| {
            event.record_index == topology.map_event_record_index
                && event.event_name.as_deref() == Some(topology.event_name.as_str())
        }),
        "resolved_cutscene_outer.map_event",
    )?;
    let revt = decode_hex(
        &map_event.raw_hex,
        "resolved_cutscene_outer.map_event.raw_hex",
    )?;
    if revt.len() != 0x1c
        || revt[0] != topology.event_type
        || revt[4] != topology.map_tool_id
        || revt[6] != map_event.priority
        || revt[7] != map_event.normal_exit_id.unwrap_or(u8::MAX)
        || revt[9] != map_event.skip_exit_id.unwrap_or(u8::MAX)
    {
        return Err(PlannerContractError::new(
            "resolved_cutscene_outer.map_event.raw_hex",
            "does not reproduce the extracted STB REVT fields",
        ));
    }
    let skip_cut_enabled = revt[8] & 1 != 0;
    let skip_cut_type = (revt[8] >> 1) & 3;
    if !skip_cut_enabled || skip_cut_type != 1 {
        return Err(PlannerContractError::new(
            "resolved_cutscene_outer.map_event.skip_cut",
            "does not match the audited enabled type-1 dispatch",
        ));
    }

    let event = exactly_one(
        event_list.events.iter().filter(|event| {
            event.index == topology.event_list_index && event.name == topology.event_name
        }),
        "resolved_cutscene_outer.event",
    )?;
    let package_staff_path = exactly_one(
        topology
            .staff_paths
            .iter()
            .filter(|staff| staff.name == "PACKAGE"),
        "resolved_cutscene_outer.package_staff",
    )?;
    if !event
        .staff_indices
        .contains(&package_staff_path.staff_index)
    {
        return Err(PlannerContractError::new(
            "resolved_cutscene_outer.package_staff",
            "is not owned by the selected event",
        ));
    }
    let play = exact_cut(event_list, package_staff_path, "PLAY")?;
    let wait = exact_cut(event_list, package_staff_path, "WAIT")?;
    if play.next_cut_index != Some(wait.index)
        || wait.next_cut_index.is_some()
        || play.start_flags != [-1; 3]
        || wait.start_flags != [-1; 3]
        || event.finish_flags != [wait.flag_id as i32, -1, -1]
        || package
            .failure_control_flow
            .all_stb_lookups_missing
            .package_cut_behavior
            != PackageModeZeroBehavior::CompletePlayCut
        || package.coverage.package_play_cut_behavior != CutsceneRuntimeCoverageStatus::Resolved
    {
        return Err(PlannerContractError::new(
            "resolved_cutscene_outer.completion",
            "cannot prove the exact PLAY -> WAIT -> event-finish chain",
        ));
    }
    let play_topology = package_staff_path
        .cuts
        .iter()
        .find(|cut| cut.cut_index == play.index)
        .ok_or_else(|| PlannerContractError::new("resolved_cutscene_outer.play", "is missing"))?;
    let wait_topology = package_staff_path
        .cuts
        .iter()
        .find(|cut| cut.cut_index == wait.index)
        .ok_or_else(|| PlannerContractError::new("resolved_cutscene_outer.wait", "is missing"))?;
    let play_has_timer = play_topology
        .parameters
        .iter()
        .any(|data| data.name == "Timer");
    let wait_has_timer = wait_topology
        .parameters
        .iter()
        .any(|data| data.name == "Timer");
    if play_has_timer || wait_has_timer {
        return Err(PlannerContractError::new(
            "resolved_cutscene_outer.wait.timer",
            "must be absent for the immediate WAIT completion proof",
        ));
    }

    let package_play = resolved_cut(play, play_has_timer);
    let package_wait = resolved_cut(wait, wait_has_timer);
    let scope = ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: ExactContext {
                content_sha256,
                runtime_configuration_sha256,
            },
        }],
    };
    let transitions = compile_dispatch_transitions(
        &scope,
        &topology.event_name,
        &profile.state_binding,
        play.flag_id,
        wait.flag_id,
        topology.normal_exit.as_ref(),
        topology.skip_exit.as_ref(),
        &profile.evidence,
    )?;
    let resolved = ResolvedCutsceneOuterEvent {
        schema: RESOLVED_CUTSCENE_OUTER_EVENT_SCHEMA.into(),
        content_sha256,
        runtime_configuration_sha256,
        wrapper_topology_sha256: topology_sha256,
        resolved_package_sha256: package_sha256,
        runtime_profile_sha256: profile.digest()?,
        stage_archive_sha256,
        stage_resource_sha256,
        event_list_resource_sha256,
        event_name: topology.event_name.clone(),
        map_event_record_index: topology.map_event_record_index,
        event_list_index: topology.event_list_index,
        event_finish_flags: event.finish_flags,
        package_play,
        package_wait,
        completion_steps: vec![
            OuterCompletionStep {
                ordinal: 0,
                kind: OuterCompletionStepKind::PackageModeZeroCompletesPlay,
                flag_id: Some(play.flag_id),
            },
            OuterCompletionStep {
                ordinal: 1,
                kind: OuterCompletionStepKind::AdvanceToWait,
                flag_id: None,
            },
            OuterCompletionStep {
                ordinal: 2,
                kind: OuterCompletionStepKind::ZeroTimerCompletesWait,
                flag_id: Some(wait.flag_id),
            },
            OuterCompletionStep {
                ordinal: 3,
                kind: OuterCompletionStepKind::EventFinishSatisfied,
                flag_id: Some(wait.flag_id),
            },
        ],
        skip_cut_enabled,
        skip_cut_type,
        state_binding: profile.state_binding.clone(),
        normal_exit: topology.normal_exit.clone().ok_or_else(|| {
            PlannerContractError::new("resolved_cutscene_outer.normal_exit", "is missing")
        })?,
        skip_exit: topology.skip_exit.clone().ok_or_else(|| {
            PlannerContractError::new("resolved_cutscene_outer.skip_exit", "is missing")
        })?,
        evidence: profile.evidence.clone(),
        transitions,
        coverage: CutsceneOuterCoverage {
            conditional_missing_stb_completion: CutsceneOuterCoverageStatus::Resolved,
            outer_exit_dispatch: CutsceneOuterCoverageStatus::Resolved,
            actor_corruption_producer: CutsceneOuterCoverageStatus::Unresolved,
            witnessed_corruption_suffix: CutsceneOuterCoverageStatus::Unresolved,
            other_return_place_writers: CutsceneOuterCoverageStatus::Unresolved,
        },
    };
    resolved.validate()?;
    Ok(resolved)
}

fn exact_cut<'a>(
    event_list: &'a ExtractedEventList,
    staff: &crate::cutscene_import::CutsceneWrapperStaffPath,
    name: &str,
) -> Result<&'a ExtractedEventCut, PlannerContractError> {
    let topology_cut = exactly_one(
        staff.cuts.iter().filter(|cut| cut.name == name),
        "resolved_cutscene_outer.package_cut",
    )?;
    event_list
        .cuts
        .get(topology_cut.cut_index as usize)
        .filter(|cut| cut.index == topology_cut.cut_index && cut.name == name)
        .ok_or_else(|| {
            PlannerContractError::new(
                "resolved_cutscene_outer.package_cut",
                "does not match the exact event-list record",
            )
        })
}

fn resolved_cut(cut: &ExtractedEventCut, has_timer_parameter: bool) -> ResolvedOuterCut {
    ResolvedOuterCut {
        cut_index: cut.index,
        name: cut.name.clone(),
        start_flags: cut.start_flags,
        completion_flag: cut.flag_id,
        next_cut_index: cut.next_cut_index,
        has_timer_parameter,
    }
}

#[allow(clippy::too_many_arguments)]
fn compile_dispatch_transitions(
    scope: &ContextScope,
    event_name: &str,
    binding: &CutsceneOuterStateBinding,
    play_flag: u32,
    finish_flag: u32,
    normal_exit: Option<&CutsceneWrapperExit>,
    skip_exit: Option<&CutsceneWrapperExit>,
    evidence: &[CutsceneOuterRuntimeEvidence],
) -> Result<Vec<CandidateTransition>, PlannerContractError> {
    let normal_exit = require_exit(normal_exit, CutsceneWrapperExitKind::Normal)?;
    let skip_exit = require_exit(skip_exit, CutsceneWrapperExitKind::Skip)?;
    let transition_prefix = format!("transition.cutscene.{event_name}");
    let mut transitions = vec![
        phase_transition(
            &format!("{transition_prefix}.missing-stb.play-complete"),
            "Complete PACKAGE PLAY after all STB lookups miss",
            TransitionKind::ResourceLoadFailure,
            scope,
            binding,
            &binding.package_play_node,
            Some((
                binding.stb_lookup_result_field.as_str(),
                StateValue::Text(binding.all_stb_lookups_missing_value.clone()),
            )),
            play_flag,
            &binding.package_wait_node,
            evidence,
        )?,
        phase_transition(
            &format!("{transition_prefix}.wait-complete"),
            "Complete zero-timer PACKAGE WAIT",
            TransitionKind::Cutscene,
            scope,
            binding,
            &binding.package_wait_node,
            Some((
                &format!("{}.{}", binding.event_local_flag_field_prefix, play_flag),
                StateValue::Boolean(true),
            )),
            finish_flag,
            &binding.outer_finish_node,
            evidence,
        )?,
        dispatch_transition(
            &format!("{transition_prefix}.outer.normal-exit"),
            &format!(
                "Dispatch completed event through the normal exit to {}",
                normal_exit.transition.destination_stage
            ),
            scope,
            binding,
            finish_flag,
            false,
            Some(false),
            Some(normal_exit),
            evidence,
        )?,
        dispatch_transition(
            &format!("{transition_prefix}.outer.skip-exit"),
            &format!(
                "Dispatch completed event through the active skip exit to {}",
                skip_exit.transition.destination_stage
            ),
            scope,
            binding,
            finish_flag,
            false,
            Some(true),
            Some(skip_exit),
            evidence,
        )?,
        dispatch_transition(
            &format!("{transition_prefix}.outer.suppressed"),
            "Close completed event with scene change suppressed",
            scope,
            binding,
            finish_flag,
            true,
            None,
            None,
            evidence,
        )?,
    ];
    transitions.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(transitions)
}

#[allow(clippy::too_many_arguments)]
fn phase_transition(
    id: &str,
    label: &str,
    transition_kind: TransitionKind,
    scope: &ContextScope,
    binding: &CutsceneOuterStateBinding,
    source_node: &str,
    additional_guard: Option<(&str, StateValue)>,
    completion_flag: u32,
    destination_node: &str,
    evidence: &[CutsceneOuterRuntimeEvidence],
) -> Result<CandidateTransition, PlannerContractError> {
    let mut terms = vec![compare(
        ValueReference::FlowNode {
            flow_component_id: binding.flow_component_id.clone(),
        },
        StateValue::Text(source_node.into()),
    )];
    if let Some((field, value)) = additional_guard {
        terms.push(compare(
            ValueReference::ComponentField {
                component_id: binding.flow_component_id.clone(),
                field: field.into(),
            },
            value,
        ));
    }
    let transition = CandidateTransition {
        id: id.into(),
        label: label.into(),
        scope: scope.clone(),
        transition_kind,
        approach_id: "cutscene.package-missing-stb.outer-completion".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All { terms },
            physical_obligation_ids: Vec::new(),
            effects: vec![
                StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: binding.flow_component_id.clone(),
                        field: format!(
                            "{}.{}",
                            binding.event_local_flag_field_prefix, completion_flag
                        ),
                    },
                    value: StateValue::Boolean(true),
                },
                StateOperation::AdvanceFlow {
                    flow_component_id: binding.flow_component_id.clone(),
                    node_id: destination_node.into(),
                },
            ],
            unknown_requirements: Vec::new(),
        },
        evidence: transition_evidence(evidence),
    };
    transition.validate()?;
    Ok(transition)
}

#[allow(clippy::too_many_arguments)]
fn dispatch_transition(
    id: &str,
    label: &str,
    scope: &ContextScope,
    binding: &CutsceneOuterStateBinding,
    finish_flag: u32,
    scene_change_suppressed: bool,
    skip_active: Option<bool>,
    exit: Option<&CutsceneWrapperExit>,
    evidence: &[CutsceneOuterRuntimeEvidence],
) -> Result<CandidateTransition, PlannerContractError> {
    let mut guards = vec![
        compare(
            ValueReference::FlowNode {
                flow_component_id: binding.flow_component_id.clone(),
            },
            StateValue::Text(binding.outer_finish_node.clone()),
        ),
        compare(
            ValueReference::ComponentField {
                component_id: binding.flow_component_id.clone(),
                field: format!("{}.{}", binding.event_local_flag_field_prefix, finish_flag),
            },
            StateValue::Boolean(true),
        ),
        compare(
            ValueReference::ComponentField {
                component_id: binding.flow_component_id.clone(),
                field: binding.scene_change_suppressed_field.clone(),
            },
            StateValue::Boolean(scene_change_suppressed),
        ),
    ];
    if let Some(skip_active) = skip_active {
        guards.push(compare(
            ValueReference::ComponentField {
                component_id: binding.flow_component_id.clone(),
                field: binding.skip_active_field.clone(),
            },
            StateValue::Boolean(skip_active),
        ));
    }
    let mut effects = Vec::new();
    if let Some(exit) = exit {
        effects.push(StateOperation::SetLocation {
            location: SceneLocation {
                stage: exit.transition.destination_stage.clone(),
                room: exit.transition.destination_room,
                layer: exit
                    .transition
                    .scene_layer
                    .map(|layer| layer as i8)
                    .unwrap_or(-1),
                spawn: i16::from(exit.transition.destination_spawn),
            },
        });
    }
    effects.push(StateOperation::ClearComponent {
        selector: ComponentSelector::Id {
            component_id: binding.flow_component_id.clone(),
        },
    });
    let transition = CandidateTransition {
        id: id.into(),
        label: label.into(),
        scope: scope.clone(),
        transition_kind: if exit.is_some() {
            TransitionKind::CutsceneSceneChange
        } else {
            TransitionKind::Cutscene
        },
        approach_id: "cutscene.package-missing-stb.outer-completion".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All { terms: guards },
            physical_obligation_ids: Vec::new(),
            effects,
            unknown_requirements: Vec::new(),
        },
        evidence: transition_evidence(evidence),
    };
    transition.validate()?;
    Ok(transition)
}

fn transition_evidence(evidence: &[CutsceneOuterRuntimeEvidence]) -> RuleEvidence {
    RuleEvidence {
        truth: TruthStatus::Established,
        records: evidence
            .iter()
            .map(|record| EvidenceRecord {
                id: record.id.clone(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(record.source_sha256),
                note: record.note.clone(),
            })
            .collect(),
    }
}

fn validate_runtime_evidence(
    evidence: &[CutsceneOuterRuntimeEvidence],
) -> Result<(), PlannerContractError> {
    if evidence.len() < 3 {
        return Err(PlannerContractError::new(
            "cutscene_outer_profile.evidence",
            "must cover event completion, REVT decoding, and exit dispatch",
        ));
    }
    let mut prior = None;
    for record in evidence {
        validate_stable_id("cutscene_outer_profile.evidence.id", &record.id)?;
        validate_label("cutscene_outer_profile.evidence.note", &record.note)?;
        if record.source_sha256 == Digest::ZERO
            || prior.is_some_and(|prior: &str| prior >= record.id.as_str())
        {
            return Err(PlannerContractError::new(
                "cutscene_outer_profile.evidence",
                "must be nonzero, unique, and sorted by ID",
            ));
        }
        prior = Some(record.id.as_str());
    }
    Ok(())
}

fn compare(left: ValueReference, value: StateValue) -> PredicateExpression {
    PredicateExpression::Compare {
        left,
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal { value },
    }
}

fn require_exit(
    exit: Option<&CutsceneWrapperExit>,
    kind: CutsceneWrapperExitKind,
) -> Result<&CutsceneWrapperExit, PlannerContractError> {
    exit.filter(|exit| exit.kind == kind).ok_or_else(|| {
        PlannerContractError::new(
            "resolved_cutscene_outer.exit",
            "is absent or has the wrong wrapper kind",
        )
    })
}

fn exactly_one<T>(
    mut values: impl Iterator<Item = T>,
    field: &'static str,
) -> Result<T, PlannerContractError> {
    let value = values
        .next()
        .ok_or_else(|| PlannerContractError::new(field, "is missing"))?;
    if values.next().is_some() {
        return Err(PlannerContractError::new(field, "is ambiguous"));
    }
    Ok(value)
}

fn decode_hex(value: &str, field: &'static str) -> Result<Vec<u8>, PlannerContractError> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PlannerContractError::new(
            field,
            "must be even-length ASCII hex",
        ));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("ASCII hex was checked");
            u8::from_str_radix(text, 16).map_err(|_| PlannerContractError::new(field, "is invalid"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orig_extraction::ExtractedSceneTransition;

    fn fixture_exit(
        kind: CutsceneWrapperExitKind,
        stage: &str,
        room: i8,
        spawn: u8,
        layer: Option<u8>,
    ) -> CutsceneWrapperExit {
        CutsceneWrapperExit {
            kind,
            transition: ExtractedSceneTransition {
                exit_id: if kind == CutsceneWrapperExitKind::Normal {
                    1
                } else {
                    2
                },
                destination_stage: stage.into(),
                destination_spawn: spawn,
                destination_room: room,
                scene_layer: layer,
                time_hour: None,
                wipe: 0,
                wipe_time: 0,
                raw_hex: "00".into(),
            },
        }
    }

    #[test]
    fn bundled_profile_is_canonical_and_source_hashes_are_current() {
        let profile = bundled_gz2e01_cutscene_outer_runtime_profile().unwrap();
        assert_eq!(profile.id, "gz2e01-demo07-02-outer-runtime");
        let sources = [
            (
                "source.event-data.completion",
                include_bytes!("../../../../../src/d/d_event_data.cpp").as_slice(),
            ),
            (
                "source.event-manager.dispatch",
                include_bytes!("../../../../../src/d/d_event_manager.cpp").as_slice(),
            ),
            (
                "source.stage.revt-bits",
                include_bytes!("../../../../../include/d/d_stage.h").as_slice(),
            ),
        ];
        for (id, bytes) in sources {
            assert_eq!(
                profile
                    .evidence
                    .iter()
                    .find(|record| record.id == id)
                    .unwrap()
                    .source_sha256,
                Digest(Sha256::digest(bytes).into())
            );
        }

        let tower_profile = CutsceneOuterRuntimeProfile::decode_canonical(include_bytes!(
            "../data/cutscene-outer-runtime-profiles/gz2e01-demo07_01.json"
        ))
        .unwrap();
        assert_eq!(tower_profile.id, "gz2e01-demo07-01-outer-runtime");
        assert_eq!(
            tower_profile.state_binding.flow_component_id,
            "cutscene.demo07_01.runtime"
        );
    }

    #[test]
    fn revt_skip_bits_are_decoded_from_raw_bytes() {
        let bytes = decode_hex(
            "0202030304ff64010302ffff0064656d6f30375f303200000000ffff",
            "test",
        )
        .unwrap();
        assert_eq!(bytes.len(), 0x1c);
        assert_eq!(bytes[8] & 1, 1);
        assert_eq!((bytes[8] >> 1) & 3, 1);
    }

    #[test]
    fn phase_graph_is_stepwise_and_rejects_transition_tampering() {
        let profile = bundled_gz2e01_cutscene_outer_runtime_profile().unwrap();
        let scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: ExactContext {
                    content_sha256: Digest([1; 32]),
                    runtime_configuration_sha256: Digest([2; 32]),
                },
            }],
        };
        let normal_exit = fixture_exit(CutsceneWrapperExitKind::Normal, "F_SP116", 0, 20, Some(8));
        let skip_exit = fixture_exit(CutsceneWrapperExitKind::Skip, "R_SP107", 3, 1, None);
        let transitions = compile_dispatch_transitions(
            &scope,
            "demo07_02",
            &profile.state_binding,
            3,
            5,
            Some(&normal_exit),
            Some(&skip_exit),
            &profile.evidence,
        )
        .unwrap();
        assert_eq!(transitions.len(), 5);
        assert!(transitions.iter().any(|transition| {
            transition.transition_kind == TransitionKind::ResourceLoadFailure
                && matches!(
                    transition.activation.effects.as_slice(),
                    [
                        StateOperation::Write { .. },
                        StateOperation::AdvanceFlow { .. }
                    ]
                )
        }));

        let tower_transitions = compile_dispatch_transitions(
            &scope,
            "demo07_01",
            &profile.state_binding,
            17,
            19,
            Some(&fixture_exit(
                CutsceneWrapperExitKind::Normal,
                "R_SP301",
                0,
                20,
                Some(8),
            )),
            Some(&skip_exit),
            &profile.evidence,
        )
        .unwrap();
        assert!(tower_transitions.iter().all(|transition| {
            transition.id.contains("demo07_01") && !transition.label.contains("Castle Town")
        }));

        let steps = vec![
            OuterCompletionStep {
                ordinal: 0,
                kind: OuterCompletionStepKind::PackageModeZeroCompletesPlay,
                flag_id: Some(3),
            },
            OuterCompletionStep {
                ordinal: 1,
                kind: OuterCompletionStepKind::AdvanceToWait,
                flag_id: None,
            },
            OuterCompletionStep {
                ordinal: 2,
                kind: OuterCompletionStepKind::ZeroTimerCompletesWait,
                flag_id: Some(5),
            },
            OuterCompletionStep {
                ordinal: 3,
                kind: OuterCompletionStepKind::EventFinishSatisfied,
                flag_id: Some(5),
            },
        ];
        let mut artifact = ResolvedCutsceneOuterEvent {
            schema: RESOLVED_CUTSCENE_OUTER_EVENT_SCHEMA.into(),
            content_sha256: Digest([1; 32]),
            runtime_configuration_sha256: Digest([2; 32]),
            wrapper_topology_sha256: Digest([3; 32]),
            resolved_package_sha256: Digest([4; 32]),
            runtime_profile_sha256: profile.digest().unwrap(),
            stage_archive_sha256: Digest([5; 32]),
            stage_resource_sha256: Digest([6; 32]),
            event_list_resource_sha256: Digest([7; 32]),
            event_name: "demo07_02".into(),
            map_event_record_index: 0,
            event_list_index: 0,
            event_finish_flags: [5, -1, -1],
            package_play: ResolvedOuterCut {
                cut_index: 0,
                name: "PLAY".into(),
                start_flags: [-1; 3],
                completion_flag: 3,
                next_cut_index: Some(1),
                has_timer_parameter: false,
            },
            package_wait: ResolvedOuterCut {
                cut_index: 1,
                name: "WAIT".into(),
                start_flags: [-1; 3],
                completion_flag: 5,
                next_cut_index: None,
                has_timer_parameter: false,
            },
            completion_steps: steps,
            skip_cut_enabled: true,
            skip_cut_type: 1,
            state_binding: profile.state_binding,
            normal_exit,
            skip_exit,
            evidence: profile.evidence,
            transitions,
            coverage: CutsceneOuterCoverage {
                conditional_missing_stb_completion: CutsceneOuterCoverageStatus::Resolved,
                outer_exit_dispatch: CutsceneOuterCoverageStatus::Resolved,
                actor_corruption_producer: CutsceneOuterCoverageStatus::Unresolved,
                witnessed_corruption_suffix: CutsceneOuterCoverageStatus::Unresolved,
                other_return_place_writers: CutsceneOuterCoverageStatus::Unresolved,
            },
        };
        artifact.validate().unwrap();
        let normal = artifact
            .transitions
            .iter_mut()
            .find(|transition| transition.id.ends_with("normal-exit"))
            .unwrap();
        normal.activation.effects[0] = StateOperation::SetLocation {
            location: SceneLocation {
                stage: "R_SP107".into(),
                room: 3,
                layer: -1,
                spawn: 1,
            },
        };
        assert_eq!(
            artifact.validate().unwrap_err().field(),
            "resolved_cutscene_outer.transitions"
        );
    }
}
