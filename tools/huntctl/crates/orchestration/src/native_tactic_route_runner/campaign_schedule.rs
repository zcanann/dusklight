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

/// Use an acquisition partition to choose a new candidate, then let the
/// learned policy own its continuation. Rotating broad-coverage partitions at
/// every decision turns one rollout into an unrelated action chain and makes
/// terminal return unable to guide completion.
pub(super) fn active_rollout_acquisition_rank(
    decision_acquisition_rank: u64,
    paired_terminal_return: bool,
    terminal_refinement_in_progress: bool,
) -> u64 {
    if paired_terminal_return || terminal_refinement_in_progress {
        0
    } else {
        decision_acquisition_rank
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
    fn terminal_refinement_owns_an_equal_incumbent_continuation_budget() {
        let rollout = ActiveTerminalRefinementRollout::new(20, Some(125)).unwrap();
        assert!(rollout.has_remaining_budget(20));
        assert!(rollout.has_remaining_budget(144));
        assert!(!rollout.has_remaining_budget(145));
        assert!(!rollout.has_remaining_budget(200));
        assert_eq!(ActiveTerminalRefinementRollout::new(20, None), None);
    }

    #[test]
    fn terminal_refinement_uses_the_learned_policy_after_candidate_selection() {
        assert_eq!(active_rollout_acquisition_rank(17, false, false), 17);
        assert_eq!(active_rollout_acquisition_rank(17, false, true), 0);
        assert_eq!(active_rollout_acquisition_rank(17, true, false), 0);
    }

    #[test]
    fn ordinary_learning_never_starts_a_paired_attribution_rollout() {
        assert!(!should_start_paired_terminal_return(false, false));
        assert!(!should_start_paired_terminal_return(false, true));
        assert!(should_start_paired_terminal_return(true, false));
        assert!(!should_start_paired_terminal_return(true, true));
    }
}
