use super::*;

/// Choose the next graph-acquisition partition from native work already spent,
/// not from the number of episodes. Terminal-supported suffixes are often only
/// a few ticks long while discovery rollouts can consume hundreds; an episode
/// cadence therefore starves the very checkpoint branches it is meant to
/// optimize.
pub(super) fn acquisition_rank_for_episode(
    acquisition: NativeTacticAcquisitionPlan,
    episode: u64,
    native_terminal_supported: bool,
    trace: &[NativeTacticDecisionTrace],
) -> u64 {
    if let Some(active) = trace.last().filter(|decision| decision.episode == episode) {
        return active.acquisition_rank;
    }
    if !native_terminal_supported {
        return acquisition.rank(episode);
    }
    let NativeTacticAcquisitionPlan::CyclicSupportAndRanks { cycle_width, .. } = acquisition else {
        return acquisition.rank(episode);
    };
    let post_terminal_start = trace
        .iter()
        .position(|decision| {
            decision.terminal || decision.best_authenticated_tick_after_decision.is_some()
        })
        .map_or(0, |index| index.saturating_add(1));
    let mut support_ticks = 0_u64;
    let mut discovery_ticks = 0_u64;
    let mut discovery_episodes = BTreeSet::new();
    for decision in &trace[post_terminal_start..] {
        let ticks = decision_evaluated_ticks(decision);
        if decision.acquisition_rank == 0 {
            support_ticks = support_ticks.saturating_add(ticks);
        } else {
            discovery_ticks = discovery_ticks.saturating_add(ticks);
            discovery_episodes.insert(decision.episode);
        }
    }
    cyclic_rank_for_native_work(
        cycle_width,
        support_ticks,
        discovery_ticks,
        discovery_episodes.len() as u64,
    )
}

fn cyclic_rank_for_native_work(
    cycle_width: u32,
    support_ticks: u64,
    discovery_ticks: u64,
    completed_discovery_episodes: u64,
) -> u64 {
    let discovery_share = u64::from(cycle_width.saturating_sub(1));
    if support_ticks.saturating_mul(discovery_share) <= discovery_ticks {
        0
    } else {
        completed_discovery_episodes.saturating_add(1)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ActiveTerminalRefinementRollout {
    source_prefix_ticks: u64,
    incumbent_ticks_to_go: u64,
}

impl ActiveTerminalRefinementRollout {
    pub(super) fn new(
        source_prefix_ticks: u64,
        incumbent_ticks_to_go: Option<u64>,
    ) -> Option<Self> {
        incumbent_ticks_to_go.map(|incumbent_ticks_to_go| Self {
            source_prefix_ticks,
            incumbent_ticks_to_go,
        })
    }

    pub(super) fn has_remaining_budget(self, current_route_ticks: u64) -> bool {
        current_route_ticks.saturating_sub(self.source_prefix_ticks) < self.incumbent_ticks_to_go
    }
}

pub(super) fn first_demonstration_intervention(
    coverage_pending: bool,
    prefer_root: bool,
    selected_uncovered_demonstration_frontier: bool,
) -> bool {
    coverage_pending && !prefer_root && selected_uncovered_demonstration_frontier
}

/// A terminal state has no legal continuation action surface. Leave any
/// pending learner publication unconsumed until the forced branch restores an
/// actionable root or interior frontier; the ordinary post-branch refresh then
/// evaluates the publication against that exact state.
pub(super) fn should_probe_policy_before_branch(terminal_restart: bool) -> bool {
    !terminal_restart
}

/// Matched policy/control continuations consume a second rollout and freeze
/// learner publication. They belong only to explicit attribution campaigns;
/// ordinary learning spends the same budget on adaptive experience.
pub(super) fn should_start_paired_terminal_return(
    evaluation_enabled: bool,
    paired_return_in_progress: bool,
) -> bool {
    evaluation_enabled && !paired_return_in_progress
}

#[cfg(test)]
mod terminal_refinement_tests {
    use super::*;

    #[test]
    fn cyclic_acquisition_balances_native_work_instead_of_episode_count() {
        assert_eq!(cyclic_rank_for_native_work(4, 0, 0, 0), 0);
        assert_eq!(cyclic_rank_for_native_work(4, 80, 0, 0), 1);
        assert_eq!(cyclic_rank_for_native_work(4, 80, 120, 1), 2);
        assert_eq!(cyclic_rank_for_native_work(4, 80, 240, 2), 0);

        // New single-lane plans use an equal support/discovery work envelope.
        assert_eq!(cyclic_rank_for_native_work(2, 40, 0, 0), 1);
        assert_eq!(cyclic_rank_for_native_work(2, 40, 39, 1), 2);
        assert_eq!(cyclic_rank_for_native_work(2, 40, 40, 2), 0);
    }

    #[test]
    fn terminal_refinement_owns_an_equal_incumbent_continuation_budget() {
        let rollout = ActiveTerminalRefinementRollout::new(20, Some(125)).unwrap();
        assert!(rollout.has_remaining_budget(20));
        assert!(rollout.has_remaining_budget(144));
        assert!(!rollout.has_remaining_budget(145));
        assert!(!rollout.has_remaining_budget(200));
        assert_eq!(ActiveTerminalRefinementRollout::new(20, None), None);
    }

    #[test]
    fn ordinary_learning_never_starts_a_paired_attribution_rollout() {
        assert!(!should_start_paired_terminal_return(false, false));
        assert!(!should_start_paired_terminal_return(false, true));
        assert!(should_start_paired_terminal_return(true, false));
        assert!(!should_start_paired_terminal_return(true, true));
    }
}
