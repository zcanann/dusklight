//! Sealed artifacts and execution loop for resumable native residual campaigns.

use crate::optimization_request::{OptimizationRequest, ResidualOptimizerConfig};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_harness_contracts::run_contract::{
    HarnessRunRequest, HarnessRunResult, HarnessTerminalReason,
};
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
pub const RESIDUAL_CAMPAIGN_EVALUATION_SCHEMA_V2: &str =
    "dusklight-residual-campaign-evaluation/v2";
pub const RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V2: &str =
    "dusklight-residual-campaign-checkpoint/v2";

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
        let mut value = Self {
            schema: RESIDUAL_CAMPAIGN_CANDIDATE_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            id,
            generation,
            sample_index,
            proposer_seed,
            genome,
            candidate,
            compilation: compiled.report.clone(),
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
            || self.schema != RESIDUAL_CAMPAIGN_CANDIDATE_SCHEMA_V1
            || self.compilation.candidate_sha256 != self.candidate.content_sha256
            || self.compilation.parent_tape_sha256 != self.candidate.parent_tape_sha256
            || self.compilation.realized_tape_sha256 == Digest::ZERO
            || !self.compilation.realized_tape_authoritative
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
        canonical_digest(b"dusklight.residual-campaign-candidate/v1\0", &canonical)
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
}

impl ResidualCampaignCheckpoint {
    pub fn seal(
        optimization: &OptimizationRequest,
        execution_binding_sha256: Digest,
        generation: u64,
        completed_candidates: u64,
        optimizer: ResidualCampaignOptimizerSnapshot,
        archive: &ResidualOutcomeArchive,
    ) -> Result<Self, ResidualCampaignError> {
        let mut value = Self {
            schema: RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V2.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: optimization.content_sha256,
            execution_binding_sha256,
            generation,
            completed_candidates,
            optimizer,
            retention: archive
                .snapshot()
                .map_err(|error| campaign_error(error.to_string()))?,
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
        if self.schema != RESIDUAL_CAMPAIGN_CHECKPOINT_SCHEMA_V2
            || self.optimization_request_sha256 != optimization.content_sha256
            || self.execution_binding_sha256 != execution_binding_sha256
            || self.completed_candidates > optimization.budgets.candidate_budget
            || self.retention.config
                != optimization
                    .residual_retention_config()
                    .map_err(|error| campaign_error(error.to_string()))?
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
        canonical_digest(b"dusklight.residual-campaign-checkpoint/v2\0", &canonical)
    }
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
    }
}
