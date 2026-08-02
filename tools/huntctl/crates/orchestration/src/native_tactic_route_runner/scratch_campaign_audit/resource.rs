use super::*;

pub(super) fn resource_audit(
    route: &NativeTacticRouteReport,
    plan: &NativeTacticExecutionPlan,
) -> Result<NativeTacticCampaignResourceAudit, NativeTacticRouteRunError> {
    let workers = u64::try_from(route.workers).map_err(route_error)?;
    let maximum_per_worker = u64::try_from(tactic_checkpoint_cache_capacity_per_worker(
        plan.budgets.memory_bytes,
        route.workers,
    )?)
    .map_err(route_error)?;
    let configured_per_worker = route.checkpoint_cache_capacity_per_worker_bytes;
    if configured_per_worker == 0 || configured_per_worker > maximum_per_worker {
        return Err(route_message(
            "native tactic reported checkpoint capacity exceeds its execution plan",
        ));
    }
    let configured_pool = configured_per_worker
        .checked_mul(workers)
        .ok_or_else(|| route_message("native tactic configured checkpoint memory overflows"))?;
    let observed_peak_worker = route.native_restore_accounting.peak_resident_bytes;
    let observed_pool = observed_peak_worker
        .checked_mul(workers)
        .ok_or_else(|| route_message("native tactic observed checkpoint memory overflows"))?;
    let declared_memory_bound = match plan.budgets.memory_bytes {
        NativeTacticResourceLimit::Bounded(bytes) => Some(bytes),
        NativeTacticResourceLimit::Unbounded => None,
    };
    let memory_bound_satisfied = observed_pool <= configured_pool
        && declared_memory_bound.is_some_and(|bound| configured_pool <= bound);
    let maximum_allowed_staleness = match plan.replay_sharing {
        NativeTacticReplaySharingPlan::GenerationBarrier => 0,
        NativeTacticReplaySharingPlan::BoundedStaleness {
            maximum_stale_replay_revisions,
        } => maximum_stale_replay_revisions,
    };
    let maximum_model_replay_lag = route.replay_sharing.maximum_model_replay_lag_revisions;
    let learner_staleness_bound_satisfied = maximum_model_replay_lag <= maximum_allowed_staleness;
    let fallback_replays = route
        .native_restore_accounting
        .direct_restore_fallback_replays;
    let prefix_materializations = route.native_restore_accounting.prefix_materializations;
    let fallback_bound_satisfied = fallback_replays <= prefix_materializations;
    let mut checkpoint_owner_counts_by_worker = vec![0_u64; route.workers];
    let mut checkpoint_owner_available_decisions = 0_u64;
    let mut checkpoint_owner_local_decisions = 0_u64;
    let mut misrouted_owner_local_decisions = 0_u64;
    for trace in route.seeds.iter().flat_map(|seed| &seed.trace) {
        if let Some(owner) = trace.checkpoint_owner_worker_slot {
            let count = checkpoint_owner_counts_by_worker
                .get_mut(owner)
                .ok_or_else(|| route_message("native tactic checkpoint owner is not a worker"))?;
            *count = count.saturating_add(1);
            checkpoint_owner_available_decisions =
                checkpoint_owner_available_decisions.saturating_add(1);
        }
        if trace.restore_source == Some(NativeTacticRestoreSource::ProcessLocalCheckpoint) {
            if trace.proposal_worker_slots.first().copied() == trace.checkpoint_owner_worker_slot
                && trace.checkpoint_owner_worker_slot.is_some()
            {
                checkpoint_owner_local_decisions =
                    checkpoint_owner_local_decisions.saturating_add(1);
            } else {
                misrouted_owner_local_decisions = misrouted_owner_local_decisions.saturating_add(1);
            }
        }
    }
    let minimum_owner_assignments = checkpoint_owner_counts_by_worker
        .iter()
        .copied()
        .min()
        .unwrap_or(0);
    let maximum_owner_assignments = checkpoint_owner_counts_by_worker
        .iter()
        .copied()
        .max()
        .unwrap_or(0);
    let checkpoint_owner_assignment_skew =
        maximum_owner_assignments.saturating_sub(minimum_owner_assignments);
    let fallback_rate_per_million_decisions =
        ratio_per_million(fallback_replays, route.total_decisions);
    let passed = memory_bound_satisfied
        && learner_staleness_bound_satisfied
        && fallback_bound_satisfied
        && misrouted_owner_local_decisions == 0;
    Ok(NativeTacticCampaignResourceAudit {
        completed_decisions: route.total_decisions,
        declared_memory_bound_bytes: declared_memory_bound,
        configured_checkpoint_cache_capacity_per_worker_bytes: configured_per_worker,
        configured_checkpoint_pool_capacity_bytes: configured_pool,
        observed_peak_worker_resident_bytes: observed_peak_worker,
        observed_checkpoint_pool_resident_upper_bound_bytes: observed_pool,
        memory_bound_satisfied,
        maximum_allowed_stale_replay_revisions: maximum_allowed_staleness,
        maximum_model_replay_lag_revisions: maximum_model_replay_lag,
        maximum_lane_refresh_gap_revisions: route.replay_sharing.maximum_observed_stale_revisions,
        learner_staleness_bound_satisfied,
        direct_restore_fallback_replays: fallback_replays,
        prefix_materializations,
        fallback_rate_per_million_decisions,
        fallback_bound_satisfied,
        checkpoint_owner_available_decisions,
        checkpoint_owner_local_decisions,
        misrouted_owner_local_decisions,
        checkpoint_owner_counts_by_worker,
        checkpoint_owner_assignment_skew,
        passed,
    })
}

pub(super) fn resource_audit_is_valid(
    resources: &NativeTacticCampaignResourceAudit,
    workers: usize,
    seeds: &[NativeTacticScratchSeedAudit],
) -> bool {
    let total_decisions = seeds
        .iter()
        .map(|seed| seed.decisions.len() as u64)
        .sum::<u64>();
    let configured_pool = resources
        .configured_checkpoint_cache_capacity_per_worker_bytes
        .checked_mul(workers as u64);
    let observed_pool = resources
        .observed_peak_worker_resident_bytes
        .checked_mul(workers as u64);
    let mut derived_owner_counts = vec![0_u64; workers];
    let mut derived_owner_available = 0_u64;
    let mut derived_owner_local = 0_u64;
    let mut derived_owner_misrouted = 0_u64;
    for decision in seeds.iter().flat_map(|seed| &seed.decisions) {
        if let Some(owner) = decision.checkpoint_owner_worker_slot {
            let Some(count) = derived_owner_counts.get_mut(owner) else {
                return false;
            };
            *count = count.saturating_add(1);
            derived_owner_available = derived_owner_available.saturating_add(1);
        }
        if decision.restore_source == Some(NativeTacticRestoreSource::ProcessLocalCheckpoint) {
            if decision.proposal_worker_slots.first().copied()
                == decision.checkpoint_owner_worker_slot
                && decision.checkpoint_owner_worker_slot.is_some()
            {
                derived_owner_local = derived_owner_local.saturating_add(1);
            } else {
                derived_owner_misrouted = derived_owner_misrouted.saturating_add(1);
            }
        }
    }
    let minimum_owner_assignments = resources
        .checkpoint_owner_counts_by_worker
        .iter()
        .copied()
        .min()
        .unwrap_or(0);
    let maximum_owner_assignments = resources
        .checkpoint_owner_counts_by_worker
        .iter()
        .copied()
        .max()
        .unwrap_or(0);
    let memory_bound_satisfied = observed_pool
        == Some(resources.observed_checkpoint_pool_resident_upper_bound_bytes)
        && configured_pool == Some(resources.configured_checkpoint_pool_capacity_bytes)
        && observed_pool.is_some_and(|observed| {
            observed <= resources.configured_checkpoint_pool_capacity_bytes
        })
        && resources
            .declared_memory_bound_bytes
            .is_some_and(|bound| resources.configured_checkpoint_pool_capacity_bytes <= bound);
    let learner_staleness_bound_satisfied = resources.maximum_model_replay_lag_revisions
        <= resources.maximum_allowed_stale_replay_revisions;
    let fallback_bound_satisfied =
        resources.direct_restore_fallback_replays <= resources.prefix_materializations;
    let passed = memory_bound_satisfied
        && learner_staleness_bound_satisfied
        && fallback_bound_satisfied
        && resources.misrouted_owner_local_decisions == 0;
    workers > 0
        && resources.completed_decisions == total_decisions
        && resources.configured_checkpoint_cache_capacity_per_worker_bytes > 0
        && resources.configured_checkpoint_cache_capacity_per_worker_bytes
            <= TACTIC_CHECKPOINT_CACHE_BYTES as u64
        && resources.checkpoint_owner_counts_by_worker.len() == workers
        && resources.checkpoint_owner_counts_by_worker == derived_owner_counts
        && resources.checkpoint_owner_available_decisions == derived_owner_available
        && resources.checkpoint_owner_local_decisions == derived_owner_local
        && resources.misrouted_owner_local_decisions == derived_owner_misrouted
        && resources.fallback_rate_per_million_decisions
            == ratio_per_million(resources.direct_restore_fallback_replays, total_decisions)
        && resources.checkpoint_owner_assignment_skew
            == maximum_owner_assignments.saturating_sub(minimum_owner_assignments)
        && resources.memory_bound_satisfied == memory_bound_satisfied
        && resources.learner_staleness_bound_satisfied == learner_staleness_bound_satisfied
        && resources.fallback_bound_satisfied == fallback_bound_satisfied
        && resources.passed == passed
}
