use super::NativeTacticAcquisitionPlan;

pub(super) fn next_branch_acquisition_rank(
    acquisition: NativeTacticAcquisitionPlan,
    current_episode: u64,
) -> Option<u64> {
    current_episode
        .checked_add(1)
        .map(|next_episode| acquisition.rank(next_episode))
}

pub(super) fn first_demonstration_intervention(
    coverage_pending: bool,
    prefer_root: bool,
    selected_uncovered_demonstration_frontier: bool,
) -> bool {
    coverage_pending && !prefer_root && selected_uncovered_demonstration_frontier
}

pub(super) fn should_schedule_branch(
    decision_index: u64,
    branch_every_decisions: u64,
    terminal_restart: bool,
    terminal_support_acquisition: bool,
    demonstration_coverage_pending: bool,
) -> bool {
    demonstration_coverage_pending
        || terminal_restart
        || terminal_support_acquisition
        || (decision_index > 0 && decision_index % branch_every_decisions == 0)
}

/// A terminal state has no legal continuation action surface. Leave any
/// pending learner publication unconsumed until the forced branch restores an
/// actionable root or interior frontier; the ordinary post-branch refresh then
/// evaluates the publication against that exact state.
pub(super) fn should_probe_policy_before_branch(terminal_restart: bool) -> bool {
    !terminal_restart
}

pub(super) fn prefer_root_for_periodic_branch(
    force_scheduled_frontier: bool,
    root_refresh_due: bool,
) -> bool {
    // Terminal and rank-zero support acquisitions own exact graph work. Root
    // refresh remains an independent cadence for ordinary exploration
    // branches and must not consume supported-interior optimization slots.
    !force_scheduled_frontier && root_refresh_due
}

/// Route rank-zero terminal optimization through the live learned frontier
/// scorer when the sealed treatment enables it. Demonstration coverage uses
/// the same enumerator in coverage mode; all other ranks stay on the graph
/// scheduler's broad-discovery rotation.
pub(super) fn should_rank_frontier_with_live_model(
    demonstration_branch: bool,
    terminal_support_acquisition: bool,
    terminal_frontier_action_value_enabled: bool,
) -> bool {
    demonstration_branch || (terminal_support_acquisition && terminal_frontier_action_value_enabled)
}
