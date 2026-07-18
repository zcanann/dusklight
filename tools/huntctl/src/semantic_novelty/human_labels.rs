//! Append-only human discovery labels exported as non-authoritative corpus metadata.

use super::headful_review::{
    HumanClassificationChoice, HumanClassificationRequest, HumanClassificationStatus,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const HUMAN_DISCOVERY_LABEL_SCHEMA: &str = "dusklight-human-discovery-label/v1";
pub const CORPUS_HUMAN_LABEL_METADATA_SCHEMA: &str = "dusklight-corpus-human-label-metadata/v1";
pub const MAX_HUMAN_LABELS: usize = 65_536;
pub const MAX_HUMAN_LABEL_NOTE_BYTES: usize = 2_048;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HumanLabelSubmission {
    pub reviewer_id: String,
    pub classification: HumanClassificationChoice,
    pub note: Option<String>,
    pub recorded_unix_millis: u64,
    pub supersedes_label_identity: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HumanDiscoveryLabel {
    pub schema: &'static str,
    pub label_identity: String,
    pub sequence: u64,
    pub classification_request_identity: String,
    pub source_artifact_identity: String,
    pub headful_replay_identity: String,
    pub replay_boundary_identity: String,
    pub objective_id: String,
    pub objective_definition_sha256: String,
    pub reviewer_id: String,
    pub classification: HumanClassificationChoice,
    pub note: Option<String>,
    pub recorded_unix_millis: u64,
    pub supersedes_label_identity: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CorpusHumanLabelMetadata {
    pub schema: &'static str,
    pub source_artifact_identity: String,
    pub objective_id: String,
    pub objective_definition_sha256: String,
    pub labels: Vec<HumanDiscoveryLabel>,
    pub replay_authority: bool,
    pub objective_rewrite_authority: bool,
}

#[derive(Clone, Debug, Default)]
pub struct HumanLabelLedger {
    records: Vec<HumanDiscoveryLabel>,
}

#[derive(Debug)]
pub struct HumanLabelError(String);

impl fmt::Display for HumanLabelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for HumanLabelError {}

impl HumanLabelLedger {
    pub fn append(
        &mut self,
        request: &HumanClassificationRequest,
        submission: HumanLabelSubmission,
    ) -> Result<&HumanDiscoveryLabel, HumanLabelError> {
        if self.records.len() >= MAX_HUMAN_LABELS {
            return Err(HumanLabelError(format!(
                "human label ledger exceeds {MAX_HUMAN_LABELS} records"
            )));
        }
        validate_request(request)?;
        validate_submission(request, &submission)?;
        let prior_for_request = self.records.iter().rev().find(|record| {
            record.classification_request_identity == request.request_identity
                && record.source_artifact_identity == request.source_artifact_identity
        });
        match (&submission.supersedes_label_identity, prior_for_request) {
            (None, Some(_)) => {
                return Err(HumanLabelError(
                    "a later label must explicitly supersede the prior append-only record".into(),
                ));
            }
            (Some(identity), Some(prior)) if identity != &prior.label_identity => {
                return Err(HumanLabelError(
                    "superseded label is not the latest record for this review request".into(),
                ));
            }
            (Some(_), None) => {
                return Err(HumanLabelError(
                    "superseded label does not exist for this review request".into(),
                ));
            }
            _ => {}
        }
        let sequence = self.records.len() as u64;
        let label_identity = label_identity(request, &submission, sequence);
        self.records.push(HumanDiscoveryLabel {
            schema: HUMAN_DISCOVERY_LABEL_SCHEMA,
            label_identity,
            sequence,
            classification_request_identity: request.request_identity.clone(),
            source_artifact_identity: request.source_artifact_identity.clone(),
            headful_replay_identity: request.headful_replay_identity.clone(),
            replay_boundary_identity: request.replay_boundary_identity.clone(),
            objective_id: request.objective_id.clone(),
            objective_definition_sha256: request.objective_definition_sha256.clone(),
            reviewer_id: submission.reviewer_id,
            classification: submission.classification,
            note: submission.note,
            recorded_unix_millis: submission.recorded_unix_millis,
            supersedes_label_identity: submission.supersedes_label_identity,
        });
        Ok(self.records.last().expect("a label was just appended"))
    }

    pub fn records(&self) -> &[HumanDiscoveryLabel] {
        &self.records
    }

    pub fn corpus_metadata(
        &self,
        source_artifact_identity: &str,
        objective_id: &str,
        objective_definition_sha256: &str,
    ) -> Result<CorpusHumanLabelMetadata, HumanLabelError> {
        validate_sha256("source artifact identity", source_artifact_identity)?;
        validate_sha256("objective definition", objective_definition_sha256)?;
        if objective_id.trim().is_empty() {
            return Err(HumanLabelError("objective ID is empty".into()));
        }
        let labels = self
            .records
            .iter()
            .filter(|record| record.source_artifact_identity == source_artifact_identity)
            .cloned()
            .collect::<Vec<_>>();
        if labels.iter().any(|record| {
            record.objective_id != objective_id
                || record.objective_definition_sha256 != objective_definition_sha256
        }) {
            return Err(HumanLabelError(
                "human labels disagree with the immutable corpus objective identity".into(),
            ));
        }
        Ok(CorpusHumanLabelMetadata {
            schema: CORPUS_HUMAN_LABEL_METADATA_SCHEMA,
            source_artifact_identity: source_artifact_identity.into(),
            objective_id: objective_id.into(),
            objective_definition_sha256: objective_definition_sha256.into(),
            labels,
            replay_authority: false,
            objective_rewrite_authority: false,
        })
    }
}

fn validate_request(request: &HumanClassificationRequest) -> Result<(), HumanLabelError> {
    if request.status != HumanClassificationStatus::Pending {
        return Err(HumanLabelError(
            "human classification request is not pending".into(),
        ));
    }
    for (label, value) in [
        ("classification request", &request.request_identity),
        ("source artifact", &request.source_artifact_identity),
        ("headful replay", &request.headful_replay_identity),
        ("replay boundary", &request.replay_boundary_identity),
        ("objective definition", &request.objective_definition_sha256),
    ] {
        validate_sha256(label, value)?;
    }
    if request.objective_id.trim().is_empty() {
        return Err(HumanLabelError("objective ID is empty".into()));
    }
    Ok(())
}

fn validate_submission(
    request: &HumanClassificationRequest,
    submission: &HumanLabelSubmission,
) -> Result<(), HumanLabelError> {
    if submission.reviewer_id.trim().is_empty()
        || submission.reviewer_id.len() > 192
        || submission.reviewer_id.chars().any(char::is_control)
    {
        return Err(HumanLabelError("reviewer ID is invalid".into()));
    }
    if !request.choices.contains(&submission.classification) {
        return Err(HumanLabelError(
            "classification is not offered by the immutable review request".into(),
        ));
    }
    if submission.note.as_ref().is_some_and(|note| {
        note.len() > MAX_HUMAN_LABEL_NOTE_BYTES || note.chars().any(char::is_control)
    }) {
        return Err(HumanLabelError("human label note is invalid".into()));
    }
    if let Some(identity) = &submission.supersedes_label_identity {
        validate_sha256("superseded label identity", identity)?;
    }
    Ok(())
}

fn label_identity(
    request: &HumanClassificationRequest,
    submission: &HumanLabelSubmission,
    sequence: u64,
) -> String {
    let encoded = serde_json::to_vec(&(
        &request.request_identity,
        &request.source_artifact_identity,
        &request.headful_replay_identity,
        &request.replay_boundary_identity,
        &request.objective_id,
        &request.objective_definition_sha256,
        &submission.reviewer_id,
        submission.classification,
        &submission.note,
        submission.recorded_unix_millis,
        &submission.supersedes_label_identity,
        sequence,
    ))
    .expect("human label identity is serializable");
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-human-discovery-label/v1\0");
    hasher.update((encoded.len() as u64).to_le_bytes());
    hasher.update(encoded);
    format!("{:x}", hasher.finalize())
}

fn validate_sha256(label: &str, digest: &str) -> Result<(), HumanLabelError> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(HumanLabelError(format!("{label} is not lowercase SHA-256")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic_novelty::archive::DiscoveryOutcomeKind;

    fn request() -> HumanClassificationRequest {
        HumanClassificationRequest {
            schema: super::super::headful_review::HUMAN_CLASSIFICATION_REQUEST_SCHEMA,
            request_identity: "11".repeat(32),
            source_artifact_identity: "22".repeat(32),
            headful_replay_identity: "33".repeat(32),
            replay_boundary_identity: "44".repeat(32),
            objective_id: "unseen-contact".into(),
            objective_definition_sha256: "55".repeat(32),
            semantic_replay_matched: true,
            outcome: DiscoveryOutcomeKind::OutOfBoundsRoute,
            attachments: Vec::new(),
            choices: vec![
                HumanClassificationChoice::ConfirmedGlitch,
                HumanClassificationChoice::ExpectedBehavior,
                HumanClassificationChoice::NeedsInvestigation,
            ],
            status: HumanClassificationStatus::Pending,
        }
    }

    fn submission(
        classification: HumanClassificationChoice,
        supersedes: Option<String>,
    ) -> HumanLabelSubmission {
        HumanLabelSubmission {
            reviewer_id: "reviewer@example".into(),
            classification,
            note: Some("reviewed headful replay".into()),
            recorded_unix_millis: 1_700_000_000_000,
            supersedes_label_identity: supersedes,
        }
    }

    #[test]
    fn correction_appends_without_rewriting_the_prior_label() {
        let request = request();
        let mut ledger = HumanLabelLedger::default();
        let first = ledger
            .append(
                &request,
                submission(HumanClassificationChoice::NeedsInvestigation, None),
            )
            .unwrap()
            .clone();
        let second = ledger
            .append(
                &request,
                submission(
                    HumanClassificationChoice::ConfirmedGlitch,
                    Some(first.label_identity.clone()),
                ),
            )
            .unwrap()
            .clone();
        assert_eq!(ledger.records().len(), 2);
        assert_eq!(ledger.records()[0], first);
        assert_eq!(second.supersedes_label_identity, Some(first.label_identity));
    }

    #[test]
    fn corpus_metadata_preserves_objective_and_has_no_rewrite_authority() {
        let request = request();
        let mut ledger = HumanLabelLedger::default();
        ledger
            .append(
                &request,
                submission(HumanClassificationChoice::ExpectedBehavior, None),
            )
            .unwrap();
        let metadata = ledger
            .corpus_metadata(
                &request.source_artifact_identity,
                &request.objective_id,
                &request.objective_definition_sha256,
            )
            .unwrap();
        assert_eq!(metadata.objective_id, request.objective_id);
        assert_eq!(
            metadata.objective_definition_sha256,
            request.objective_definition_sha256
        );
        assert_eq!(metadata.labels.len(), 1);
        assert!(!metadata.replay_authority);
        assert!(!metadata.objective_rewrite_authority);
    }

    #[test]
    fn unlabeled_second_write_cannot_silently_replace_a_label() {
        let request = request();
        let mut ledger = HumanLabelLedger::default();
        ledger
            .append(
                &request,
                submission(HumanClassificationChoice::NeedsInvestigation, None),
            )
            .unwrap();
        assert!(
            ledger
                .append(
                    &request,
                    submission(HumanClassificationChoice::ConfirmedGlitch, None),
                )
                .unwrap_err()
                .to_string()
                .contains("supersede")
        );
        assert_eq!(ledger.records().len(), 1);
    }
}
