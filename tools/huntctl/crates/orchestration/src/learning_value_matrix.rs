//! Reproducible materialization of Gate 4 experimental cells.
//!
//! This module derives cell requests from an already validated checkpoint
//! request. It changes only plan-owned experimental dimensions and gives every
//! cell an isolated resume namespace.

use crate::learning_value_comparison::{
    LearningValueCheckpoint, LearningValueComparisonPlan, LearningValueTreatment,
    LearningValueTreatmentKind,
};
use crate::optimization_request::{OptimizationRequest, ResidualOptimizerConfig};
use std::error::Error;
use std::fmt;
use std::path::Path;

pub fn materialize_residual_cell_request(
    plan: &LearningValueComparisonPlan,
    checkpoint_id: &str,
    deterministic_seed: u64,
    treatment_kind: LearningValueTreatmentKind,
    base: &OptimizationRequest,
    repository_root: &Path,
) -> Result<OptimizationRequest, LearningValueMatrixError> {
    plan.validate_files(repository_root).map_err(matrix_error)?;
    base.validate_files(repository_root).map_err(matrix_error)?;

    if !plan.deterministic_seeds.contains(&deterministic_seed) {
        return Err(matrix_message(
            "learning-value cell seed is absent from the sealed plan",
        ));
    }
    let checkpoint = plan
        .held_out_checkpoints
        .iter()
        .find(|checkpoint| checkpoint.id == checkpoint_id)
        .ok_or_else(|| {
            matrix_message("learning-value cell checkpoint is absent from the sealed plan")
        })?;
    let treatment = plan
        .treatments
        .iter()
        .find(|treatment| treatment.kind() == treatment_kind)
        .ok_or_else(|| {
            matrix_message("learning-value cell treatment is absent from the sealed plan")
        })?;
    let optimizer = baseline_optimizer(treatment)?.clone();
    validate_base(plan, checkpoint, base)?;

    let mut request = base.clone();
    request.id = cell_id(&plan.id, checkpoint_id, treatment_kind, deterministic_seed)?;
    request.budgets.candidate_budget = optimizer_candidates(&optimizer)?;
    request.budgets.simulated_tick_budget = treatment.budget().refinement_simulated_ticks;
    request.execution.workers = 1;
    request.execution.deterministic_seeds = vec![deterministic_seed];
    request.execution.repetitions = plan.repetitions_per_cell;
    request.proposal.optimizer = optimizer;
    request.proposal.critic_ranking = None;
    request.resume.state_path = format!("build/campaigns/{}/state.json", request.id);
    request.resume.journal_path = format!("build/campaigns/{}/journal.jsonl", request.id);
    request.resume.checkpoint_every_candidates = request
        .resume
        .checkpoint_every_candidates
        .min(request.budgets.candidate_budget)
        .max(1);
    request.retention.failed_episode_limit = Some(request.budgets.candidate_budget);
    request.horizon_tightening = None;
    request.reverse_curriculum = None;
    request.refresh_content_sha256().map_err(matrix_error)?;
    request
        .validate_files(repository_root)
        .map_err(matrix_error)?;
    Ok(request)
}

pub fn materialize_learning_cell_optimization(
    plan: &LearningValueComparisonPlan,
    checkpoint_id: &str,
    deterministic_seed: u64,
    treatment_kind: LearningValueTreatmentKind,
    base: &OptimizationRequest,
    repository_root: &Path,
) -> Result<OptimizationRequest, LearningValueMatrixError> {
    plan.validate_files(repository_root).map_err(matrix_error)?;
    base.validate_files(repository_root).map_err(matrix_error)?;
    if !plan.deterministic_seeds.contains(&deterministic_seed) {
        return Err(matrix_message(
            "learning-value cell seed is absent from the sealed plan",
        ));
    }
    let checkpoint = plan
        .held_out_checkpoints
        .iter()
        .find(|checkpoint| checkpoint.id == checkpoint_id)
        .ok_or_else(|| {
            matrix_message("learning-value cell checkpoint is absent from the sealed plan")
        })?;
    let treatment = plan
        .treatments
        .iter()
        .find(|treatment| treatment.kind() == treatment_kind)
        .ok_or_else(|| {
            matrix_message("learning-value cell treatment is absent from the sealed plan")
        })?;
    if !matches!(
        treatment,
        LearningValueTreatment::DemonstrationAssistedStateReactive { .. }
            | LearningValueTreatment::FromScratchStateReactive { .. }
            | LearningValueTreatment::LearnedThenResidualRefinement { .. }
    ) || treatment.budget().discovery_simulated_ticks == 0
    {
        return Err(matrix_message(
            "only plan-owned state-reactive treatments can materialize learning authority",
        ));
    }
    validate_base(plan, checkpoint, base)?;

    let mut request = base.clone();
    request.id = cell_id(&plan.id, checkpoint_id, treatment_kind, deterministic_seed)?;
    request.budgets.simulated_tick_budget = treatment.budget().discovery_simulated_ticks;
    request.execution.workers = 1;
    request.execution.deterministic_seeds = vec![deterministic_seed];
    request.execution.repetitions = plan.repetitions_per_cell;
    request.proposal.critic_ranking = None;
    request.resume.state_path = format!("build/campaigns/{}/residual-state.json", request.id);
    request.resume.journal_path = format!("build/campaigns/{}/residual-journal.jsonl", request.id);
    request.horizon_tightening = None;
    request.reverse_curriculum = None;
    request.refresh_content_sha256().map_err(matrix_error)?;
    request
        .validate_files(repository_root)
        .map_err(matrix_error)?;
    Ok(request)
}

pub fn residual_treatment_from_slug(
    value: &str,
) -> Result<LearningValueTreatmentKind, LearningValueMatrixError> {
    match value {
        "independent-random-residual" | "independent_random_residual" => {
            Ok(LearningValueTreatmentKind::IndependentRandomResidual)
        }
        "cem-residual" | "cem_residual" => Ok(LearningValueTreatmentKind::CemResidual),
        _ => Err(matrix_message(
            "residual cell treatment must be independent-random-residual or cem-residual",
        )),
    }
}

pub fn learning_treatment_from_slug(
    value: &str,
) -> Result<LearningValueTreatmentKind, LearningValueMatrixError> {
    match value {
        "demonstration-assisted-state-reactive" | "demonstration_assisted_state_reactive" => {
            Ok(LearningValueTreatmentKind::DemonstrationAssistedStateReactive)
        }
        "from-scratch-state-reactive" | "from_scratch_state_reactive" => {
            Ok(LearningValueTreatmentKind::FromScratchStateReactive)
        }
        "learned-then-residual-refinement" | "learned_then_residual_refinement" => {
            Ok(LearningValueTreatmentKind::LearnedThenResidualRefinement)
        }
        _ => Err(matrix_message(
            "learning cell treatment must be demonstration-assisted-state-reactive, from-scratch-state-reactive, or learned-then-residual-refinement",
        )),
    }
}

fn baseline_optimizer(
    treatment: &LearningValueTreatment,
) -> Result<&ResidualOptimizerConfig, LearningValueMatrixError> {
    match treatment {
        LearningValueTreatment::IndependentRandomResidual { optimizer, budget }
        | LearningValueTreatment::CemResidual { optimizer, budget }
            if budget.discovery_simulated_ticks == 0 && budget.refinement_simulated_ticks > 0 =>
        {
            Ok(optimizer)
        }
        _ => Err(matrix_message(
            "only plan-owned residual baseline treatments can materialize residual cells",
        )),
    }
}

fn validate_base(
    plan: &LearningValueComparisonPlan,
    checkpoint: &LearningValueCheckpoint,
    base: &OptimizationRequest,
) -> Result<(), LearningValueMatrixError> {
    let incumbent = base
        .incumbent
        .as_ref()
        .ok_or_else(|| matrix_message("residual cell base request lacks an incumbent"))?;
    if incumbent.tape != checkpoint.source
        || base.route.source_boundary_index != checkpoint.source_boundary_index
        || base.route.native_source_boundary_fingerprint
            != checkpoint.native_source_boundary_fingerprint
        || base.terminal_predicate.program_sha256 != plan.terminal_program_sha256
        || base.terminal_predicate.definition_sha256 != plan.terminal_definition_sha256
    {
        return Err(matrix_message(
            "residual cell base request differs from its planned checkpoint or terminal",
        ));
    }
    Ok(())
}

fn optimizer_candidates(
    optimizer: &ResidualOptimizerConfig,
) -> Result<u64, LearningValueMatrixError> {
    match optimizer {
        ResidualOptimizerConfig::Random { samples } => Ok(*samples),
        ResidualOptimizerConfig::Cem {
            population,
            generations,
            ..
        } => u64::from(*population)
            .checked_mul(u64::from(*generations))
            .ok_or_else(|| matrix_message("learning-value optimizer candidate count overflowed")),
    }
}

fn cell_id(
    plan_id: &str,
    checkpoint_id: &str,
    treatment: LearningValueTreatmentKind,
    seed: u64,
) -> Result<String, LearningValueMatrixError> {
    let treatment = match treatment {
        LearningValueTreatmentKind::IndependentRandomResidual => "random",
        LearningValueTreatmentKind::CemResidual => "cem",
        LearningValueTreatmentKind::DemonstrationAssistedStateReactive => "demo",
        LearningValueTreatmentKind::FromScratchStateReactive => "scratch",
        LearningValueTreatmentKind::LearnedThenResidualRefinement => "learned",
    };
    let id = format!("{plan_id}-{checkpoint_id}-{treatment}-{seed}");
    if id.len() > 128 {
        return Err(matrix_message(
            "learning-value residual cell id exceeds the request limit",
        ));
    }
    Ok(id)
}

fn matrix_error(source: impl Error) -> LearningValueMatrixError {
    LearningValueMatrixError(source.to_string())
}

fn matrix_message(message: impl Into<String>) -> LearningValueMatrixError {
    LearningValueMatrixError(message.into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LearningValueMatrixError(String);

impl fmt::Display for LearningValueMatrixError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for LearningValueMatrixError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residual_treatment_slugs_are_explicit() {
        assert_eq!(
            residual_treatment_from_slug("independent-random-residual").unwrap(),
            LearningValueTreatmentKind::IndependentRandomResidual
        );
        assert_eq!(
            residual_treatment_from_slug("cem-residual").unwrap(),
            LearningValueTreatmentKind::CemResidual
        );
        assert!(residual_treatment_from_slug("learned-then-residual-refinement").is_err());
    }

    #[test]
    fn learning_treatment_slugs_are_explicit() {
        assert_eq!(
            learning_treatment_from_slug("demonstration-assisted-state-reactive").unwrap(),
            LearningValueTreatmentKind::DemonstrationAssistedStateReactive
        );
        assert_eq!(
            learning_treatment_from_slug("from-scratch-state-reactive").unwrap(),
            LearningValueTreatmentKind::FromScratchStateReactive
        );
        assert_eq!(
            learning_treatment_from_slug("learned-then-residual-refinement").unwrap(),
            LearningValueTreatmentKind::LearnedThenResidualRefinement
        );
        assert!(learning_treatment_from_slug("cem-residual").is_err());
    }

    #[test]
    fn optimizer_candidate_count_is_exact() {
        assert_eq!(
            optimizer_candidates(&ResidualOptimizerConfig::Random { samples: 1_024 }).unwrap(),
            1_024
        );
        assert_eq!(
            optimizer_candidates(&ResidualOptimizerConfig::Cem {
                population: 64,
                elites: 8,
                generations: 16,
                smoothing_millionths: 250_000,
            })
            .unwrap(),
            1_024
        );
    }

    #[test]
    fn cell_identity_separates_every_experimental_dimension() {
        assert_eq!(
            cell_id(
                "gate4",
                "q131",
                LearningValueTreatmentKind::CemResidual,
                104_729,
            )
            .unwrap(),
            "gate4-q131-cem-104729"
        );
        assert!(
            cell_id(
                &"x".repeat(128),
                "q131",
                LearningValueTreatmentKind::CemResidual,
                104_729,
            )
            .is_err()
        );
    }
}
