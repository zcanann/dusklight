use super::*;

pub(super) fn first_demonstration_intervention(
    coverage_pending: bool,
    prefer_root: bool,
    acquisition: Option<&TacticFrontierAcquisition>,
) -> bool {
    coverage_pending
        && !prefer_root
        && acquisition.is_some_and(|acquisition| acquisition.expansion_count == 0)
}

pub(super) fn should_schedule_branch(
    decision_index: u64,
    branch_every_decisions: u64,
    terminal_restart: bool,
    terminal_support_acquisition: bool,
) -> bool {
    terminal_restart
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
