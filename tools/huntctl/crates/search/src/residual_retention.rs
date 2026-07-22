//! Exact-terminal retention, ranking, and diversity policy for residual search.

use crate::residual_action::{
    CompiledResidualCandidate, RESIDUAL_COMPILATION_SCHEMA_V1, ResidualCompilationReport,
};
use crate::search::tape_input_complexity;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const RESIDUAL_RETENTION_SCHEMA_V1: &str = "dusklight-residual-retention/v1";
pub const RESIDUAL_RETENTION_SNAPSHOT_SCHEMA_V1: &str = "dusklight-residual-retention-snapshot/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FailureRetentionPolicy {
    DiversityReservoir { capacity: u64 },
    All,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualRetentionConfig {
    pub parent_tape_sha256: Digest,
    pub terminal_program_sha256: Digest,
    pub terminal_definition_sha256: Digest,
    pub exploration_horizon_ticks: u64,
    pub promotion_before_tick: u64,
    pub maximum_candidates: u64,
    pub failures: FailureRetentionPolicy,
}

impl ResidualRetentionConfig {
    pub fn validate(&self) -> Result<(), ResidualRetentionError> {
        if self.parent_tape_sha256 == Digest::ZERO
            || self.terminal_program_sha256 == Digest::ZERO
            || self.terminal_definition_sha256 == Digest::ZERO
            || self.promotion_before_tick == 0
            || self.exploration_horizon_ticks <= self.promotion_before_tick
            || self.maximum_candidates == 0
            || matches!(
                self.failures,
                FailureRetentionPolicy::DiversityReservoir { capacity: 0 }
            )
        {
            return Err(retention_error(
                "residual retention configuration is invalid",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExactTerminalVerdict {
    Reached { first_hit_tick: u64 },
    Miss,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualEvaluationEvidence {
    pub candidate_sha256: Digest,
    pub realized_tape_sha256: Digest,
    pub terminal_program_sha256: Digest,
    pub terminal_definition_sha256: Digest,
    pub evaluation_sha256: Digest,
    pub episode_sha256: Digest,
    pub behavior_sha256: Digest,
    pub verdict: ExactTerminalVerdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shaped_progress_millionths: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_risk_events: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredResidualRisk {
    pub intervention_frames: u64,
    pub residual_components: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_risk_events: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedResidualSuccess {
    pub candidate_sha256: Digest,
    pub parent_tape_sha256: Digest,
    pub realized_tape_sha256: Digest,
    pub realized_tape: Vec<u8>,
    pub first_hit_tick: u64,
    pub tape_frames: u64,
    pub input_complexity: u64,
    pub declared_risk: DeclaredResidualRisk,
    pub behavior_sha256: Digest,
    pub episode_sha256: Digest,
    pub exact_evaluations: Vec<Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimized_from: Option<Digest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedResidualFailure {
    pub candidate_sha256: Digest,
    pub parent_tape_sha256: Digest,
    pub realized_tape_sha256: Digest,
    pub realized_tape: Vec<u8>,
    pub behavior_sha256: Digest,
    pub episode_sha256: Digest,
    pub evaluation_sha256: Digest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shaped_progress_millionths: Option<i64>,
    pub reservoir_priority: Digest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetainedOutcome {
    Success,
    Failure,
    RepeatedSuccessProof,
    RepeatedFailureEvidence,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatedResidualTape {
    pub realized_tape_sha256: Digest,
    pub verdict: ExactTerminalVerdict,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualEvidenceBinding {
    pub evidence_sha256: Digest,
    pub realized_tape_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualRetentionSnapshot {
    pub schema: String,
    pub content_sha256: Digest,
    pub config: ResidualRetentionConfig,
    pub successes: Vec<RetainedResidualSuccess>,
    pub failures: Vec<RetainedResidualFailure>,
    pub evaluated_tapes: Vec<EvaluatedResidualTape>,
    pub evaluation_bindings: Vec<ResidualEvidenceBinding>,
    pub episode_bindings: Vec<ResidualEvidenceBinding>,
}

impl ResidualRetentionSnapshot {
    pub fn validate(&self) -> Result<(), ResidualRetentionError> {
        self.config.validate()?;
        if self.schema != RESIDUAL_RETENTION_SNAPSHOT_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_identity()?
            || self.evaluated_tapes.len() as u64 > self.config.maximum_candidates
            || self.evaluation_bindings.len() as u64
                > self.config.maximum_candidates.saturating_mul(100)
            || self.episode_bindings.len() as u64
                > self.config.maximum_candidates.saturating_mul(100)
            || !self
                .evaluated_tapes
                .windows(2)
                .all(|pair| pair[0].realized_tape_sha256 < pair[1].realized_tape_sha256)
            || !bindings_are_canonical(&self.evaluation_bindings)
            || !bindings_are_canonical(&self.episode_bindings)
            || !strictly_sorted_by(&self.successes, compare_success)
            || !strictly_sorted_by(&self.failures, compare_failure)
        {
            return Err(retention_error(
                "residual retention snapshot seal or canonical ordering is invalid",
            ));
        }

        let evaluated = self
            .evaluated_tapes
            .iter()
            .map(|entry| (entry.realized_tape_sha256, entry.verdict))
            .collect::<BTreeMap<_, _>>();
        let evaluations = self
            .evaluation_bindings
            .iter()
            .map(|entry| (entry.evidence_sha256, entry.realized_tape_sha256))
            .collect::<BTreeMap<_, _>>();
        let episodes = self
            .episode_bindings
            .iter()
            .map(|entry| (entry.evidence_sha256, entry.realized_tape_sha256))
            .collect::<BTreeMap<_, _>>();
        if evaluated.keys().any(|digest| *digest == Digest::ZERO)
            || evaluations
                .iter()
                .any(|(evidence, tape)| *evidence == Digest::ZERO || !evaluated.contains_key(tape))
            || episodes
                .iter()
                .any(|(evidence, tape)| *evidence == Digest::ZERO || !evaluated.contains_key(tape))
            || evaluated.keys().any(|tape| {
                !evaluations.values().any(|bound| bound == tape)
                    || !episodes.values().any(|bound| bound == tape)
            })
        {
            return Err(retention_error(
                "residual retention snapshot contains detached evidence bindings",
            ));
        }

        for success in &self.successes {
            if !valid_snapshot_success(success, &self.config, &evaluated, &evaluations, &episodes) {
                return Err(retention_error(
                    "retained residual success is invalid or detached",
                ));
            }
        }
        for success in self
            .successes
            .iter()
            .filter(|success| success.minimized_from.is_some())
        {
            let source = self
                .successes
                .iter()
                .find(|source| Some(source.realized_tape_sha256) == success.minimized_from)
                .ok_or_else(|| retention_error("minimized success is detached from its source"))?;
            if (success.tape_frames, success.input_complexity)
                >= (source.tape_frames, source.input_complexity)
            {
                return Err(retention_error(
                    "minimized success is not strictly simpler than its source",
                ));
            }
        }
        for failure in &self.failures {
            if !valid_snapshot_failure(failure, &self.config, &evaluated, &evaluations, &episodes) {
                return Err(retention_error(
                    "retained residual failure is invalid or detached",
                ));
            }
        }
        if matches!(
            self.config.failures,
            FailureRetentionPolicy::DiversityReservoir { capacity }
                if self.failures.len() as u64 > capacity
        ) {
            return Err(retention_error(
                "residual failure reservoir exceeds its configured capacity",
            ));
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, ResidualRetentionError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.residual-retention-snapshot/v1\0", &canonical)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HorizonTighteningEvidence {
    pub current_horizon_ticks: u64,
    pub proposed_horizon_ticks: u64,
    pub retained_successes: u64,
    pub supporting_successes: u64,
    pub supporting_behavior_classes: u64,
    pub support_millionths: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HorizonSupportPolicy {
    pub minimum_successes: u64,
    pub minimum_behavior_classes: u64,
    pub minimum_support_millionths: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct ResidualGenerationEvaluation<'a> {
    pub compiled: &'a CompiledResidualCandidate,
    pub evidence: &'a ResidualEvaluationEvidence,
}

/// Ranks one complete optimizer generation without consulting the bounded
/// retention reservoir. This guarantees that every pending CEM sample receives
/// one rank even when a failure is intentionally dropped from long-term storage.
pub fn rank_residual_generation(
    config: &ResidualRetentionConfig,
    evaluations: &[ResidualGenerationEvaluation<'_>],
) -> Result<Vec<Digest>, ResidualRetentionError> {
    config.validate()?;
    if evaluations.is_empty() {
        return Err(retention_error("residual generation cannot be empty"));
    }
    let mut candidates = BTreeSet::new();
    let mut tapes = BTreeSet::new();
    let mut ranked = Vec::with_capacity(evaluations.len());
    for evaluation in evaluations {
        validate_evidence(config, evaluation.compiled, evaluation.evidence)?;
        let tape = validate_compilation(config, evaluation.compiled)?;
        if !candidates.insert(evaluation.evidence.candidate_sha256)
            || !tapes.insert(evaluation.evidence.realized_tape_sha256)
        {
            return Err(retention_error(
                "residual generation repeats a candidate or realized tape",
            ));
        }
        let success = match evaluation.evidence.verdict {
            ExactTerminalVerdict::Reached { first_hit_tick }
                if first_hit_tick > 0 && first_hit_tick <= config.exploration_horizon_ticks =>
            {
                Some((
                    first_hit_tick,
                    tape.frames.len() as u64,
                    tape_input_complexity(&tape),
                    declared_risk(
                        &evaluation.compiled.report,
                        evaluation.evidence.native_risk_events,
                    )?,
                ))
            }
            ExactTerminalVerdict::Reached { .. } => {
                return Err(retention_error(
                    "terminal success lies outside the exploration horizon",
                ));
            }
            ExactTerminalVerdict::Miss => None,
        };
        ranked.push((evaluation, success));
    }
    ranked.sort_by(|(left, left_success), (right, right_success)| {
        match (left_success, right_success) {
            (Some(left_score), Some(right_score)) => left_score
                .cmp(right_score)
                .then_with(|| {
                    left.evidence
                        .behavior_sha256
                        .cmp(&right.evidence.behavior_sha256)
                })
                .then_with(|| {
                    left.evidence
                        .realized_tape_sha256
                        .cmp(&right.evidence.realized_tape_sha256)
                })
                .then_with(|| {
                    left.evidence
                        .candidate_sha256
                        .cmp(&right.evidence.candidate_sha256)
                }),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => left
                .evidence
                .behavior_sha256
                .cmp(&right.evidence.behavior_sha256)
                .then_with(|| {
                    failure_priority(left.compiled, left.evidence)
                        .cmp(&failure_priority(right.compiled, right.evidence))
                })
                .then_with(|| {
                    left.evidence
                        .candidate_sha256
                        .cmp(&right.evidence.candidate_sha256)
                }),
        }
    });
    Ok(ranked
        .into_iter()
        .map(|(evaluation, _)| evaluation.evidence.candidate_sha256)
        .collect())
}

#[derive(Clone, Debug)]
pub struct ResidualOutcomeArchive {
    config: ResidualRetentionConfig,
    successes: Vec<RetainedResidualSuccess>,
    failures: Vec<RetainedResidualFailure>,
    evaluated_tapes: BTreeMap<Digest, ExactTerminalVerdict>,
    evaluation_bindings: BTreeMap<Digest, Digest>,
    episode_bindings: BTreeMap<Digest, Digest>,
}

impl ResidualOutcomeArchive {
    pub fn new(config: ResidualRetentionConfig) -> Result<Self, ResidualRetentionError> {
        config.validate()?;
        Ok(Self {
            config,
            successes: Vec::new(),
            failures: Vec::new(),
            evaluated_tapes: BTreeMap::new(),
            evaluation_bindings: BTreeMap::new(),
            episode_bindings: BTreeMap::new(),
        })
    }

    pub fn successes(&self) -> &[RetainedResidualSuccess] {
        &self.successes
    }

    pub fn failures(&self) -> &[RetainedResidualFailure] {
        &self.failures
    }

    pub fn restore(snapshot: ResidualRetentionSnapshot) -> Result<Self, ResidualRetentionError> {
        snapshot.validate()?;
        Ok(Self {
            config: snapshot.config,
            successes: snapshot.successes,
            failures: snapshot.failures,
            evaluated_tapes: snapshot
                .evaluated_tapes
                .into_iter()
                .map(|entry| (entry.realized_tape_sha256, entry.verdict))
                .collect(),
            evaluation_bindings: snapshot
                .evaluation_bindings
                .into_iter()
                .map(|entry| (entry.evidence_sha256, entry.realized_tape_sha256))
                .collect(),
            episode_bindings: snapshot
                .episode_bindings
                .into_iter()
                .map(|entry| (entry.evidence_sha256, entry.realized_tape_sha256))
                .collect(),
        })
    }

    pub fn snapshot(&self) -> Result<ResidualRetentionSnapshot, ResidualRetentionError> {
        let mut snapshot = ResidualRetentionSnapshot {
            schema: RESIDUAL_RETENTION_SNAPSHOT_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            config: self.config.clone(),
            successes: self.successes.clone(),
            failures: self.failures.clone(),
            evaluated_tapes: self
                .evaluated_tapes
                .iter()
                .map(|(realized_tape_sha256, verdict)| EvaluatedResidualTape {
                    realized_tape_sha256: *realized_tape_sha256,
                    verdict: *verdict,
                })
                .collect(),
            evaluation_bindings: self
                .evaluation_bindings
                .iter()
                .map(
                    |(evidence_sha256, realized_tape_sha256)| ResidualEvidenceBinding {
                        evidence_sha256: *evidence_sha256,
                        realized_tape_sha256: *realized_tape_sha256,
                    },
                )
                .collect(),
            episode_bindings: self
                .episode_bindings
                .iter()
                .map(
                    |(evidence_sha256, realized_tape_sha256)| ResidualEvidenceBinding {
                        evidence_sha256: *evidence_sha256,
                        realized_tape_sha256: *realized_tape_sha256,
                    },
                )
                .collect(),
        };
        snapshot.content_sha256 = snapshot.compute_identity()?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn record(
        &mut self,
        compiled: &CompiledResidualCandidate,
        evidence: ResidualEvaluationEvidence,
    ) -> Result<RetainedOutcome, ResidualRetentionError> {
        self.record_internal(compiled, evidence, None)
    }

    pub fn accept_minimized(
        &mut self,
        discovered_tape_sha256: Digest,
        minimized: &CompiledResidualCandidate,
        evidence: ResidualEvaluationEvidence,
    ) -> Result<RetainedOutcome, ResidualRetentionError> {
        let discovered = self
            .successes
            .iter()
            .find(|success| success.realized_tape_sha256 == discovered_tape_sha256)
            .ok_or_else(|| retention_error("minimization requires a retained exact success"))?;
        let minimized_tape = validate_compilation(&self.config, minimized)?;
        let minimized_simplicity = (
            minimized_tape.frames.len() as u64,
            tape_input_complexity(&minimized_tape),
        );
        if minimized_simplicity >= (discovered.tape_frames, discovered.input_complexity) {
            return Err(retention_error(
                "minimized tape must be strictly simpler than its discovered success",
            ));
        }
        if !matches!(evidence.verdict, ExactTerminalVerdict::Reached { .. }) {
            return Err(retention_error(
                "minimization acceptance requires an exact terminal replay",
            ));
        }
        self.record_internal(minimized, evidence, Some(discovered_tape_sha256))
    }

    pub fn diverse_success_elites(&self, limit: usize) -> Vec<&RetainedResidualSuccess> {
        if limit == 0 {
            return Vec::new();
        }
        let mut selected = Vec::new();
        let mut behaviors = BTreeSet::new();
        for success in &self.successes {
            if behaviors.insert(success.behavior_sha256) {
                selected.push(success);
                if selected.len() == limit {
                    return selected;
                }
            }
        }
        let selected_tapes = selected
            .iter()
            .map(|success| success.realized_tape_sha256)
            .collect::<BTreeSet<_>>();
        for success in &self.successes {
            if !selected_tapes.contains(&success.realized_tape_sha256) {
                selected.push(success);
                if selected.len() == limit {
                    break;
                }
            }
        }
        selected
    }

    pub fn horizon_tightening_evidence(
        &self,
        proposed_horizon_ticks: u64,
        policy: HorizonSupportPolicy,
    ) -> Result<HorizonTighteningEvidence, ResidualRetentionError> {
        if proposed_horizon_ticks <= self.config.promotion_before_tick
            || proposed_horizon_ticks >= self.config.exploration_horizon_ticks
            || policy.minimum_successes == 0
            || policy.minimum_behavior_classes == 0
            || policy.minimum_support_millionths > 1_000_000
            || self.successes.is_empty()
        {
            return Err(retention_error("horizon tightening proposal is invalid"));
        }
        let supporting = self
            .successes
            .iter()
            .filter(|success| success.first_hit_tick <= proposed_horizon_ticks)
            .collect::<Vec<_>>();
        let behaviors = supporting
            .iter()
            .map(|success| success.behavior_sha256)
            .collect::<BTreeSet<_>>()
            .len() as u64;
        let support = (supporting.len() as u64 * 1_000_000 / self.successes.len() as u64) as u32;
        if (supporting.len() as u64) < policy.minimum_successes
            || behaviors < policy.minimum_behavior_classes
            || support < policy.minimum_support_millionths
        {
            return Err(retention_error(
                "retained successful basin does not support the tighter horizon",
            ));
        }
        Ok(HorizonTighteningEvidence {
            current_horizon_ticks: self.config.exploration_horizon_ticks,
            proposed_horizon_ticks,
            retained_successes: self.successes.len() as u64,
            supporting_successes: supporting.len() as u64,
            supporting_behavior_classes: behaviors,
            support_millionths: support,
        })
    }

    /// Exact successes rank first. Failures follow by stable diversity and
    /// content identity; shaped progress is intentionally absent.
    pub fn optimizer_rank(&self) -> Vec<Digest> {
        self.successes
            .iter()
            .map(|success| success.candidate_sha256)
            .chain(self.failures.iter().map(|failure| failure.candidate_sha256))
            .collect()
    }

    fn record_internal(
        &mut self,
        compiled: &CompiledResidualCandidate,
        evidence: ResidualEvaluationEvidence,
        minimized_from: Option<Digest>,
    ) -> Result<RetainedOutcome, ResidualRetentionError> {
        if self.evaluated_tapes.len() as u64 >= self.config.maximum_candidates
            && !self
                .evaluated_tapes
                .contains_key(&compiled.report.realized_tape_sha256)
        {
            return Err(retention_error(
                "residual archive candidate budget is exhausted",
            ));
        }
        if !self
            .evaluation_bindings
            .contains_key(&evidence.evaluation_sha256)
            && self.evaluation_bindings.len() as u64
                >= self.config.maximum_candidates.saturating_mul(100)
        {
            return Err(retention_error(
                "residual archive repetition-evidence budget is exhausted",
            ));
        }
        validate_evidence(&self.config, compiled, &evidence)?;
        let tape = validate_compilation(&self.config, compiled)?;
        if let ExactTerminalVerdict::Reached { first_hit_tick } = evidence.verdict
            && (first_hit_tick == 0 || first_hit_tick > self.config.exploration_horizon_ticks)
        {
            return Err(retention_error(
                "terminal success lies outside the exploration horizon",
            ));
        }
        if self
            .evaluation_bindings
            .get(&evidence.evaluation_sha256)
            .is_some_and(|tape| *tape != compiled.report.realized_tape_sha256)
            || self
                .episode_bindings
                .get(&evidence.episode_sha256)
                .is_some_and(|tape| *tape != compiled.report.realized_tape_sha256)
        {
            return Err(retention_error(
                "residual evaluation or episode is already bound to another tape",
            ));
        }
        if self
            .evaluated_tapes
            .get(&compiled.report.realized_tape_sha256)
            .is_some_and(|verdict| *verdict != evidence.verdict)
        {
            return Err(retention_error(
                "repeated exact terminal verdict is nondeterministic",
            ));
        }
        if let ExactTerminalVerdict::Reached { first_hit_tick } = evidence.verdict
            && self.successes.iter().any(|success| {
                success.realized_tape_sha256 == compiled.report.realized_tape_sha256
                    && (success.first_hit_tick != first_hit_tick
                        || success.behavior_sha256 != evidence.behavior_sha256)
            })
        {
            return Err(retention_error(
                "repeated exact success is nondeterministic",
            ));
        }
        if let ExactTerminalVerdict::Reached { .. } = evidence.verdict
            && let Some(existing) = self.successes.iter().find(|success| {
                success.realized_tape_sha256 == compiled.report.realized_tape_sha256
            })
        {
            if minimized_from.is_some()
                && existing.minimized_from.is_some()
                && existing.minimized_from != minimized_from
            {
                return Err(retention_error(
                    "minimized success is already bound to another source",
                ));
            }
            if existing
                .exact_evaluations
                .binary_search(&evidence.evaluation_sha256)
                .is_ok()
            {
                if self.evaluation_bindings.get(&evidence.evaluation_sha256)
                    != Some(&compiled.report.realized_tape_sha256)
                    || self.episode_bindings.get(&evidence.episode_sha256)
                        != Some(&compiled.report.realized_tape_sha256)
                {
                    return Err(retention_error(
                        "repeated exact success evidence is not idempotent",
                    ));
                }
                return Ok(RetainedOutcome::RepeatedSuccessProof);
            }
        }
        if matches!(evidence.verdict, ExactTerminalVerdict::Miss)
            && let Some(existing) = self.failures.iter().find(|failure| {
                failure.realized_tape_sha256 == compiled.report.realized_tape_sha256
                    && failure.evaluation_sha256 == evidence.evaluation_sha256
            })
        {
            if existing.candidate_sha256 != evidence.candidate_sha256
                || existing.behavior_sha256 != evidence.behavior_sha256
                || existing.episode_sha256 != evidence.episode_sha256
                || existing.shaped_progress_millionths != evidence.shaped_progress_millionths
            {
                return Err(retention_error(
                    "repeated residual failure evidence is nondeterministic",
                ));
            }
            return Ok(RetainedOutcome::RepeatedFailureEvidence);
        }
        let success_risk = matches!(evidence.verdict, ExactTerminalVerdict::Reached { .. })
            .then(|| declared_risk(&compiled.report, evidence.native_risk_events))
            .transpose()?;
        self.evaluated_tapes
            .insert(compiled.report.realized_tape_sha256, evidence.verdict);
        self.evaluation_bindings.insert(
            evidence.evaluation_sha256,
            compiled.report.realized_tape_sha256,
        );
        self.episode_bindings.insert(
            evidence.episode_sha256,
            compiled.report.realized_tape_sha256,
        );
        match evidence.verdict {
            ExactTerminalVerdict::Reached { first_hit_tick } => {
                if let Some(existing) = self.successes.iter_mut().find(|success| {
                    success.realized_tape_sha256 == compiled.report.realized_tape_sha256
                }) {
                    existing.exact_evaluations.push(evidence.evaluation_sha256);
                    existing.exact_evaluations.sort_unstable();
                    if existing.minimized_from.is_none() {
                        existing.minimized_from = minimized_from;
                    }
                    return Ok(RetainedOutcome::RepeatedSuccessProof);
                }
                self.successes.push(RetainedResidualSuccess {
                    candidate_sha256: compiled.report.candidate_sha256,
                    parent_tape_sha256: compiled.report.parent_tape_sha256,
                    realized_tape_sha256: compiled.report.realized_tape_sha256,
                    realized_tape: compiled.bytes.clone(),
                    first_hit_tick,
                    tape_frames: tape.frames.len() as u64,
                    input_complexity: tape_input_complexity(&tape),
                    declared_risk: success_risk
                        .ok_or_else(|| retention_error("reached verdict lost its declared risk"))?,
                    behavior_sha256: evidence.behavior_sha256,
                    episode_sha256: evidence.episode_sha256,
                    exact_evaluations: vec![evidence.evaluation_sha256],
                    minimized_from,
                });
                self.successes.sort_by(compare_success);
                Ok(RetainedOutcome::Success)
            }
            ExactTerminalVerdict::Miss => {
                let failure = RetainedResidualFailure {
                    candidate_sha256: compiled.report.candidate_sha256,
                    parent_tape_sha256: compiled.report.parent_tape_sha256,
                    realized_tape_sha256: compiled.report.realized_tape_sha256,
                    realized_tape: compiled.bytes.clone(),
                    behavior_sha256: evidence.behavior_sha256,
                    episode_sha256: evidence.episode_sha256,
                    evaluation_sha256: evidence.evaluation_sha256,
                    shaped_progress_millionths: evidence.shaped_progress_millionths,
                    reservoir_priority: failure_priority(compiled, &evidence),
                };
                self.failures.push(failure);
                self.apply_failure_policy();
                Ok(RetainedOutcome::Failure)
            }
        }
    }

    fn apply_failure_policy(&mut self) {
        self.failures.sort_by(compare_failure);
        let FailureRetentionPolicy::DiversityReservoir { capacity } = self.config.failures else {
            return;
        };
        if self.failures.len() as u64 <= capacity {
            return;
        }
        let mut first_by_behavior = BTreeMap::new();
        for (index, failure) in self.failures.iter().enumerate() {
            first_by_behavior
                .entry(failure.behavior_sha256)
                .or_insert(index);
        }
        let mut selected = first_by_behavior.values().copied().collect::<Vec<_>>();
        selected.sort_by_key(|index| self.failures[*index].reservoir_priority);
        selected.truncate(capacity as usize);
        let mut selected = selected.into_iter().collect::<BTreeSet<_>>();
        if selected.len() < capacity as usize {
            let mut remaining = (0..self.failures.len())
                .filter(|index| !selected.contains(index))
                .collect::<Vec<_>>();
            remaining.sort_by_key(|index| self.failures[*index].reservoir_priority);
            selected.extend(
                remaining
                    .into_iter()
                    .take(capacity as usize - selected.len()),
            );
        }
        self.failures = self
            .failures
            .drain(..)
            .enumerate()
            .filter_map(|(index, failure)| selected.contains(&index).then_some(failure))
            .collect();
        self.failures.sort_by(compare_failure);
    }
}

fn strictly_sorted_by<T>(values: &[T], compare: fn(&T, &T) -> Ordering) -> bool {
    values
        .windows(2)
        .all(|pair| compare(&pair[0], &pair[1]) == Ordering::Less)
}

fn bindings_are_canonical(bindings: &[ResidualEvidenceBinding]) -> bool {
    bindings.iter().all(|binding| {
        binding.evidence_sha256 != Digest::ZERO && binding.realized_tape_sha256 != Digest::ZERO
    }) && bindings
        .windows(2)
        .all(|pair| pair[0].evidence_sha256 < pair[1].evidence_sha256)
}

fn valid_snapshot_success(
    success: &RetainedResidualSuccess,
    config: &ResidualRetentionConfig,
    verdicts: &BTreeMap<Digest, ExactTerminalVerdict>,
    evaluations: &BTreeMap<Digest, Digest>,
    episodes: &BTreeMap<Digest, Digest>,
) -> bool {
    success.candidate_sha256 != Digest::ZERO
        && success.parent_tape_sha256 == config.parent_tape_sha256
        && success.realized_tape_sha256 == sha256(&success.realized_tape)
        && success.first_hit_tick > 0
        && success.first_hit_tick <= config.exploration_horizon_ticks
        && success.behavior_sha256 != Digest::ZERO
        && success.episode_sha256 != Digest::ZERO
        && success.declared_risk.intervention_frames > 0
        && success.declared_risk.residual_components > 0
        && !success.exact_evaluations.is_empty()
        && success
            .exact_evaluations
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        && verdicts.get(&success.realized_tape_sha256)
            == Some(&ExactTerminalVerdict::Reached {
                first_hit_tick: success.first_hit_tick,
            })
        && success
            .exact_evaluations
            .iter()
            .all(|evaluation| evaluations.get(evaluation) == Some(&success.realized_tape_sha256))
        && episodes.get(&success.episode_sha256) == Some(&success.realized_tape_sha256)
        && success
            .minimized_from
            .is_none_or(|source| source != Digest::ZERO && source != success.realized_tape_sha256)
        && snapshot_tape_metrics(
            &success.realized_tape,
            success.tape_frames,
            success.input_complexity,
        )
}

fn valid_snapshot_failure(
    failure: &RetainedResidualFailure,
    config: &ResidualRetentionConfig,
    verdicts: &BTreeMap<Digest, ExactTerminalVerdict>,
    evaluations: &BTreeMap<Digest, Digest>,
    episodes: &BTreeMap<Digest, Digest>,
) -> bool {
    failure.candidate_sha256 != Digest::ZERO
        && failure.parent_tape_sha256 == config.parent_tape_sha256
        && failure.realized_tape_sha256 == sha256(&failure.realized_tape)
        && failure.behavior_sha256 != Digest::ZERO
        && failure.episode_sha256 != Digest::ZERO
        && failure.evaluation_sha256 != Digest::ZERO
        && verdicts.get(&failure.realized_tape_sha256) == Some(&ExactTerminalVerdict::Miss)
        && evaluations.get(&failure.evaluation_sha256) == Some(&failure.realized_tape_sha256)
        && episodes.get(&failure.episode_sha256) == Some(&failure.realized_tape_sha256)
        && failure.reservoir_priority
            == failure_priority_values(
                failure.behavior_sha256,
                failure.realized_tape_sha256,
                failure.evaluation_sha256,
            )
        && InputTape::decode(&failure.realized_tape).is_ok()
}

fn snapshot_tape_metrics(bytes: &[u8], frame_count: u64, complexity: u64) -> bool {
    InputTape::decode(bytes).is_ok_and(|decoded| {
        decoded.tape.frames.len() as u64 == frame_count
            && tape_input_complexity(&decoded.tape) == complexity
    })
}

fn validate_evidence(
    config: &ResidualRetentionConfig,
    compiled: &CompiledResidualCandidate,
    evidence: &ResidualEvaluationEvidence,
) -> Result<(), ResidualRetentionError> {
    if evidence.candidate_sha256 != compiled.report.candidate_sha256
        || evidence.realized_tape_sha256 != compiled.report.realized_tape_sha256
        || evidence.terminal_program_sha256 != config.terminal_program_sha256
        || evidence.terminal_definition_sha256 != config.terminal_definition_sha256
        || evidence.evaluation_sha256 == Digest::ZERO
        || evidence.episode_sha256 == Digest::ZERO
        || evidence.behavior_sha256 == Digest::ZERO
    {
        return Err(retention_error(
            "residual evaluation is incomplete or uses a different terminal predicate",
        ));
    }
    Ok(())
}

fn validate_compilation(
    config: &ResidualRetentionConfig,
    compiled: &CompiledResidualCandidate,
) -> Result<InputTape, ResidualRetentionError> {
    if compiled.report.schema != RESIDUAL_COMPILATION_SCHEMA_V1
        || !compiled.report.realized_tape_authoritative
        || compiled.report.candidate_sha256 == Digest::ZERO
        || compiled.report.parent_tape_sha256 != config.parent_tape_sha256
        || compiled.report.realized_tape_sha256 != sha256(&compiled.bytes)
    {
        return Err(retention_error("residual compilation identity is invalid"));
    }
    let decoded = InputTape::decode(&compiled.bytes)
        .map_err(|source| retention_error(source.to_string()))?
        .tape;
    if decoded != compiled.tape || decoded.frames.len() as u64 != compiled.report.frame_count {
        return Err(retention_error(
            "authoritative realized tape differs from its compilation report",
        ));
    }
    Ok(decoded)
}

fn declared_risk(
    report: &ResidualCompilationReport,
    native_risk_events: Option<u64>,
) -> Result<DeclaredResidualRisk, ResidualRetentionError> {
    let intervention_frames = report
        .intervention_span
        .end_frame_exclusive
        .checked_sub(report.intervention_span.start_frame)
        .ok_or_else(|| retention_error("residual intervention span is invalid"))?;
    let residual_components = report
        .analog_residuals
        .checked_add(report.button_residuals)
        .ok_or_else(|| retention_error("residual component count overflowed"))?;
    if intervention_frames == 0 || residual_components == 0 {
        return Err(retention_error("residual declared risk is empty"));
    }
    Ok(DeclaredResidualRisk {
        intervention_frames,
        residual_components,
        native_risk_events,
    })
}

fn compare_success(left: &RetainedResidualSuccess, right: &RetainedResidualSuccess) -> Ordering {
    left.first_hit_tick
        .cmp(&right.first_hit_tick)
        .then(left.tape_frames.cmp(&right.tape_frames))
        .then(left.input_complexity.cmp(&right.input_complexity))
        .then_with(|| compare_optional_risk(left, right))
        .then(
            left.declared_risk
                .intervention_frames
                .cmp(&right.declared_risk.intervention_frames),
        )
        .then(
            left.declared_risk
                .residual_components
                .cmp(&right.declared_risk.residual_components),
        )
        .then(left.realized_tape_sha256.cmp(&right.realized_tape_sha256))
}

fn compare_optional_risk(
    left: &RetainedResidualSuccess,
    right: &RetainedResidualSuccess,
) -> Ordering {
    match (
        left.declared_risk.native_risk_events,
        right.declared_risk.native_risk_events,
    ) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_failure(left: &RetainedResidualFailure, right: &RetainedResidualFailure) -> Ordering {
    left.behavior_sha256
        .cmp(&right.behavior_sha256)
        .then(left.reservoir_priority.cmp(&right.reservoir_priority))
        .then(left.realized_tape_sha256.cmp(&right.realized_tape_sha256))
        .then(left.evaluation_sha256.cmp(&right.evaluation_sha256))
}

fn failure_priority(
    compiled: &CompiledResidualCandidate,
    evidence: &ResidualEvaluationEvidence,
) -> Digest {
    failure_priority_values(
        evidence.behavior_sha256,
        compiled.report.realized_tape_sha256,
        evidence.evaluation_sha256,
    )
}

fn failure_priority_values(
    behavior_sha256: Digest,
    realized_tape_sha256: Digest,
    evaluation_sha256: Digest,
) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.residual-failure-reservoir/v1\0");
    hasher.update(behavior_sha256.0);
    hasher.update(realized_tape_sha256.0);
    hasher.update(evaluation_sha256.0);
    Digest(hasher.finalize().into())
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, ResidualRetentionError> {
    let bytes = serde_json::to_vec(value).map_err(|source| retention_error(source.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResidualRetentionError(String);

fn retention_error(message: impl Into<String>) -> ResidualRetentionError {
    ResidualRetentionError(message.into())
}

impl fmt::Display for ResidualRetentionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ResidualRetentionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::residual_action::{
        AnalogChannel, AnalogResidual, ResidualCandidate, TemporalBasis, compile_residual_candidate,
    };
    use dusklight_automation_contracts::tape::InputFrame;

    fn digest(byte: u8) -> Digest {
        Digest([byte; 32])
    }

    fn config(parent_bytes: &[u8], capacity: u64) -> ResidualRetentionConfig {
        ResidualRetentionConfig {
            parent_tape_sha256: sha256(parent_bytes),
            terminal_program_sha256: digest(1),
            terminal_definition_sha256: digest(2),
            exploration_horizon_ticks: 160,
            promotion_before_tick: 125,
            maximum_candidates: 64,
            failures: FailureRetentionPolicy::DiversityReservoir { capacity },
        }
    }

    fn parent() -> (InputTape, Vec<u8>) {
        let tape = InputTape {
            frames: (0..32)
                .map(|_| InputFrame {
                    owned_ports: 1,
                    ..InputFrame::default()
                })
                .collect(),
            ..InputTape::default()
        };
        let bytes = tape.encode().unwrap();
        (tape, bytes)
    }

    fn compiled(
        parent: &InputTape,
        bytes: &[u8],
        frame: u64,
        delta: i16,
    ) -> CompiledResidualCandidate {
        let candidate = ResidualCandidate::seal(
            bytes,
            vec![AnalogResidual {
                port: 0,
                channel: AnalogChannel::MainX,
                basis: TemporalBasis::ExactFrame { frame, delta },
            }],
            vec![],
        )
        .unwrap();
        compile_residual_candidate(parent, bytes, &candidate).unwrap()
    }

    fn evidence(
        compiled: &CompiledResidualCandidate,
        byte: u8,
        behavior: u8,
        verdict: ExactTerminalVerdict,
        shaped: i64,
    ) -> ResidualEvaluationEvidence {
        ResidualEvaluationEvidence {
            candidate_sha256: compiled.report.candidate_sha256,
            realized_tape_sha256: compiled.report.realized_tape_sha256,
            terminal_program_sha256: digest(1),
            terminal_definition_sha256: digest(2),
            evaluation_sha256: digest(byte),
            episode_sha256: digest(byte.wrapping_add(64)),
            behavior_sha256: digest(behavior),
            verdict,
            shaped_progress_millionths: Some(shaped),
            native_risk_events: Some(0),
        }
    }

    #[test]
    fn retains_every_horizon_success_and_ranks_without_shaped_reward() {
        let (parent, bytes) = parent();
        let mut archive = ResidualOutcomeArchive::new(config(&bytes, 8)).unwrap();
        let slower = compiled(&parent, &bytes, 4, 5);
        archive
            .record(
                &slower,
                evidence(
                    &slower,
                    10,
                    20,
                    ExactTerminalVerdict::Reached {
                        first_hit_tick: 150,
                    },
                    i64::MAX,
                ),
            )
            .unwrap();
        let faster = compiled(&parent, &bytes, 8, 6);
        archive
            .record(
                &faster,
                evidence(
                    &faster,
                    11,
                    21,
                    ExactTerminalVerdict::Reached {
                        first_hit_tick: 124,
                    },
                    i64::MIN,
                ),
            )
            .unwrap();
        assert_eq!(archive.successes().len(), 2);
        assert_eq!(archive.successes()[0].first_hit_tick, 124);
        assert_eq!(archive.successes()[1].first_hit_tick, 150);
    }

    #[test]
    fn generation_rank_keeps_every_pending_failure_outside_the_reservoir() {
        let (parent, bytes) = parent();
        let slow = compiled(&parent, &bytes, 2, 3);
        let miss_a = compiled(&parent, &bytes, 3, 4);
        let fast = compiled(&parent, &bytes, 4, 5);
        let miss_b = compiled(&parent, &bytes, 5, 6);
        let slow_evidence = evidence(
            &slow,
            40,
            9,
            ExactTerminalVerdict::Reached {
                first_hit_tick: 150,
            },
            i64::MAX,
        );
        let miss_a_evidence = evidence(&miss_a, 41, 5, ExactTerminalVerdict::Miss, i64::MAX);
        let fast_evidence = evidence(
            &fast,
            42,
            8,
            ExactTerminalVerdict::Reached {
                first_hit_tick: 120,
            },
            i64::MIN,
        );
        let miss_b_evidence = evidence(&miss_b, 43, 6, ExactTerminalVerdict::Miss, i64::MIN);
        let ranked = rank_residual_generation(
            &config(&bytes, 1),
            &[
                ResidualGenerationEvaluation {
                    compiled: &slow,
                    evidence: &slow_evidence,
                },
                ResidualGenerationEvaluation {
                    compiled: &miss_a,
                    evidence: &miss_a_evidence,
                },
                ResidualGenerationEvaluation {
                    compiled: &fast,
                    evidence: &fast_evidence,
                },
                ResidualGenerationEvaluation {
                    compiled: &miss_b,
                    evidence: &miss_b_evidence,
                },
            ],
        )
        .unwrap();
        assert_eq!(ranked.len(), 4);
        assert_eq!(ranked[0], fast.report.candidate_sha256);
        assert_eq!(ranked[1], slow.report.candidate_sha256);
        assert!(ranked.contains(&miss_a.report.candidate_sha256));
        assert!(ranked.contains(&miss_b.report.candidate_sha256));
    }

    #[test]
    fn misses_remain_failure_experience_and_reservoir_prefers_diversity() {
        let (parent, bytes) = parent();
        let mut archive = ResidualOutcomeArchive::new(config(&bytes, 2)).unwrap();
        for (index, behavior) in [30_u8, 30, 31].into_iter().enumerate() {
            let candidate = compiled(&parent, &bytes, index as u64, index as i16 + 1);
            archive
                .record(
                    &candidate,
                    evidence(
                        &candidate,
                        20 + index as u8,
                        behavior,
                        ExactTerminalVerdict::Miss,
                        i64::MAX,
                    ),
                )
                .unwrap();
        }
        assert!(archive.successes().is_empty());
        assert_eq!(archive.failures().len(), 2);
        assert_eq!(
            archive
                .failures()
                .iter()
                .map(|failure| failure.behavior_sha256)
                .collect::<BTreeSet<_>>()
                .len(),
            2
        );
    }

    #[test]
    fn diverse_elites_and_horizon_tightening_require_a_supported_basin() {
        let (parent, bytes) = parent();
        let mut archive = ResidualOutcomeArchive::new(config(&bytes, 8)).unwrap();
        for (index, (tick, behavior)) in [(130, 40), (132, 41), (138, 40), (155, 42)]
            .into_iter()
            .enumerate()
        {
            let candidate = compiled(&parent, &bytes, index as u64, index as i16 + 1);
            archive
                .record(
                    &candidate,
                    evidence(
                        &candidate,
                        30 + index as u8,
                        behavior,
                        ExactTerminalVerdict::Reached {
                            first_hit_tick: tick,
                        },
                        0,
                    ),
                )
                .unwrap();
        }
        let elites = archive.diverse_success_elites(3);
        assert_eq!(
            elites
                .iter()
                .map(|success| success.behavior_sha256)
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
        let strict = HorizonSupportPolicy {
            minimum_successes: 4,
            minimum_behavior_classes: 3,
            minimum_support_millionths: 1_000_000,
        };
        assert!(archive.horizon_tightening_evidence(145, strict).is_err());
        let supported = archive
            .horizon_tightening_evidence(
                145,
                HorizonSupportPolicy {
                    minimum_successes: 3,
                    minimum_behavior_classes: 2,
                    minimum_support_millionths: 750_000,
                },
            )
            .unwrap();
        assert_eq!(supported.supporting_successes, 3);
    }

    #[test]
    fn minimization_requires_prior_discovery_strict_simplicity_and_exact_replay() {
        let (parent, bytes) = parent();
        let original = {
            let candidate = ResidualCandidate::seal(
                &bytes,
                vec![
                    AnalogResidual {
                        port: 0,
                        channel: AnalogChannel::MainX,
                        basis: TemporalBasis::ExactFrame { frame: 3, delta: 5 },
                    },
                    AnalogResidual {
                        port: 0,
                        channel: AnalogChannel::MainX,
                        basis: TemporalBasis::ExactFrame { frame: 8, delta: 5 },
                    },
                ],
                vec![],
            )
            .unwrap();
            compile_residual_candidate(&parent, &bytes, &candidate).unwrap()
        };
        let minimized = compiled(&parent, &bytes, 3, 5);
        let mut archive = ResidualOutcomeArchive::new(config(&bytes, 8)).unwrap();
        assert!(
            archive
                .accept_minimized(
                    original.report.realized_tape_sha256,
                    &minimized,
                    evidence(
                        &minimized,
                        50,
                        50,
                        ExactTerminalVerdict::Reached {
                            first_hit_tick: 130,
                        },
                        0,
                    )
                )
                .is_err()
        );
        archive
            .record(
                &original,
                evidence(
                    &original,
                    51,
                    50,
                    ExactTerminalVerdict::Reached {
                        first_hit_tick: 130,
                    },
                    0,
                ),
            )
            .unwrap();
        archive
            .accept_minimized(
                original.report.realized_tape_sha256,
                &minimized,
                evidence(
                    &minimized,
                    52,
                    51,
                    ExactTerminalVerdict::Reached {
                        first_hit_tick: 130,
                    },
                    0,
                ),
            )
            .unwrap();
        assert_eq!(archive.successes().len(), 2);
        assert!(archive.successes().iter().any(|success| {
            success.minimized_from == Some(original.report.realized_tape_sha256)
        }));
    }

    #[test]
    fn detached_predicates_and_out_of_horizon_hits_fail_closed() {
        let (parent, bytes) = parent();
        let candidate = compiled(&parent, &bytes, 1, 1);
        let mut archive = ResidualOutcomeArchive::new(config(&bytes, 8)).unwrap();
        let mut detached = evidence(&candidate, 60, 60, ExactTerminalVerdict::Miss, 0);
        detached.terminal_program_sha256 = digest(9);
        assert!(archive.record(&candidate, detached).is_err());
        assert!(
            archive
                .record(
                    &candidate,
                    evidence(
                        &candidate,
                        61,
                        60,
                        ExactTerminalVerdict::Reached {
                            first_hit_tick: 161,
                        },
                        0,
                    )
                )
                .is_err()
        );
        assert!(archive.snapshot().unwrap().evaluated_tapes.is_empty());
    }

    #[test]
    fn sealed_snapshot_restores_success_failure_and_dropped_history_exactly() {
        let (parent, bytes) = parent();
        let mut archive = ResidualOutcomeArchive::new(config(&bytes, 2)).unwrap();
        for (index, behavior) in [70_u8, 70, 71].into_iter().enumerate() {
            let candidate = compiled(&parent, &bytes, index as u64, index as i16 + 1);
            archive
                .record(
                    &candidate,
                    evidence(
                        &candidate,
                        70 + index as u8,
                        behavior,
                        ExactTerminalVerdict::Miss,
                        index as i64,
                    ),
                )
                .unwrap();
        }
        let success = compiled(&parent, &bytes, 8, 12);
        archive
            .record(
                &success,
                evidence(
                    &success,
                    80,
                    80,
                    ExactTerminalVerdict::Reached {
                        first_hit_tick: 140,
                    },
                    i64::MIN,
                ),
            )
            .unwrap();
        let snapshot = archive.snapshot().unwrap();
        assert_eq!(snapshot.failures.len(), 2);
        assert_eq!(snapshot.evaluated_tapes.len(), 4);
        assert_eq!(snapshot.evaluation_bindings.len(), 4);
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        let decoded: ResidualRetentionSnapshot = serde_json::from_slice(&bytes).unwrap();
        let restored = ResidualOutcomeArchive::restore(decoded).unwrap();
        assert_eq!(restored.snapshot().unwrap(), snapshot);
        assert_eq!(restored.optimizer_rank(), archive.optimizer_rank());

        let mut tampered = snapshot;
        tampered.successes[0].realized_tape[0] ^= 1;
        tampered.content_sha256 = tampered.compute_identity().unwrap();
        assert!(ResidualOutcomeArchive::restore(tampered).is_err());
    }

    #[test]
    fn evidence_reuse_and_terminal_disagreement_fail_without_partial_mutation() {
        let (parent, bytes) = parent();
        let first = compiled(&parent, &bytes, 2, 4);
        let second = compiled(&parent, &bytes, 3, 5);
        let mut archive = ResidualOutcomeArchive::new(config(&bytes, 8)).unwrap();
        let miss = evidence(&first, 90, 90, ExactTerminalVerdict::Miss, i64::MAX);
        archive.record(&first, miss).unwrap();
        let before = archive.snapshot().unwrap();

        let mut reused = evidence(&second, 90, 91, ExactTerminalVerdict::Miss, i64::MIN);
        reused.episode_sha256 = digest(91);
        assert!(archive.record(&second, reused).is_err());
        assert_eq!(archive.snapshot().unwrap(), before);

        let disagreement = evidence(
            &first,
            92,
            90,
            ExactTerminalVerdict::Reached {
                first_hit_tick: 130,
            },
            0,
        );
        assert!(archive.record(&first, disagreement).is_err());
        assert_eq!(archive.snapshot().unwrap(), before);

        let mut detached = evidence(&second, 93, 91, ExactTerminalVerdict::Miss, 0);
        detached.candidate_sha256 = digest(99);
        assert!(archive.record(&second, detached).is_err());
        assert_eq!(archive.snapshot().unwrap(), before);
    }
}
