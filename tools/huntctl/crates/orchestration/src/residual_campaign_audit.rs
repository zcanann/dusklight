//! Content-sealed, incrementally reproducible evidence for residual campaigns.

use crate::optimization_request::{OptimizationRequest, ResidualOptimizerConfig};
use crate::optimization_resume::OptimizationResumeState;
use crate::residual_campaign::{ResidualCampaignCandidate, ResidualCampaignOptimizerSnapshot};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_search::residual_action::{
    AnalogChannel, ButtonResidualMode, ResidualWindow, TemporalBasis,
};
use dusklight_search::residual_retention::{ExactTerminalVerdict, ResidualRetentionSnapshot};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const RESIDUAL_CAMPAIGN_AUDIT_SCHEMA_V1: &str = "dusklight-residual-campaign-audit/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCampaignAudit {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub retention_sha256: Digest,
    pub optimizer_sha256: Digest,
    pub evidence_chain_sha256: Digest,
    pub sealed_candidates: u64,
    pub completed_candidates: u64,
    pub pending_candidates: u64,
    pub charged_simulated_ticks: u64,
    pub demonstration_simulated_ticks: u64,
    pub candidate_simulated_ticks: u64,
    pub unique_compiled_tapes: u64,
    pub successful_candidates: u64,
    pub failed_candidates: u64,
    pub evaluated_episodes: u64,
    pub successful_episodes: u64,
    pub successful_episode_rate_millionths: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_first_hit_tick: Option<u64>,
    pub first_hit_distribution: Vec<ResidualFirstHitCount>,
    pub successful_behavior_classes: u64,
    pub coverage: Vec<ResidualCoverageCount>,
    pub improvement_by_simulated_tick: Vec<ResidualImprovementPoint>,
    pub optimizer_health: ResidualOptimizerHealth,
    pub declared_budget_complete: bool,
    pub diagnosis: ResidualCampaignDiagnosis,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCoverageCount {
    pub dimension: String,
    pub value: String,
    pub candidates: u64,
    pub components: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualFirstHitCount {
    pub first_hit_tick: u64,
    pub candidates: u64,
    pub episodes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualImprovementPoint {
    pub candidate_id: String,
    pub charged_simulated_ticks: u64,
    pub first_hit_tick: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualOptimizerHealth {
    pub kind: String,
    pub expected_candidates: u64,
    pub generation: u64,
    pub pending_candidates: u64,
    pub attempted_genomes: u64,
    pub accepted_unique_tapes: u64,
    pub rejected_or_duplicate_genomes: u64,
    pub rejected_invalid_genomes: u64,
    pub rejected_duplicate_tapes: u64,
    pub rejected_unclassified_genomes: u64,
    pub proposal_rejection_millionths: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub most_concentrated_category_millionths: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidualCampaignDiagnosis {
    InProgress,
    WinnerFound,
    CompletedSuccessWithoutImprovement,
    CompletedWithoutSuccess,
    TickBudgetTruncated,
    ProposalGenerationStalled,
}

pub(crate) struct ResidualAuditCompletion<'a> {
    pub candidate: &'a ResidualCampaignCandidate,
    pub result_sha256: Digest,
    pub simulated_ticks: u64,
    pub verdict: ExactTerminalVerdict,
}

#[derive(Serialize)]
struct ResidualAuditEvidenceBinding<'a> {
    candidate_id: &'a str,
    candidate_sha256: Digest,
    realized_tape_sha256: Digest,
    result_sha256: Digest,
    simulated_ticks: u64,
    verdict: ExactTerminalVerdict,
}

impl ResidualCampaignAudit {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn advance(
        optimization: &OptimizationRequest,
        execution_binding_sha256: Digest,
        resume: &OptimizationResumeState,
        optimizer: &ResidualCampaignOptimizerSnapshot,
        retention: &ResidualRetentionSnapshot,
        previous: Option<&Self>,
        completions: &[ResidualAuditCompletion<'_>],
    ) -> Result<Self, ResidualCampaignAuditError> {
        retention
            .validate()
            .map_err(|error| audit_error(error.to_string()))?;
        let optimizer_health = optimizer_health(optimization, optimizer)?;
        let prior_completed = previous.map_or(0, |audit| audit.completed_candidates);
        if prior_completed.checked_add(completions.len() as u64)
            != Some(resume.completed_candidates)
            || optimizer_health.accepted_unique_tapes != resume.candidates.len() as u64
        {
            return Err(audit_error(
                "residual campaign audit suffix differs from resume progress",
            ));
        }
        if let Some(previous) = previous {
            previous.validate(optimization, execution_binding_sha256)?;
            if previous.completed_candidates > resume.completed_candidates
                || previous.sealed_candidates > resume.candidates.len() as u64
            {
                return Err(audit_error(
                    "residual campaign audit cannot move progress backward",
                ));
            }
        }

        let mut coverage = previous
            .map(|audit| {
                audit
                    .coverage
                    .iter()
                    .cloned()
                    .map(|row| ((row.dimension.clone(), row.value.clone()), row))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_else(|| empty_coverage(optimization));
        ensure_coverage_shape(optimization, &coverage)?;
        let mut first_hits = previous
            .map(|audit| {
                audit
                    .first_hit_distribution
                    .iter()
                    .map(|row| (row.first_hit_tick, *row))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let mut improvements = previous
            .map(|audit| audit.improvement_by_simulated_tick.clone())
            .unwrap_or_default();
        let mut evidence_chain = previous.map_or(Digest::ZERO, |audit| audit.evidence_chain_sha256);
        let mut candidate_ticks = previous.map_or(0, |audit| audit.candidate_simulated_ticks);
        let mut best_improvement = improvements
            .last()
            .map_or(optimization.budgets.promotion_before_tick, |point| {
                point.first_hit_tick
            });
        let repetitions = u64::from(optimization.execution.repetitions);

        for completion in completions {
            completion
                .candidate
                .validate()
                .map_err(|error| audit_error(error.to_string()))?;
            if completion.result_sha256 == Digest::ZERO || completion.simulated_ticks == 0 {
                return Err(audit_error(
                    "residual campaign audit completion is incomplete",
                ));
            }
            accumulate_coverage(optimization, completion.candidate, &mut coverage)?;
            candidate_ticks = candidate_ticks
                .checked_add(completion.simulated_ticks)
                .ok_or_else(|| audit_error("residual campaign audit tick count overflowed"))?;
            let binding = ResidualAuditEvidenceBinding {
                candidate_id: &completion.candidate.id,
                candidate_sha256: completion.candidate.content_sha256,
                realized_tape_sha256: completion.candidate.compilation.realized_tape_sha256,
                result_sha256: completion.result_sha256,
                simulated_ticks: completion.simulated_ticks,
                verdict: completion.verdict,
            };
            evidence_chain = canonical_digest(
                b"dusklight.residual-campaign-audit-evidence/v1\0",
                &(evidence_chain, binding),
            )?;
            if let ExactTerminalVerdict::Reached { first_hit_tick } = completion.verdict {
                let row = first_hits
                    .entry(first_hit_tick)
                    .or_insert(ResidualFirstHitCount {
                        first_hit_tick,
                        candidates: 0,
                        episodes: 0,
                    });
                row.candidates = row
                    .candidates
                    .checked_add(1)
                    .ok_or_else(|| audit_error("residual success count overflowed"))?;
                row.episodes = row
                    .episodes
                    .checked_add(repetitions)
                    .ok_or_else(|| audit_error("residual success episode count overflowed"))?;
                if first_hit_tick < best_improvement {
                    best_improvement = first_hit_tick;
                    improvements.push(ResidualImprovementPoint {
                        candidate_id: completion.candidate.id.clone(),
                        charged_simulated_ticks: resume
                            .demonstration_simulated_ticks
                            .checked_add(candidate_ticks)
                            .ok_or_else(|| {
                                audit_error("residual improvement tick count overflowed")
                            })?,
                        first_hit_tick,
                    });
                }
            }
        }

        let successful_candidates = retention
            .evaluated_tapes
            .iter()
            .filter(|entry| matches!(entry.verdict, ExactTerminalVerdict::Reached { .. }))
            .count() as u64;
        let failed_candidates = retention.evaluated_tapes.len() as u64 - successful_candidates;
        if retention.evaluated_tapes.len() as u64 != resume.completed_candidates
            || candidate_ticks.checked_add(resume.demonstration_simulated_ticks)
                != Some(resume.charged_simulated_ticks)
        {
            return Err(audit_error(
                "residual campaign audit outcomes or tick charge differ from retained evidence",
            ));
        }
        let evaluated_episodes = resume
            .completed_candidates
            .checked_mul(repetitions)
            .ok_or_else(|| audit_error("residual evaluated episode count overflowed"))?;
        let successful_episodes = successful_candidates
            .checked_mul(repetitions)
            .ok_or_else(|| audit_error("residual successful episode count overflowed"))?;
        let successful_episode_rate_millionths = if evaluated_episodes == 0 {
            0
        } else {
            u32::try_from(successful_episodes.saturating_mul(1_000_000) / evaluated_episodes)
                .map_err(|_| audit_error("residual success rate overflowed"))?
        };
        let best_first_hit_tick = retention
            .successes
            .iter()
            .map(|success| success.first_hit_tick)
            .min();
        let successful_behavior_classes = retention
            .successes
            .iter()
            .map(|success| success.behavior_sha256)
            .collect::<BTreeSet<_>>()
            .len() as u64;
        let declared_budget_complete = optimizer_complete(optimization, optimizer, resume)?;
        let diagnosis = diagnosis(
            optimization,
            resume,
            &optimizer_health,
            declared_budget_complete,
            successful_candidates,
            best_first_hit_tick,
        );
        let mut value = Self {
            schema: RESIDUAL_CAMPAIGN_AUDIT_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: optimization.content_sha256,
            execution_binding_sha256,
            retention_sha256: retention.content_sha256,
            optimizer_sha256: optimizer_sha256(optimizer),
            evidence_chain_sha256: evidence_chain,
            sealed_candidates: resume.candidates.len() as u64,
            completed_candidates: resume.completed_candidates,
            pending_candidates: resume.pending_candidate_ids.len() as u64,
            charged_simulated_ticks: resume.charged_simulated_ticks,
            demonstration_simulated_ticks: resume.demonstration_simulated_ticks,
            candidate_simulated_ticks: candidate_ticks,
            unique_compiled_tapes: resume.candidates.len() as u64,
            successful_candidates,
            failed_candidates,
            evaluated_episodes,
            successful_episodes,
            successful_episode_rate_millionths,
            best_first_hit_tick,
            first_hit_distribution: first_hits.into_values().collect(),
            successful_behavior_classes,
            coverage: coverage.into_values().collect(),
            improvement_by_simulated_tick: improvements,
            optimizer_health,
            declared_budget_complete,
            diagnosis,
        };
        value.content_sha256 = value.identity()?;
        value.validate(optimization, execution_binding_sha256)?;
        Ok(value)
    }

    pub fn validate(
        &self,
        optimization: &OptimizationRequest,
        execution_binding_sha256: Digest,
    ) -> Result<(), ResidualCampaignAuditError> {
        let success_candidates = self
            .first_hit_distribution
            .iter()
            .try_fold(0_u64, |total, row| total.checked_add(row.candidates));
        let success_episodes = self
            .first_hit_distribution
            .iter()
            .try_fold(0_u64, |total, row| total.checked_add(row.episodes));
        let coverage = self
            .coverage
            .iter()
            .cloned()
            .map(|row| ((row.dimension.clone(), row.value.clone()), row))
            .collect::<BTreeMap<_, _>>();
        ensure_coverage_shape(optimization, &coverage)?;
        let expected_health = expected_health_shape(optimization, &self.optimizer_health);
        let expected_complete = self.optimizer_health.accepted_unique_tapes
            >= self.optimizer_health.expected_candidates
            && self.completed_candidates >= self.optimizer_health.expected_candidates
            && self.optimizer_health.pending_candidates == 0
            && match optimization.proposal.optimizer {
                ResidualOptimizerConfig::Random { .. } => true,
                ResidualOptimizerConfig::Cem { generations, .. } => {
                    self.optimizer_health.generation >= u64::from(generations)
                }
            };
        let expected_diagnosis = audit_diagnosis(self, optimization);
        if self.schema != RESIDUAL_CAMPAIGN_AUDIT_SCHEMA_V1
            || self.optimization_request_sha256 != optimization.content_sha256
            || self.execution_binding_sha256 != execution_binding_sha256
            || self.retention_sha256 == Digest::ZERO
            || self.optimizer_sha256 == Digest::ZERO
            || self.sealed_candidates != self.unique_compiled_tapes
            || self.completed_candidates > self.sealed_candidates
            || self.pending_candidates != self.sealed_candidates - self.completed_candidates
            || self
                .demonstration_simulated_ticks
                .checked_add(self.candidate_simulated_ticks)
                != Some(self.charged_simulated_ticks)
            || self
                .successful_candidates
                .checked_add(self.failed_candidates)
                != Some(self.completed_candidates)
            || self.evaluated_episodes
                != self
                    .completed_candidates
                    .saturating_mul(u64::from(optimization.execution.repetitions))
            || self.successful_episodes
                != self
                    .successful_candidates
                    .saturating_mul(u64::from(optimization.execution.repetitions))
            || success_candidates != Some(self.successful_candidates)
            || success_episodes != Some(self.successful_episodes)
            || self.successful_episode_rate_millionths
                != if self.evaluated_episodes == 0 {
                    0
                } else {
                    (self.successful_episodes.saturating_mul(1_000_000) / self.evaluated_episodes)
                        as u32
                }
            || self.first_hit_distribution.windows(2).any(|pair| {
                pair[0].first_hit_tick >= pair[1].first_hit_tick
                    || pair[0].candidates == 0
                    || pair[0].episodes == 0
            })
            || self
                .first_hit_distribution
                .last()
                .is_some_and(|row| row.candidates == 0 || row.episodes == 0)
            || self.best_first_hit_tick
                != self
                    .first_hit_distribution
                    .first()
                    .map(|row| row.first_hit_tick)
            || self.successful_behavior_classes > self.successful_candidates
            || self.coverage.iter().any(|row| {
                row.candidates > self.completed_candidates || row.candidates > row.components
            })
            || self
                .first_hit_distribution
                .iter()
                .any(|row| row.first_hit_tick > optimization.budgets.exploration_horizon_ticks)
            || !valid_improvements(self, optimization)
            || !expected_health
            || self.optimizer_health.accepted_unique_tapes != self.sealed_candidates
            || self.optimizer_health.attempted_genomes < self.optimizer_health.accepted_unique_tapes
            || self.optimizer_health.rejected_or_duplicate_genomes
                != self.optimizer_health.attempted_genomes
                    - self.optimizer_health.accepted_unique_tapes
            || self
                .optimizer_health
                .rejected_invalid_genomes
                .checked_add(self.optimizer_health.rejected_duplicate_tapes)
                .and_then(|classified| {
                    classified.checked_add(self.optimizer_health.rejected_unclassified_genomes)
                })
                != Some(self.optimizer_health.rejected_or_duplicate_genomes)
            || self.optimizer_health.proposal_rejection_millionths
                != if self.optimizer_health.attempted_genomes == 0 {
                    0
                } else {
                    (self
                        .optimizer_health
                        .rejected_or_duplicate_genomes
                        .saturating_mul(1_000_000)
                        / self.optimizer_health.attempted_genomes) as u32
                }
            || self.declared_budget_complete != expected_complete
            || self.diagnosis != expected_diagnosis
            || self.completed_candidates == 0 && self.evidence_chain_sha256 != Digest::ZERO
            || self.completed_candidates > 0 && self.evidence_chain_sha256 == Digest::ZERO
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
        {
            return Err(audit_error(
                "residual campaign audit is invalid, noncanonical, or detached",
            ));
        }
        Ok(())
    }

    fn identity(&self) -> Result<Digest, ResidualCampaignAuditError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.residual-campaign-audit/v1\0", &canonical)
    }
}

fn expected_health_shape(
    optimization: &OptimizationRequest,
    health: &ResidualOptimizerHealth,
) -> bool {
    match optimization.proposal.optimizer {
        ResidualOptimizerConfig::Random { samples } => {
            health.kind == "random"
                && health.expected_candidates == samples
                && health.generation == 0
                && health.pending_candidates == 0
                && health.most_concentrated_category_millionths.is_none()
        }
        ResidualOptimizerConfig::Cem {
            population,
            generations,
            ..
        } => {
            health.kind == "cem"
                && health.expected_candidates
                    == u64::from(population).saturating_mul(u64::from(generations))
                && health
                    .most_concentrated_category_millionths
                    .is_some_and(|value| value <= 1_000_000)
        }
    }
}

fn audit_diagnosis(
    audit: &ResidualCampaignAudit,
    optimization: &OptimizationRequest,
) -> ResidualCampaignDiagnosis {
    if audit
        .best_first_hit_tick
        .is_some_and(|tick| tick < optimization.budgets.promotion_before_tick)
    {
        ResidualCampaignDiagnosis::WinnerFound
    } else if audit.declared_budget_complete && audit.successful_candidates > 0 {
        ResidualCampaignDiagnosis::CompletedSuccessWithoutImprovement
    } else if audit.declared_budget_complete {
        ResidualCampaignDiagnosis::CompletedWithoutSuccess
    } else if audit.charged_simulated_ticks >= optimization.budgets.simulated_tick_budget {
        ResidualCampaignDiagnosis::TickBudgetTruncated
    } else if audit.optimizer_health.attempted_genomes > 0
        && audit.optimizer_health.accepted_unique_tapes == 0
    {
        ResidualCampaignDiagnosis::ProposalGenerationStalled
    } else {
        ResidualCampaignDiagnosis::InProgress
    }
}

fn valid_improvements(audit: &ResidualCampaignAudit, optimization: &OptimizationRequest) -> bool {
    let mut prior_ticks = 0;
    let mut prior_hit = optimization.budgets.promotion_before_tick;
    for point in &audit.improvement_by_simulated_tick {
        if point.candidate_id.is_empty()
            || point.charged_simulated_ticks <= prior_ticks
            || point.charged_simulated_ticks > audit.charged_simulated_ticks
            || point.first_hit_tick >= prior_hit
        {
            return false;
        }
        prior_ticks = point.charged_simulated_ticks;
        prior_hit = point.first_hit_tick;
    }
    true
}

fn diagnosis(
    optimization: &OptimizationRequest,
    resume: &OptimizationResumeState,
    health: &ResidualOptimizerHealth,
    complete: bool,
    successes: u64,
    best: Option<u64>,
) -> ResidualCampaignDiagnosis {
    if best.is_some_and(|tick| tick < optimization.budgets.promotion_before_tick) {
        ResidualCampaignDiagnosis::WinnerFound
    } else if complete && successes > 0 {
        ResidualCampaignDiagnosis::CompletedSuccessWithoutImprovement
    } else if complete {
        ResidualCampaignDiagnosis::CompletedWithoutSuccess
    } else if resume.charged_simulated_ticks >= optimization.budgets.simulated_tick_budget {
        ResidualCampaignDiagnosis::TickBudgetTruncated
    } else if health.attempted_genomes > 0 && health.accepted_unique_tapes == 0 {
        ResidualCampaignDiagnosis::ProposalGenerationStalled
    } else {
        ResidualCampaignDiagnosis::InProgress
    }
}

fn optimizer_complete(
    optimization: &OptimizationRequest,
    optimizer: &ResidualCampaignOptimizerSnapshot,
    resume: &OptimizationResumeState,
) -> Result<bool, ResidualCampaignAuditError> {
    Ok(match (&optimization.proposal.optimizer, optimizer) {
        (
            ResidualOptimizerConfig::Random { samples },
            ResidualCampaignOptimizerSnapshot::Random { state },
        ) => state.produced_candidates >= *samples && resume.completed_candidates >= *samples,
        (
            ResidualOptimizerConfig::Cem {
                population,
                generations,
                ..
            },
            ResidualCampaignOptimizerSnapshot::Cem { state },
        ) => {
            state.pending.is_empty()
                && state.generation >= *generations
                && resume.completed_candidates
                    >= u64::from(*population).saturating_mul(u64::from(*generations))
        }
        _ => {
            return Err(audit_error(
                "residual audit optimizer kind differs from request",
            ));
        }
    })
}

fn optimizer_health(
    optimization: &OptimizationRequest,
    optimizer: &ResidualCampaignOptimizerSnapshot,
) -> Result<ResidualOptimizerHealth, ResidualCampaignAuditError> {
    let (
        kind,
        expected,
        generation,
        pending,
        attempted,
        accepted,
        invalid,
        duplicate,
        concentrated,
    ) = match (&optimization.proposal.optimizer, optimizer) {
        (
            ResidualOptimizerConfig::Random { samples },
            ResidualCampaignOptimizerSnapshot::Random { state },
        ) => (
            "random",
            *samples,
            0,
            0,
            state.attempted_genomes,
            state.seen_tape_sha256.len() as u64,
            state.rejected_invalid_genomes,
            state.rejected_duplicate_tapes,
            None,
        ),
        (
            ResidualOptimizerConfig::Cem {
                population,
                generations,
                ..
            },
            ResidualCampaignOptimizerSnapshot::Cem { state },
        ) => (
            "cem",
            u64::from(*population).saturating_mul(u64::from(*generations)),
            u64::from(state.generation),
            state.pending.len() as u64,
            state.attempted_genomes,
            state.seen_tape_sha256.len() as u64,
            state.rejected_invalid_genomes,
            state.rejected_duplicate_tapes,
            state
                .distributions
                .iter()
                .flat_map(distribution_weights)
                .max(),
        ),
        _ => {
            return Err(audit_error(
                "residual audit optimizer kind differs from request",
            ));
        }
    };
    let rejected = attempted
        .checked_sub(accepted)
        .ok_or_else(|| audit_error("residual optimizer accepted more tapes than it attempted"))?;
    let classified = invalid
        .checked_add(duplicate)
        .filter(|classified| *classified <= rejected)
        .ok_or_else(|| audit_error("residual optimizer rejection classes exceed all rejections"))?;
    let rejection_rate = if attempted == 0 {
        0
    } else {
        u32::try_from(rejected.saturating_mul(1_000_000) / attempted)
            .map_err(|_| audit_error("residual proposal rejection rate overflowed"))?
    };
    Ok(ResidualOptimizerHealth {
        kind: kind.into(),
        expected_candidates: expected,
        generation,
        pending_candidates: pending,
        attempted_genomes: attempted,
        accepted_unique_tapes: accepted,
        rejected_or_duplicate_genomes: rejected,
        rejected_invalid_genomes: invalid,
        rejected_duplicate_tapes: duplicate,
        rejected_unclassified_genomes: rejected - classified,
        proposal_rejection_millionths: rejection_rate,
        most_concentrated_category_millionths: concentrated,
    })
}

fn distribution_weights(
    distribution: &dusklight_search::residual_optimizer::ResidualGeneDistribution,
) -> impl Iterator<Item = u32> + '_ {
    distribution
        .enabled
        .weights_millionths
        .iter()
        .chain(&distribution.kind.weights_millionths)
        .chain(&distribution.port.weights_millionths)
        .chain(&distribution.channel.weights_millionths)
        .chain(&distribution.basis.weights_millionths)
        .chain(&distribution.start.weights_millionths)
        .chain(&distribution.duration.weights_millionths)
        .chain(
            distribution
                .deltas
                .iter()
                .flat_map(|delta| &delta.weights_millionths),
        )
        .chain(&distribution.button.weights_millionths)
        .chain(&distribution.button_mode.weights_millionths)
        .copied()
}

fn optimizer_sha256(optimizer: &ResidualCampaignOptimizerSnapshot) -> Digest {
    match optimizer {
        ResidualCampaignOptimizerSnapshot::Random { state } => state.content_sha256,
        ResidualCampaignOptimizerSnapshot::Cem { state } => state.content_sha256,
    }
}

fn empty_coverage(
    optimization: &OptimizationRequest,
) -> BTreeMap<(String, String), ResidualCoverageCount> {
    let mut values = BTreeMap::new();
    for channel in &optimization.proposal.search_space.analog_channels {
        insert_empty(&mut values, "action", analog_channel_name(*channel));
    }
    insert_empty(&mut values, "action", "button_press");
    insert_empty(&mut values, "action", "button_release");
    for basis in [
        "exact_frame",
        "window_2",
        "window_4",
        "window_8",
        "window_16",
        "window_32",
        "piecewise_linear_ramp",
        "cubic_control_curve",
        "button_hold",
    ] {
        insert_empty(&mut values, "temporal_basis", basis);
    }
    for index in 0..8 {
        insert_empty(&mut values, "start_octant", &index.to_string());
    }
    values
}

fn insert_empty(
    values: &mut BTreeMap<(String, String), ResidualCoverageCount>,
    dimension: &str,
    value: &str,
) {
    let row = ResidualCoverageCount {
        dimension: dimension.into(),
        value: value.into(),
        candidates: 0,
        components: 0,
    };
    values.insert((row.dimension.clone(), row.value.clone()), row);
}

fn ensure_coverage_shape(
    optimization: &OptimizationRequest,
    coverage: &BTreeMap<(String, String), ResidualCoverageCount>,
) -> Result<(), ResidualCampaignAuditError> {
    let expected = empty_coverage(optimization);
    if coverage.len() != expected.len()
        || coverage.keys().ne(expected.keys())
        || coverage.iter().any(|(key, row)| {
            key != &(row.dimension.clone(), row.value.clone()) || row.candidates > row.components
        })
    {
        return Err(audit_error(
            "residual campaign audit coverage shape is noncanonical",
        ));
    }
    Ok(())
}

fn accumulate_coverage(
    optimization: &OptimizationRequest,
    candidate: &ResidualCampaignCandidate,
    coverage: &mut BTreeMap<(String, String), ResidualCoverageCount>,
) -> Result<(), ResidualCampaignAuditError> {
    let mut present = BTreeSet::new();
    for analog in &candidate.candidate.analog {
        increment(
            coverage,
            "action",
            analog_channel_name(analog.channel),
            &mut present,
        )?;
        increment(
            coverage,
            "temporal_basis",
            temporal_basis_name(&analog.basis),
            &mut present,
        )?;
        increment(
            coverage,
            "start_octant",
            &start_octant(optimization, basis_start(&analog.basis))?.to_string(),
            &mut present,
        )?;
    }
    for button in &candidate.candidate.buttons {
        increment(
            coverage,
            "action",
            match button.mode {
                ButtonResidualMode::Press => "button_press",
                ButtonResidualMode::Release => "button_release",
            },
            &mut present,
        )?;
        increment(coverage, "temporal_basis", "button_hold", &mut present)?;
        increment(
            coverage,
            "start_octant",
            &start_octant(optimization, button.start_frame)?.to_string(),
            &mut present,
        )?;
    }
    for key in present {
        coverage
            .get_mut(&key)
            .ok_or_else(|| audit_error("residual coverage category disappeared"))?
            .candidates += 1;
    }
    Ok(())
}

fn increment(
    coverage: &mut BTreeMap<(String, String), ResidualCoverageCount>,
    dimension: &str,
    value: &str,
    present: &mut BTreeSet<(String, String)>,
) -> Result<(), ResidualCampaignAuditError> {
    let key = (dimension.to_string(), value.to_string());
    let row = coverage
        .get_mut(&key)
        .ok_or_else(|| audit_error("residual coverage category is outside the sealed surface"))?;
    row.components = row
        .components
        .checked_add(1)
        .ok_or_else(|| audit_error("residual coverage component count overflowed"))?;
    present.insert(key);
    Ok(())
}

fn start_octant(
    optimization: &OptimizationRequest,
    frame: u64,
) -> Result<u64, ResidualCampaignAuditError> {
    let start = optimization.proposal.search_space.start_frame;
    let end = optimization.proposal.search_space.end_frame_exclusive;
    let width = end
        .checked_sub(start)
        .ok_or_else(|| audit_error("residual search frame domain is invalid"))?;
    if frame < start || frame >= end || width == 0 {
        return Err(audit_error(
            "residual component begins outside the sealed search frame domain",
        ));
    }
    Ok(((frame - start).saturating_mul(8) / width).min(7))
}

fn basis_start(basis: &TemporalBasis) -> u64 {
    match basis {
        TemporalBasis::ExactFrame { frame, .. } => *frame,
        TemporalBasis::Window { start_frame, .. }
        | TemporalBasis::PiecewiseLinearRamp { start_frame, .. }
        | TemporalBasis::CubicControlCurve { start_frame, .. } => *start_frame,
    }
}

fn temporal_basis_name(basis: &TemporalBasis) -> &'static str {
    match basis {
        TemporalBasis::ExactFrame { .. } => "exact_frame",
        TemporalBasis::Window { window, .. } => match window {
            ResidualWindow::Frames2 => "window_2",
            ResidualWindow::Frames4 => "window_4",
            ResidualWindow::Frames8 => "window_8",
            ResidualWindow::Frames16 => "window_16",
            ResidualWindow::Frames32 => "window_32",
        },
        TemporalBasis::PiecewiseLinearRamp { .. } => "piecewise_linear_ramp",
        TemporalBasis::CubicControlCurve { .. } => "cubic_control_curve",
    }
}

fn analog_channel_name(channel: AnalogChannel) -> &'static str {
    match channel {
        AnalogChannel::MainX => "main_x",
        AnalogChannel::MainY => "main_y",
        AnalogChannel::CameraX => "camera_x",
        AnalogChannel::CameraY => "camera_y",
    }
}

fn canonical_digest<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Digest, ResidualCampaignAuditError> {
    let bytes = serde_json::to_vec(value).map_err(|error| audit_error(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Debug)]
pub struct ResidualCampaignAuditError(String);

fn audit_error(message: impl Into<String>) -> ResidualCampaignAuditError {
    ResidualCampaignAuditError(message.into())
}

impl fmt::Display for ResidualCampaignAuditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ResidualCampaignAuditError {}
