//! Resumable, content-bound stage boot survey ledger.

use dusklight_automation_contracts::artifact::Digest;
use dusklight_world::stage_boot_catalog::{StageBootCandidate, StageBootCatalog};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const STAGE_SURVEY_LEDGER_SCHEMA: &str = "dusklight-stage-survey-ledger/v1";
pub const STAGE_SURVEY_FIDELITY: &str = "headless-fixed-step-unpaced-30hz/v1";
const MAX_ATTEMPTS_PER_CASE: u8 = 8;
const MAX_DIAGNOSTIC_BYTES: usize = 4096;

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
    pub state_sequence_sha256: Option<Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_origin: Option<StageSurveyObservedOrigin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic_code: Option<String>,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageSurveyProgress {
    pub total: usize,
    pub finalized: usize,
    pub pending: usize,
    pub attempted: usize,
    pub classifications: BTreeMap<String, usize>,
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
        if self.policy.probe_ticks == 0
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

fn validate_attempt(attempt: &StageSurveyAttempt) -> Result<(), StageSurveyError> {
    if attempt.number == 0
        || attempt
            .diagnostic_code
            .as_ref()
            .is_some_and(|code| code.is_empty() || code.len() > MAX_DIAGNOSTIC_BYTES)
        || attempt.observation_sha256 == Some(Digest::ZERO)
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
    {
        return Err(StageSurveyError::invalid("survey attempt is invalid"));
    }
    match attempt.outcome {
        StageSurveyAttemptOutcome::Ready
            if attempt.exit_code != Some(0)
                || attempt.observation_sha256.is_none()
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
                host_timeout_millis: 120_000,
                maximum_attempts_per_case: maximum_attempts,
                fidelity_profile: STAGE_SURVEY_FIDELITY.into(),
            },
        )
        .unwrap()
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
            state_sequence_sha256: None,
            observed_origin: None,
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
            state_sequence_sha256: Some(digest(9)),
            observed_origin: Some(StageSurveyObservedOrigin {
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
}
