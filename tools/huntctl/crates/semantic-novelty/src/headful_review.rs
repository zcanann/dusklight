//! Automatic headless-to-headful replay and human-classification handoff.

use super::archive::{
    DiscoveryArchivePartitionKey, DiscoveryFidelity, DiscoveryOutcomeKind,
    DiscoveryRetentionQuality,
};
use super::proposal_signal::SemanticNoveltyProposalSignal;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

pub const HEADFUL_REPLAY_REQUEST_SCHEMA: &str = "dusklight-headful-replay-request/v1";
pub const HUMAN_CLASSIFICATION_REQUEST_SCHEMA: &str = "dusklight-human-classification-request/v1";
pub const MAX_PENDING_HEADFUL_REVIEWS: usize = 1_024;
pub const MAX_REVIEWED_ARTIFACTS: usize = 65_536;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadfulReviewPolicy {
    pub minimum_proposal_signal: u64,
}

impl Default for HeadfulReviewPolicy {
    fn default() -> Self {
        Self {
            minimum_proposal_signal: 10,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromisingHeadlessDiscovery {
    pub partition: DiscoveryArchivePartitionKey,
    pub artifact_identity: String,
    pub tape_identity: String,
    pub tape_path: PathBuf,
    pub replay_boundary_identity: String,
    pub objective_id: String,
    pub objective_definition_sha256: String,
    pub outcome: DiscoveryOutcomeKind,
    pub quality: DiscoveryRetentionQuality,
    pub proposal_signal: SemanticNoveltyProposalSignal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TerminalCapturePlan {
    pub terminal_thumbnail_png: bool,
    pub short_video: bool,
    pub video_reason: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HeadfulReplayRequest {
    pub schema: &'static str,
    pub request_identity: String,
    pub source_partition: DiscoveryArchivePartitionKey,
    pub target_partition: DiscoveryArchivePartitionKey,
    pub artifact_identity: String,
    pub tape_identity: String,
    pub tape_path: PathBuf,
    pub replay_boundary_identity: String,
    pub objective_id: String,
    pub objective_definition_sha256: String,
    pub outcome: DiscoveryOutcomeKind,
    pub capture: TerminalCapturePlan,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAttachmentKind {
    TerminalThumbnailPng,
    ShortVideo,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReviewAttachment {
    pub kind: ReviewAttachmentKind,
    pub sha256: String,
    pub size: u64,
    pub media_type: String,
    pub relative_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadfulReplayEvidence {
    pub replay_identity: String,
    pub replay_boundary_identity: String,
    pub objective_id: String,
    pub objective_definition_sha256: String,
    pub semantic_replay_matched: bool,
    pub attachments: Vec<ReviewAttachment>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanClassificationStatus {
    Pending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanClassificationChoice {
    ConfirmedGlitch,
    ExpectedBehavior,
    DuplicateKnownSymptom,
    RenderingArtifact,
    InfrastructureFault,
    NeedsInvestigation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HumanClassificationRequest {
    pub schema: &'static str,
    pub request_identity: String,
    pub source_artifact_identity: String,
    pub headful_replay_identity: String,
    pub replay_boundary_identity: String,
    pub objective_id: String,
    pub objective_definition_sha256: String,
    pub semantic_replay_matched: bool,
    pub outcome: DiscoveryOutcomeKind,
    pub attachments: Vec<ReviewAttachment>,
    pub choices: Vec<HumanClassificationChoice>,
    pub status: HumanClassificationStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HeadfulEnqueueDecision {
    Enqueued(Box<HeadfulReplayRequest>),
    BelowSignalThreshold,
    UnsupportedEvidence,
    AlreadyQueued,
    QueueAtCapacity,
}

#[derive(Clone, Debug, Default)]
pub struct HeadfulReviewQueue {
    queued_artifacts: BTreeSet<String>,
    pending: BTreeMap<String, HeadfulReplayRequest>,
}

#[derive(Debug)]
pub struct HeadfulReviewError(String);

impl fmt::Display for HeadfulReviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for HeadfulReviewError {}

impl HeadfulReviewQueue {
    pub fn enqueue_promising(
        &mut self,
        discovery: PromisingHeadlessDiscovery,
        target_headful_fidelity_identity: String,
        policy: &HeadfulReviewPolicy,
    ) -> Result<HeadfulEnqueueDecision, HeadfulReviewError> {
        validate_sha256("artifact identity", &discovery.artifact_identity)?;
        validate_sha256("tape identity", &discovery.tape_identity)?;
        validate_sha256(
            "source replay boundary identity",
            &discovery.replay_boundary_identity,
        )?;
        if discovery.objective_id.trim().is_empty() {
            return Err(HeadfulReviewError("objective ID is empty".into()));
        }
        validate_sha256(
            "objective definition",
            &discovery.objective_definition_sha256,
        )?;
        validate_sha256(
            "target headful fidelity identity",
            &target_headful_fidelity_identity,
        )?;
        if discovery.partition.fidelity != DiscoveryFidelity::Headless {
            return Err(HeadfulReviewError(
                "automatic headful review requires a headless source partition".into(),
            ));
        }
        if discovery.quality.evidence_rank == 0 {
            return Ok(HeadfulEnqueueDecision::UnsupportedEvidence);
        }
        if discovery.proposal_signal.proposal_ordering_score() < policy.minimum_proposal_signal {
            return Ok(HeadfulEnqueueDecision::BelowSignalThreshold);
        }
        if self.queued_artifacts.contains(&discovery.artifact_identity) {
            return Ok(HeadfulEnqueueDecision::AlreadyQueued);
        }
        if self.pending.len() >= MAX_PENDING_HEADFUL_REVIEWS
            || self.queued_artifacts.len() >= MAX_REVIEWED_ARTIFACTS
        {
            return Ok(HeadfulEnqueueDecision::QueueAtCapacity);
        }
        let target_partition = DiscoveryArchivePartitionKey {
            scenario_name: discovery.partition.scenario_name.clone(),
            scenario_identity: discovery.partition.scenario_identity.clone(),
            fidelity: DiscoveryFidelity::Headful,
            fidelity_identity: target_headful_fidelity_identity,
        };
        let capture = capture_plan(&discovery.outcome);
        let request_identity = replay_request_identity(
            &discovery.artifact_identity,
            &discovery.tape_identity,
            &discovery.replay_boundary_identity,
            &discovery.objective_id,
            &discovery.objective_definition_sha256,
            &target_partition,
            &capture,
        );
        let request = HeadfulReplayRequest {
            schema: HEADFUL_REPLAY_REQUEST_SCHEMA,
            request_identity: request_identity.clone(),
            source_partition: discovery.partition,
            target_partition,
            artifact_identity: discovery.artifact_identity.clone(),
            tape_identity: discovery.tape_identity,
            tape_path: discovery.tape_path,
            replay_boundary_identity: discovery.replay_boundary_identity,
            objective_id: discovery.objective_id,
            objective_definition_sha256: discovery.objective_definition_sha256,
            outcome: discovery.outcome,
            capture,
        };
        self.queued_artifacts.insert(discovery.artifact_identity);
        self.pending.insert(request_identity, request.clone());
        Ok(HeadfulEnqueueDecision::Enqueued(Box::new(request)))
    }

    pub fn complete_replay(
        &mut self,
        request_identity: &str,
        mut evidence: HeadfulReplayEvidence,
    ) -> Result<HumanClassificationRequest, HeadfulReviewError> {
        let request =
            self.pending.get(request_identity).cloned().ok_or_else(|| {
                HeadfulReviewError("headful replay request is not pending".into())
            })?;
        validate_sha256("headful replay identity", &evidence.replay_identity)?;
        validate_sha256(
            "headful replay boundary identity",
            &evidence.replay_boundary_identity,
        )?;
        if evidence.replay_boundary_identity != request.replay_boundary_identity {
            return Err(HeadfulReviewError(
                "headful replay changed the source replay boundary".into(),
            ));
        }
        if evidence.objective_id != request.objective_id
            || evidence.objective_definition_sha256 != request.objective_definition_sha256
        {
            return Err(HeadfulReviewError(
                "headful replay changed the source objective definition".into(),
            ));
        }
        let mut attachment_kinds = BTreeSet::new();
        for attachment in &evidence.attachments {
            validate_attachment(attachment)?;
            if !attachment_kinds.insert(attachment.kind) {
                return Err(HeadfulReviewError(
                    "headful replay contains a duplicate attachment kind".into(),
                ));
            }
        }
        evidence
            .attachments
            .sort_by_key(|attachment| attachment.kind);
        if !evidence
            .attachments
            .iter()
            .any(|attachment| attachment.kind == ReviewAttachmentKind::TerminalThumbnailPng)
        {
            return Err(HeadfulReviewError(
                "headful replay lacks its required terminal thumbnail".into(),
            ));
        }
        if request.capture.short_video
            && !evidence
                .attachments
                .iter()
                .any(|attachment| attachment.kind == ReviewAttachmentKind::ShortVideo)
        {
            return Err(HeadfulReviewError(
                "headful temporal replay lacks its requested short video".into(),
            ));
        }
        self.pending.remove(request_identity);
        Ok(HumanClassificationRequest {
            schema: HUMAN_CLASSIFICATION_REQUEST_SCHEMA,
            request_identity: request.request_identity,
            source_artifact_identity: request.artifact_identity,
            headful_replay_identity: evidence.replay_identity,
            replay_boundary_identity: evidence.replay_boundary_identity,
            objective_id: request.objective_id,
            objective_definition_sha256: request.objective_definition_sha256,
            semantic_replay_matched: evidence.semantic_replay_matched,
            outcome: request.outcome,
            attachments: evidence.attachments,
            choices: vec![
                HumanClassificationChoice::ConfirmedGlitch,
                HumanClassificationChoice::ExpectedBehavior,
                HumanClassificationChoice::DuplicateKnownSymptom,
                HumanClassificationChoice::RenderingArtifact,
                HumanClassificationChoice::InfrastructureFault,
                HumanClassificationChoice::NeedsInvestigation,
            ],
            status: HumanClassificationStatus::Pending,
        })
    }
}

fn capture_plan(outcome: &DiscoveryOutcomeKind) -> TerminalCapturePlan {
    let video_reason = match outcome {
        DiscoveryOutcomeKind::Hang => Some("temporal_stall"),
        DiscoveryOutcomeKind::OutOfBoundsRoute => Some("route_motion"),
        DiscoveryOutcomeKind::Corruption => Some("state_evolution"),
        DiscoveryOutcomeKind::EventSequence => Some("event_timing"),
        DiscoveryOutcomeKind::Goal
        | DiscoveryOutcomeKind::Crash
        | DiscoveryOutcomeKind::Other(_) => None,
    };
    TerminalCapturePlan {
        terminal_thumbnail_png: true,
        short_video: video_reason.is_some(),
        video_reason,
    }
}

fn replay_request_identity(
    artifact: &str,
    tape: &str,
    replay_boundary: &str,
    objective_id: &str,
    objective_definition_sha256: &str,
    target: &DiscoveryArchivePartitionKey,
    capture: &TerminalCapturePlan,
) -> String {
    let encoded = serde_json::to_vec(&(
        artifact,
        tape,
        replay_boundary,
        objective_id,
        objective_definition_sha256,
        target,
        capture,
    ))
    .expect("headful replay request identity is serializable");
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-headful-replay-request/v1\0");
    hasher.update((encoded.len() as u64).to_le_bytes());
    hasher.update(encoded);
    format!("{:x}", hasher.finalize())
}

fn validate_attachment(attachment: &ReviewAttachment) -> Result<(), HeadfulReviewError> {
    validate_sha256("review attachment", &attachment.sha256)?;
    let expected_media_type = match attachment.kind {
        ReviewAttachmentKind::TerminalThumbnailPng => "image/png",
        ReviewAttachmentKind::ShortVideo => "video/mp4",
    };
    if attachment.size == 0
        || attachment.media_type != expected_media_type
        || attachment.relative_path.as_os_str().is_empty()
        || attachment.relative_path.is_absolute()
        || attachment
            .relative_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(HeadfulReviewError(
            "review attachment is empty or has an unsafe path".into(),
        ));
    }
    Ok(())
}

fn validate_sha256(label: &str, digest: &str) -> Result<(), HeadfulReviewError> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(HeadfulReviewError(format!(
            "{label} is not lowercase SHA-256"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{SEMANTIC_NOVELTY_ASSESSMENT_SCHEMA, SemanticNoveltyAssessment};
    use crate::proposal_signal::{
        SemanticNoveltyProposalSignal, SemanticNoveltyProposalSignalConfig,
    };

    fn discovery(outcome: DiscoveryOutcomeKind) -> PromisingHeadlessDiscovery {
        let assessment = SemanticNoveltyAssessment {
            schema: SEMANTIC_NOVELTY_ASSESSMENT_SCHEMA,
            descriptor_identity: "44".repeat(32),
            catalog_observed_episodes_before: 0,
            rare_support_episode_ceiling: 3,
            first_seen_transitions: Vec::new(),
            rare_state_combinations: Vec::new(),
            semantic_novel: true,
            spatial_distance_used: false,
        };
        PromisingHeadlessDiscovery {
            partition: DiscoveryArchivePartitionKey {
                scenario_name: "intro".into(),
                scenario_identity: "11".repeat(32),
                fidelity: DiscoveryFidelity::Headless,
                fidelity_identity: "22".repeat(32),
            },
            artifact_identity: "33".repeat(32),
            tape_identity: "44".repeat(32),
            tape_path: PathBuf::from("artifacts/candidate.tape"),
            replay_boundary_identity: "77".repeat(32),
            objective_id: "unseen-contact".into(),
            objective_definition_sha256: "aa".repeat(32),
            outcome,
            quality: DiscoveryRetentionQuality {
                evidence_rank: 2,
                cold_replay_passes: 2,
                milestone_depth: 1,
                minimized_tape_frames: 10,
            },
            proposal_signal: SemanticNoveltyProposalSignal::from_assessment(
                assessment,
                SemanticNoveltyProposalSignalConfig {
                    first_seen_transition_weight: 0,
                    rare_combination_weight: 0,
                    maximum_signal: 1,
                },
            )
            .unwrap(),
        }
    }

    fn attachment(kind: ReviewAttachmentKind, digest: &str) -> ReviewAttachment {
        ReviewAttachment {
            kind,
            sha256: digest.repeat(32),
            size: 100,
            media_type: match kind {
                ReviewAttachmentKind::TerminalThumbnailPng => "image/png",
                ReviewAttachmentKind::ShortVideo => "video/mp4",
            }
            .into(),
            relative_path: match kind {
                ReviewAttachmentKind::TerminalThumbnailPng => "review/terminal.png",
                ReviewAttachmentKind::ShortVideo => "review/replay.mp4",
            }
            .into(),
        }
    }

    #[test]
    fn promising_headless_temporal_discovery_enqueues_headful_video_replay() {
        let mut queue = HeadfulReviewQueue::default();
        let decision = queue
            .enqueue_promising(
                discovery(DiscoveryOutcomeKind::Hang),
                "55".repeat(32),
                &HeadfulReviewPolicy {
                    minimum_proposal_signal: 0,
                },
            )
            .unwrap();
        let HeadfulEnqueueDecision::Enqueued(request) = decision else {
            panic!("expected a headful replay request");
        };
        assert_eq!(
            request.target_partition.fidelity,
            DiscoveryFidelity::Headful
        );
        assert!(request.capture.terminal_thumbnail_png);
        assert!(request.capture.short_video);
    }

    #[test]
    fn completed_replay_with_attachments_requests_human_classification() {
        let mut queue = HeadfulReviewQueue::default();
        let HeadfulEnqueueDecision::Enqueued(request) = queue
            .enqueue_promising(
                discovery(DiscoveryOutcomeKind::OutOfBoundsRoute),
                "55".repeat(32),
                &HeadfulReviewPolicy {
                    minimum_proposal_signal: 0,
                },
            )
            .unwrap()
        else {
            panic!("expected a headful replay request");
        };
        let review = queue
            .complete_replay(
                &request.request_identity,
                HeadfulReplayEvidence {
                    replay_identity: "66".repeat(32),
                    replay_boundary_identity: "77".repeat(32),
                    objective_id: "unseen-contact".into(),
                    objective_definition_sha256: "aa".repeat(32),
                    semantic_replay_matched: true,
                    attachments: vec![
                        attachment(ReviewAttachmentKind::TerminalThumbnailPng, "88"),
                        attachment(ReviewAttachmentKind::ShortVideo, "99"),
                    ],
                },
            )
            .unwrap();
        assert_eq!(review.status, HumanClassificationStatus::Pending);
        assert_eq!(review.attachments.len(), 2);
        assert!(
            review
                .choices
                .contains(&HumanClassificationChoice::ConfirmedGlitch)
        );
    }

    #[test]
    fn required_video_cannot_be_silently_omitted() {
        let mut queue = HeadfulReviewQueue::default();
        let HeadfulEnqueueDecision::Enqueued(request) = queue
            .enqueue_promising(
                discovery(DiscoveryOutcomeKind::EventSequence),
                "55".repeat(32),
                &HeadfulReviewPolicy {
                    minimum_proposal_signal: 0,
                },
            )
            .unwrap()
        else {
            panic!("expected a headful replay request");
        };
        assert!(
            queue
                .complete_replay(
                    &request.request_identity,
                    HeadfulReplayEvidence {
                        replay_identity: "66".repeat(32),
                        replay_boundary_identity: "77".repeat(32),
                        objective_id: "unseen-contact".into(),
                        objective_definition_sha256: "aa".repeat(32),
                        semantic_replay_matched: true,
                        attachments: vec![attachment(
                            ReviewAttachmentKind::TerminalThumbnailPng,
                            "88"
                        )],
                    },
                )
                .unwrap_err()
                .to_string()
                .contains("short video")
        );
        assert!(
            queue
                .complete_replay(
                    &request.request_identity,
                    HeadfulReplayEvidence {
                        replay_identity: "66".repeat(32),
                        replay_boundary_identity: "77".repeat(32),
                        objective_id: "unseen-contact".into(),
                        objective_definition_sha256: "aa".repeat(32),
                        semantic_replay_matched: true,
                        attachments: vec![
                            attachment(ReviewAttachmentKind::TerminalThumbnailPng, "88"),
                            attachment(ReviewAttachmentKind::ShortVideo, "99"),
                        ],
                    },
                )
                .is_ok()
        );
    }
}
