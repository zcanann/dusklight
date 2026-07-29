use super::{
    Digest, NativeGenericExecutionStrategy, NativeTacticExecutionPlan,
    NativeTacticExecutionPlanRequest, NativeTacticPlanBudgets, NativeTacticResourceLimit,
    OptimizationRequest, TacticProposalPolicy, option, u64_option, usize_option,
};
use dusklight_learning::tactic_value_treatment::TacticValueTreatment;
use dusklight_orchestration::{
    native_tactic_route_runner::NativeTacticReplaySharingPlan,
    optimization_request::CampaignClass,
};
use std::error::Error;

pub(super) fn native_tactic_execution_plan(
    learn_args: &[String],
    request: &OptimizationRequest,
    seeds: &[u64],
    proposal_policy: TacticProposalPolicy,
    execution_strategy: NativeGenericExecutionStrategy,
    promoted_tactic_registry_sha256: Option<Digest>,
) -> Result<NativeTacticExecutionPlan, Box<dyn Error>> {
    let decisions_per_lane = u64_option(learn_args, "--decisions-per-seed", 256)?;
    let proposal_width_per_decision = usize_option(learn_args, "--proposals-per-decision", 4)?;
    let refit_every_decisions = u64_option(learn_args, "--refit-every", 4)?;
    let default_lanes_per_generation = if proposal_policy == TacticProposalPolicy::Learned {
        seeds.len().min(4).max(1)
    } else {
        seeds.len().max(1)
    };
    let replay_sharing = option(learn_args, "--maximum-stale-replay-revisions")
        .map(|value| {
            Ok::<_, Box<dyn Error>>(NativeTacticReplaySharingPlan::BoundedStaleness {
                maximum_stale_replay_revisions: value.parse()?,
            })
        })
        .transpose()?
        .unwrap_or(default_replay_sharing(
            proposal_policy,
            proposal_width_per_decision,
            refit_every_decisions,
        )?);
    NativeTacticExecutionPlan::build(NativeTacticExecutionPlanRequest {
        seeds: seeds.to_vec(),
        proposal_policy,
        value_treatment: value_treatment(
            learn_args,
            default_value_treatment(request.campaign_class),
        )?,
        execution_strategy,
        promoted_tactic_registry_sha256,
        lanes_per_generation: usize_option(
            learn_args,
            "--lanes-per-generation",
            default_lanes_per_generation,
        )?,
        proposal_width_per_decision,
        branch_every_decisions: u64_option(learn_args, "--branch-every", 8)?,
        refit_every_decisions,
        root_refresh_cadence: 4,
        epsilon_per_million: option(learn_args, "--epsilon-per-million")
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or(350_000),
        demonstration_chunk_ticks: option(learn_args, "--demonstration-chunk-ticks")
            .map(|value| value.parse())
            .transpose()?,
        replay_sharing,
        budgets: NativeTacticPlanBudgets {
            decisions_per_lane,
            native_ticks: NativeTacticResourceLimit::Bounded(request.budgets.simulated_tick_budget),
            memory_bytes: option(learn_args, "--memory-bytes")
                .map(|value| value.parse().map(NativeTacticResourceLimit::Bounded))
                .transpose()?
                .unwrap_or(NativeTacticResourceLimit::Unbounded),
            wall_micros: option(learn_args, "--wall-micros")
                .map(|value| value.parse().map(NativeTacticResourceLimit::Bounded))
                .transpose()?
                .unwrap_or(NativeTacticResourceLimit::Unbounded),
        },
    })
    .map_err(Into::into)
}

fn value_treatment(
    learn_args: &[String],
    default: TacticValueTreatment,
) -> Result<TacticValueTreatment, Box<dyn Error>> {
    let value = option(learn_args, "--value-treatment");
    match value.as_deref() {
        None => Ok(default),
        Some("local_generalized_fitted_q_knn") => {
            Ok(TacticValueTreatment::LocalGeneralizedFittedQKnnV1)
        }
        Some("goal_relabeled_fitted_q_knn") => {
            Ok(TacticValueTreatment::GoalRelabeledFittedQKnnV2)
        }
        Some("continuous_fitted_q_forest") => {
            Ok(TacticValueTreatment::ContinuousFittedQForestV1)
        }
        Some(value) => Err(format!(
            "unsupported --value-treatment {value:?}; expected continuous_fitted_q_forest, goal_relabeled_fitted_q_knn, or local_generalized_fitted_q_knn"
        )
        .into()),
    }
}

fn default_value_treatment(campaign_class: CampaignClass) -> TacticValueTreatment {
    if campaign_class == CampaignClass::FromScratchDiscovery {
        TacticValueTreatment::GoalRelabeledFittedQKnnV2
    } else {
        TacticValueTreatment::LocalGeneralizedFittedQKnnV1
    }
}

fn default_replay_sharing(
    proposal_policy: TacticProposalPolicy,
    proposal_width_per_decision: usize,
    refit_every_decisions: u64,
) -> Result<NativeTacticReplaySharingPlan, Box<dyn Error>> {
    if proposal_policy != TacticProposalPolicy::Learned {
        return Ok(NativeTacticReplaySharingPlan::GenerationBarrier);
    }
    let proposal_width = u64::try_from(proposal_width_per_decision)?;
    let maximum_stale_replay_revisions = proposal_width
        .checked_mul(refit_every_decisions)
        .ok_or("default live replay staleness bound overflowed")?;
    Ok(NativeTacticReplaySharingPlan::BoundedStaleness {
        maximum_stale_replay_revisions,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CampaignClass, NativeTacticReplaySharingPlan, TacticProposalPolicy, TacticValueTreatment,
        default_replay_sharing, default_value_treatment, value_treatment,
    };

    #[test]
    fn learned_routes_default_to_live_replay_at_the_refit_cadence() {
        assert_eq!(
            default_replay_sharing(TacticProposalPolicy::Learned, 4, 2).unwrap(),
            NativeTacticReplaySharingPlan::BoundedStaleness {
                maximum_stale_replay_revisions: 8,
            }
        );
        assert_eq!(
            default_replay_sharing(TacticProposalPolicy::StructuredNonLearning, 4, 2).unwrap(),
            NativeTacticReplaySharingPlan::GenerationBarrier
        );
    }

    #[test]
    fn scratch_defaults_to_achieved_goal_learning_and_overrides_remain_explicit() {
        assert_eq!(
            default_value_treatment(CampaignClass::FromScratchDiscovery),
            TacticValueTreatment::GoalRelabeledFittedQKnnV2
        );
        assert_eq!(
            default_value_treatment(CampaignClass::DemonstrationAssistedDiscovery),
            TacticValueTreatment::LocalGeneralizedFittedQKnnV1
        );
        assert_eq!(
            default_value_treatment(CampaignClass::LocalTasRefinement),
            TacticValueTreatment::LocalGeneralizedFittedQKnnV1
        );
        assert_eq!(
            value_treatment(&[], TacticValueTreatment::LocalGeneralizedFittedQKnnV1).unwrap(),
            TacticValueTreatment::LocalGeneralizedFittedQKnnV1
        );
        assert_eq!(
            value_treatment(&[], TacticValueTreatment::GoalRelabeledFittedQKnnV2).unwrap(),
            TacticValueTreatment::GoalRelabeledFittedQKnnV2
        );
        assert_eq!(
            value_treatment(&[
                "--value-treatment".into(),
                "goal_relabeled_fitted_q_knn".into(),
            ], TacticValueTreatment::LocalGeneralizedFittedQKnnV1)
            .unwrap(),
            TacticValueTreatment::GoalRelabeledFittedQKnnV2
        );
        assert_eq!(
            value_treatment(&[
                "--value-treatment".into(),
                "continuous_fitted_q_forest".into(),
            ], TacticValueTreatment::LocalGeneralizedFittedQKnnV1)
            .unwrap(),
            TacticValueTreatment::ContinuousFittedQForestV1
        );
        assert!(
            value_treatment(
                &["--value-treatment".into(), "unknown".into()],
                TacticValueTreatment::LocalGeneralizedFittedQKnnV1,
            )
            .is_err()
        );
    }
}
