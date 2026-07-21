//! Resumable, content-bound stage boot survey ledger.

use crate::stage_survey_artifact::{
    compact_survey_artifact, compressed_artifact_path, read_survey_artifact,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::{InputFrame, InputTape, RawPadState, TapeBoot};
use dusklight_evidence::semantic_state_hash::SemanticStateHashSeries;
use dusklight_harness_contracts::run_contract::sha256_artifact_file;
use dusklight_trace::trace::{self, TraceAppliedPads, TraceChannel};
use dusklight_world::stage_boot_catalog::{StageBootCandidate, StageBootCatalog};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const STAGE_SURVEY_LEDGER_SCHEMA: &str = "dusklight-stage-survey-ledger/v1";
pub const STAGE_SURVEY_FIDELITY: &str = "headless-fixed-step-unpaced-30hz/v1";
const MAX_ATTEMPTS_PER_CASE: u8 = 8;
const MAX_DIAGNOSTIC_BYTES: usize = 4096;
const MAX_STDERR_INSPECTION_BYTES: u64 = 1024 * 1024;
const EMPTY_CARD_IDENTITY_DOMAIN: &[u8] = b"dusklight-stage-survey-empty-card/v1\0";
const OBSERVATION_SCHEMA_DOMAIN: &[u8] =
    b"dusklight-stage-survey-all-trace-plus-learning-actor-catalog/v3\0";
const SURVEY_SETTINGS_DOMAIN: &[u8] = b"dusklight-stage-survey-settings/v1\0";
const SURVEY_CVARS: [&str; 4] = [
    "game.instantSaves=true",
    "backend.cardFileType=1",
    "backend.wasPresetChosen=true",
    "game.enableMenuPointer=false",
];
const BUTTON_L: u16 = 0x0040;
const BUTTON_A: u16 = 0x0100;
const BUTTON_B: u16 = 0x0200;
const BUTTON_X: u16 = 0x0400;
const BUTTON_Y: u16 = 0x0800;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageSurveyProbeKind {
    #[default]
    Neutral,
    Movement,
    Camera,
    Targeting,
    BasicActions,
    ContactSweep,
}

impl StageSurveyProbeKind {
    pub const fn minimum_ticks(self) -> u32 {
        match self {
            Self::Neutral => 1,
            Self::Movement | Self::Camera | Self::Targeting => 4,
            Self::BasicActions => 16,
            Self::ContactSweep => 80,
        }
    }
}

fn is_neutral_probe(value: &StageSurveyProbeKind) -> bool {
    *value == StageSurveyProbeKind::Neutral
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageSurveyIdentity {
    pub catalog_sha256: Digest,
    pub executable_sha256: Digest,
    pub game_data_sha256: Digest,
    pub card_fixture_sha256: Digest,
    pub observation_schema_sha256: Digest,
    pub settings_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageSurveyPolicy {
    pub probe_ticks: u32,
    #[serde(default, skip_serializing_if = "is_neutral_probe")]
    pub probe: StageSurveyProbeKind,
    pub host_timeout_millis: u64,
    pub maximum_attempts_per_case: u8,
    pub fidelity_profile: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageSurveyAttemptOutcome {
    Ready,
    NativeReadinessTimeout,
    NativeRejected,
    ProcessExitFailure,
    ProcessCrash,
    HostTimeout,
    ObservationRejected,
    LaunchFailure,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageSurveyClassification {
    Ready,
    InvalidCombination,
    ConditionalSetup,
    UnmetPrerequisite,
    RepeatedReadinessTimeout,
    RepeatedProcessFailure,
    RepeatedHostTimeout,
    ObservationFailure,
    LaunchFailure,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageSurveyObservedOrigin {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    pub room: i8,
    pub point: i16,
    pub layer: i8,
    pub player_ready: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageSurveyAttempt {
    pub number: u8,
    pub outcome: StageSurveyAttemptOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub elapsed_millis: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation_sha256: Option<Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_catalog_sha256: Option<Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_actor_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retained_actor_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_catalog_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_sequence_sha256: Option<Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_origin: Option<StageSurveyObservedOrigin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_final: Option<StageSurveyObservedOrigin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageSurveyArtifactCompactionSummary {
    pub schema: String,
    pub ledger_sha256: Digest,
    pub ready_cases: u32,
    pub verified_artifacts: u32,
    pub compacted_artifacts: u32,
    pub logical_raw_bytes: u64,
    pub stored_bytes: u64,
    pub storage_savings_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageSurveyCase {
    pub candidate_id: String,
    pub attempts: Vec<StageSurveyAttempt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<StageSurveyClassification>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageSurveyLedger {
    pub schema: String,
    pub identity: StageSurveyIdentity,
    pub policy: StageSurveyPolicy,
    pub cases: Vec<StageSurveyCase>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StageSurveyProgress {
    pub total: usize,
    pub finalized: usize,
    pub pending: usize,
    pub attempted: usize,
    pub classifications: BTreeMap<String, usize>,
}

#[derive(Clone, Debug)]
pub struct StageSurveyExecutionConfig {
    pub executable: PathBuf,
    pub game_data: PathBuf,
    pub state_root: PathBuf,
    pub game_args: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ActorCatalogSummary {
    schema: String,
    simulation_tick: u64,
    stage: String,
    room: i8,
    layer: i8,
    observed_actor_count: u32,
    retained_actor_count: u32,
    truncated: bool,
    learning_actor_population: LearningActorPopulationSummary,
    learning_dynamic_collision_population: LearningDynamicCollisionPopulationSummary,
    learning_player_resources: LearningPlayerResourcesSummary,
    learning_player_relationships: LearningPlayerRelationshipsSummary,
    learning_player_collision_solver: LearningPlayerCollisionSolverSummary,
}

#[derive(Debug, Deserialize)]
struct LearningActorPopulationSummary {
    source_schema: String,
    observed_actor_count: u32,
    retained_actor_count: u32,
    truncated: bool,
}

#[derive(Debug, Deserialize)]
struct LearningDynamicCollisionPopulationSummary {
    source_schema: String,
    present: bool,
    retained_collider_count: u32,
    truncated: bool,
    colliders: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct LearningPlayerResourcesSummary {
    source_schema: String,
    present: bool,
}

#[derive(Debug, Deserialize)]
struct LearningPlayerRelationshipsSummary {
    source_schema: String,
    present: bool,
}

#[derive(Debug, Deserialize)]
struct LearningPlayerCollisionSolverSummary {
    source_schema: String,
    present: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageSurveyError(String);

impl StageSurveyError {
    fn invalid(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for StageSurveyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for StageSurveyError {}

impl From<serde_json::Error> for StageSurveyError {
    fn from(value: serde_json::Error) -> Self {
        Self(value.to_string())
    }
}

impl StageSurveyLedger {
    pub fn new(
        catalog: &StageBootCatalog,
        identity: StageSurveyIdentity,
        policy: StageSurveyPolicy,
    ) -> Result<Self, StageSurveyError> {
        catalog
            .validate()
            .map_err(|error| StageSurveyError::invalid(error.to_string()))?;
        let expected_catalog = catalog
            .digest()
            .map_err(|error| StageSurveyError::invalid(error.to_string()))?;
        if identity.catalog_sha256 != expected_catalog {
            return Err(StageSurveyError::invalid(
                "survey identity does not match the canonical boot catalog",
            ));
        }
        let ledger = Self {
            schema: STAGE_SURVEY_LEDGER_SCHEMA.into(),
            identity,
            policy,
            cases: Vec::new(),
        };
        ledger.validate(catalog)?;
        Ok(ledger)
    }

    pub fn validate(&self, catalog: &StageBootCatalog) -> Result<(), StageSurveyError> {
        catalog
            .validate()
            .map_err(|error| StageSurveyError::invalid(error.to_string()))?;
        if self.schema != STAGE_SURVEY_LEDGER_SCHEMA
            || self.identity.catalog_sha256
                != catalog
                    .digest()
                    .map_err(|error| StageSurveyError::invalid(error.to_string()))?
        {
            return Err(StageSurveyError::invalid(
                "survey schema or boot-catalog identity is invalid",
            ));
        }
        for (name, digest) in [
            ("executable", self.identity.executable_sha256),
            ("game data", self.identity.game_data_sha256),
            ("card fixture", self.identity.card_fixture_sha256),
            (
                "observation schema",
                self.identity.observation_schema_sha256,
            ),
            ("settings", self.identity.settings_sha256),
        ] {
            if digest == Digest::ZERO {
                return Err(StageSurveyError::invalid(format!(
                    "survey {name} identity must be nonzero"
                )));
            }
        }
        if self.policy.probe_ticks < self.policy.probe.minimum_ticks()
            || self.policy.host_timeout_millis == 0
            || self.policy.maximum_attempts_per_case == 0
            || self.policy.maximum_attempts_per_case > MAX_ATTEMPTS_PER_CASE
            || self.policy.fidelity_profile != STAGE_SURVEY_FIDELITY
        {
            return Err(StageSurveyError::invalid("survey policy is invalid"));
        }

        let candidates = catalog
            .candidates
            .iter()
            .map(|candidate| (candidate.id.as_str(), candidate))
            .collect::<BTreeMap<_, _>>();
        let mut previous = None;
        for case in &self.cases {
            if !candidates.contains_key(case.candidate_id.as_str())
                || previous.is_some_and(|prior: &str| prior >= case.candidate_id.as_str())
                || case.attempts.is_empty()
                || case.attempts.len() > usize::from(self.policy.maximum_attempts_per_case)
            {
                return Err(StageSurveyError::invalid(
                    "survey cases are unknown, empty, duplicate, or noncanonical",
                ));
            }
            previous = Some(case.candidate_id.as_str());
            for (index, attempt) in case.attempts.iter().enumerate() {
                if usize::from(attempt.number) != index + 1 {
                    return Err(StageSurveyError::invalid(
                        "survey attempt numbers must be contiguous and one-based",
                    ));
                }
                validate_attempt(attempt)?;
            }
            validate_classification(case, self.policy.maximum_attempts_per_case)?;
        }
        Ok(())
    }

    pub fn next_candidates<'a>(
        &self,
        catalog: &'a StageBootCatalog,
        limit: usize,
    ) -> Result<Vec<&'a StageBootCandidate>, StageSurveyError> {
        self.validate(catalog)?;
        if limit == 0 {
            return Err(StageSurveyError::invalid(
                "survey scheduling limit must be nonzero",
            ));
        }
        let cases = self
            .cases
            .iter()
            .map(|case| (case.candidate_id.as_str(), case))
            .collect::<BTreeMap<_, _>>();
        Ok(catalog
            .candidates
            .iter()
            .filter(|candidate| {
                cases.get(candidate.id.as_str()).is_none_or(|case| {
                    case.classification.is_none()
                        && case.attempts.len() < usize::from(self.policy.maximum_attempts_per_case)
                })
            })
            .take(limit)
            .collect())
    }

    pub fn record_attempt(
        &mut self,
        catalog: &StageBootCatalog,
        candidate_id: &str,
        mut attempt: StageSurveyAttempt,
    ) -> Result<(), StageSurveyError> {
        self.validate(catalog)?;
        if !catalog
            .candidates
            .iter()
            .any(|candidate| candidate.id == candidate_id)
        {
            return Err(StageSurveyError::invalid("unknown survey candidate"));
        }
        let index = match self
            .cases
            .binary_search_by(|case| case.candidate_id.as_str().cmp(candidate_id))
        {
            Ok(index) => index,
            Err(index) => {
                self.cases.insert(
                    index,
                    StageSurveyCase {
                        candidate_id: candidate_id.into(),
                        attempts: Vec::new(),
                        classification: None,
                    },
                );
                index
            }
        };
        let case = &mut self.cases[index];
        if case.classification.is_some() {
            return Err(StageSurveyError::invalid(
                "finalized survey case cannot accept another attempt",
            ));
        }
        if case.attempts.len() >= usize::from(self.policy.maximum_attempts_per_case) {
            return Err(StageSurveyError::invalid(
                "survey case exhausted its bounded attempt budget",
            ));
        }
        attempt.number = u8::try_from(case.attempts.len() + 1)
            .map_err(|_| StageSurveyError::invalid("survey attempt number overflowed"))?;
        validate_attempt(&attempt)?;
        case.attempts.push(attempt);
        auto_finalize(case, self.policy.maximum_attempts_per_case);
        self.validate(catalog)
    }

    pub fn classify(
        &mut self,
        catalog: &StageBootCatalog,
        candidate_id: &str,
        classification: StageSurveyClassification,
    ) -> Result<(), StageSurveyError> {
        self.validate(catalog)?;
        let case = self
            .cases
            .iter_mut()
            .find(|case| case.candidate_id == candidate_id)
            .ok_or_else(|| StageSurveyError::invalid("cannot classify an unattempted case"))?;
        if case.classification.is_some() {
            return Err(StageSurveyError::invalid(
                "survey case already has a terminal classification",
            ));
        }
        case.classification = Some(classification);
        self.validate(catalog)
    }

    pub fn progress(
        &self,
        catalog: &StageBootCatalog,
    ) -> Result<StageSurveyProgress, StageSurveyError> {
        self.validate(catalog)?;
        let finalized = self
            .cases
            .iter()
            .filter(|case| case.classification.is_some())
            .count();
        let mut classifications = BTreeMap::new();
        for case in &self.cases {
            if let Some(classification) = case.classification {
                *classifications
                    .entry(classification_name(classification).to_owned())
                    .or_default() += 1;
            }
        }
        Ok(StageSurveyProgress {
            total: catalog.candidates.len(),
            finalized,
            pending: catalog.candidates.len() - finalized,
            attempted: self.cases.len(),
            classifications,
        })
    }

    pub fn canonical_bytes(&self, catalog: &StageBootCatalog) -> Result<Vec<u8>, StageSurveyError> {
        self.validate(catalog)?;
        let mut bytes = serde_json::to_vec(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn decode_canonical(
        bytes: &[u8],
        catalog: &StageBootCatalog,
    ) -> Result<Self, StageSurveyError> {
        let ledger: Self = serde_json::from_slice(bytes)?;
        ledger.validate(catalog)?;
        if ledger.canonical_bytes(catalog)? != bytes {
            return Err(StageSurveyError::invalid(
                "survey ledger is not canonical JSON",
            ));
        }
        Ok(ledger)
    }

    pub fn digest(&self, catalog: &StageBootCatalog) -> Result<Digest, StageSurveyError> {
        Ok(Digest(
            Sha256::digest(self.canonical_bytes(catalog)?).into(),
        ))
    }
}

pub fn stage_survey_identity(
    catalog: &StageBootCatalog,
    executable: &Path,
    game_data: &Path,
    policy: &StageSurveyPolicy,
    game_args: &[String],
) -> Result<StageSurveyIdentity, StageSurveyError> {
    catalog
        .validate()
        .map_err(|error| StageSurveyError::invalid(error.to_string()))?;
    let settings = serde_json::to_vec(&(policy, SURVEY_CVARS, game_args))?;
    Ok(StageSurveyIdentity {
        catalog_sha256: catalog
            .digest()
            .map_err(|error| StageSurveyError::invalid(error.to_string()))?,
        executable_sha256: sha256_artifact_file(executable)
            .map_err(|error| StageSurveyError::invalid(error.to_string()))?,
        game_data_sha256: sha256_artifact_file(game_data)
            .map_err(|error| StageSurveyError::invalid(error.to_string()))?,
        card_fixture_sha256: domain_digest(EMPTY_CARD_IDENTITY_DOMAIN, &[]),
        observation_schema_sha256: domain_digest(OBSERVATION_SCHEMA_DOMAIN, &[]),
        settings_sha256: domain_digest(SURVEY_SETTINGS_DOMAIN, &settings),
    })
}

pub fn execute_stage_survey_attempt(
    candidate: &StageBootCandidate,
    policy: &StageSurveyPolicy,
    attempt_number: u8,
    config: &StageSurveyExecutionConfig,
) -> Result<StageSurveyAttempt, StageSurveyError> {
    if attempt_number == 0
        || policy.probe_ticks < policy.probe.minimum_ticks()
        || policy.host_timeout_millis == 0
        || policy.fidelity_profile != STAGE_SURVEY_FIDELITY
        || !config.executable.is_file()
        || !config.game_data.is_file()
    {
        return Err(StageSurveyError::invalid(
            "stage survey execution configuration is invalid",
        ));
    }

    let case_digest = stage_survey_case_storage_id(&candidate.id);
    let case_root = config
        .state_root
        .join("cases")
        .join(case_digest.to_string());
    fs::create_dir_all(&case_root).map_err(|error| StageSurveyError::invalid(error.to_string()))?;
    let attempt_root = unique_attempt_root(&case_root, attempt_number)?;
    let renderer_cache = config.state_root.join("renderer-cache");
    fs::create_dir_all(&renderer_cache)
        .map_err(|error| StageSurveyError::invalid(error.to_string()))?;

    let tape_path = attempt_root.join("probe.tape");
    let trace_path = attempt_root.join("observation.trace");
    let actor_catalog_path = attempt_root.join("actors.json");
    let stdout_path = attempt_root.join("stdout.log");
    let stderr_path = attempt_root.join("stderr.log");
    let tape = survey_probe_tape(candidate, policy)?;
    fs::write(
        &tape_path,
        tape.encode()
            .map_err(|error| StageSurveyError::invalid(error.to_string()))?,
    )
    .map_err(|error| StageSurveyError::invalid(error.to_string()))?;

    let stdout =
        File::create(&stdout_path).map_err(|error| StageSurveyError::invalid(error.to_string()))?;
    let stderr =
        File::create(&stderr_path).map_err(|error| StageSurveyError::invalid(error.to_string()))?;
    let mut command = Command::new(&config.executable);
    command
        .args(&config.game_args)
        .arg("--dvd")
        .arg(&config.game_data)
        .arg("--input-tape")
        .arg(&tape_path)
        .arg("--automation-tick-budget")
        // The tape player becomes exhausted at the next pre-input boundary.
        // Leave one guard tick so `--exit-after-tape` wins instead of the
        // budget-exhausted exit code after the final recorded simulation tick.
        .arg((u64::from(policy.probe_ticks) + 1).to_string())
        .arg("--automation-data-root")
        .arg(&attempt_root)
        .arg("--renderer-cache-root")
        .arg(&renderer_cache)
        .arg("--gameplay-trace")
        .arg(&trace_path)
        .arg("--gameplay-trace-channels")
        .arg("all")
        .arg("--actor-catalog")
        .arg(&actor_catalog_path)
        .arg("--fixed-step")
        .arg("--unpaced")
        .arg("--headless")
        .arg("--exit-after-tape")
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    for value in SURVEY_CVARS {
        command.arg("--cvar").arg(value);
    }
    configure_hidden_child(&mut command);

    let started = Instant::now();
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            return Ok(failed_attempt(
                attempt_number,
                StageSurveyAttemptOutcome::LaunchFailure,
                None,
                started.elapsed(),
                "process_launch_failed",
            ));
        }
    };
    let timeout = Duration::from_millis(policy.host_timeout_millis);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(failed_attempt(
                    attempt_number,
                    StageSurveyAttemptOutcome::HostTimeout,
                    None,
                    started.elapsed(),
                    "host_timeout",
                ));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(failed_attempt(
                    attempt_number,
                    StageSurveyAttemptOutcome::ProcessExitFailure,
                    None,
                    started.elapsed(),
                    "process_wait_failed",
                ));
            }
        }
    };
    let elapsed = started.elapsed();
    if !status.success() {
        let stderr = read_bounded(&stderr_path).unwrap_or_default();
        let stdout = read_bounded(&stdout_path).unwrap_or_default();
        let diagnostics = format!("{stderr}\n{stdout}");
        let (outcome, diagnostic) = if diagnostics.contains("Stage fixture loading timed out") {
            (
                StageSurveyAttemptOutcome::NativeReadinessTimeout,
                "native_readiness_timeout",
            )
        } else if diagnostics.contains("Stage-boot replay origin assertion failed")
            || diagnostics.contains("Scenario fixture tick-zero establishment failed")
        {
            (
                StageSurveyAttemptOutcome::NativeRejected,
                "native_origin_rejected",
            )
        } else if diagnostics.contains("fork-only game-state observers disabled")
            || diagnostics.contains("fork-only automation observers disabled")
        {
            (
                StageSurveyAttemptOutcome::ObservationRejected,
                "observer_build_required",
            )
        } else if status.code().is_none_or(|code| code < 0) {
            (StageSurveyAttemptOutcome::ProcessCrash, "process_crash")
        } else {
            (
                StageSurveyAttemptOutcome::ProcessExitFailure,
                "process_exit_failure",
            )
        };
        return Ok(failed_attempt(
            attempt_number,
            outcome,
            status.code(),
            elapsed,
            diagnostic,
        ));
    }

    match validate_successful_probe(
        candidate,
        policy,
        &tape,
        &trace_path,
        &actor_catalog_path,
        attempt_number,
        elapsed,
    ) {
        Ok(attempt) => {
            let observation_sha256 = attempt.observation_sha256.ok_or_else(|| {
                StageSurveyError::invalid("ready survey attempt has no observation identity")
            })?;
            let actor_catalog_sha256 = attempt.actor_catalog_sha256.ok_or_else(|| {
                StageSurveyError::invalid("ready survey attempt has no actor identity")
            })?;
            compact_survey_artifact(&trace_path, observation_sha256)
                .map_err(|error| StageSurveyError::invalid(error.to_string()))?;
            compact_survey_artifact(&actor_catalog_path, actor_catalog_sha256)
                .map_err(|error| StageSurveyError::invalid(error.to_string()))?;
            Ok(attempt)
        }
        Err(diagnostic) => Ok(failed_attempt(
            attempt_number,
            StageSurveyAttemptOutcome::ObservationRejected,
            status.code(),
            elapsed,
            diagnostic,
        )),
    }
}

pub fn compact_stage_survey_artifacts(
    catalog: &StageBootCatalog,
    ledger: &StageSurveyLedger,
    state_root: &Path,
) -> Result<StageSurveyArtifactCompactionSummary, StageSurveyError> {
    ledger.validate(catalog)?;
    let ledger_sha256 = ledger.digest(catalog)?;
    let mut summary = StageSurveyArtifactCompactionSummary {
        schema: "dusklight-stage-survey-artifact-compaction/v1".into(),
        ledger_sha256,
        ready_cases: 0,
        verified_artifacts: 0,
        compacted_artifacts: 0,
        logical_raw_bytes: 0,
        stored_bytes: 0,
        storage_savings_bytes: 0,
    };
    for case in &ledger.cases {
        if case.classification != Some(StageSurveyClassification::Ready) {
            continue;
        }
        let attempt = case.attempts.last().ok_or_else(|| {
            StageSurveyError::invalid("ready survey case has no retained attempt")
        })?;
        let observation_sha256 = attempt.observation_sha256.ok_or_else(|| {
            StageSurveyError::invalid("ready survey attempt has no observation identity")
        })?;
        let actor_catalog_sha256 = attempt.actor_catalog_sha256.ok_or_else(|| {
            StageSurveyError::invalid("ready survey attempt has no actor identity")
        })?;
        let case_root = state_root
            .join("cases")
            .join(stage_survey_case_storage_id(&case.candidate_id).to_string());
        compact_matching_artifacts(
            &case_root,
            attempt.number,
            "observation.trace",
            observation_sha256,
            &mut summary,
        )?;
        compact_matching_artifacts(
            &case_root,
            attempt.number,
            "actors.json",
            actor_catalog_sha256,
            &mut summary,
        )?;
        summary.ready_cases = summary
            .ready_cases
            .checked_add(1)
            .ok_or_else(|| StageSurveyError::invalid("ready survey case count overflowed"))?;
    }
    summary.storage_savings_bytes = summary
        .logical_raw_bytes
        .saturating_sub(summary.stored_bytes);
    Ok(summary)
}

fn compact_matching_artifacts(
    case_root: &Path,
    attempt_number: u8,
    file_name: &str,
    expected_digest: Digest,
    summary: &mut StageSurveyArtifactCompactionSummary,
) -> Result<(), StageSurveyError> {
    let prefix = format!("attempt-{attempt_number:03}-run-");
    let mut paths = if case_root.is_dir() {
        fs::read_dir(case_root)
            .map_err(|error| StageSurveyError::invalid(error.to_string()))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.is_dir()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with(&prefix))
            })
            .map(|path| path.join(file_name))
            .filter(|path| path.is_file() || compressed_artifact_path(path).is_file())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    paths.sort();
    let mut matched = false;
    for path in paths {
        let Ok(Some(raw)) = read_survey_artifact(&path, expected_digest) else {
            continue;
        };
        matched = true;
        summary.verified_artifacts = summary
            .verified_artifacts
            .checked_add(1)
            .ok_or_else(|| StageSurveyError::invalid("verified artifact count overflowed"))?;
        let raw_len = u64::try_from(raw.len())
            .map_err(|_| StageSurveyError::invalid("survey artifact length overflowed"))?;
        summary.logical_raw_bytes = summary
            .logical_raw_bytes
            .checked_add(raw_len)
            .ok_or_else(|| StageSurveyError::invalid("logical raw byte count overflowed"))?;
        if compact_survey_artifact(&path, expected_digest)
            .map_err(|error| StageSurveyError::invalid(error.to_string()))?
        {
            summary.compacted_artifacts = summary
                .compacted_artifacts
                .checked_add(1)
                .ok_or_else(|| StageSurveyError::invalid("compacted artifact count overflowed"))?;
        }
        let stored_len = fs::metadata(compressed_artifact_path(&path))
            .map_err(|error| StageSurveyError::invalid(error.to_string()))?
            .len();
        summary.stored_bytes = summary
            .stored_bytes
            .checked_add(stored_len)
            .ok_or_else(|| StageSurveyError::invalid("stored byte count overflowed"))?;
    }
    if !matched {
        return Err(StageSurveyError::invalid(format!(
            "ledger-bound survey artifact is missing or corrupt: {file_name}"
        )));
    }
    Ok(())
}

pub(crate) fn survey_probe_tape(
    candidate: &StageBootCandidate,
    policy: &StageSurveyPolicy,
) -> Result<InputTape, StageSurveyError> {
    if policy.probe_ticks < policy.probe.minimum_ticks() {
        return Err(StageSurveyError::invalid(
            "probe tick count is below the selected probe minimum",
        ));
    }
    let frame_count = usize::try_from(policy.probe_ticks)
        .map_err(|_| StageSurveyError::invalid("probe tick count does not fit memory"))?;
    let mut frame = InputFrame::default();
    frame.owned_ports = 1;
    frame.pads[0] = RawPadState::default();
    let mut frames = vec![frame; frame_count];
    match policy.probe {
        StageSurveyProbeKind::Neutral => {}
        StageSurveyProbeKind::Movement => {
            for frame in active_probe_frames(&mut frames) {
                frame.pads[0].stick_y = 100;
            }
        }
        StageSurveyProbeKind::Camera => {
            for frame in active_probe_frames(&mut frames) {
                frame.pads[0].substick_x = 80;
            }
        }
        StageSurveyProbeKind::Targeting => {
            for frame in active_probe_frames(&mut frames) {
                frame.pads[0].buttons = BUTTON_L;
            }
        }
        StageSurveyProbeKind::BasicActions => {
            let stride = frame_count / 5;
            for (index, button) in [BUTTON_A, BUTTON_B, BUTTON_X, BUTTON_Y]
                .into_iter()
                .enumerate()
            {
                let frame_index = stride * (index + 1);
                frames[frame_index].pads[0].buttons = button;
            }
        }
        StageSurveyProbeKind::ContactSweep => {
            const DIRECTIONS: [(i8, i8); 8] = [
                (0, 100),
                (71, 71),
                (100, 0),
                (71, -71),
                (0, -100),
                (-71, -71),
                (-100, 0),
                (-71, 71),
            ];
            let active = active_probe_frames(&mut frames);
            let active_len = active.len();
            for (index, frame) in active.iter_mut().enumerate() {
                let phase = index * (DIRECTIONS.len() * 2) / active_len;
                if phase % 2 == 0 {
                    let (stick_x, stick_y) = DIRECTIONS[phase / 2];
                    frame.pads[0].stick_x = stick_x;
                    frame.pads[0].stick_y = stick_y;
                }
            }
        }
    }
    Ok(InputTape {
        boot: TapeBoot::Stage {
            stage: candidate.stage.clone(),
            room: candidate.room,
            point: candidate.point,
            layer: candidate.layer,
            save_slot: None,
            fixture: None,
        },
        tick_rate_numerator: 30,
        tick_rate_denominator: 1,
        frames,
    })
}

fn active_probe_frames(frames: &mut [InputFrame]) -> &mut [InputFrame] {
    let margin = frames.len() / 4;
    let end = frames.len() - margin;
    &mut frames[margin..end]
}

fn validate_successful_probe(
    candidate: &StageBootCandidate,
    policy: &StageSurveyPolicy,
    tape: &InputTape,
    trace_path: &Path,
    actor_catalog_path: &Path,
    attempt_number: u8,
    elapsed: Duration,
) -> Result<StageSurveyAttempt, &'static str> {
    let trace_bytes = fs::read(trace_path).map_err(|_| "trace_missing")?;
    let decoded = trace::decode(&trace_bytes).map_err(|_| "trace_decode_failed")?;
    let requested_all = TraceChannel::ALL
        .iter()
        .fold(0_u64, |mask, channel| mask | channel.bit());
    if decoded.boot != tape.boot
        || decoded.tick_rate_numerator != 30
        || decoded.tick_rate_denominator != 1
        || decoded.requested_channels != requested_all
        || decoded.capacity_exhausted
        || decoded.records.len() != policy.probe_ticks as usize
    {
        return Err("trace_envelope_mismatch");
    }
    if decoded
        .records
        .iter()
        .zip(&tape.frames)
        .any(|(record, frame)| !applied_pad_matches_frame(record.applied_pads.as_ref(), frame))
    {
        return Err("trace_applied_pad_mismatch");
    }
    let origin = decoded.records.first().ok_or("trace_empty")?;
    if origin.stage_name != candidate.stage
        || origin.room != candidate.room
        || origin.point != candidate.point
        || (candidate.layer >= 0 && origin.layer != candidate.layer)
        || origin.player_session_process_id.is_none()
    {
        return Err("trace_origin_mismatch");
    }
    let state_series = SemanticStateHashSeries::build(&decoded, &trace_bytes)
        .map_err(|_| "state_series_failed")?;

    let actor_bytes = fs::read(actor_catalog_path).map_err(|_| "actor_catalog_missing")?;
    let actor_catalog: ActorCatalogSummary =
        serde_json::from_slice(&actor_bytes).map_err(|_| "actor_catalog_decode_failed")?;
    let final_observation = decoded.records.last().ok_or("trace_empty")?;
    if actor_catalog.schema != "dusklight.actor-catalog.v8"
        || actor_catalog.simulation_tick != final_observation.simulation_tick
        || actor_catalog.stage != final_observation.stage_name
        || actor_catalog.room != final_observation.room
        || actor_catalog.layer != final_observation.layer
        || actor_catalog.truncated
        || actor_catalog.observed_actor_count != actor_catalog.retained_actor_count
        || actor_catalog.learning_actor_population.source_schema
            != "dusklight-learning-observation/v15"
        || actor_catalog.learning_actor_population.truncated
        || actor_catalog.learning_actor_population.observed_actor_count
            != actor_catalog.learning_actor_population.retained_actor_count
        || actor_catalog.learning_actor_population.retained_actor_count
            != actor_catalog.retained_actor_count
        || actor_catalog
            .learning_dynamic_collision_population
            .source_schema
            != "dusklight-learning-observation/v15"
        || !actor_catalog.learning_dynamic_collision_population.present
        || actor_catalog
            .learning_dynamic_collision_population
            .truncated
        || actor_catalog
            .learning_dynamic_collision_population
            .retained_collider_count
            > 0x500
        || actor_catalog
            .learning_dynamic_collision_population
            .retained_collider_count as usize
            != actor_catalog
                .learning_dynamic_collision_population
                .colliders
                .len()
        || actor_catalog.learning_player_resources.source_schema
            != "dusklight-learning-observation/v15"
        || !actor_catalog.learning_player_resources.present
        || actor_catalog.learning_player_relationships.source_schema
            != "dusklight-learning-observation/v15"
        || !actor_catalog.learning_player_relationships.present
        || actor_catalog.learning_player_collision_solver.source_schema
            != "dusklight-learning-observation/v15"
        || !actor_catalog.learning_player_collision_solver.present
    {
        return Err("actor_catalog_incomplete");
    }

    Ok(StageSurveyAttempt {
        number: attempt_number,
        outcome: StageSurveyAttemptOutcome::Ready,
        exit_code: Some(0),
        elapsed_millis: duration_millis(elapsed),
        observation_sha256: Some(Digest(Sha256::digest(&trace_bytes).into())),
        actor_catalog_sha256: Some(Digest(Sha256::digest(&actor_bytes).into())),
        observed_actor_count: Some(actor_catalog.observed_actor_count),
        retained_actor_count: Some(actor_catalog.retained_actor_count),
        actor_catalog_truncated: Some(actor_catalog.truncated),
        state_sequence_sha256: Some(state_series.series_sha256),
        observed_origin: Some(StageSurveyObservedOrigin {
            stage: Some(origin.stage_name.clone()),
            room: origin.room,
            point: origin.point,
            layer: origin.layer,
            player_ready: true,
        }),
        observed_final: Some(StageSurveyObservedOrigin {
            stage: Some(final_observation.stage_name.clone()),
            room: final_observation.room,
            point: final_observation.point,
            layer: final_observation.layer,
            player_ready: final_observation.player_session_process_id.is_some(),
        }),
        diagnostic_code: None,
    })
}

pub(crate) fn applied_pad_matches_frame(
    applied: Option<&TraceAppliedPads>,
    frame: &InputFrame,
) -> bool {
    let Some(applied) = applied else {
        return false;
    };
    if applied.owned_ports != frame.owned_ports {
        return false;
    }
    (0..applied.pads.len()).all(|port| {
        let bit = 1_u8 << port;
        frame.owned_ports & bit == 0
            || (applied.valid_ports & bit != 0 && applied.pads[port] == frame.pads[port])
    })
}

fn failed_attempt(
    number: u8,
    outcome: StageSurveyAttemptOutcome,
    exit_code: Option<i32>,
    elapsed: Duration,
    diagnostic: &'static str,
) -> StageSurveyAttempt {
    StageSurveyAttempt {
        number,
        outcome,
        exit_code,
        elapsed_millis: duration_millis(elapsed),
        observation_sha256: None,
        actor_catalog_sha256: None,
        observed_actor_count: None,
        retained_actor_count: None,
        actor_catalog_truncated: None,
        state_sequence_sha256: None,
        observed_origin: None,
        observed_final: None,
        diagnostic_code: Some(diagnostic.into()),
    }
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn unique_attempt_root(case_root: &Path, attempt_number: u8) -> Result<PathBuf, StageSurveyError> {
    for recovery in 0_u16..=u16::MAX {
        let path = case_root.join(format!("attempt-{attempt_number:03}-run-{recovery:05}"));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(StageSurveyError::invalid(error.to_string())),
        }
    }
    Err(StageSurveyError::invalid(
        "survey attempt recovery namespace exhausted",
    ))
}

fn read_bounded(path: &Path) -> Result<String, std::io::Error> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_STDERR_INSPECTION_BYTES)
        .read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn domain_digest(domain: &[u8], payload: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(payload);
    Digest(hasher.finalize().into())
}

pub fn stage_survey_case_storage_id(candidate_id: &str) -> Digest {
    domain_digest(b"dusklight-stage-survey-case/v1\0", candidate_id.as_bytes())
}

#[cfg(windows)]
fn configure_hidden_child(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_hidden_child(_: &mut Command) {}

fn validate_attempt(attempt: &StageSurveyAttempt) -> Result<(), StageSurveyError> {
    if attempt.number == 0
        || attempt
            .diagnostic_code
            .as_ref()
            .is_some_and(|code| code.is_empty() || code.len() > MAX_DIAGNOSTIC_BYTES)
        || attempt.observation_sha256 == Some(Digest::ZERO)
        || attempt.actor_catalog_sha256 == Some(Digest::ZERO)
        || attempt.state_sequence_sha256 == Some(Digest::ZERO)
        || attempt
            .observed_origin
            .as_ref()
            .and_then(|origin| origin.stage.as_deref())
            .is_some_and(|stage| {
                stage.is_empty()
                    || stage.len() > 8
                    || !stage.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            })
        || attempt
            .observed_final
            .as_ref()
            .and_then(|final_state| final_state.stage.as_deref())
            .is_some_and(|stage| {
                stage.is_empty()
                    || stage.len() > 8
                    || !stage.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            })
    {
        return Err(StageSurveyError::invalid("survey attempt is invalid"));
    }
    match attempt.outcome {
        StageSurveyAttemptOutcome::Ready
            if attempt.exit_code != Some(0)
                || attempt.observation_sha256.is_none()
                || attempt.actor_catalog_sha256.is_none()
                || attempt.observed_actor_count.is_none()
                || attempt.retained_actor_count != attempt.observed_actor_count
                || attempt.actor_catalog_truncated != Some(false)
                || attempt.state_sequence_sha256.is_none()
                || attempt.observed_origin.is_none() =>
        {
            return Err(StageSurveyError::invalid(
                "ready attempt lacks successful observation evidence",
            ));
        }
        StageSurveyAttemptOutcome::HostTimeout | StageSurveyAttemptOutcome::LaunchFailure
            if attempt.exit_code.is_some() =>
        {
            return Err(StageSurveyError::invalid(
                "host timeout or launch failure cannot claim an exit code",
            ));
        }
        _ => {}
    }
    Ok(())
}

fn validate_classification(
    case: &StageSurveyCase,
    maximum_attempts: u8,
) -> Result<(), StageSurveyError> {
    let Some(classification) = case.classification else {
        if case.attempts.len() >= usize::from(maximum_attempts) {
            return Err(StageSurveyError::invalid(
                "attempt-exhausted case must have a terminal classification",
            ));
        }
        return Ok(());
    };
    let last = case.attempts.last().unwrap().outcome;
    let valid = match classification {
        StageSurveyClassification::Ready => last == StageSurveyAttemptOutcome::Ready,
        StageSurveyClassification::InvalidCombination
        | StageSurveyClassification::ConditionalSetup
        | StageSurveyClassification::UnmetPrerequisite => {
            last == StageSurveyAttemptOutcome::NativeRejected
                || last == StageSurveyAttemptOutcome::NativeReadinessTimeout
        }
        StageSurveyClassification::RepeatedReadinessTimeout => {
            case.attempts.len() == usize::from(maximum_attempts)
                && last == StageSurveyAttemptOutcome::NativeReadinessTimeout
        }
        StageSurveyClassification::RepeatedProcessFailure => {
            case.attempts.len() == usize::from(maximum_attempts)
                && matches!(
                    last,
                    StageSurveyAttemptOutcome::ProcessExitFailure
                        | StageSurveyAttemptOutcome::ProcessCrash
                )
        }
        StageSurveyClassification::RepeatedHostTimeout => {
            case.attempts.len() == usize::from(maximum_attempts)
                && last == StageSurveyAttemptOutcome::HostTimeout
        }
        StageSurveyClassification::ObservationFailure => {
            case.attempts.len() == usize::from(maximum_attempts)
                && last == StageSurveyAttemptOutcome::ObservationRejected
        }
        StageSurveyClassification::LaunchFailure => {
            case.attempts.len() == usize::from(maximum_attempts)
                && last == StageSurveyAttemptOutcome::LaunchFailure
        }
    };
    if !valid {
        return Err(StageSurveyError::invalid(
            "terminal classification disagrees with retained attempts",
        ));
    }
    Ok(())
}

fn auto_finalize(case: &mut StageSurveyCase, maximum_attempts: u8) {
    let Some(last) = case.attempts.last() else {
        return;
    };
    if last.outcome == StageSurveyAttemptOutcome::Ready {
        case.classification = Some(StageSurveyClassification::Ready);
        return;
    }
    if case.attempts.len() < usize::from(maximum_attempts) {
        return;
    }
    case.classification = Some(match last.outcome {
        StageSurveyAttemptOutcome::NativeReadinessTimeout => {
            StageSurveyClassification::RepeatedReadinessTimeout
        }
        StageSurveyAttemptOutcome::ProcessExitFailure | StageSurveyAttemptOutcome::ProcessCrash => {
            StageSurveyClassification::RepeatedProcessFailure
        }
        StageSurveyAttemptOutcome::HostTimeout => StageSurveyClassification::RepeatedHostTimeout,
        StageSurveyAttemptOutcome::ObservationRejected => {
            StageSurveyClassification::ObservationFailure
        }
        StageSurveyAttemptOutcome::LaunchFailure => StageSurveyClassification::LaunchFailure,
        StageSurveyAttemptOutcome::NativeRejected => StageSurveyClassification::UnmetPrerequisite,
        StageSurveyAttemptOutcome::Ready => StageSurveyClassification::Ready,
    });
}

fn classification_name(classification: StageSurveyClassification) -> &'static str {
    match classification {
        StageSurveyClassification::Ready => "ready",
        StageSurveyClassification::InvalidCombination => "invalid_combination",
        StageSurveyClassification::ConditionalSetup => "conditional_setup",
        StageSurveyClassification::UnmetPrerequisite => "unmet_prerequisite",
        StageSurveyClassification::RepeatedReadinessTimeout => "repeated_readiness_timeout",
        StageSurveyClassification::RepeatedProcessFailure => "repeated_process_failure",
        StageSurveyClassification::RepeatedHostTimeout => "repeated_host_timeout",
        StageSurveyClassification::ObservationFailure => "observation_failure",
        StageSurveyClassification::LaunchFailure => "launch_failure",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_world::stage_boot_catalog::{
        BootLayerSource, BootLayerSourceKind, BootPointSource, BootPointSourceKind,
        STAGE_BOOT_CATALOG_SCHEMA, StageCatalogStatus, StageInventoryStatus,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn digest(byte: u8) -> Digest {
        Digest([byte; 32])
    }

    fn catalog() -> StageBootCatalog {
        let candidates = [0_i16, 1]
            .into_iter()
            .map(|point| StageBootCandidate {
                id: format!("F_SP103/room/0/point/{point}/layer/-1"),
                stage: "F_SP103".into(),
                room: 0,
                point,
                layer: -1,
                point_sources: vec![BootPointSource {
                    kind: BootPointSourceKind::RetailPlayerSpawn,
                    stable_id: Some(format!("spawn-{point}")),
                }],
                layer_sources: vec![BootLayerSource {
                    kind: BootLayerSourceKind::ResolvedDefault,
                    chunk_tag: None,
                }],
            })
            .collect::<Vec<_>>();
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
                player_spawn_count: 2,
                candidate_count: 2,
            }],
            candidates,
        }
    }

    fn ledger(catalog: &StageBootCatalog, maximum_attempts: u8) -> StageSurveyLedger {
        StageSurveyLedger::new(
            catalog,
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
                host_timeout_millis: 120_000,
                maximum_attempts_per_case: maximum_attempts,
                fidelity_profile: STAGE_SURVEY_FIDELITY.into(),
            },
        )
        .unwrap()
    }

    fn temporary_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "dusklight-stage-survey-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn failed(outcome: StageSurveyAttemptOutcome) -> StageSurveyAttempt {
        StageSurveyAttempt {
            number: 0,
            outcome,
            exit_code: (outcome != StageSurveyAttemptOutcome::HostTimeout
                && outcome != StageSurveyAttemptOutcome::LaunchFailure)
                .then_some(1),
            elapsed_millis: 50,
            observation_sha256: None,
            actor_catalog_sha256: None,
            observed_actor_count: None,
            retained_actor_count: None,
            actor_catalog_truncated: None,
            state_sequence_sha256: None,
            observed_origin: None,
            observed_final: None,
            diagnostic_code: Some("test_failure".into()),
        }
    }

    fn ready() -> StageSurveyAttempt {
        StageSurveyAttempt {
            number: 0,
            outcome: StageSurveyAttemptOutcome::Ready,
            exit_code: Some(0),
            elapsed_millis: 40,
            observation_sha256: Some(digest(8)),
            actor_catalog_sha256: Some(digest(9)),
            observed_actor_count: Some(48),
            retained_actor_count: Some(48),
            actor_catalog_truncated: Some(false),
            state_sequence_sha256: Some(digest(10)),
            observed_origin: Some(StageSurveyObservedOrigin {
                stage: Some("F_SP103".into()),
                room: 0,
                point: 0,
                layer: 3,
                player_ready: true,
            }),
            observed_final: Some(StageSurveyObservedOrigin {
                stage: Some("F_SP103".into()),
                room: 0,
                point: 0,
                layer: 3,
                player_ready: true,
            }),
            diagnostic_code: None,
        }
    }

    #[test]
    fn resume_schedules_only_unfinalized_candidates() {
        let catalog = catalog();
        let mut ledger = ledger(&catalog, 2);
        let first = catalog.candidates[0].id.clone();
        ledger
            .record_attempt(
                &catalog,
                &first,
                failed(StageSurveyAttemptOutcome::NativeReadinessTimeout),
            )
            .unwrap();
        assert_eq!(ledger.next_candidates(&catalog, 10).unwrap().len(), 2);
        ledger
            .record_attempt(
                &catalog,
                &first,
                failed(StageSurveyAttemptOutcome::NativeReadinessTimeout),
            )
            .unwrap();
        assert_eq!(
            ledger.cases[0].classification,
            Some(StageSurveyClassification::RepeatedReadinessTimeout)
        );
        assert_eq!(ledger.next_candidates(&catalog, 10).unwrap().len(), 1);
    }

    #[test]
    fn ready_case_finalizes_immediately_and_round_trips_canonically() {
        let catalog = catalog();
        let mut ledger = ledger(&catalog, 3);
        ledger
            .record_attempt(&catalog, &catalog.candidates[0].id, ready())
            .unwrap();
        assert_eq!(
            ledger.progress(&catalog).unwrap(),
            StageSurveyProgress {
                total: 2,
                finalized: 1,
                pending: 1,
                attempted: 1,
                classifications: BTreeMap::from([("ready".into(), 1)]),
            }
        );
        let bytes = ledger.canonical_bytes(&catalog).unwrap();
        assert_eq!(
            StageSurveyLedger::decode_canonical(&bytes, &catalog).unwrap(),
            ledger
        );
        assert_ne!(ledger.digest(&catalog).unwrap(), Digest::ZERO);
    }

    #[test]
    fn compaction_preserves_ledger_identity_and_is_repeatable() {
        let catalog = catalog();
        let mut ledger = ledger(&catalog, 1);
        let observation = vec![0x31; 192 * 1024];
        let actors = vec![0x72; 96 * 1024];
        let mut attempt = ready();
        attempt.observation_sha256 = Some(Digest(Sha256::digest(&observation).into()));
        attempt.actor_catalog_sha256 = Some(Digest(Sha256::digest(&actors).into()));
        ledger
            .record_attempt(&catalog, &catalog.candidates[0].id, attempt)
            .unwrap();

        let root = temporary_root("artifact-compaction");
        let attempt_number = ledger.cases[0].attempts[0].number;
        let artifact_root = root
            .join("cases")
            .join(stage_survey_case_storage_id(&catalog.candidates[0].id).to_string())
            .join(format!("attempt-{attempt_number:03}-run-00000"));
        fs::create_dir_all(&artifact_root).unwrap();
        let observation_path = artifact_root.join("observation.trace");
        let actor_path = artifact_root.join("actors.json");
        fs::write(&observation_path, &observation).unwrap();
        fs::write(&actor_path, &actors).unwrap();

        let first = compact_stage_survey_artifacts(&catalog, &ledger, &root).unwrap();
        assert_eq!(
            first.schema,
            "dusklight-stage-survey-artifact-compaction/v1"
        );
        assert_eq!(first.ledger_sha256, ledger.digest(&catalog).unwrap());
        assert_eq!(first.ready_cases, 1);
        assert_eq!(first.verified_artifacts, 2);
        assert_eq!(first.compacted_artifacts, 2);
        assert_eq!(
            first.logical_raw_bytes,
            u64::try_from(observation.len() + actors.len()).unwrap()
        );
        assert!(first.stored_bytes < first.logical_raw_bytes);
        assert_eq!(
            first.storage_savings_bytes,
            first.logical_raw_bytes - first.stored_bytes
        );
        assert!(!observation_path.exists());
        assert!(!actor_path.exists());

        let second = compact_stage_survey_artifacts(&catalog, &ledger, &root).unwrap();
        assert_eq!(second.compacted_artifacts, 0);
        assert_eq!(second.verified_artifacts, first.verified_artifacts);
        assert_eq!(second.logical_raw_bytes, first.logical_raw_bytes);
        assert_eq!(second.stored_bytes, first.stored_bytes);
        assert_eq!(second.storage_savings_bytes, first.storage_savings_bytes);
        assert_eq!(
            read_survey_artifact(
                &observation_path,
                ledger.cases[0].attempts[0].observation_sha256.unwrap()
            )
            .unwrap(),
            Some(observation)
        );
        assert_eq!(
            read_survey_artifact(
                &actor_path,
                ledger.cases[0].attempts[0].actor_catalog_sha256.unwrap()
            )
            .unwrap(),
            Some(actors)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn identity_mismatch_and_attempt_after_finalization_fail_closed() {
        let catalog = catalog();
        let mut ledger = ledger(&catalog, 2);
        let candidate = catalog.candidates[0].id.clone();
        ledger
            .record_attempt(&catalog, &candidate, ready())
            .unwrap();
        assert!(
            ledger
                .record_attempt(
                    &catalog,
                    &candidate,
                    failed(StageSurveyAttemptOutcome::ProcessCrash)
                )
                .is_err()
        );
        let mut other = catalog.clone();
        other.candidates[0].point = 99;
        other.candidates[0].id = "F_SP103/room/0/point/99/layer/-1".into();
        assert!(ledger.validate(&other).is_err());
    }

    #[test]
    fn neutral_probe_owns_only_port_zero_and_preserves_exact_boot_origin() {
        let candidate = &catalog().candidates[0];
        let tape = survey_probe_tape(
            candidate,
            &StageSurveyPolicy {
                probe_ticks: 30,
                probe: StageSurveyProbeKind::Neutral,
                host_timeout_millis: 120_000,
                maximum_attempts_per_case: 1,
                fidelity_profile: STAGE_SURVEY_FIDELITY.into(),
            },
        )
        .unwrap();
        assert_eq!(
            tape.boot,
            TapeBoot::Stage {
                stage: "F_SP103".into(),
                room: 0,
                point: 0,
                layer: -1,
                save_slot: None,
                fixture: None,
            }
        );
        assert_eq!(tape.frames.len(), 30);
        assert!(
            tape.frames.iter().all(|frame| {
                frame.owned_ports == 1 && frame.pads == [RawPadState::default(); 4]
            })
        );
    }

    #[test]
    fn generic_probe_profiles_change_only_the_declared_pad_factor() {
        let candidate = &catalog().candidates[0];
        let build = |probe| {
            survey_probe_tape(
                candidate,
                &StageSurveyPolicy {
                    probe_ticks: 20,
                    probe,
                    host_timeout_millis: 120_000,
                    maximum_attempts_per_case: 1,
                    fidelity_profile: STAGE_SURVEY_FIDELITY.into(),
                },
            )
            .unwrap()
        };

        let movement = build(StageSurveyProbeKind::Movement);
        assert!(movement.frames[..5].iter().all(neutral_frame));
        assert!(movement.frames[15..].iter().all(neutral_frame));
        assert!(movement.frames[5..15].iter().all(|frame| {
            frame.owned_ports == 1
                && frame.pads[0].stick_y == 100
                && frame.pads[0].stick_x == 0
                && frame.pads[0].buttons == 0
        }));

        let camera = build(StageSurveyProbeKind::Camera);
        assert!(camera.frames[5..15].iter().all(|frame| {
            frame.pads[0].substick_x == 80
                && frame.pads[0].substick_y == 0
                && frame.pads[0].buttons == 0
        }));

        let targeting = build(StageSurveyProbeKind::Targeting);
        assert!(
            targeting.frames[5..15]
                .iter()
                .all(|frame| frame.pads[0].buttons == BUTTON_L)
        );

        let actions = build(StageSurveyProbeKind::BasicActions);
        let presses = actions
            .frames
            .iter()
            .filter_map(|frame| (frame.pads[0].buttons != 0).then_some(frame.pads[0].buttons))
            .collect::<Vec<_>>();
        assert_eq!(presses, [BUTTON_A, BUTTON_B, BUTTON_X, BUTTON_Y]);
        assert!(actions.frames.iter().all(|frame| {
            frame.owned_ports == 1
                && frame.pads[0].stick_x == 0
                && frame.pads[0].stick_y == 0
                && frame.pads[0].substick_x == 0
                && frame.pads[0].substick_y == 0
        }));
    }

    #[test]
    fn contact_sweep_covers_eight_directions_with_neutral_release_phases() {
        let tape = survey_probe_tape(
            &catalog().candidates[0],
            &StageSurveyPolicy {
                probe_ticks: 80,
                probe: StageSurveyProbeKind::ContactSweep,
                host_timeout_millis: 120_000,
                maximum_attempts_per_case: 1,
                fidelity_profile: STAGE_SURVEY_FIDELITY.into(),
            },
        )
        .unwrap();
        assert!(tape.frames[..20].iter().all(neutral_frame));
        assert!(tape.frames[60..].iter().all(neutral_frame));
        let active = &tape.frames[20..60];
        for direction in [
            (0, 100),
            (71, 71),
            (100, 0),
            (71, -71),
            (0, -100),
            (-71, -71),
            (-100, 0),
            (-71, 71),
        ] {
            assert!(
                active
                    .iter()
                    .any(|frame| { (frame.pads[0].stick_x, frame.pads[0].stick_y) == direction })
            );
        }
        assert!(active.iter().any(neutral_frame));
        assert!(tape.frames.iter().all(|frame| {
            frame.owned_ports == 1
                && frame.pads[0].buttons == 0
                && frame.pads[0].substick_x == 0
                && frame.pads[0].substick_y == 0
                && frame.pads[0].trigger_left == 0
                && frame.pads[0].trigger_right == 0
        }));
    }

    #[test]
    fn neutral_policy_preserves_legacy_canonical_shape_and_probe_minima_fail_closed() {
        let catalog = catalog();
        let neutral = ledger(&catalog, 1);
        let bytes = neutral.canonical_bytes(&catalog).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(!text.contains("\"probe\":"));
        assert_eq!(
            StageSurveyLedger::decode_canonical(&bytes, &catalog).unwrap(),
            neutral
        );

        let mut movement = neutral.clone();
        movement.policy.probe = StageSurveyProbeKind::Movement;
        movement.policy.probe_ticks = 2;
        assert!(movement.validate(&catalog).is_err());
        assert!(survey_probe_tape(&catalog.candidates[0], &movement.policy).is_err());

        movement.policy.probe_ticks = 4;
        let bytes = movement.canonical_bytes(&catalog).unwrap();
        assert!(
            std::str::from_utf8(&bytes)
                .unwrap()
                .contains("\"probe\":\"movement\"")
        );

        movement.policy.probe = StageSurveyProbeKind::ContactSweep;
        movement.policy.probe_ticks = 79;
        assert!(movement.validate(&catalog).is_err());
        movement.policy.probe_ticks = 80;
        assert!(movement.validate(&catalog).is_ok());
    }

    #[test]
    fn probe_acceptance_requires_exact_consumed_pad_on_every_owned_port() {
        let mut frame = InputFrame {
            owned_ports: 1,
            ..InputFrame::default()
        };
        frame.pads[0].stick_y = 100;
        let mut applied = TraceAppliedPads {
            valid_ports: 1,
            owned_ports: 1,
            pads: frame.pads,
        };
        assert!(applied_pad_matches_frame(Some(&applied), &frame));

        applied.pads[0].stick_y = 99;
        assert!(!applied_pad_matches_frame(Some(&applied), &frame));
        applied.pads[0] = frame.pads[0];
        applied.valid_ports = 0;
        assert!(!applied_pad_matches_frame(Some(&applied), &frame));
        applied.valid_ports = 1;
        applied.owned_ports = 3;
        assert!(!applied_pad_matches_frame(Some(&applied), &frame));
        assert!(!applied_pad_matches_frame(None, &frame));

        applied.owned_ports = 1;
        applied.pads[1].buttons = BUTTON_A;
        assert!(applied_pad_matches_frame(Some(&applied), &frame));
    }

    fn neutral_frame(frame: &InputFrame) -> bool {
        frame.owned_ports == 1 && frame.pads == [RawPadState::default(); 4]
    }
}
