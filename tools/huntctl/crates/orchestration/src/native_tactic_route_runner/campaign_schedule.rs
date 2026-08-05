use super::NativeTacticAcquisitionPlan;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ScheduledBranchAcquisition {
    /// Acquisition partition that owns both the restored frontier and the
    /// action selected from it.
    pub rank: u64,
    pub terminal_support: bool,
    pub demonstration: bool,
}

pub(super) fn next_branch_acquisition_rank(
    acquisition: NativeTacticAcquisitionPlan,
    current_episode: u64,
) -> Option<u64> {
    current_episode
        .checked_add(1)
        .map(|next_episode| acquisition.rank(next_episode))
}

pub(super) fn scheduled_branch_acquisition(
    acquisition: NativeTacticAcquisitionPlan,
    episode: u64,
    terminal_restart: bool,
    native_terminal_supported: bool,
    demonstration_coverage_pending: bool,
) -> ScheduledBranchAcquisition {
    let planned_rank = acquisition.rank(episode);
    let terminal_support = native_terminal_supported && planned_rank == 0;
    // A supported terminal frontier is optimization authority, while the
    // demonstration curriculum is only coverage. Do not let permanently
    // pending demonstration coverage consume rank-zero optimization slots.
    let demonstration = demonstration_coverage_pending && !terminal_restart && !terminal_support;
    ScheduledBranchAcquisition {
        rank: if terminal_restart || terminal_support {
            0
        } else {
            planned_rank
        },
        terminal_support,
        demonstration,
    }
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

/// Route learned acquisitions through the live frontier scorer while keeping
/// an explicit broad-exploration partition. Before native terminal support,
/// goal-relabeled reachability is the only learned objective available and
/// must be allowed to choose which retained checkpoint to branch from. Once a
/// terminal exists, only rank-zero optimization uses the terminal action-value
/// head; the other ranks remain on graph-wide coverage/novelty rotation.
pub(super) fn should_rank_frontier_with_live_model(
    demonstration_branch: bool,
    native_terminal_supported: bool,
    terminal_support_acquisition: bool,
    learned_exploitation_acquisition: bool,
    goal_reachability_enabled: bool,
    terminal_frontier_action_value_enabled: bool,
) -> bool {
    demonstration_branch
        || (!native_terminal_supported
            && learned_exploitation_acquisition
            && goal_reachability_enabled)
        || (terminal_support_acquisition && terminal_frontier_action_value_enabled)
}
