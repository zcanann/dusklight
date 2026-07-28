//! Actor-population coverage derived from completed stage-survey evidence.
//!
//! This audits the read-only end-of-probe actor snapshot. It deliberately does
//! not claim that the per-tick learner observation is complete; that requires
//! a separately retained native learning episode for the same case.

use crate::stage_survey::{
    StageSurveyAttemptOutcome, StageSurveyClassification, StageSurveyLedger,
    stage_survey_case_storage_id,
};
use crate::stage_survey_artifact::{compressed_artifact_path, read_survey_artifact};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_evidence::native_episode_shard::{
    LEARNING_OBSERVATION_SCHEMA_V27, LEARNING_OBSERVATION_SCHEMA_V28,
};
use dusklight_world::stage_boot_catalog::StageBootCatalog;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub const STAGE_ACTOR_COVERAGE_SCHEMA_V7: &str = "dusklight-stage-actor-coverage/v7";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageActorEvidenceStatus {
    VerifiedCompleteSnapshot,
    ArtifactMissing,
    ArtifactRejected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageActorCaseCoverage {
    pub candidate_id: String,
    pub stage: String,
    pub room: i8,
    pub point: i16,
    pub requested_layer: i8,
    pub observed_layer: i8,
    pub status: StageActorEvidenceStatus,
    pub actor_catalog_sha256: Digest,
    pub observed_actor_count: u32,
    pub retained_actor_count: u32,
    pub learning_actor_count: u32,
    pub unique_profile_count: u32,
    pub enemy_actor_count: u32,
    pub profile_names: Vec<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageActorProfileCoverage {
    pub profile_name: i16,
    pub actor_names: Vec<i16>,
    pub symbolic_names: Vec<String>,
    pub groups: Vec<u8>,
    pub enemy_values: Vec<bool>,
    pub verified_case_count: u32,
    pub stage_count: u32,
    pub actor_instance_count: u64,
    pub identity_ambiguous: bool,
    pub fields: Vec<StageActorProfileFieldCoverage>,
    pub stages: Vec<StageActorProfileStageCoverage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageActorProfileStageCoverage {
    pub stage: String,
    pub verified_case_count: u32,
    pub actor_instance_count: u64,
    pub fields: Vec<StageActorProfileFieldCoverage>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageActorFieldCoverageStatus {
    Present,
    Varying,
    Absent,
    Ambiguous,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageActorProfileFieldCoverage {
    pub path: String,
    pub status: StageActorFieldCoverageStatus,
    pub sampled_actors: u64,
    pub missing_actors: u64,
    pub value_samples: u64,
    pub null_samples: u64,
    pub true_samples: u64,
    pub distinct_nonnull_values: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageActorStageCoverage {
    pub stage: String,
    pub ready_case_count: u32,
    pub verified_case_count: u32,
    pub actor_instance_count: u64,
    pub profile_names: Vec<i16>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageActorCoverageReport {
    pub schema: String,
    pub catalog_sha256: Digest,
    pub ledger_sha256: Digest,
    pub ready_case_count: u32,
    pub verified_case_count: u32,
    pub rejected_case_count: u32,
    pub cases: Vec<StageActorCaseCoverage>,
    pub stages: Vec<StageActorStageCoverage>,
    pub profiles: Vec<StageActorProfileCoverage>,
    pub report_sha256: Digest,
}

#[derive(Debug, Deserialize)]
struct ActorCatalogSnapshot {
    schema: String,
    simulation_tick: u64,
    stage: String,
    room: i8,
    layer: i8,
    observed_actor_count: u32,
    retained_actor_count: u32,
    truncated: bool,
    actors: Vec<ActorCatalogActor>,
    learning_actor_population: LearningActorPopulation,
}

#[derive(Debug, Deserialize)]
struct ActorCatalogActor {
    process_id: u32,
    parent_process_id: u32,
    actor_type: i32,
    process_subtype: i32,
    parameters: u32,
    status: u32,
    condition: u32,
    actor_name: i16,
    profile_name: i16,
    symbolic_name: String,
    set_id: u16,
    health: i16,
    home_room: i8,
    old_room: i8,
    current_room: i8,
    group: u8,
    argument: i8,
    pause_flag: u8,
    process_init_state: i8,
    process_create_phase: u8,
    cull_type: u8,
    demo_actor_id: u8,
    carry_type: u8,
    heap_present: bool,
    model_present: bool,
    joint_collision_present: bool,
    home_position: [f32; 3],
    old_position: [f32; 3],
    current_position: [f32; 3],
    scale: [f32; 3],
    gravity: f32,
    max_fall_speed: f32,
    eye_position: [f32; 3],
    home_angle: [i16; 3],
    old_angle: [i16; 3],
    is_enemy: bool,
    enemy_base: Option<LearningActorEnemyBase>,
    trigger_volume: Option<LearningActorTriggerVolume>,
    door20: Option<LearningActorDoor20>,
}

#[derive(Debug, Deserialize)]
struct LearningActorPopulation {
    source_schema: String,
    observed_actor_count: u32,
    retained_actor_count: u32,
    truncated: bool,
    actors: Vec<LearningActor>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LearningActor {
    runtime_generation: u64,
    parent_runtime_generation: u32,
    actor_type: i32,
    process_subtype: i32,
    parameters: u32,
    status: u32,
    condition: u32,
    actor_name: i16,
    profile_name: i16,
    set_id: u16,
    health: i16,
    home_room: i8,
    old_room: i8,
    current_room: i8,
    group: u8,
    argument: i8,
    pause_flag: u8,
    process_init_state: i8,
    process_create_phase: u8,
    cull_type: u8,
    demo_actor_id: u8,
    carry_type: u8,
    heap_present: bool,
    model_present: bool,
    joint_collision_present: bool,
    home_position: [f32; 3],
    old_position: [f32; 3],
    current_position: [f32; 3],
    velocity: [f32; 3],
    forward_speed: f32,
    scale: [f32; 3],
    gravity: f32,
    max_fall_speed: f32,
    eye_position: [f32; 3],
    home_angle: [i16; 3],
    old_angle: [i16; 3],
    current_angle: [i16; 3],
    shape_angle: [i16; 3],
    attention: Option<LearningActorAttention>,
    event_participation: Option<LearningActorEventParticipation>,
    return_place_writer: Option<LearningActorReturnPlaceWriter>,
    enemy_base: Option<LearningActorEnemyBase>,
    trigger_volume: Option<LearningActorTriggerVolume>,
    door20: Option<LearningActorDoor20>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct LearningActorDoor20Switch {
    id: u8,
    set: bool,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct LearningActorDoor20 {
    kind: u8,
    door_model: u8,
    front_option: u8,
    back_option: u8,
    front_room: u8,
    back_room: u8,
    exit_number: u8,
    message_door: bool,
    front_switch: Option<LearningActorDoor20Switch>,
    back_switch: Option<LearningActorDoor20Switch>,
    unlock_effect_switch: Option<LearningActorDoor20Switch>,
    front_event: u8,
    back_event: u8,
    message_number: u16,
    action: String,
    active_side: String,
    event_variant: u8,
    locked: bool,
    background_collision_released: bool,
    unlock_effect_triggered: bool,
    key_type: u8,
    enemy_clear_debounce: u8,
    opening_active: bool,
    closing_active: bool,
    door_angle: i16,
    stopper_side: String,
    front_stopper_status: String,
    back_stopper_status: String,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct LearningActorEnemyBase {
    flags: u16,
    throw_mode: u8,
    down_position: [f32; 3],
    head_lock_position: [f32; 3],
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct LearningActorTriggerVolume {
    kind: String,
    shape: String,
    enabled: bool,
    vertical_unbounded: bool,
    behavior: u16,
    center: [f32; 3],
    half_extent: [f32; 3],
    yaw: i16,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LearningActorAttention {
    flags: u32,
    position: [f32; 3],
    distance_indices: [u8; 9],
    auxiliary: i16,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LearningActorEventParticipation {
    command: u16,
    condition: u16,
    event_id: i16,
    map_tool_id: u8,
    index: u8,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LearningActorReturnPlaceWriter {
    save_room: i8,
    save_point: u8,
    switch_room: i8,
    required_event_set: u16,
    required_event_unset: u16,
    required_switch_set: u8,
    required_switch_unset: u8,
    no_telop_clear: bool,
    event_set_satisfied: bool,
    event_unset_satisfied: bool,
    switch_set_satisfied: bool,
    switch_unset_satisfied: bool,
    eligible: bool,
}

#[derive(Default)]
struct ProfileAccumulator {
    actor_names: BTreeSet<i16>,
    symbolic_names: BTreeSet<String>,
    groups: BTreeSet<u8>,
    enemy_values: BTreeSet<bool>,
    cases: BTreeSet<String>,
    actor_instance_count: u64,
    fields: BTreeMap<String, ProfileFieldAccumulator>,
    stages: BTreeMap<String, StageProfileAccumulator>,
}

#[derive(Default)]
struct StageProfileAccumulator {
    cases: BTreeSet<String>,
    actor_instance_count: u64,
    fields: BTreeMap<String, ProfileFieldAccumulator>,
}

#[derive(Default)]
struct ProfileFieldAccumulator {
    sampled_actors: BTreeSet<u64>,
    value_samples: u64,
    null_samples: u64,
    true_samples: u64,
    distinct_nonnull_values: BTreeSet<[u8; 32]>,
}

#[derive(Default)]
struct StageAccumulator {
    ready_cases: u32,
    verified_cases: u32,
    actor_instance_count: u64,
    profiles: BTreeSet<i16>,
}

impl StageActorCoverageReport {
    pub fn build(
        catalog: &StageBootCatalog,
        ledger: &StageSurveyLedger,
        state_root: &Path,
    ) -> Result<Self, StageActorCoverageError> {
        ledger
            .validate(catalog)
            .map_err(|error| StageActorCoverageError::new(error.to_string()))?;
        let catalog_sha256 = catalog
            .digest()
            .map_err(|error| StageActorCoverageError::new(error.to_string()))?;
        let ledger_sha256 = ledger
            .digest(catalog)
            .map_err(|error| StageActorCoverageError::new(error.to_string()))?;
        let candidates = catalog
            .candidates
            .iter()
            .map(|candidate| (candidate.id.as_str(), candidate))
            .collect::<BTreeMap<_, _>>();
        let mut cases = Vec::new();
        let mut profiles = BTreeMap::<i16, ProfileAccumulator>::new();
        let mut stages = BTreeMap::<String, StageAccumulator>::new();

        for case in &ledger.cases {
            if case.classification != Some(StageSurveyClassification::Ready) {
                continue;
            }
            let candidate = candidates[case.candidate_id.as_str()];
            let attempt = case.attempts.last().ok_or_else(|| {
                StageActorCoverageError::new("ready survey case has no retained attempt")
            })?;
            if attempt.outcome != StageSurveyAttemptOutcome::Ready {
                return Err(StageActorCoverageError::new(
                    "ready survey classification does not end in a ready attempt",
                ));
            }
            let expected_digest = attempt.actor_catalog_sha256.ok_or_else(|| {
                StageActorCoverageError::new("ready survey attempt has no actor artifact identity")
            })?;
            let expected_observed = attempt.observed_actor_count.ok_or_else(|| {
                StageActorCoverageError::new("ready survey attempt has no observed actor count")
            })?;
            let expected_retained = attempt.retained_actor_count.ok_or_else(|| {
                StageActorCoverageError::new("ready survey attempt has no retained actor count")
            })?;
            let origin = attempt.observed_origin.as_ref().ok_or_else(|| {
                StageActorCoverageError::new("ready survey attempt has no observed origin")
            })?;
            let final_state = attempt.observed_final.as_ref().unwrap_or(origin);
            let stage = final_state
                .stage
                .clone()
                .unwrap_or_else(|| candidate.stage.clone());
            stages.entry(stage.clone()).or_default().ready_cases += 1;

            let artifact =
                locate_actor_artifact(state_root, &candidate.id, attempt.number, expected_digest)?;
            let (status, snapshot, diagnostic) = match artifact {
                None => (
                    StageActorEvidenceStatus::ArtifactMissing,
                    None,
                    Some("actor_catalog_artifact_missing".into()),
                ),
                Some(bytes) => match validate_snapshot(
                    &bytes,
                    expected_observed,
                    expected_retained,
                    &stage,
                    final_state.room,
                    final_state.layer,
                    u64::from(ledger.policy.probe_ticks - 1),
                ) {
                    Ok(snapshot) => (
                        StageActorEvidenceStatus::VerifiedCompleteSnapshot,
                        Some(snapshot),
                        None,
                    ),
                    Err(error) => (
                        StageActorEvidenceStatus::ArtifactRejected,
                        None,
                        Some(error),
                    ),
                },
            };
            let learning_actor_count = snapshot.as_ref().map_or(0, |snapshot| {
                snapshot.learning_actor_population.retained_actor_count
            });
            let mut profile_names = BTreeSet::new();
            let mut enemy_actor_count = 0_u32;
            if let Some(snapshot) = &snapshot {
                let stage_accumulator = stages.get_mut(&stage).expect("inserted above");
                stage_accumulator.verified_cases += 1;
                stage_accumulator.actor_instance_count += u64::from(snapshot.retained_actor_count);
                for (actor, learning_actor) in snapshot
                    .actors
                    .iter()
                    .zip(&snapshot.learning_actor_population.actors)
                {
                    profile_names.insert(actor.profile_name);
                    enemy_actor_count += u32::from(actor.is_enemy);
                    let profile = profiles.entry(actor.profile_name).or_default();
                    profile.actor_names.insert(actor.actor_name);
                    if !actor.symbolic_name.is_empty() {
                        profile.symbolic_names.insert(actor.symbolic_name.clone());
                    }
                    profile.groups.insert(actor.group);
                    profile.enemy_values.insert(actor.is_enemy);
                    profile.cases.insert(candidate.id.clone());
                    let profile_sample = profile.actor_instance_count;
                    accumulate_learning_actor_fields(
                        &mut profile.fields,
                        learning_actor,
                        profile_sample,
                    )?;
                    profile.actor_instance_count += 1;
                    let stage_profile = profile.stages.entry(stage.clone()).or_default();
                    stage_profile.cases.insert(candidate.id.clone());
                    let stage_sample = stage_profile.actor_instance_count;
                    accumulate_learning_actor_fields(
                        &mut stage_profile.fields,
                        learning_actor,
                        stage_sample,
                    )?;
                    stage_profile.actor_instance_count += 1;
                }
                stage_accumulator
                    .profiles
                    .extend(profile_names.iter().copied());
            }
            cases.push(StageActorCaseCoverage {
                candidate_id: candidate.id.clone(),
                stage,
                room: candidate.room,
                point: candidate.point,
                requested_layer: candidate.layer,
                observed_layer: final_state.layer,
                status,
                actor_catalog_sha256: expected_digest,
                observed_actor_count: expected_observed,
                retained_actor_count: expected_retained,
                learning_actor_count,
                unique_profile_count: profile_names.len() as u32,
                enemy_actor_count,
                profile_names: profile_names.into_iter().collect(),
                diagnostic,
            });
        }

        let profiles = profiles
            .into_iter()
            .map(|(profile_name, value)| {
                let stages = value
                    .stages
                    .into_iter()
                    .map(|(stage, value)| StageActorProfileStageCoverage {
                        stage,
                        verified_case_count: value.cases.len() as u32,
                        actor_instance_count: value.actor_instance_count,
                        fields: finish_profile_fields(value.fields, value.actor_instance_count),
                    })
                    .collect::<Vec<_>>();
                StageActorProfileCoverage {
                    profile_name,
                    identity_ambiguous: profile_identity_ambiguous(
                        value.actor_names.len(),
                        value.symbolic_names.len(),
                        value.groups.len(),
                        value.enemy_values.len(),
                    ),
                    actor_names: value.actor_names.into_iter().collect(),
                    symbolic_names: value.symbolic_names.into_iter().collect(),
                    groups: value.groups.into_iter().collect(),
                    enemy_values: value.enemy_values.into_iter().collect(),
                    verified_case_count: value.cases.len() as u32,
                    stage_count: stages.len() as u32,
                    actor_instance_count: value.actor_instance_count,
                    fields: finish_profile_fields(value.fields, value.actor_instance_count),
                    stages,
                }
            })
            .collect::<Vec<_>>();
        let stages = stages
            .into_iter()
            .map(|(stage, value)| StageActorStageCoverage {
                stage,
                ready_case_count: value.ready_cases,
                verified_case_count: value.verified_cases,
                actor_instance_count: value.actor_instance_count,
                profile_names: value.profiles.into_iter().collect(),
            })
            .collect::<Vec<_>>();
        let verified_case_count = cases
            .iter()
            .filter(|case| case.status == StageActorEvidenceStatus::VerifiedCompleteSnapshot)
            .count() as u32;
        let mut report = Self {
            schema: STAGE_ACTOR_COVERAGE_SCHEMA_V7.into(),
            catalog_sha256,
            ledger_sha256,
            ready_case_count: cases.len() as u32,
            verified_case_count,
            rejected_case_count: cases.len() as u32 - verified_case_count,
            cases,
            stages,
            profiles,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = report.compute_digest()?;
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), StageActorCoverageError> {
        if self.schema != STAGE_ACTOR_COVERAGE_SCHEMA_V7
            || self.catalog_sha256 == Digest::ZERO
            || self.ledger_sha256 == Digest::ZERO
            || self.report_sha256 == Digest::ZERO
            || self.ready_case_count as usize != self.cases.len()
            || self.verified_case_count + self.rejected_case_count != self.ready_case_count
            || self
                .cases
                .windows(2)
                .any(|pair| pair[0].candidate_id >= pair[1].candidate_id)
            || self
                .stages
                .windows(2)
                .any(|pair| pair[0].stage >= pair[1].stage)
            || self
                .profiles
                .windows(2)
                .any(|pair| pair[0].profile_name >= pair[1].profile_name)
            || self.report_sha256 != self.compute_digest()?
        {
            return Err(StageActorCoverageError::new(
                "stage actor coverage report is invalid",
            ));
        }
        let stages = self
            .stages
            .iter()
            .map(|stage| (stage.stage.as_str(), stage))
            .collect::<BTreeMap<_, _>>();
        for profile in &self.profiles {
            if profile.actor_instance_count == 0
                || profile.verified_case_count == 0
                || profile.actor_names.is_empty()
                || profile.groups.is_empty()
                || profile.enemy_values.is_empty()
                || !strictly_ordered(&profile.actor_names)
                || !strictly_ordered(&profile.symbolic_names)
                || !strictly_ordered(&profile.groups)
                || !strictly_ordered(&profile.enemy_values)
                || profile.identity_ambiguous
                    != profile_identity_ambiguous(
                        profile.actor_names.len(),
                        profile.symbolic_names.len(),
                        profile.groups.len(),
                        profile.enemy_values.len(),
                    )
                || profile.stage_count as usize != profile.stages.len()
                || profile.stages.is_empty()
                || profile
                    .stages
                    .windows(2)
                    .any(|pair| pair[0].stage >= pair[1].stage)
                || profile
                    .stages
                    .iter()
                    .map(|stage| stage.actor_instance_count)
                    .sum::<u64>()
                    != profile.actor_instance_count
                || !valid_profile_fields(&profile.fields, profile.actor_instance_count)
                || profile.stages.iter().any(|stage| {
                    let Some(stage_summary) = stages.get(stage.stage.as_str()) else {
                        return true;
                    };
                    stage.stage.is_empty()
                        || stage.verified_case_count == 0
                        || stage.verified_case_count > profile.verified_case_count
                        || stage.verified_case_count > stage_summary.verified_case_count
                        || stage.actor_instance_count == 0
                        || stage.actor_instance_count > stage_summary.actor_instance_count
                        || !stage_summary.profile_names.contains(&profile.profile_name)
                        || !valid_profile_fields(&stage.fields, stage.actor_instance_count)
                })
            {
                return Err(StageActorCoverageError::new(
                    "stage actor profile field coverage is invalid",
                ));
            }
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, StageActorCoverageError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec(self)
            .map_err(|error| StageActorCoverageError::new(error.to_string()))?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn compute_digest(&self) -> Result<Digest, StageActorCoverageError> {
        let mut canonical = self.clone();
        canonical.report_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| StageActorCoverageError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.stage-actor-coverage.identity/v3\0");
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn strictly_ordered<T: Ord>(values: &[T]) -> bool {
    !values.windows(2).any(|pair| pair[0] >= pair[1])
}

fn profile_identity_ambiguous(
    actor_name_count: usize,
    symbolic_name_count: usize,
    group_count: usize,
    enemy_value_count: usize,
) -> bool {
    actor_name_count != 1 || symbolic_name_count > 1 || group_count != 1 || enemy_value_count != 1
}

fn valid_profile_fields(fields: &[StageActorProfileFieldCoverage], samples: u64) -> bool {
    !fields.is_empty()
        && !fields.windows(2).any(|pair| pair[0].path >= pair[1].path)
        && fields.iter().all(|field| {
            !field.path.is_empty()
                && field.sampled_actors > 0
                && field.sampled_actors <= samples
                && field.missing_actors == samples - field.sampled_actors
                && field.value_samples == field.sampled_actors
                && field.null_samples <= field.value_samples
                && field.true_samples <= field.value_samples - field.null_samples
                && field.distinct_nonnull_values <= field.value_samples - field.null_samples
                && field.status == profile_field_status(field, samples)
        })
}

fn profile_field_status(
    field: &StageActorProfileFieldCoverage,
    samples: u64,
) -> StageActorFieldCoverageStatus {
    if field.sampled_actors < samples
        || (field.null_samples > 0 && field.distinct_nonnull_values > 0)
    {
        StageActorFieldCoverageStatus::Ambiguous
    } else if field.null_samples == field.value_samples {
        StageActorFieldCoverageStatus::Absent
    } else if field.distinct_nonnull_values > 1 {
        StageActorFieldCoverageStatus::Varying
    } else {
        StageActorFieldCoverageStatus::Present
    }
}

fn accumulate_learning_actor_fields(
    fields: &mut BTreeMap<String, ProfileFieldAccumulator>,
    actor: &LearningActor,
    actor_sample: u64,
) -> Result<(), StageActorCoverageError> {
    let value = serde_json::to_value(actor)
        .map_err(|error| StageActorCoverageError::new(error.to_string()))?;
    let object = value.as_object().ok_or_else(|| {
        StageActorCoverageError::new("learning actor did not serialize as an object")
    })?;
    for (name, value) in object {
        accumulate_profile_field(name, value, actor_sample, fields)?;
    }
    Ok(())
}

fn accumulate_profile_field(
    path: &str,
    value: &Value,
    actor_sample: u64,
    fields: &mut BTreeMap<String, ProfileFieldAccumulator>,
) -> Result<(), StageActorCoverageError> {
    let field = fields.entry(path.to_string()).or_default();
    field.sampled_actors.insert(actor_sample);
    field.value_samples += 1;
    field.null_samples += u64::from(value.is_null());
    field.true_samples += u64::from(value.as_bool() == Some(true));
    if !value.is_null() {
        let bytes = serde_json::to_vec(value)
            .map_err(|error| StageActorCoverageError::new(error.to_string()))?;
        field
            .distinct_nonnull_values
            .insert(Sha256::digest(bytes).into());
    }
    match value {
        Value::Object(object) => {
            for (name, child) in object {
                accumulate_profile_field(&format!("{path}.{name}"), child, actor_sample, fields)?;
            }
        }
        Value::Array(array) => {
            for (index, child) in array.iter().enumerate() {
                accumulate_profile_field(&format!("{path}[{index}]"), child, actor_sample, fields)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn finish_profile_fields(
    fields: BTreeMap<String, ProfileFieldAccumulator>,
    actor_samples: u64,
) -> Vec<StageActorProfileFieldCoverage> {
    fields
        .into_iter()
        .map(|(path, value)| {
            let sampled_actors = value.sampled_actors.len() as u64;
            let mut field = StageActorProfileFieldCoverage {
                path,
                status: StageActorFieldCoverageStatus::Present,
                sampled_actors,
                missing_actors: actor_samples - sampled_actors,
                value_samples: value.value_samples,
                null_samples: value.null_samples,
                true_samples: value.true_samples,
                distinct_nonnull_values: value.distinct_nonnull_values.len() as u64,
            };
            field.status = profile_field_status(&field, actor_samples);
            field
        })
        .collect()
}

fn locate_actor_artifact(
    state_root: &Path,
    candidate_id: &str,
    attempt_number: u8,
    expected_digest: Digest,
) -> Result<Option<Vec<u8>>, StageActorCoverageError> {
    let case_root = state_root
        .join("cases")
        .join(stage_survey_case_storage_id(candidate_id).to_string());
    let prefix = format!("attempt-{attempt_number:03}-run-");
    let mut candidates = read_directories(&case_root)?
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix))
        })
        .map(|path| path.join("actors.json"))
        .filter(|path| path.is_file() || compressed_artifact_path(path).is_file())
        .collect::<Vec<_>>();
    candidates.sort();
    let mut rejected = None;
    for path in candidates {
        match read_survey_artifact(&path, expected_digest) {
            Ok(Some(bytes)) => return Ok(Some(bytes)),
            Ok(None) => {}
            Err(error) => rejected = Some(error.to_string()),
        }
    }
    if let Some(error) = rejected {
        return Err(StageActorCoverageError::new(error));
    }
    Ok(None)
}

fn read_directories(path: &Path) -> Result<Vec<PathBuf>, StageActorCoverageError> {
    if !path.is_dir() {
        return Ok(Vec::new());
    }
    let mut directories = Vec::new();
    for entry in
        fs::read_dir(path).map_err(|error| StageActorCoverageError::new(error.to_string()))?
    {
        let path = entry
            .map_err(|error| StageActorCoverageError::new(error.to_string()))?
            .path();
        if path.is_dir() {
            directories.push(path);
        }
    }
    Ok(directories)
}

fn validate_snapshot(
    bytes: &[u8],
    expected_observed: u32,
    expected_retained: u32,
    expected_stage: &str,
    expected_room: i8,
    expected_layer: i8,
    expected_simulation_tick: u64,
) -> Result<ActorCatalogSnapshot, String> {
    let snapshot: ActorCatalogSnapshot =
        serde_json::from_slice(bytes).map_err(|_| "actor_catalog_decode_failed".to_owned())?;
    let unique_processes = snapshot
        .actors
        .iter()
        .map(|actor| actor.process_id)
        .collect::<BTreeSet<_>>();
    let learning = &snapshot.learning_actor_population;
    let unique_learning_generations = learning
        .actors
        .iter()
        .map(|actor| actor.runtime_generation)
        .collect::<BTreeSet<_>>();
    if snapshot.schema != "dusklight.actor-catalog.v10"
        || snapshot.simulation_tick != expected_simulation_tick
        || snapshot.stage != expected_stage
        || snapshot.room != expected_room
        || snapshot.layer != expected_layer
        || snapshot.observed_actor_count != expected_observed
        || snapshot.retained_actor_count != expected_retained
        || snapshot.truncated
        || snapshot.observed_actor_count != snapshot.retained_actor_count
        || snapshot.actors.len() != snapshot.retained_actor_count as usize
        || unique_processes.len() != snapshot.actors.len()
        || snapshot.actors.iter().any(|actor| {
            !valid_door20(
                actor.actor_name,
                actor.parameters,
                actor.home_angle,
                actor.door20.as_ref(),
            )
        })
    {
        return Err("actor_catalog_invariant_mismatch".into());
    }
    if !matches!(
        learning.source_schema.as_str(),
        LEARNING_OBSERVATION_SCHEMA_V27 | LEARNING_OBSERVATION_SCHEMA_V28
    ) || learning.truncated
        || learning.observed_actor_count != learning.retained_actor_count
        || learning.retained_actor_count != snapshot.retained_actor_count
        || learning.actors.len() != learning.retained_actor_count as usize
        || unique_learning_generations.len() != learning.actors.len()
        || learning.actors.iter().any(|actor| {
            !valid_door20(
                actor.actor_name,
                actor.parameters,
                actor.home_angle,
                actor.door20.as_ref(),
            )
        })
        || snapshot
            .actors
            .iter()
            .zip(&learning.actors)
            .any(|(catalog, learner)| !same_actor_at_boundary(catalog, learner))
    {
        return Err("learning_actor_population_invariant_mismatch".into());
    }
    Ok(snapshot)
}

fn valid_door20(
    actor_name: i16,
    parameters: u32,
    home_angle: [i16; 3],
    door: Option<&LearningActorDoor20>,
) -> bool {
    const DOOR20: i16 = 0x0e8;
    let Some(door) = door else {
        return actor_name != DOOR20;
    };
    if actor_name != DOOR20 {
        return false;
    }
    let home_x = home_angle[0] as u16;
    let home_z = home_angle[2] as u16;
    let switch_matches = |switch: Option<&LearningActorDoor20Switch>, id: u8| match switch {
        None => id == u8::MAX,
        Some(switch) => id != u8::MAX && switch.id == id,
    };
    door.kind == (parameters & 0x1f) as u8
        && door.door_model == ((parameters >> 5) & 0x7) as u8
        && door.front_option == ((parameters >> 8) & 0x3) as u8
        && door.back_option == ((parameters >> 10) & 0x7) as u8
        && door.front_room == ((parameters >> 13) & 0x3f) as u8
        && door.back_room == ((parameters >> 19) & 0x3f) as u8
        && door.exit_number == ((parameters >> 25) & 0x3f) as u8
        && door.message_door == (parameters >> 31 != 0)
        && switch_matches(door.front_switch.as_ref(), (home_z & 0xff) as u8)
        && switch_matches(door.back_switch.as_ref(), (home_z >> 8) as u8)
        && switch_matches(door.unlock_effect_switch.as_ref(), (home_x >> 8) as u8)
        && door.front_event == (home_x & 0xff) as u8
        && door.back_event == (home_x >> 8) as u8
        && door.message_number == home_x
        && matches!(
            door.action.as_str(),
            "init" | "wait" | "stop_close" | "demo"
        )
        && matches!(door.active_side.as_str(), "front" | "back" | "neither")
        && door.event_variant <= 18
        && (!door.locked || door.front_option == 2 || door.back_option == 2)
        && door.key_type <= 1
        && (door.key_type != 1 || door.kind == 9)
        && door.enemy_clear_debounce <= 65
        && !(door.opening_active && door.closing_active)
        && matches!(door.stopper_side.as_str(), "front" | "back")
        && matches!(
            door.front_stopper_status.as_str(),
            "room_unavailable" | "open" | "closed"
        )
        && matches!(
            door.back_stopper_status.as_str(),
            "room_unavailable" | "open" | "closed"
        )
}

fn same_actor_at_boundary(catalog: &ActorCatalogActor, learner: &LearningActor) -> bool {
    u64::from(catalog.process_id) == learner.runtime_generation
        && catalog.parent_process_id == learner.parent_runtime_generation
        && catalog.actor_type == learner.actor_type
        && catalog.process_subtype == learner.process_subtype
        && catalog.parameters == learner.parameters
        && catalog.status == learner.status
        && catalog.condition == learner.condition
        && catalog.actor_name == learner.actor_name
        && catalog.profile_name == learner.profile_name
        && catalog.set_id == learner.set_id
        && catalog.health == learner.health
        && catalog.home_room == learner.home_room
        && catalog.old_room == learner.old_room
        && catalog.current_room == learner.current_room
        && catalog.group == learner.group
        && catalog.enemy_base == learner.enemy_base
        && catalog.trigger_volume == learner.trigger_volume
        && catalog.door20 == learner.door20
        && catalog.argument == learner.argument
        && catalog.pause_flag == learner.pause_flag
        && catalog.process_init_state == learner.process_init_state
        && catalog.process_create_phase == learner.process_create_phase
        && catalog.cull_type == learner.cull_type
        && catalog.demo_actor_id == learner.demo_actor_id
        && catalog.carry_type == learner.carry_type
        && catalog.heap_present == learner.heap_present
        && catalog.model_present == learner.model_present
        && catalog.joint_collision_present == learner.joint_collision_present
        && same_float3(catalog.home_position, learner.home_position)
        && same_float3(catalog.old_position, learner.old_position)
        && same_float3(catalog.current_position, learner.current_position)
        && same_float3(catalog.scale, learner.scale)
        && same_float(catalog.gravity, learner.gravity)
        && same_float(catalog.max_fall_speed, learner.max_fall_speed)
        && same_float3(catalog.eye_position, learner.eye_position)
        && catalog.home_angle == learner.home_angle
        && catalog.old_angle == learner.old_angle
}

fn same_float3(left: [f32; 3], right: [f32; 3]) -> bool {
    left.into_iter()
        .zip(right)
        .all(|(left, right)| left.to_bits() == right.to_bits())
}

fn same_float(left: f32, right: f32) -> bool {
    left.to_bits() == right.to_bits()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageActorCoverageError(String);

impl StageActorCoverageError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for StageActorCoverageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for StageActorCoverageError {}

#[cfg(test)]
mod tests;
