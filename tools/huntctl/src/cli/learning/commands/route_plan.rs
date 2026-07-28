use super::{
    Digest, NativeGenericExecutionStrategy, NativeTacticExecutionPlan,
    NativeTacticExecutionPlanRequest, NativeTacticPlanBudgets, NativeTacticResourceLimit,
    OptimizationRequest, TacticProposalPolicy, option, u64_option, usize_option,
};
use dusklight_orchestration::native_tactic_route_runner::NativeTacticReplaySharingPlan;
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
    let default_lanes_per_generation = if proposal_policy == TacticProposalPolicy::Learned {
        seeds.len().min(4).max(1)
    } else {
        seeds.len().max(1)
    };
    NativeTacticExecutionPlan::build(NativeTacticExecutionPlanRequest {
        seeds: seeds.to_vec(),
        proposal_policy,
        execution_strategy,
        promoted_tactic_registry_sha256,
        lanes_per_generation: usize_option(
            learn_args,
            "--lanes-per-generation",
            default_lanes_per_generation,
        )?,
        proposal_width_per_decision: usize_option(learn_args, "--proposals-per-decision", 4)?,
        branch_every_decisions: u64_option(learn_args, "--branch-every", 8)?,
        refit_every_decisions: u64_option(learn_args, "--refit-every", 4)?,
        root_refresh_cadence: 4,
        epsilon_per_million: option(learn_args, "--epsilon-per-million")
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or(350_000),
        demonstration_chunk_ticks: option(learn_args, "--demonstration-chunk-ticks")
            .map(|value| value.parse())
            .transpose()?,
        replay_sharing: option(learn_args, "--maximum-stale-replay-revisions")
            .map(|value| {
                Ok::<_, Box<dyn Error>>(NativeTacticReplaySharingPlan::BoundedStaleness {
                    maximum_stale_replay_revisions: value.parse()?,
                })
            })
            .transpose()?
            .unwrap_or(NativeTacticReplaySharingPlan::GenerationBarrier),
        budgets: NativeTacticPlanBudgets {
            decisions_per_lane,
            native_ticks: NativeTacticResourceLimit::Bounded(request.budgets.simulated_tick_budget),
            memory_bytes: NativeTacticResourceLimit::Unbounded,
            wall_micros: NativeTacticResourceLimit::Unbounded,
        },
    })
    .map_err(Into::into)
}
