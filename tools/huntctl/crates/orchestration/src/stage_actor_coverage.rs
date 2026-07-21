//! Actor-population coverage derived from completed stage-survey evidence.
//!
//! This audits the read-only end-of-probe actor snapshot. It deliberately does
//! not claim that the per-tick learner observation is complete; that requires
//! a separately retained native learning episode for the same case.

use crate::stage_survey::{
    StageSurveyAttemptOutcome, StageSurveyClassification, StageSurveyLedger,
    stage_survey_case_storage_id,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_world::stage_boot_catalog::StageBootCatalog;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub const STAGE_ACTOR_COVERAGE_SCHEMA_V3: &str = "dusklight-stage-actor-coverage/v3";

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
}

#[derive(Debug, Deserialize)]
struct LearningActorPopulation {
    source_schema: String,
    observed_actor_count: u32,
    retained_actor_count: u32,
    truncated: bool,
    actors: Vec<LearningActor>,
}

#[derive(Debug, Deserialize)]
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
    scale: [f32; 3],
    gravity: f32,
    max_fall_speed: f32,
    eye_position: [f32; 3],
    home_angle: [i16; 3],
    old_angle: [i16; 3],
}

#[derive(Default)]
struct ProfileAccumulator {
    actor_names: BTreeSet<i16>,
    symbolic_names: BTreeSet<String>,
    groups: BTreeSet<u8>,
    enemy_values: BTreeSet<bool>,
    cases: BTreeSet<String>,
    stages: BTreeSet<String>,
    actor_instance_count: u64,
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
                for actor in &snapshot.actors {
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
                    profile.stages.insert(stage.clone());
                    profile.actor_instance_count += 1;
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
            .map(|(profile_name, value)| StageActorProfileCoverage {
                profile_name,
                identity_ambiguous: value.actor_names.len() != 1
                    || value.groups.len() != 1
                    || value.enemy_values.len() != 1,
                actor_names: value.actor_names.into_iter().collect(),
                symbolic_names: value.symbolic_names.into_iter().collect(),
                groups: value.groups.into_iter().collect(),
                enemy_values: value.enemy_values.into_iter().collect(),
                verified_case_count: value.cases.len() as u32,
                stage_count: value.stages.len() as u32,
                actor_instance_count: value.actor_instance_count,
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
            schema: STAGE_ACTOR_COVERAGE_SCHEMA_V3.into(),
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
        if self.schema != STAGE_ACTOR_COVERAGE_SCHEMA_V3
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
        hasher.update(b"dusklight.stage-actor-coverage.identity/v1\0");
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
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
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    candidates.sort();
    for path in candidates {
        let bytes =
            fs::read(&path).map_err(|error| StageActorCoverageError::new(error.to_string()))?;
        if Digest(Sha256::digest(&bytes).into()) == expected_digest {
            return Ok(Some(bytes));
        }
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
    if snapshot.schema != "dusklight.actor-catalog.v7"
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
    {
        return Err("actor_catalog_invariant_mismatch".into());
    }
    if learning.source_schema != "dusklight-learning-observation/v12"
        || learning.truncated
        || learning.observed_actor_count != learning.retained_actor_count
        || learning.retained_actor_count != snapshot.retained_actor_count
        || learning.actors.len() != learning.retained_actor_count as usize
        || unique_learning_generations.len() != learning.actors.len()
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
mod tests {
    use super::*;
    use crate::stage_survey::{
        STAGE_SURVEY_FIDELITY, StageSurveyAttempt, StageSurveyIdentity, StageSurveyObservedOrigin,
        StageSurveyPolicy, StageSurveyProbeKind,
    };
    use dusklight_world::stage_boot_catalog::{
        BootLayerSource, BootLayerSourceKind, BootPointSource, BootPointSourceKind,
        STAGE_BOOT_CATALOG_SCHEMA, StageBootCandidate, StageCatalogStatus, StageInventoryStatus,
    };
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEMPORARY_ROOT: AtomicU64 = AtomicU64::new(0);

    fn digest(byte: u8) -> Digest {
        Digest([byte; 32])
    }

    fn catalog() -> StageBootCatalog {
        StageBootCatalog {
            schema: STAGE_BOOT_CATALOG_SCHEMA.into(),
            known_loader_sha256: None,
            stages: vec![StageCatalogStatus {
                stage: "F_SP103".into(),
                resources_present: true,
                inventory_status: StageInventoryStatus::Complete,
                inventory_sha256: Some(digest(7)),
                diagnostic: None,
                room_count: 1,
                player_spawn_count: 1,
                candidate_count: 1,
            }],
            candidates: vec![StageBootCandidate {
                id: "F_SP103/room/0/point/0/layer/-1".into(),
                stage: "F_SP103".into(),
                room: 0,
                point: 0,
                layer: -1,
                point_sources: vec![BootPointSource {
                    kind: BootPointSourceKind::RetailPlayerSpawn,
                    stable_id: Some("spawn-0".into()),
                }],
                layer_sources: vec![BootLayerSource {
                    kind: BootLayerSourceKind::ResolvedDefault,
                    chunk_tag: None,
                }],
            }],
        }
    }

    fn temporary_root() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_TEMPORARY_ROOT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "stage-actor-coverage-{}-{nonce}-{sequence}",
            std::process::id()
        ))
    }

    fn fixture_with_learning_generation(
        first_learning_generation: u64,
    ) -> (StageBootCatalog, StageSurveyLedger, PathBuf) {
        let catalog = catalog();
        let mut ledger = StageSurveyLedger::new(
            &catalog,
            StageSurveyIdentity {
                catalog_sha256: catalog.digest().unwrap(),
                executable_sha256: digest(1),
                game_data_sha256: digest(2),
                card_fixture_sha256: digest(3),
                observation_schema_sha256: digest(4),
                settings_sha256: digest(5),
            },
            StageSurveyPolicy {
                probe_ticks: 30,
                probe: StageSurveyProbeKind::Neutral,
                host_timeout_millis: 1_000,
                maximum_attempts_per_case: 1,
                fidelity_profile: STAGE_SURVEY_FIDELITY.into(),
            },
        )
        .unwrap();
        let catalog_actors = vec![
            json!({"process_id": 4, "parent_process_id": 4294967295_u32,
                "actor_type": 1, "process_subtype": 2, "parameters": 1, "status": 2,
                "condition": 3, "actor_name": 253, "profile_name": 253,
                "symbolic_name": "fpcNm_ALINK_e", "set_id": 0, "health": 10,
                "home_room": 0, "old_room": 0, "current_room": 0, "group": 1,
                "argument": 0, "pause_flag": 0, "process_init_state": 1,
                "process_create_phase": 2, "cull_type": 3, "demo_actor_id": 4,
                "carry_type": 5, "heap_present": true, "model_present": true,
                "joint_collision_present": false, "home_position": [1.0, 2.0, 3.0],
                "old_position": [3.0, 4.0, 5.0], "current_position": [4.0, 5.0, 6.0],
                "scale": [1.0, 1.0, 1.0], "gravity": -1.0, "max_fall_speed": -20.0,
                "eye_position": [4.0, 7.0, 6.0], "home_angle": [1, 2, 3],
                "old_angle": [4, 5, 6], "is_enemy": false}),
            json!({"process_id": 8, "parent_process_id": 4, "actor_type": 6,
                "process_subtype": 7, "parameters": 3, "status": 4, "condition": 5,
                "actor_name": 291, "profile_name": 291, "symbolic_name": "fpcNm_NPC_e",
                "set_id": 1, "health": 5, "home_room": 0, "old_room": 0,
                "current_room": 0, "group": 2, "argument": -1, "pause_flag": 1,
                "process_init_state": 2, "process_create_phase": 3, "cull_type": 4,
                "demo_actor_id": 5, "carry_type": 6, "heap_present": false,
                "model_present": true, "joint_collision_present": true,
                "home_position": [7.0, 8.0, 9.0], "old_position": [9.0, 10.0, 11.0],
                "current_position": [10.0, 11.0, 12.0], "scale": [2.0, 2.0, 2.0],
                "gravity": -2.0, "max_fall_speed": -30.0,
                "eye_position": [10.0, 13.0, 12.0], "home_angle": [7, 8, 9],
                "old_angle": [10, 11, 12], "is_enemy": true}),
        ];
        let learning_actors = vec![
            json!({"runtime_generation": first_learning_generation,
                "parent_runtime_generation": 4294967295_u32, "actor_type": 1,
                "process_subtype": 2, "parameters": 1, "status": 2, "condition": 3,
                "actor_name": 253, "profile_name": 253, "set_id": 0, "health": 10,
                "home_room": 0, "old_room": 0, "current_room": 0, "group": 1,
                "argument": 0, "pause_flag": 0, "process_init_state": 1,
                "process_create_phase": 2, "cull_type": 3, "demo_actor_id": 4,
                "carry_type": 5, "heap_present": true, "model_present": true,
                "joint_collision_present": false, "home_position": [1.0, 2.0, 3.0],
                "old_position": [3.0, 4.0, 5.0], "current_position": [4.0, 5.0, 6.0],
                "scale": [1.0, 1.0, 1.0], "gravity": -1.0, "max_fall_speed": -20.0,
                "eye_position": [4.0, 7.0, 6.0], "home_angle": [1, 2, 3],
                "old_angle": [4, 5, 6]}),
            json!({"runtime_generation": 8, "parent_runtime_generation": 4,
                "actor_type": 6, "process_subtype": 7, "parameters": 3, "status": 4,
                "condition": 5, "actor_name": 291, "profile_name": 291, "set_id": 1,
                "health": 5, "home_room": 0, "old_room": 0, "current_room": 0,
                "group": 2, "argument": -1, "pause_flag": 1, "process_init_state": 2,
                "process_create_phase": 3, "cull_type": 4, "demo_actor_id": 5,
                "carry_type": 6, "heap_present": false, "model_present": true,
                "joint_collision_present": true, "home_position": [7.0, 8.0, 9.0],
                "old_position": [9.0, 10.0, 11.0], "current_position": [10.0, 11.0, 12.0],
                "scale": [2.0, 2.0, 2.0], "gravity": -2.0, "max_fall_speed": -30.0,
                "eye_position": [10.0, 13.0, 12.0], "home_angle": [7, 8, 9],
                "old_angle": [10, 11, 12]}),
        ];
        let actor_bytes = serde_json::to_vec_pretty(&json!({
            "schema": "dusklight.actor-catalog.v7", "simulation_tick": 29,
            "stage": "F_SP103", "room": 0, "layer": 0, "observed_actor_count": 2,
            "retained_actor_count": 2, "truncated": false, "actors": catalog_actors,
            "learning_actor_population": {
                "source_schema": "dusklight-learning-observation/v12",
                "observed_actor_count": 2, "retained_actor_count": 2,
                "truncated": false, "actors": learning_actors
            }
        }))
        .unwrap();
        let actor_digest = Digest(Sha256::digest(&actor_bytes).into());
        ledger
            .record_attempt(
                &catalog,
                &catalog.candidates[0].id,
                StageSurveyAttempt {
                    number: 1,
                    outcome: StageSurveyAttemptOutcome::Ready,
                    exit_code: Some(0),
                    elapsed_millis: 20,
                    observation_sha256: Some(digest(8)),
                    actor_catalog_sha256: Some(actor_digest),
                    observed_actor_count: Some(2),
                    retained_actor_count: Some(2),
                    actor_catalog_truncated: Some(false),
                    state_sequence_sha256: Some(digest(9)),
                    observed_origin: Some(StageSurveyObservedOrigin {
                        stage: Some("F_SP103".into()),
                        room: 0,
                        point: 0,
                        layer: 0,
                        player_ready: true,
                    }),
                    observed_final: Some(StageSurveyObservedOrigin {
                        stage: Some("F_SP103".into()),
                        room: 0,
                        point: 0,
                        layer: 0,
                        player_ready: true,
                    }),
                    diagnostic_code: None,
                },
            )
            .unwrap();
        let root = temporary_root();
        let artifact_root = root
            .join("cases")
            .join(stage_survey_case_storage_id(&catalog.candidates[0].id).to_string())
            .join("attempt-001-run-00000");
        fs::create_dir_all(&artifact_root).unwrap();
        fs::write(artifact_root.join("actors.json"), actor_bytes).unwrap();
        (catalog, ledger, root)
    }

    fn fixture() -> (StageBootCatalog, StageSurveyLedger, PathBuf) {
        fixture_with_learning_generation(4)
    }

    #[test]
    fn aggregates_verified_complete_actor_snapshots_by_stage_and_profile() {
        let (catalog, ledger, root) = fixture();
        let report = StageActorCoverageReport::build(&catalog, &ledger, &root).unwrap();
        assert_eq!(report.ready_case_count, 1);
        assert_eq!(report.verified_case_count, 1);
        assert_eq!(report.rejected_case_count, 0);
        assert_eq!(report.profiles.len(), 2);
        assert_eq!(report.cases[0].enemy_actor_count, 1);
        assert_eq!(report.stages[0].actor_instance_count, 2);
        assert_ne!(report.report_sha256, Digest::ZERO);
        assert!(report.canonical_bytes().unwrap().ends_with(b"\n"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_artifact_remains_explicit_instead_of_becoming_zero_actors() {
        let (catalog, ledger, root) = fixture();
        fs::remove_dir_all(root.join("cases")).unwrap();
        let report = StageActorCoverageReport::build(&catalog, &ledger, &root).unwrap();
        assert_eq!(report.verified_case_count, 0);
        assert_eq!(report.rejected_case_count, 1);
        assert_eq!(
            report.cases[0].status,
            StageActorEvidenceStatus::ArtifactMissing
        );
        assert!(report.profiles.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_a_complete_but_different_learning_actor_population() {
        let (catalog, ledger, root) = fixture_with_learning_generation(5);
        let report = StageActorCoverageReport::build(&catalog, &ledger, &root).unwrap();
        assert_eq!(report.verified_case_count, 0);
        assert_eq!(report.rejected_case_count, 1);
        assert_eq!(
            report.cases[0].status,
            StageActorEvidenceStatus::ArtifactRejected
        );
        assert_eq!(
            report.cases[0].diagnostic.as_deref(),
            Some("learning_actor_population_invariant_mismatch")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
