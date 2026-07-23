//! Sealed artifacts and execution loop for resumable native residual campaigns.

use crate::optimization_request::{OptimizationRequest, ResidualOptimizerConfig};
use crate::residual_campaign_audit::ResidualCampaignAudit;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_harness_contracts::run_contract::{
    HarnessRunRequest, HarnessRunResult, HarnessTerminalReason,
};
use dusklight_learning::native_replay_corpus::NativeReplayCorpus;
use dusklight_search::residual_action::{
    CompiledResidualCandidate, ResidualCandidate, ResidualCompilationReport,
};
use dusklight_search::residual_optimizer::{
    ResidualCemConfig, ResidualCemOptimizer, ResidualCemSnapshot, ResidualGenome,
    ResidualRandomSampler, ResidualRandomSnapshot,
};
use dusklight_search::residual_retention::{
    ExactTerminalVerdict, ResidualEvaluationEvidence, ResidualOutcomeArchive,
    ResidualRetentionSnapshot,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const RESIDUAL_CAMPAIGN_CANDIDATE_SCHEMA_V1: &str = "dusklight-residual-campaign-candidate/v1";
pub const RESIDUAL_CAMPAIGN_CANDIDATE_SCHEMA_V2: &str = "dusklight-residual-campaign-candidate/v2";
pub const RESIDUAL_CAMPAIGN_EVALUATION_SCHEMA_V2: &str =
    "dusklight-residual-campaign-evaluation/v2";
pub const RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V2: &str =
    "dusklight-residual-campaign-checkpoint/v2";
pub const RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V3: &str =
    "dusklight-residual-campaign-checkpoint/v3";
pub const RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V4: &str =
    "dusklight-residual-campaign-checkpoint/v4";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCampaignCandidate {
    pub schema: String,
    pub content_sha256: Digest,
    pub id: String,
    pub generation: u64,
    pub sample_index: u32,
    pub proposer_seed: u64,
    pub genome: ResidualGenome,
    pub candidate: ResidualCandidate,
    pub compilation: ResidualCompilationReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critic_ranking: Option<ResidualCampaignCriticRanking>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCampaignCriticRanking {
    pub report_sha256: Digest,
    pub critic_sha256: Digest,
    pub parent_corpus_sha256: Digest,
    pub rank: usize,
    pub affected_frames: usize,
    pub scored_frames: usize,
    pub unsupported_action_frames: usize,
    pub conservative_mean_advantage_bits: u64,
    pub exact_simulation_authority: bool,
    pub promotion_authority: bool,
}

impl ResidualCampaignCandidate {
    pub fn seal(
        id: String,
        generation: u64,
        sample_index: u32,
        proposer_seed: u64,
        genome: ResidualGenome,
        candidate: ResidualCandidate,
        compiled: &CompiledResidualCandidate,
    ) -> Result<Self, ResidualCampaignError> {
        Self::seal_with_critic_ranking(
            id,
            generation,
            sample_index,
            proposer_seed,
            genome,
            candidate,
            compiled,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn seal_with_critic_ranking(
        id: String,
        generation: u64,
        sample_index: u32,
        proposer_seed: u64,
        genome: ResidualGenome,
        candidate: ResidualCandidate,
        compiled: &CompiledResidualCandidate,
        critic_ranking: Option<ResidualCampaignCriticRanking>,
    ) -> Result<Self, ResidualCampaignError> {
        let mut value = Self {
            schema: if critic_ranking.is_some() {
                RESIDUAL_CAMPAIGN_CANDIDATE_SCHEMA_V2.into()
            } else {
                RESIDUAL_CAMPAIGN_CANDIDATE_SCHEMA_V1.into()
            },
            content_sha256: Digest::ZERO,
            id,
            generation,
            sample_index,
            proposer_seed,
            genome,
            candidate,
            compilation: compiled.report.clone(),
            critic_ranking,
        };
        value.content_sha256 = value.identity()?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), ResidualCampaignError> {
        self.candidate
            .validate()
            .map_err(|error| campaign_error(error.to_string()))?;
        if !valid_id(&self.id)
            || !matches!(
                (self.schema.as_str(), self.critic_ranking.is_some()),
                (RESIDUAL_CAMPAIGN_CANDIDATE_SCHEMA_V1, false)
                    | (RESIDUAL_CAMPAIGN_CANDIDATE_SCHEMA_V2, true)
            )
            || self.compilation.candidate_sha256 != self.candidate.content_sha256
            || self.compilation.parent_tape_sha256 != self.candidate.parent_tape_sha256
            || self.compilation.realized_tape_sha256 == Digest::ZERO
            || !self.compilation.realized_tape_authoritative
            || self.critic_ranking.as_ref().is_some_and(|ranking| {
                [
                    ranking.report_sha256,
                    ranking.critic_sha256,
                    ranking.parent_corpus_sha256,
                ]
                .contains(&Digest::ZERO)
                    || ranking.affected_frames == 0
                    || ranking.scored_frames > ranking.affected_frames
                    || ranking.unsupported_action_frames > ranking.affected_frames
                    || ranking.scored_frames + ranking.unsupported_action_frames
                        != ranking.affected_frames
                    || !f64::from_bits(ranking.conservative_mean_advantage_bits).is_finite()
                    || !ranking.exact_simulation_authority
                    || ranking.promotion_authority
            })
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
        {
            return Err(campaign_error(
                "residual campaign candidate is invalid or detached",
            ));
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, ResidualCampaignError> {
        pretty_json(self)
    }

    fn identity(&self) -> Result<Digest, ResidualCampaignError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        let domain = match self.schema.as_str() {
            RESIDUAL_CAMPAIGN_CANDIDATE_SCHEMA_V1 => {
                b"dusklight.residual-campaign-candidate/v1\0".as_slice()
            }
            RESIDUAL_CAMPAIGN_CANDIDATE_SCHEMA_V2 => {
                b"dusklight.residual-campaign-candidate/v2\0".as_slice()
            }
            _ => return Err(campaign_error("unsupported residual candidate schema")),
        };
        canonical_digest(domain, &canonical)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualNativeAttempt {
    pub repetition: u16,
    pub rng_seed: u64,
    pub request: ArtifactReference,
    pub request_content_sha256: Digest,
    pub result: ArtifactReference,
    pub result_content_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCampaignEvaluation {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request_sha256: Digest,
    /// Identity of the authenticated execution surface. Generic harness runs
    /// bind the harness template; checkpoint-backed runs bind their sealed
    /// native execution manifest.
    pub execution_binding_sha256: Digest,
    pub candidate_id: String,
    pub candidate_sha256: Digest,
    pub realized_tape_sha256: Digest,
    pub attempts: Vec<ResidualNativeAttempt>,
    pub simulated_ticks: u64,
    pub evidence: ResidualEvaluationEvidence,
}

impl ResidualCampaignEvaluation {
    pub fn from_native(
        optimization: &OptimizationRequest,
        template: &HarnessRunRequest,
        candidate: &ResidualCampaignCandidate,
        attempts: Vec<(ResidualNativeAttempt, HarnessRunRequest, HarnessRunResult)>,
    ) -> Result<Self, ResidualCampaignError> {
        if attempts.len() != usize::from(optimization.execution.repetitions) {
            return Err(campaign_error(
                "residual evaluation does not contain every repetition",
            ));
        }
        let mut verdict = None;
        let mut simulated_ticks = 0_u64;
        let mut request_digests = Vec::with_capacity(attempts.len());
        let mut result_digests = Vec::with_capacity(attempts.len());
        let mut behavior = Vec::with_capacity(attempts.len());
        for (index, (attempt, request, result)) in attempts.iter().enumerate() {
            let repetition = u16::try_from(index + 1)
                .map_err(|_| campaign_error("residual repetition index overflowed"))?;
            result
                .validate_against(request)
                .map_err(|error| campaign_error(error.to_string()))?;
            if attempt.repetition != repetition
                || attempt.rng_seed != request.rng_seed
                || attempt.request_content_sha256 != request.content_sha256
                || attempt.result_content_sha256 != result.content_sha256
                || attempt.request.sha256 == Digest::ZERO
                || attempt.result.sha256 == Digest::ZERO
                || !result.artifacts.complete
            {
                return Err(campaign_error(
                    "native residual attempt differs from its sealed request or result",
                ));
            }
            let current = exact_verdict(result)?;
            if verdict.is_some_and(|prior| prior != current) {
                return Err(campaign_error(
                    "native residual repetitions disagree on exact terminal outcome",
                ));
            }
            verdict = Some(current);
            simulated_ticks = simulated_ticks
                .checked_add(result.timing.logical_ticks)
                .ok_or_else(|| campaign_error("residual simulated tick charge overflowed"))?;
            request_digests.push(request.content_sha256);
            result_digests.push(result.content_sha256);
            behavior.push(behavior_part(result)?);
        }
        let evidence = ResidualEvaluationEvidence {
            candidate_sha256: candidate.candidate.content_sha256,
            realized_tape_sha256: candidate.compilation.realized_tape_sha256,
            terminal_program_sha256: optimization.terminal_predicate.program_sha256,
            terminal_definition_sha256: optimization.terminal_predicate.definition_sha256,
            evaluation_sha256: canonical_digest(
                b"dusklight.residual-native-evaluation/v1\0",
                &(request_digests, &result_digests),
            )?,
            episode_sha256: canonical_digest(
                b"dusklight.residual-native-episode/v1\0",
                &result_digests,
            )?,
            behavior_sha256: canonical_digest(
                b"dusklight.residual-native-behavior/v1\0",
                &behavior,
            )?,
            verdict: verdict.ok_or_else(|| campaign_error("residual evaluation is empty"))?,
            shaped_progress_millionths: None,
            native_risk_events: None,
        };
        Self::seal(
            optimization,
            template,
            candidate,
            attempts
                .into_iter()
                .map(|(attempt, _, _)| attempt)
                .collect(),
            simulated_ticks,
            evidence,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn seal(
        optimization: &OptimizationRequest,
        template: &HarnessRunRequest,
        candidate: &ResidualCampaignCandidate,
        attempts: Vec<ResidualNativeAttempt>,
        simulated_ticks: u64,
        evidence: ResidualEvaluationEvidence,
    ) -> Result<Self, ResidualCampaignError> {
        let mut value = Self {
            schema: RESIDUAL_CAMPAIGN_EVALUATION_SCHEMA_V2.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: optimization.content_sha256,
            execution_binding_sha256: template.content_sha256,
            candidate_id: candidate.id.clone(),
            candidate_sha256: candidate.candidate.content_sha256,
            realized_tape_sha256: candidate.compilation.realized_tape_sha256,
            attempts,
            simulated_ticks,
            evidence,
        };
        value.content_sha256 = value.identity()?;
        value.validate_for_execution(optimization, template.content_sha256, candidate)?;
        Ok(value)
    }

    pub fn validate(
        &self,
        optimization: &OptimizationRequest,
        template: &HarnessRunRequest,
        candidate: &ResidualCampaignCandidate,
    ) -> Result<(), ResidualCampaignError> {
        self.validate_for_execution(optimization, template.content_sha256, candidate)
    }

    pub fn validate_for_execution(
        &self,
        optimization: &OptimizationRequest,
        execution_binding_sha256: Digest,
        candidate: &ResidualCampaignCandidate,
    ) -> Result<(), ResidualCampaignError> {
        if self.schema != RESIDUAL_CAMPAIGN_EVALUATION_SCHEMA_V2
            || self.optimization_request_sha256 != optimization.content_sha256
            || self.execution_binding_sha256 != execution_binding_sha256
            || self.candidate_id != candidate.id
            || self.candidate_sha256 != candidate.candidate.content_sha256
            || self.realized_tape_sha256 != candidate.compilation.realized_tape_sha256
            || self.attempts.len() != usize::from(optimization.execution.repetitions)
            || self.simulated_ticks == 0
            || self.simulated_ticks
                > optimization
                    .budgets
                    .exploration_horizon_ticks
                    .saturating_mul(u64::from(optimization.execution.repetitions))
            || self.evidence.candidate_sha256 != self.candidate_sha256
            || self.evidence.realized_tape_sha256 != self.realized_tape_sha256
            || self.evidence.terminal_program_sha256
                != optimization.terminal_predicate.program_sha256
            || self.evidence.terminal_definition_sha256
                != optimization.terminal_predicate.definition_sha256
            || self.attempts.windows(2).any(|pair| {
                pair[0].repetition >= pair[1].repetition || pair[0].rng_seed == pair[1].rng_seed
            })
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
        {
            return Err(campaign_error(
                "residual campaign evaluation is invalid or detached",
            ));
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, ResidualCampaignError> {
        pretty_json(self)
    }

    fn identity(&self) -> Result<Digest, ResidualCampaignError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.residual-campaign-evaluation/v2\0", &canonical)
    }
}

fn exact_verdict(result: &HarnessRunResult) -> Result<ExactTerminalVerdict, ResidualCampaignError> {
    match (
        result.terminal,
        result.objective.reached,
        result.objective.first_hit_tick,
    ) {
        (HarnessTerminalReason::Reached, true, Some(first_hit_tick)) if first_hit_tick > 0 => {
            Ok(ExactTerminalVerdict::Reached { first_hit_tick })
        }
        (HarnessTerminalReason::Exhausted, false, None) => Ok(ExactTerminalVerdict::Miss),
        _ => Err(campaign_error(
            "native attempt is neither an exact success nor an exact horizon miss",
        )),
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum NativeBehaviorPart<'a> {
    Reached {
        first_hit_tick: u64,
        boundary_fingerprint: &'a str,
    },
    Miss {
        gameplay_trace_sha256: Digest,
    },
}

fn behavior_part(
    result: &HarnessRunResult,
) -> Result<NativeBehaviorPart<'_>, ResidualCampaignError> {
    match exact_verdict(result)? {
        ExactTerminalVerdict::Reached { first_hit_tick } => Ok(NativeBehaviorPart::Reached {
            first_hit_tick,
            boundary_fingerprint: &result
                .objective
                .boundary_fingerprint
                .as_ref()
                .ok_or_else(|| campaign_error("exact success lacks a boundary fingerprint"))?
                .digest,
        }),
        ExactTerminalVerdict::Miss => Ok(NativeBehaviorPart::Miss {
            gameplay_trace_sha256: result
                .artifacts
                .gameplay_trace
                .as_ref()
                .ok_or_else(|| campaign_error("exact miss lacks a gameplay trace"))?
                .sha256,
        }),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResidualCampaignOptimizerSnapshot {
    Random { state: ResidualRandomSnapshot },
    Cem { state: ResidualCemSnapshot },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCampaignCheckpoint {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub generation: u64,
    pub completed_candidates: u64,
    pub optimizer: ResidualCampaignOptimizerSnapshot,
    pub retention: ResidualRetentionSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_corpus: Option<ResidualReplayCheckpoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit: Option<ResidualCampaignAudit>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualReplayCheckpoint {
    pub artifact: ArtifactReference,
    pub generation: u32,
    pub entries: u64,
    pub transitions: u64,
    pub successes: u64,
    pub failures: u64,
}

impl ResidualReplayCheckpoint {
    pub fn seal(
        artifact: ArtifactReference,
        corpus: &NativeReplayCorpus,
    ) -> Result<Self, ResidualCampaignError> {
        corpus
            .validate()
            .map_err(|error| campaign_error(error.to_string()))?;
        let value = Self {
            artifact,
            generation: corpus.generation,
            entries: u64::try_from(corpus.report.entries)
                .map_err(|_| campaign_error("replay entry count overflowed"))?,
            transitions: corpus.report.transitions,
            successes: u64::try_from(corpus.report.successes)
                .map_err(|_| campaign_error("replay success count overflowed"))?,
            failures: u64::try_from(corpus.report.failures)
                .map_err(|_| campaign_error("replay failure count overflowed"))?,
        };
        value.validate_corpus(corpus)?;
        Ok(value)
    }

    pub fn validate_corpus(
        &self,
        corpus: &NativeReplayCorpus,
    ) -> Result<(), ResidualCampaignError> {
        corpus
            .validate()
            .map_err(|error| campaign_error(error.to_string()))?;
        if !valid_artifact_reference(&self.artifact)
            || self.generation != corpus.generation
            || self.entries != corpus.report.entries as u64
            || self.transitions != corpus.report.transitions
            || self.successes != corpus.report.successes as u64
            || self.failures != corpus.report.failures as u64
            || self.successes.checked_add(self.failures) != Some(self.entries)
        {
            return Err(campaign_error(
                "residual replay checkpoint summary is invalid or detached",
            ));
        }
        Ok(())
    }

    fn validate_shape(&self) -> bool {
        valid_artifact_reference(&self.artifact)
            && self.generation > 0
            && self.entries > 0
            && self.transitions > 0
            && self.successes.checked_add(self.failures) == Some(self.entries)
    }
}

impl ResidualCampaignCheckpoint {
    pub fn seal(
        optimization: &OptimizationRequest,
        execution_binding_sha256: Digest,
        generation: u64,
        completed_candidates: u64,
        optimizer: ResidualCampaignOptimizerSnapshot,
        archive: &ResidualOutcomeArchive,
        replay_corpus: Option<ResidualReplayCheckpoint>,
    ) -> Result<Self, ResidualCampaignError> {
        let mut value = Self {
            schema: RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V3.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: optimization.content_sha256,
            execution_binding_sha256,
            generation,
            completed_candidates,
            optimizer,
            retention: archive
                .snapshot()
                .map_err(|error| campaign_error(error.to_string()))?,
            replay_corpus,
            audit: None,
        };
        value.content_sha256 = value.identity()?;
        value.validate(optimization, execution_binding_sha256)?;
        Ok(value)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn seal_with_audit(
        optimization: &OptimizationRequest,
        execution_binding_sha256: Digest,
        generation: u64,
        completed_candidates: u64,
        optimizer: ResidualCampaignOptimizerSnapshot,
        retention: ResidualRetentionSnapshot,
        replay_corpus: Option<ResidualReplayCheckpoint>,
        audit: ResidualCampaignAudit,
    ) -> Result<Self, ResidualCampaignError> {
        let mut value = Self {
            schema: RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V4.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: optimization.content_sha256,
            execution_binding_sha256,
            generation,
            completed_candidates,
            optimizer,
            retention,
            replay_corpus,
            audit: Some(audit),
        };
        value.content_sha256 = value.identity()?;
        value.validate(optimization, execution_binding_sha256)?;
        Ok(value)
    }

    pub fn validate(
        &self,
        optimization: &OptimizationRequest,
        execution_binding_sha256: Digest,
    ) -> Result<(), ResidualCampaignError> {
        self.retention
            .validate()
            .map_err(|error| campaign_error(error.to_string()))?;
        if !matches!(
            self.schema.as_str(),
            RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V2
                | RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V3
                | RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V4
        ) || (self.schema == RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V2
            && (self.replay_corpus.is_some() || self.audit.is_some()))
            || (self.schema == RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V3 && self.audit.is_some())
            || (self.schema == RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V4 && self.audit.is_none())
            || self.optimization_request_sha256 != optimization.content_sha256
            || self.execution_binding_sha256 != execution_binding_sha256
            || self.completed_candidates > optimization.budgets.candidate_budget
            || self
                .replay_corpus
                .as_ref()
                .is_some_and(|replay| !replay.validate_shape())
            || self.retention.config
                != optimization
                    .residual_retention_config()
                    .map_err(|error| campaign_error(error.to_string()))?
            || self.audit.as_ref().is_some_and(|audit| {
                audit
                    .validate(optimization, execution_binding_sha256)
                    .is_err()
                    || audit.retention_sha256 != self.retention.content_sha256
                    || audit.optimizer_sha256 != optimizer_snapshot_sha256(&self.optimizer)
                    || audit.completed_candidates != self.completed_candidates
            })
            || !optimizer_kind_matches(&self.optimizer, &optimization.proposal.optimizer)
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
        {
            return Err(campaign_error(
                "residual campaign checkpoint is invalid or detached",
            ));
        }
        Ok(())
    }

    pub fn restore_optimizer(
        &self,
        optimization: &OptimizationRequest,
        parent_bytes: &[u8],
    ) -> Result<ResidualCampaignOptimizer, ResidualCampaignError> {
        match (&self.optimizer, &optimization.proposal.optimizer) {
            (
                ResidualCampaignOptimizerSnapshot::Random { state },
                ResidualOptimizerConfig::Random { .. },
            ) => Ok(ResidualCampaignOptimizer::Random(
                ResidualRandomSampler::restore(
                    optimization.proposal.search_space.clone(),
                    parent_bytes,
                    state.clone(),
                )
                .map_err(|error| campaign_error(error.to_string()))?,
            )),
            (
                ResidualCampaignOptimizerSnapshot::Cem { state },
                ResidualOptimizerConfig::Cem {
                    population,
                    elites,
                    smoothing_millionths,
                    ..
                },
            ) => Ok(ResidualCampaignOptimizer::Cem(
                ResidualCemOptimizer::restore(
                    optimization.proposal.search_space.clone(),
                    ResidualCemConfig {
                        population: *population as usize,
                        elites: *elites as usize,
                        smoothing_millionths: *smoothing_millionths,
                        seed: optimization.execution.deterministic_seeds[0],
                    },
                    parent_bytes,
                    state.clone(),
                )
                .map_err(|error| campaign_error(error.to_string()))?,
            )),
            _ => Err(campaign_error("residual optimizer kind changed")),
        }
    }

    pub fn restore_archive(&self) -> Result<ResidualOutcomeArchive, ResidualCampaignError> {
        ResidualOutcomeArchive::restore(self.retention.clone())
            .map_err(|error| campaign_error(error.to_string()))
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, ResidualCampaignError> {
        pretty_json(self)
    }

    fn identity(&self) -> Result<Digest, ResidualCampaignError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        let domain = match self.schema.as_str() {
            RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V2 => {
                b"dusklight.residual-campaign-checkpoint/v2\0".as_slice()
            }
            RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V3 => {
                b"dusklight.residual-campaign-checkpoint/v3\0".as_slice()
            }
            RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V4 => {
                b"dusklight.residual-campaign-checkpoint/v4\0".as_slice()
            }
            _ => {
                return Err(campaign_error(
                    "unsupported residual campaign checkpoint schema",
                ));
            }
        };
        canonical_digest(domain, &canonical)
    }
}

fn optimizer_snapshot_sha256(snapshot: &ResidualCampaignOptimizerSnapshot) -> Digest {
    match snapshot {
        ResidualCampaignOptimizerSnapshot::Random { state } => state.content_sha256,
        ResidualCampaignOptimizerSnapshot::Cem { state } => state.content_sha256,
    }
}

fn valid_artifact_reference(reference: &ArtifactReference) -> bool {
    let path = std::path::Path::new(&reference.path);
    reference.sha256 != Digest::ZERO
        && !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

#[derive(Debug)]
pub enum ResidualCampaignOptimizer {
    Random(ResidualRandomSampler),
    Cem(ResidualCemOptimizer),
}

impl ResidualCampaignOptimizer {
    pub fn snapshot(&self) -> Result<ResidualCampaignOptimizerSnapshot, ResidualCampaignError> {
        match self {
            Self::Random(value) => Ok(ResidualCampaignOptimizerSnapshot::Random {
                state: value
                    .snapshot()
                    .map_err(|error| campaign_error(error.to_string()))?,
            }),
            Self::Cem(value) => Ok(ResidualCampaignOptimizerSnapshot::Cem {
                state: value
                    .snapshot()
                    .map_err(|error| campaign_error(error.to_string()))?,
            }),
        }
    }
}

fn optimizer_kind_matches(
    snapshot: &ResidualCampaignOptimizerSnapshot,
    config: &ResidualOptimizerConfig,
) -> bool {
    matches!(
        (snapshot, config),
        (
            ResidualCampaignOptimizerSnapshot::Random { .. },
            ResidualOptimizerConfig::Random { .. }
        ) | (
            ResidualCampaignOptimizerSnapshot::Cem { .. },
            ResidualOptimizerConfig::Cem { .. }
        )
    )
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn pretty_json(value: &impl Serialize) -> Result<Vec<u8>, ResidualCampaignError> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|error| campaign_error(error.to_string()))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, ResidualCampaignError> {
    let bytes = serde_json::to_vec(value).map_err(|error| campaign_error(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResidualCampaignError(String);

fn campaign_error(message: impl Into<String>) -> ResidualCampaignError {
    ResidualCampaignError(message.into())
}

impl fmt::Display for ResidualCampaignError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ResidualCampaignError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimization_resume::{OptimizationResumeCandidate, OptimizationResumeState};
    use crate::residual_campaign_audit::{
        ResidualAuditCompletion, ResidualCampaignAudit, ResidualCampaignDiagnosis,
    };
    use crate::residual_campaign_runner::new_optimizer;
    use dusklight_automation_contracts::tape::{InputFrame, InputTape};
    use dusklight_search::residual_action::{
        AnalogChannel, AnalogResidual, TemporalBasis, compile_residual_candidate_to_horizon,
    };
    use dusklight_search::residual_optimizer::{
        ResidualGene, ResidualGeneButtonMode, ResidualGeneKind,
    };

    #[test]
    fn candidate_envelope_binds_genome_candidate_and_horizon_tape() {
        let parent = InputTape {
            frames: vec![
                InputFrame {
                    owned_ports: 1,
                    ..InputFrame::default()
                };
                40
            ],
            ..InputTape::default()
        };
        let parent_bytes = parent.encode().unwrap();
        let candidate = ResidualCandidate::seal(
            &parent_bytes,
            vec![AnalogResidual {
                port: 0,
                channel: AnalogChannel::MainX,
                basis: TemporalBasis::ExactFrame { frame: 4, delta: 8 },
            }],
            vec![],
        )
        .unwrap();
        let compiled =
            compile_residual_candidate_to_horizon(&parent, &parent_bytes, &candidate, 64).unwrap();
        let genome = ResidualGenome {
            genes: vec![ResidualGene {
                enabled: true,
                kind: ResidualGeneKind::Analog,
                port_index: 0,
                channel_index: 0,
                basis_index: 0,
                start_index: 4,
                duration_index: 0,
                delta_indices: [0; 4],
                button_index: 0,
                button_mode: ResidualGeneButtonMode::Press,
            }],
        };
        let envelope = ResidualCampaignCandidate::seal(
            "g000000-s00000-candidate".into(),
            0,
            0,
            17,
            genome,
            candidate,
            &compiled,
        )
        .unwrap();
        assert_eq!(envelope.compilation.frame_count, 64);
        assert_eq!(
            envelope.compilation.realized_tape_sha256,
            Digest(Sha256::digest(&compiled.bytes).into())
        );
        let mut tampered = envelope.clone();
        tampered.compilation.frame_count = 63;
        assert!(tampered.validate().is_err());

        let ranked = ResidualCampaignCandidate::seal_with_critic_ranking(
            "g000000-s00000-ranked".into(),
            0,
            0,
            17,
            envelope.genome.clone(),
            envelope.candidate.clone(),
            &compiled,
            Some(ResidualCampaignCriticRanking {
                report_sha256: Digest([41; 32]),
                critic_sha256: Digest([42; 32]),
                parent_corpus_sha256: Digest([43; 32]),
                rank: 0,
                affected_frames: 1,
                scored_frames: 1,
                unsupported_action_frames: 0,
                conservative_mean_advantage_bits: 2.5_f64.to_bits(),
                exact_simulation_authority: true,
                promotion_authority: false,
            }),
        )
        .unwrap();
        let mut promoted_by_critic = ranked.clone();
        promoted_by_critic
            .critic_ranking
            .as_mut()
            .unwrap()
            .promotion_authority = true;
        assert!(promoted_by_critic.validate().is_err());
        let mut downgraded = ranked.clone();
        downgraded.schema = RESIDUAL_CAMPAIGN_CANDIDATE_SCHEMA_V1.into();
        downgraded.content_sha256 = Digest::ZERO;
        downgraded.content_sha256 = downgraded.identity().unwrap();
        assert!(downgraded.validate().is_err());
        let mut nonfinite = ranked;
        nonfinite
            .critic_ranking
            .as_mut()
            .unwrap()
            .conservative_mean_advantage_bits = f64::NAN.to_bits();
        assert!(nonfinite.validate().is_err());
    }

    #[test]
    fn replay_checkpoint_v3_is_bound_and_replayless_v2_remains_valid() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap();
        let optimization: OptimizationRequest = serde_json::from_slice(
            &std::fs::read(root.join(
                "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
            ))
            .unwrap(),
        )
        .unwrap();
        let incumbent = optimization.incumbent.as_ref().unwrap();
        let parent = std::fs::read(root.join(&incumbent.tape.path)).unwrap();
        let optimizer = new_optimizer(&optimization, &parent).unwrap();
        let archive =
            ResidualOutcomeArchive::new(optimization.residual_retention_config().unwrap()).unwrap();
        let replay = ResidualReplayCheckpoint {
            artifact: ArtifactReference {
                path: "build/campaigns/test/replay/generation-00000001.json".into(),
                sha256: Digest([9; 32]),
            },
            generation: 1,
            entries: 1,
            transitions: 1,
            successes: 1,
            failures: 0,
        };
        let checkpoint = ResidualCampaignCheckpoint::seal(
            &optimization,
            Digest([8; 32]),
            0,
            0,
            optimizer.snapshot().unwrap(),
            &archive,
            Some(replay),
        )
        .unwrap();
        assert_eq!(checkpoint.schema, RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V3);
        checkpoint.validate(&optimization, Digest([8; 32])).unwrap();

        let mut legacy = checkpoint.clone();
        legacy.schema = RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V2.into();
        legacy.replay_corpus = None;
        legacy.content_sha256 = Digest::ZERO;
        legacy.content_sha256 = legacy.identity().unwrap();
        let bytes = serde_json::to_vec(&legacy).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("replay_corpus"));
        let decoded: ResidualCampaignCheckpoint = serde_json::from_slice(&bytes).unwrap();
        decoded.validate(&optimization, Digest([8; 32])).unwrap();

        let mut invalid_legacy = legacy;
        invalid_legacy.replay_corpus = Some(ResidualReplayCheckpoint {
            artifact: ArtifactReference {
                path: "../foreign.json".into(),
                sha256: Digest([7; 32]),
            },
            generation: 1,
            entries: 1,
            transitions: 1,
            successes: 0,
            failures: 1,
        });
        assert!(
            invalid_legacy
                .validate(&optimization, Digest([8; 32]))
                .is_err()
        );
    }

    #[test]
    fn v4_audit_reports_exact_search_coverage_and_rejects_tampering() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap();
        let mut optimization: OptimizationRequest = serde_json::from_slice(
            &std::fs::read(root.join(
                "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
            ))
            .unwrap(),
        )
        .unwrap();
        optimization.proposal.optimizer = ResidualOptimizerConfig::Random { samples: 1 };
        optimization.budgets.candidate_budget = 1;
        optimization.resume.checkpoint_every_candidates = 1;
        optimization.refresh_content_sha256().unwrap();
        let incumbent = optimization.incumbent.as_ref().unwrap();
        let parent_bytes = std::fs::read(root.join(&incumbent.tape.path)).unwrap();
        let parent = InputTape::decode(&parent_bytes).unwrap().tape;
        let mut optimizer = new_optimizer(&optimization, &parent_bytes).unwrap();
        let empty_optimizer_snapshot = optimizer.snapshot().unwrap();
        let ResidualCampaignOptimizer::Random(random) = &mut optimizer else {
            panic!("expected random optimizer");
        };
        let proposal = random
            .sample(&parent, &parent_bytes, 1)
            .unwrap()
            .proposals
            .pop()
            .unwrap();
        let envelope = ResidualCampaignCandidate::seal(
            "g000000-s00000-audit".into(),
            0,
            0,
            optimization.execution.deterministic_seeds[0],
            proposal.genome,
            proposal.candidate,
            &proposal.compiled,
        )
        .unwrap();
        let evidence = ResidualEvaluationEvidence {
            candidate_sha256: envelope.candidate.content_sha256,
            realized_tape_sha256: envelope.compilation.realized_tape_sha256,
            terminal_program_sha256: optimization.terminal_predicate.program_sha256,
            terminal_definition_sha256: optimization.terminal_predicate.definition_sha256,
            evaluation_sha256: Digest([31; 32]),
            episode_sha256: Digest([32; 32]),
            behavior_sha256: Digest([33; 32]),
            verdict: ExactTerminalVerdict::Reached {
                first_hit_tick: 124,
            },
            shaped_progress_millionths: None,
            native_risk_events: None,
        };
        let mut archive =
            ResidualOutcomeArchive::new(optimization.residual_retention_config().unwrap()).unwrap();
        archive.record(&proposal.compiled, evidence).unwrap();
        let candidate_reference = ArtifactReference {
            path: "build/campaigns/audit/candidate.json".into(),
            sha256: envelope.content_sha256,
        };
        let tape_reference = ArtifactReference {
            path: "build/campaigns/audit/candidate.tape".into(),
            sha256: envelope.compilation.realized_tape_sha256,
        };
        let result_reference = ArtifactReference {
            path: "build/campaigns/audit/evaluation.json".into(),
            sha256: Digest([34; 32]),
        };
        let resume = OptimizationResumeState {
            schema: crate::optimization_resume::OPTIMIZATION_RESUME_STATE_SCHEMA_V2.into(),
            request_sha256: optimization.content_sha256,
            journal_sha256: Digest([35; 32]),
            valid_journal_bytes: 1,
            record_count: 1,
            last_record_sha256: Digest([36; 32]),
            next_sequence: 2,
            demonstration: Some(ArtifactReference {
                path: "build/campaigns/audit/demonstration.json".into(),
                sha256: Digest([37; 32]),
            }),
            demonstration_simulated_ticks: 125,
            candidates: vec![OptimizationResumeCandidate {
                id: envelope.id.clone(),
                candidate: candidate_reference.clone(),
                candidate_sha256: candidate_reference.sha256,
                compiled_tape: tape_reference.clone(),
                compiled_tape_sha256: tape_reference.sha256,
                generation: 0,
                proposer_seed: envelope.proposer_seed,
                result: Some(result_reference.clone()),
                result_sha256: Some(result_reference.sha256),
                simulated_ticks: Some(124),
            }],
            completed_candidates: 1,
            charged_simulated_ticks: 249,
            pending_candidate_ids: vec![],
            latest_optimizer_checkpoint: None,
            uncheckpointed_completions: 1,
            state_sha256: Digest([38; 32]),
        };
        let optimizer_snapshot = optimizer.snapshot().unwrap();
        let retention = archive.snapshot().unwrap();
        let completion = ResidualAuditCompletion {
            candidate: &envelope,
            result_sha256: result_reference.sha256,
            simulated_ticks: 124,
            verdict: ExactTerminalVerdict::Reached {
                first_hit_tick: 124,
            },
        };
        let execution = Digest([39; 32]);
        let mut initial_resume = resume.clone();
        initial_resume.candidates.clear();
        initial_resume.completed_candidates = 0;
        initial_resume.charged_simulated_ticks = 125;
        initial_resume.uncheckpointed_completions = 0;
        let empty_retention =
            ResidualOutcomeArchive::new(optimization.residual_retention_config().unwrap())
                .unwrap()
                .snapshot()
                .unwrap();
        let initial_audit = ResidualCampaignAudit::advance(
            &optimization,
            execution,
            &initial_resume,
            &empty_optimizer_snapshot,
            &empty_retention,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(
            initial_audit.diagnosis,
            ResidualCampaignDiagnosis::InProgress
        );
        let audit = ResidualCampaignAudit::advance(
            &optimization,
            execution,
            &resume,
            &optimizer_snapshot,
            &retention,
            Some(&initial_audit),
            &[completion],
        )
        .unwrap();
        assert_eq!(audit.diagnosis, ResidualCampaignDiagnosis::WinnerFound);
        assert_eq!(audit.successful_episode_rate_millionths, 1_000_000);
        assert_eq!(audit.successful_behavior_classes, 1);
        assert_eq!(
            audit.improvement_by_simulated_tick[0].charged_simulated_ticks,
            249
        );
        assert!(audit.coverage.iter().any(|row| row.components > 0));
        assert!(audit.coverage.iter().any(|row| row.components == 0));

        let checkpoint = ResidualCampaignCheckpoint::seal_with_audit(
            &optimization,
            execution,
            1,
            1,
            optimizer_snapshot,
            retention,
            None,
            audit.clone(),
        )
        .unwrap();
        assert_eq!(checkpoint.schema, RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V4);
        checkpoint.validate(&optimization, execution).unwrap();

        let mut tampered = checkpoint;
        tampered
            .audit
            .as_mut()
            .unwrap()
            .successful_episode_rate_millionths = 0;
        assert!(tampered.validate(&optimization, execution).is_err());
    }
}
