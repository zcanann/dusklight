use super::{
    Digest, NativeGenericExecutionStrategy, NativeTacticExecutionPlan,
    NativeTacticExecutionPlanRequest, NativeTacticPlanBudgets, NativeTacticResourceLimit,
    OptimizationRequest, TacticProposalPolicy, flag, option, u64_option, usize_option,
};
use dusklight_learning::tactic_value_treatment::TacticValueTreatment;
use dusklight_orchestration::{
    native_tactic_route_runner::NativeTacticReplaySharingPlan, optimization_request::CampaignClass,
};
use std::error::Error;

const SEALED_PLAN_SHAPING_OPTIONS: [&str; 14] = [
    "--seed",
    "--proposal-policy",
    "--value-treatment",
    "--execution-strategy",
    "--decisions-per-seed",
    "--proposals-per-decision",
    "--lanes-per-generation",
    "--refit-every",
    "--epsilon-per-million",
    "--demonstration-chunk-ticks",
    "--paired-terminal-return-evaluation",
    "--maximum-stale-replay-revisions",
    "--memory-bytes",
    "--wall-micros",
];

pub(super) fn sealed_plan_shape_conflict(learn_args: &[String]) -> Option<&'static str> {
    SEALED_PLAN_SHAPING_OPTIONS
        .into_iter()
        .find(|name| learn_args.iter().any(|argument| argument == name))
}

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
    // Live replay currently has no deterministic cross-lane publication
    // protocol. Keep the default schedule reproducible while proposal workers
    // still execute one decision's batch in parallel. Generation-barrier plans
    // may opt into additional lanes explicitly.
    let default_lanes_per_generation = 1;
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
        refit_every_decisions,
        root_refresh_cadence: 4,
        epsilon_per_million: option(learn_args, "--epsilon-per-million")
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or(350_000),
        demonstration_chunk_ticks: option(learn_args, "--demonstration-chunk-ticks")
            .map(|value| value.parse())
            .transpose()?,
        paired_terminal_return_evaluation: flag(learn_args, "--paired-terminal-return-evaluation"),
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
        Some("goal_relabeled_frontier_double_q") => {
            Ok(TacticValueTreatment::GoalRelabeledFrontierDoubleQV3)
        }
        Some("goal_relabeled_universal_frontier_double_q") => {
            Ok(TacticValueTreatment::GoalRelabeledUniversalFrontierDoubleQV4)
        }
        Some("continuous_fitted_q_forest") => {
            Ok(TacticValueTreatment::ContinuousFittedQForestV1)
        }
        Some(value) => Err(format!(
            "unsupported --value-treatment {value:?}; expected continuous_fitted_q_forest, goal_relabeled_fitted_q_knn, goal_relabeled_frontier_double_q, goal_relabeled_universal_frontier_double_q, or local_generalized_fitted_q_knn"
        )
        .into()),
    }
}

fn default_value_treatment(campaign_class: CampaignClass) -> TacticValueTreatment {
    if campaign_class == CampaignClass::FromScratchDiscovery {
        TacticValueTreatment::GoalRelabeledUniversalFrontierDoubleQV4
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
        CampaignClass, NativeTacticReplaySharingPlan, SEALED_PLAN_SHAPING_OPTIONS,
        TacticProposalPolicy, TacticValueTreatment, default_replay_sharing,
        default_value_treatment, sealed_plan_shape_conflict, value_treatment,
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
            default_replay_sharing(TacticProposalPolicy::FrozenPolicy, 4, 2).unwrap(),
            NativeTacticReplaySharingPlan::GenerationBarrier
        );
        assert_eq!(
            default_replay_sharing(TacticProposalPolicy::StructuredNonLearning, 4, 2).unwrap(),
            NativeTacticReplaySharingPlan::GenerationBarrier
        );
    }

    #[test]
    fn frontier_double_q_treatment_is_selectable_from_the_cli() {
        assert_eq!(
            value_treatment(
                &[
                    "--value-treatment".into(),
                    "goal_relabeled_frontier_double_q".into(),
                ],
                TacticValueTreatment::LocalGeneralizedFittedQKnnV1,
            )
            .unwrap(),
            TacticValueTreatment::GoalRelabeledFrontierDoubleQV3,
        );
        assert_eq!(
            value_treatment(
                &[
                    "--value-treatment".into(),
                    "goal_relabeled_universal_frontier_double_q".into(),
                ],
                TacticValueTreatment::LocalGeneralizedFittedQKnnV1,
            )
            .unwrap(),
            TacticValueTreatment::GoalRelabeledUniversalFrontierDoubleQV4,
        );
    }

    #[test]
    fn scratch_defaults_to_achieved_goal_and_terminal_frontier_learning() {
        assert_eq!(
            default_value_treatment(CampaignClass::FromScratchDiscovery),
            TacticValueTreatment::GoalRelabeledUniversalFrontierDoubleQV4
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
            value_treatment(
                &[
                    "--value-treatment".into(),
                    "goal_relabeled_fitted_q_knn".into(),
                ],
                TacticValueTreatment::LocalGeneralizedFittedQKnnV1
            )
            .unwrap(),
            TacticValueTreatment::GoalRelabeledFittedQKnnV2
        );
        assert_eq!(
            value_treatment(
                &[
                    "--value-treatment".into(),
                    "continuous_fitted_q_forest".into(),
                ],
                TacticValueTreatment::LocalGeneralizedFittedQKnnV1
            )
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

    #[test]
    fn sealed_plan_rejects_shape_overrides_but_allows_runtime_capacity_controls() {
        for option in SEALED_PLAN_SHAPING_OPTIONS {
            let args = vec![
                "--plan".into(),
                "plan.dtp".into(),
                option.into(),
                "1".into(),
            ];
            assert_eq!(sealed_plan_shape_conflict(&args), Some(option));
        }
        assert_eq!(
            sealed_plan_shape_conflict(&[
                "--plan".into(),
                "plan.dtp".into(),
                "--workers".into(),
                "1".into(),
                "--checkpoint-capacity-workers".into(),
                "16".into(),
            ]),
            None
        );
    }
}
