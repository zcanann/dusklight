//! Independently replayed evidence adapters for Gate 4 comparison cells.

use crate::learning_value_comparison::{
    LearningValueCheckpoint, LearningValueComparisonPlan, LearningValueTreatment,
    LearningValueTreatmentKind,
};
use crate::native_goal_learning_loop::{
    NativeGoalLearningCheckpointReport, NativeGoalLearningLoopRequest, NativeGoalLearningLoopState,
};
use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::optimization_request::{OptimizationRequest, ResidualOptimizerConfig};
use crate::residual_campaign::ResidualCampaignCheckpoint;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_learning::native_replay_corpus::DemonstrationMode;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path};

pub const LEARNING_VALUE_CELL_DRAFT_SCHEMA_V1: &str = "dusklight-learning-value-cell-draft/v1";
pub const LEARNING_VALUE_CELL_EVIDENCE_SCHEMA_V1: &str =
    "dusklight-learning-value-cell-evidence/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningValueCellDraft {
    pub schema: String,
    pub checkpoint_id: String,
    pub deterministic_seed: u64,
    pub treatment: LearningValueTreatmentKind,
    pub phases: Vec<LearningValuePhaseSource>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LearningValuePhaseSource {
    Residual {
        optimization_request: ArtifactReference,
        execution_binding: ArtifactReference,
        final_checkpoint: ArtifactReference,
    },
    StateReactive {
        loop_request: ArtifactReference,
        optimization_request: ArtifactReference,
        execution_binding: ArtifactReference,
        loop_state: ArtifactReference,
        checkpoint_report: ArtifactReference,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningValueCellEvidence {
    pub schema: String,
    pub content_sha256: Digest,
    pub plan_sha256: Digest,
    pub checkpoint_id: String,
    pub deterministic_seed: u64,
    pub treatment: LearningValueTreatmentKind,
    pub phases: Vec<LearningValuePhaseEvidence>,
    pub metrics: LearningValuePerformanceMetrics,
    pub promotion_authority: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningValuePhaseEvidence {
    pub source: LearningValuePhaseSource,
    pub metrics: LearningValuePerformanceMetrics,
    pub terminal_program_sha256: Digest,
    pub terminal_definition_sha256: Digest,
    pub realized_tape_sha256: Vec<Digest>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningValuePerformanceMetrics {
    pub charged_simulated_ticks: u64,
    pub evaluated_episodes: u64,
    pub successful_episodes: u64,
    pub successful_episode_rate_millionths: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_first_hit_tick: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_success_charged_simulated_ticks: Option<u64>,
}

impl LearningValueCellEvidence {
    pub fn seal(
        draft: LearningValueCellDraft,
        plan: &LearningValueComparisonPlan,
        repository_root: &Path,
    ) -> Result<Self, LearningValueEvidenceError> {
        plan.validate_files(repository_root)
            .map_err(evidence_error)?;
        validate_draft(&draft, plan)?;
        let checkpoint = plan_checkpoint(plan, &draft.checkpoint_id)?;
        let treatment = plan_treatment(plan, draft.treatment)?;
        let phases = evaluate_phases(
            repository_root,
            plan,
            checkpoint,
            treatment,
            draft.deterministic_seed,
            &draft.phases,
        )?;
        let metrics = combine_metrics(&phases)?;
        let mut evidence = Self {
            schema: LEARNING_VALUE_CELL_EVIDENCE_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            plan_sha256: plan.content_sha256,
            checkpoint_id: draft.checkpoint_id,
            deterministic_seed: draft.deterministic_seed,
            treatment: draft.treatment,
            phases,
            metrics,
            promotion_authority: false,
        };
        evidence.content_sha256 = evidence.identity()?;
        evidence.validate_files(plan, repository_root)?;
        Ok(evidence)
    }

    pub fn validate_files(
        &self,
        plan: &LearningValueComparisonPlan,
        repository_root: &Path,
    ) -> Result<(), LearningValueEvidenceError> {
        plan.validate_files(repository_root)
            .map_err(evidence_error)?;
        if self.schema != LEARNING_VALUE_CELL_EVIDENCE_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.plan_sha256 != plan.content_sha256
            || !plan.deterministic_seeds.contains(&self.deterministic_seed)
            || self.promotion_authority
            || self.content_sha256 != self.identity()?
        {
            return Err(evidence_message(
                "learning-value cell evidence or seal is invalid",
            ));
        }
        let checkpoint = plan_checkpoint(plan, &self.checkpoint_id)?;
        let treatment = plan_treatment(plan, self.treatment)?;
        validate_phase_shape(self.treatment, &self.phases)?;
        if self.phases.iter().any(|phase| {
            phase.terminal_program_sha256 != plan.terminal_program_sha256
                || phase.terminal_definition_sha256 != plan.terminal_definition_sha256
                || phase.realized_tape_sha256.is_empty()
                || phase
                    .realized_tape_sha256
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                || validate_metrics(&phase.metrics).is_err()
        }) {
            return Err(evidence_message(
                "learning-value phase terminal, tape set, or metrics are invalid",
            ));
        }
        let sources = self
            .phases
            .iter()
            .map(|phase| phase.source.clone())
            .collect::<Vec<_>>();
        let reproduced = evaluate_phases(
            repository_root,
            plan,
            checkpoint,
            treatment,
            self.deterministic_seed,
            &sources,
        )?;
        if reproduced != self.phases || combine_metrics(&reproduced)? != self.metrics {
            return Err(evidence_message(
                "learning-value cell metrics differ from independently replayed source artifacts",
            ));
        }
        let budget = treatment.budget();
        let (discovery, refinement) = phase_charges(&self.phases);
        if discovery > budget.discovery_simulated_ticks
            || refinement > budget.refinement_simulated_ticks
            || self.metrics.charged_simulated_ticks > plan.simulated_tick_budget_per_cell
        {
            return Err(evidence_message(
                "learning-value cell exceeds its sealed discovery or refinement tick cap",
            ));
        }
        if self.treatment == LearningValueTreatmentKind::LearnedThenResidualRefinement {
            let learned = &self.phases[0].realized_tape_sha256;
            let residual_incumbent = residual_incumbent_sha256(repository_root, &self.phases[1])?;
            if !learned.contains(&residual_incumbent) {
                return Err(evidence_message(
                    "learned residual refinement incumbent is not an exact realized learning-phase tape",
                ));
            }
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, LearningValueEvidenceError> {
        let mut bytes = serde_json::to_vec_pretty(self).map_err(evidence_error)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn identity(&self) -> Result<Digest, LearningValueEvidenceError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.learning-value-cell-evidence/v1\0", &canonical)
    }
}

fn validate_draft(
    draft: &LearningValueCellDraft,
    plan: &LearningValueComparisonPlan,
) -> Result<(), LearningValueEvidenceError> {
    if draft.schema != LEARNING_VALUE_CELL_DRAFT_SCHEMA_V1
        || !plan.deterministic_seeds.contains(&draft.deterministic_seed)
    {
        return Err(evidence_message(
            "learning-value cell draft schema or deterministic seed is invalid",
        ));
    }
    plan_checkpoint(plan, &draft.checkpoint_id)?;
    plan_treatment(plan, draft.treatment)?;
    validate_source_shape(draft.treatment, &draft.phases)
}

fn validate_source_shape(
    treatment: LearningValueTreatmentKind,
    phases: &[LearningValuePhaseSource],
) -> Result<(), LearningValueEvidenceError> {
    let valid = match treatment {
        LearningValueTreatmentKind::IndependentRandomResidual
        | LearningValueTreatmentKind::CemResidual => {
            matches!(phases, [LearningValuePhaseSource::Residual { .. }])
        }
        LearningValueTreatmentKind::DemonstrationAssistedStateReactive
        | LearningValueTreatmentKind::FromScratchStateReactive => {
            matches!(phases, [LearningValuePhaseSource::StateReactive { .. }])
        }
        LearningValueTreatmentKind::LearnedThenResidualRefinement => matches!(
            phases,
            [
                LearningValuePhaseSource::StateReactive { .. },
                LearningValuePhaseSource::Residual { .. }
            ]
        ),
    };
    if valid {
        Ok(())
    } else {
        Err(evidence_message(
            "learning-value cell phase sequence differs from its sealed treatment",
        ))
    }
}

fn validate_phase_shape(
    treatment: LearningValueTreatmentKind,
    phases: &[LearningValuePhaseEvidence],
) -> Result<(), LearningValueEvidenceError> {
    validate_source_shape(
        treatment,
        &phases
            .iter()
            .map(|phase| phase.source.clone())
            .collect::<Vec<_>>(),
    )
}

fn evaluate_phases(
    root: &Path,
    plan: &LearningValueComparisonPlan,
    checkpoint: &LearningValueCheckpoint,
    treatment: &LearningValueTreatment,
    seed: u64,
    sources: &[LearningValuePhaseSource],
) -> Result<Vec<LearningValuePhaseEvidence>, LearningValueEvidenceError> {
    validate_source_shape(treatment.kind(), sources)?;
    sources
        .iter()
        .map(|source| match source {
            LearningValuePhaseSource::Residual { .. } => {
                evaluate_residual_phase(root, plan, checkpoint, treatment, seed, source.clone())
            }
            LearningValuePhaseSource::StateReactive { .. } => {
                evaluate_learning_phase(root, plan, checkpoint, treatment, seed, source.clone())
            }
        })
        .collect()
}

fn evaluate_residual_phase(
    root: &Path,
    plan: &LearningValueComparisonPlan,
    checkpoint: &LearningValueCheckpoint,
    treatment: &LearningValueTreatment,
    seed: u64,
    source: LearningValuePhaseSource,
) -> Result<LearningValuePhaseEvidence, LearningValueEvidenceError> {
    let LearningValuePhaseSource::Residual {
        optimization_request,
        execution_binding,
        final_checkpoint,
    } = &source
    else {
        unreachable!()
    };
    let optimization: OptimizationRequest = read_json(root, optimization_request)?;
    optimization.validate_files(root).map_err(evidence_error)?;
    let execution: NativeResidualExecutionBinding = read_json(root, execution_binding)?;
    execution
        .validate_files(root, &optimization)
        .map_err(evidence_error)?;
    let campaign: ResidualCampaignCheckpoint = read_json(root, final_checkpoint)?;
    campaign
        .validate(&optimization, execution.content_sha256)
        .map_err(evidence_error)?;
    let audit = campaign.audit.as_ref().ok_or_else(|| {
        evidence_message("learning-value residual phase requires a v4 checkpoint audit")
    })?;
    if !audit.declared_budget_complete
        || optimization.execution.deterministic_seeds != [seed]
        || optimization.execution.repetitions != plan.repetitions_per_cell
        || optimization.budgets.simulated_tick_budget
            != treatment.budget().refinement_simulated_ticks
        || optimization.terminal_predicate.program_sha256 != plan.terminal_program_sha256
        || optimization.terminal_predicate.definition_sha256 != plan.terminal_definition_sha256
        || !optimization_matches_checkpoint(&optimization, checkpoint)
        || !residual_optimizer_matches(treatment, &optimization.proposal.optimizer)
    {
        return Err(evidence_message(
            "learning-value residual phase differs from its plan, seed, terminal, optimizer, or completed budget",
        ));
    }
    let incumbent = optimization
        .incumbent
        .as_ref()
        .ok_or_else(|| evidence_message("learning-value residual phase lacks an incumbent"))?;
    if treatment.kind() != LearningValueTreatmentKind::LearnedThenResidualRefinement
        && incumbent.tape != checkpoint.source
    {
        return Err(evidence_message(
            "learning-value residual baseline does not start from the held-out checkpoint tape",
        ));
    }
    let first_success = audit
        .improvement_by_simulated_tick
        .first()
        .map(|point| point.charged_simulated_ticks);
    let metrics = LearningValuePerformanceMetrics {
        charged_simulated_ticks: audit.charged_simulated_ticks,
        evaluated_episodes: audit.evaluated_episodes,
        successful_episodes: audit.successful_episodes,
        successful_episode_rate_millionths: audit.successful_episode_rate_millionths,
        best_first_hit_tick: audit.best_first_hit_tick,
        first_success_charged_simulated_ticks: first_success,
    };
    validate_metrics(&metrics)?;
    Ok(LearningValuePhaseEvidence {
        source,
        metrics,
        terminal_program_sha256: optimization.terminal_predicate.program_sha256,
        terminal_definition_sha256: optimization.terminal_predicate.definition_sha256,
        realized_tape_sha256: campaign
            .retention
            .evaluated_tapes
            .iter()
            .map(|entry| entry.realized_tape_sha256)
            .collect(),
    })
}

fn evaluate_learning_phase(
    root: &Path,
    plan: &LearningValueComparisonPlan,
    checkpoint: &LearningValueCheckpoint,
    treatment: &LearningValueTreatment,
    seed: u64,
    source: LearningValuePhaseSource,
) -> Result<LearningValuePhaseEvidence, LearningValueEvidenceError> {
    let LearningValuePhaseSource::StateReactive {
        loop_request,
        optimization_request,
        execution_binding,
        loop_state,
        checkpoint_report,
    } = &source
    else {
        unreachable!()
    };
    let optimization: OptimizationRequest = read_json(root, optimization_request)?;
    optimization.validate_files(root).map_err(evidence_error)?;
    let execution: NativeResidualExecutionBinding = read_json(root, execution_binding)?;
    execution
        .validate_files(root, &optimization)
        .map_err(evidence_error)?;
    let request: NativeGoalLearningLoopRequest = read_json(root, loop_request)?;
    request
        .validate_files(root, &optimization, &execution)
        .map_err(evidence_error)?;
    let state: NativeGoalLearningLoopState = read_json(root, loop_state)?;
    state.validate().map_err(evidence_error)?;
    let report: NativeGoalLearningCheckpointReport = read_json(root, checkpoint_report)?;
    report.validate().map_err(evidence_error)?;
    if state.request_sha256 != request.content_sha256
        || report.source_loop_state_sha256 != state.state_sha256
        || request.resume.state_path != loop_state.path
        || state.stopped.is_none()
        || optimization.execution.deterministic_seeds != [seed]
        || optimization.execution.repetitions != plan.repetitions_per_cell
        || request.simulated_tick_budget != treatment.budget().discovery_simulated_ticks
        || optimization.terminal_predicate.program_sha256 != plan.terminal_program_sha256
        || optimization.terminal_predicate.definition_sha256 != plan.terminal_definition_sha256
        || !optimization_matches_checkpoint(&optimization, checkpoint)
        || optimization.incumbent.as_ref().map(|value| &value.tape) != Some(&checkpoint.source)
        || !learning_mode_matches(treatment, request.demonstration_mode)
    {
        return Err(evidence_message(
            "learning-value state-reactive phase differs from its request, state, report, seed, checkpoint, terminal, or demonstration mode",
        ));
    }
    let mut charged = 0_u64;
    let mut episodes = 0_u64;
    let mut successes = 0_u64;
    let mut best = None;
    let mut first_success = None;
    let mut realized = BTreeSet::new();
    for generation in state
        .generations
        .iter()
        .filter(|generation| generation.committed_record_sha256.is_some())
    {
        for reference in generation
            .episode_shards
            .as_ref()
            .ok_or_else(|| evidence_message("committed learning generation lacks episode shards"))?
        {
            let path = referenced_path(root, reference)?;
            let shard = NativeEpisodeShard::read(&path).map_err(evidence_error)?;
            if shard.content_sha256 != reference.sha256 {
                return Err(evidence_message(
                    "learning episode shard differs from its artifact reference",
                ));
            }
            for episode in &shard.episodes {
                let prior = charged;
                charged = charged
                    .checked_add(u64::from(episode.ticks_executed))
                    .ok_or_else(|| evidence_message("learning phase tick count overflowed"))?;
                episodes = episodes
                    .checked_add(1)
                    .ok_or_else(|| evidence_message("learning phase episode count overflowed"))?;
                if episode.success {
                    successes = successes.checked_add(1).ok_or_else(|| {
                        evidence_message("learning phase success count overflowed")
                    })?;
                    let hit = u64::from(episode.first_hit_tick.ok_or_else(|| {
                        evidence_message("successful learning episode lacks first-hit tick")
                    })?);
                    best = Some(best.map_or(hit, |value: u64| value.min(hit)));
                    first_success.get_or_insert(prior.saturating_add(hit).saturating_add(1));
                }
            }
        }
        for reference in generation
            .realized_tapes
            .as_ref()
            .ok_or_else(|| evidence_message("committed learning generation lacks realized tapes"))?
        {
            read_reference(root, reference)?;
            realized.insert(reference.sha256);
        }
    }
    if charged != report.charged_simulated_ticks
        || episodes
            != report
                .checkpoints
                .iter()
                .map(|checkpoint| checkpoint.rollouts)
                .sum::<u64>()
        || successes
            != report
                .checkpoints
                .iter()
                .map(|checkpoint| checkpoint.terminal_successes)
                .sum::<u64>()
    {
        return Err(evidence_message(
            "learning episode shards do not reproduce the sealed checkpoint report",
        ));
    }
    let metrics = metrics(charged, episodes, successes, best, first_success)?;
    Ok(LearningValuePhaseEvidence {
        source,
        metrics,
        terminal_program_sha256: optimization.terminal_predicate.program_sha256,
        terminal_definition_sha256: optimization.terminal_predicate.definition_sha256,
        realized_tape_sha256: realized.into_iter().collect(),
    })
}

fn combine_metrics(
    phases: &[LearningValuePhaseEvidence],
) -> Result<LearningValuePerformanceMetrics, LearningValueEvidenceError> {
    let mut charged = 0_u64;
    let mut episodes = 0_u64;
    let mut successes = 0_u64;
    let mut best = None;
    let mut first = None;
    for phase in phases {
        let prior = charged;
        charged = charged
            .checked_add(phase.metrics.charged_simulated_ticks)
            .ok_or_else(|| evidence_message("cell tick count overflowed"))?;
        episodes = episodes
            .checked_add(phase.metrics.evaluated_episodes)
            .ok_or_else(|| evidence_message("cell episode count overflowed"))?;
        successes = successes
            .checked_add(phase.metrics.successful_episodes)
            .ok_or_else(|| evidence_message("cell success count overflowed"))?;
        if let Some(hit) = phase.metrics.best_first_hit_tick {
            best = Some(best.map_or(hit, |value: u64| value.min(hit)));
        }
        if first.is_none() {
            first = phase
                .metrics
                .first_success_charged_simulated_ticks
                .and_then(|value| prior.checked_add(value));
        }
    }
    metrics(charged, episodes, successes, best, first)
}

fn metrics(
    charged: u64,
    episodes: u64,
    successes: u64,
    best: Option<u64>,
    first: Option<u64>,
) -> Result<LearningValuePerformanceMetrics, LearningValueEvidenceError> {
    let rate = if episodes == 0 {
        0
    } else {
        u32::try_from(successes.saturating_mul(1_000_000) / episodes)
            .map_err(|_| evidence_message("success rate overflowed"))?
    };
    let value = LearningValuePerformanceMetrics {
        charged_simulated_ticks: charged,
        evaluated_episodes: episodes,
        successful_episodes: successes,
        successful_episode_rate_millionths: rate,
        best_first_hit_tick: best,
        first_success_charged_simulated_ticks: first,
    };
    validate_metrics(&value)?;
    Ok(value)
}

fn validate_metrics(
    metrics: &LearningValuePerformanceMetrics,
) -> Result<(), LearningValueEvidenceError> {
    if metrics.charged_simulated_ticks == 0
        || metrics.evaluated_episodes == 0
        || metrics.successful_episodes > metrics.evaluated_episodes
        || metrics.successful_episode_rate_millionths
            != u32::try_from(
                metrics.successful_episodes.saturating_mul(1_000_000) / metrics.evaluated_episodes,
            )
            .unwrap_or(u32::MAX)
        || (metrics.successful_episodes == 0)
            != (metrics.best_first_hit_tick.is_none()
                && metrics.first_success_charged_simulated_ticks.is_none())
        || metrics
            .first_success_charged_simulated_ticks
            .is_some_and(|value| value == 0 || value > metrics.charged_simulated_ticks)
    {
        return Err(evidence_message(
            "learning-value performance metrics are internally inconsistent",
        ));
    }
    Ok(())
}

fn residual_optimizer_matches(
    treatment: &LearningValueTreatment,
    optimizer: &ResidualOptimizerConfig,
) -> bool {
    match treatment {
        LearningValueTreatment::IndependentRandomResidual {
            optimizer: expected,
            ..
        }
        | LearningValueTreatment::CemResidual {
            optimizer: expected,
            ..
        } => optimizer == expected,
        LearningValueTreatment::LearnedThenResidualRefinement {
            refinement_optimizer,
            ..
        } => optimizer == refinement_optimizer,
        _ => false,
    }
}

fn optimization_matches_checkpoint(
    optimization: &OptimizationRequest,
    checkpoint: &LearningValueCheckpoint,
) -> bool {
    optimization.route.source_boundary_index == checkpoint.source_boundary_index
        && optimization.route.native_source_boundary_fingerprint
            == checkpoint.native_source_boundary_fingerprint
}

fn learning_mode_matches(treatment: &LearningValueTreatment, mode: DemonstrationMode) -> bool {
    match treatment.kind() {
        LearningValueTreatmentKind::DemonstrationAssistedStateReactive
        | LearningValueTreatmentKind::LearnedThenResidualRefinement => {
            mode != DemonstrationMode::Absent
        }
        LearningValueTreatmentKind::FromScratchStateReactive => mode == DemonstrationMode::Absent,
        _ => false,
    }
}

fn phase_charges(phases: &[LearningValuePhaseEvidence]) -> (u64, u64) {
    phases
        .iter()
        .fold((0, 0), |(discovery, residual), phase| match phase.source {
            LearningValuePhaseSource::StateReactive { .. } => (
                discovery.saturating_add(phase.metrics.charged_simulated_ticks),
                residual,
            ),
            LearningValuePhaseSource::Residual { .. } => (
                discovery,
                residual.saturating_add(phase.metrics.charged_simulated_ticks),
            ),
        })
}

fn residual_incumbent_sha256(
    root: &Path,
    phase: &LearningValuePhaseEvidence,
) -> Result<Digest, LearningValueEvidenceError> {
    let LearningValuePhaseSource::Residual {
        optimization_request,
        ..
    } = &phase.source
    else {
        return Err(evidence_message("expected residual refinement phase"));
    };
    let optimization: OptimizationRequest = read_json(root, optimization_request)?;
    Ok(optimization
        .incumbent
        .ok_or_else(|| evidence_message("residual refinement lacks incumbent"))?
        .tape
        .sha256)
}

fn plan_checkpoint<'a>(
    plan: &'a LearningValueComparisonPlan,
    id: &str,
) -> Result<&'a LearningValueCheckpoint, LearningValueEvidenceError> {
    plan.held_out_checkpoints
        .iter()
        .find(|checkpoint| checkpoint.id == id)
        .ok_or_else(|| evidence_message("learning-value cell checkpoint is absent from the plan"))
}

fn plan_treatment(
    plan: &LearningValueComparisonPlan,
    kind: LearningValueTreatmentKind,
) -> Result<&LearningValueTreatment, LearningValueEvidenceError> {
    plan.treatments
        .iter()
        .find(|treatment| treatment.kind() == kind)
        .ok_or_else(|| evidence_message("learning-value cell treatment is absent from the plan"))
}

fn read_json<T: for<'de> Deserialize<'de>>(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<T, LearningValueEvidenceError> {
    serde_json::from_slice(&read_reference(root, reference)?).map_err(evidence_error)
}

fn read_reference(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<Vec<u8>, LearningValueEvidenceError> {
    let path = referenced_path(root, reference)?;
    let bytes = fs::read(path).map_err(evidence_error)?;
    if sha256(&bytes) != reference.sha256 {
        return Err(evidence_message(
            "learning-value artifact content differs from its reference",
        ));
    }
    Ok(bytes)
}

fn referenced_path(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<std::path::PathBuf, LearningValueEvidenceError> {
    let root = root.canonicalize().map_err(evidence_error)?;
    let relative = Path::new(&reference.path);
    if reference.sha256 == Digest::ZERO
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(evidence_message(
            "learning-value artifact reference is invalid",
        ));
    }
    let path = root.join(relative).canonicalize().map_err(evidence_error)?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(evidence_message(
            "learning-value artifact must resolve to a repository file",
        ));
    }
    Ok(path)
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, LearningValueEvidenceError> {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(serde_json::to_vec(value).map_err(evidence_error)?);
    Ok(Digest(hasher.finalize().into()))
}

fn sha256(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Digest(hasher.finalize().into())
}

#[derive(Debug)]
pub struct LearningValueEvidenceError(String);

impl fmt::Display for LearningValueEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Error for LearningValueEvidenceError {}

fn evidence_message(message: impl Into<String>) -> LearningValueEvidenceError {
    LearningValueEvidenceError(message.into())
}

fn evidence_error(error: impl Error) -> LearningValueEvidenceError {
    evidence_message(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_combination_charges_phase_order_and_recomputes_rate() {
        let source = LearningValuePhaseSource::Residual {
            optimization_request: reference("request"),
            execution_binding: reference("execution"),
            final_checkpoint: reference("checkpoint"),
        };
        let phases = vec![
            LearningValuePhaseEvidence {
                source: source.clone(),
                metrics: LearningValuePerformanceMetrics {
                    charged_simulated_ticks: 100,
                    evaluated_episodes: 2,
                    successful_episodes: 0,
                    successful_episode_rate_millionths: 0,
                    best_first_hit_tick: None,
                    first_success_charged_simulated_ticks: None,
                },
                terminal_program_sha256: Digest([2; 32]),
                terminal_definition_sha256: Digest([3; 32]),
                realized_tape_sha256: vec![Digest([4; 32])],
            },
            LearningValuePhaseEvidence {
                source,
                metrics: LearningValuePerformanceMetrics {
                    charged_simulated_ticks: 50,
                    evaluated_episodes: 2,
                    successful_episodes: 1,
                    successful_episode_rate_millionths: 500_000,
                    best_first_hit_tick: Some(7),
                    first_success_charged_simulated_ticks: Some(30),
                },
                terminal_program_sha256: Digest([2; 32]),
                terminal_definition_sha256: Digest([3; 32]),
                realized_tape_sha256: vec![Digest([5; 32])],
            },
        ];
        let metrics = combine_metrics(&phases).unwrap();
        assert_eq!(metrics.charged_simulated_ticks, 150);
        assert_eq!(metrics.successful_episode_rate_millionths, 250_000);
        assert_eq!(metrics.first_success_charged_simulated_ticks, Some(130));
    }

    #[test]
    fn phase_shape_rejects_wrong_or_reordered_adapters() {
        let residual = LearningValuePhaseSource::Residual {
            optimization_request: reference("request"),
            execution_binding: reference("execution"),
            final_checkpoint: reference("checkpoint"),
        };
        assert!(
            validate_source_shape(
                LearningValueTreatmentKind::LearnedThenResidualRefinement,
                std::slice::from_ref(&residual),
            )
            .is_err()
        );
    }

    fn reference(path: &str) -> ArtifactReference {
        ArtifactReference {
            path: path.into(),
            sha256: Digest([1; 32]),
        }
    }
}
