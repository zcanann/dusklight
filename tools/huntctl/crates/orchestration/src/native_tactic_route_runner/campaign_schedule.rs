use dusklight_automation_contracts::artifact::Digest;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ActiveTerminalRefinementRollout {
    source_prefix_ticks: u64,
    incumbent_ticks_to_go: u64,
}

/// One local perturbation followed by an authenticated incumbent suffix. The
/// suffix rejoins at the most similar future state on the incumbent rather
/// than assuming equal elapsed time means equal route progress. It is exposed
/// as an ordinary action and admitted as ordinary experience; this state only
/// makes the two-decision candidate resumable and prevents unrelated actions
/// from replacing its continuation.
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

/// Pick the future incumbent boundary most similar to the post-perturbation
/// state. Features are normalized over this candidate cohort and weighted by
/// the same semantic-family weights used by generalized tactic learning. Route
/// headroom is only a tie-breaker, so a distant future state cannot win merely
/// because it would produce a shorter candidate.
pub(super) fn select_incumbent_rejoin_offset(
    query: &[f32],
    candidates: &[(usize, Vec<f32>)],
    weights: &[f32],
) -> Option<usize> {
    let width = query.len();
    if width == 0
        || weights.len() != width
        || query.iter().any(|value| !value.is_finite())
        || weights
            .iter()
            .any(|weight| !weight.is_finite() || *weight <= 0.0)
        || candidates.is_empty()
        || candidates.iter().any(|(_, features)| {
            features.len() != width || features.iter().any(|value| !value.is_finite())
        })
    {
        return None;
    }

    let mut minimum = query.to_vec();
    let mut maximum = query.to_vec();
    for (_, features) in candidates {
        for index in 0..width {
            minimum[index] = minimum[index].min(features[index]);
            maximum[index] = maximum[index].max(features[index]);
        }
    }
    let ranges = minimum
        .iter()
        .zip(&maximum)
        .map(|(minimum, maximum)| maximum - minimum)
        .collect::<Vec<_>>();

    candidates
        .iter()
        .map(|(offset, features)| {
            let mut total = 0.0_f32;
            let mut active_weight = 0.0_f32;
            for index in 0..width {
                if ranges[index] <= 1.0e-6
                    && (query[index] - minimum[index]).abs() <= 1.0e-6
                    && (features[index] - minimum[index]).abs() <= 1.0e-6
                {
                    continue;
                }
                let delta = (query[index] - features[index]) / ranges[index];
                total += delta.clamp(-4.0, 4.0).powi(2) * weights[index];
                active_weight += weights[index];
            }
            let distance = if active_weight <= f32::EPSILON {
                0.0
            } else {
                total / active_weight
            };
            (*offset, distance)
        })
        .min_by(
            |(left_offset, left_distance), (right_offset, right_distance)| {
                left_distance
                    .total_cmp(right_distance)
                    .then_with(|| right_offset.cmp(left_offset))
            },
        )
        .map(|(offset, _)| offset)
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
    fn incumbent_rejoin_prefers_state_similarity_then_shortcut_headroom() {
        let candidates = vec![
            (10, vec![0.0, 0.0]),
            (20, vec![1.0, 0.0]),
            (30, vec![4.0, 0.0]),
        ];
        assert_eq!(
            select_incumbent_rejoin_offset(&[0.9, 0.0], &candidates, &[1.0, 1.0]),
            Some(20)
        );
        assert_eq!(
            select_incumbent_rejoin_offset(
                &[1.0, 0.0],
                &[(20, vec![1.0, 0.0]), (24, vec![1.0, 0.0])],
                &[1.0, 1.0],
            ),
            Some(24),
            "equal state matches should retain the larger shortcut"
        );
        assert_eq!(select_incumbent_rejoin_offset(&[], &candidates, &[]), None);
    }

    #[test]
    fn ordinary_learning_never_starts_a_paired_attribution_rollout() {
        assert!(!should_start_paired_terminal_return(false, false));
        assert!(!should_start_paired_terminal_return(false, true));
        assert!(should_start_paired_terminal_return(true, false));
        assert!(!should_start_paired_terminal_return(true, true));
    }
}
