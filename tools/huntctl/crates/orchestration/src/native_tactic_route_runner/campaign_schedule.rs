use dusklight_automation_contracts::artifact::Digest;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ActiveTerminalRefinementRollout {
    source_prefix_ticks: u64,
    incumbent_ticks_to_go: u64,
}

/// One local perturbation followed by the time-aligned authenticated
/// incumbent suffix. The suffix is exposed as an ordinary action and admitted
/// as ordinary experience; this state only makes the two-decision candidate
/// resumable and prevents unrelated actions from replacing its continuation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ActiveIncumbentContinuation {
    terminal_route_checkpoint_sha256: Digest,
    executed_actions: u64,
}

impl ActiveIncumbentContinuation {
    pub(super) fn new(
        terminal_route_checkpoint_sha256: Digest,
        executed_actions: u64,
    ) -> Option<Self> {
        (terminal_route_checkpoint_sha256 != Digest::ZERO).then_some(Self {
            terminal_route_checkpoint_sha256,
            executed_actions,
        })
    }

    pub(super) fn terminal_route_checkpoint_sha256(self) -> Digest {
        self.terminal_route_checkpoint_sha256
    }

    pub(super) fn should_execute_suffix(self) -> bool {
        self.executed_actions == 1
    }

    pub(super) fn candidate_completed(self) -> bool {
        self.executed_actions >= 2
    }

    pub(super) fn record_executed_action(&mut self) -> Result<(), &'static str> {
        self.executed_actions = self
            .executed_actions
            .checked_add(1)
            .ok_or("incumbent continuation action count overflowed")?;
        Ok(())
    }
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
    fn terminal_refinement_owns_an_equal_incumbent_continuation_budget() {
        let rollout = ActiveTerminalRefinementRollout::new(20, Some(125)).unwrap();
        assert!(rollout.has_remaining_budget(20));
        assert!(rollout.has_remaining_budget(144));
        assert!(!rollout.has_remaining_budget(145));
        assert!(!rollout.has_remaining_budget(200));
        assert_eq!(ActiveTerminalRefinementRollout::new(20, None), None);
    }

    #[test]
    fn incumbent_continuation_is_one_perturbation_then_one_suffix() {
        let route = Digest([7; 32]);
        let mut rollout = ActiveIncumbentContinuation::new(route, 0).unwrap();
        assert_eq!(rollout.terminal_route_checkpoint_sha256(), route);
        assert!(!rollout.should_execute_suffix());
        assert!(!rollout.candidate_completed());
        rollout.record_executed_action().unwrap();
        assert!(rollout.should_execute_suffix());
        assert!(!rollout.candidate_completed());
        rollout.record_executed_action().unwrap();
        assert!(!rollout.should_execute_suffix());
        assert!(rollout.candidate_completed());
        assert!(ActiveIncumbentContinuation::new(Digest::ZERO, 0).is_none());
    }

    #[test]
    fn ordinary_learning_never_starts_a_paired_attribution_rollout() {
        assert!(!should_start_paired_terminal_return(false, false));
        assert!(!should_start_paired_terminal_return(false, true));
        assert!(should_start_paired_terminal_return(true, false));
        assert!(!should_start_paired_terminal_return(true, true));
    }
}
