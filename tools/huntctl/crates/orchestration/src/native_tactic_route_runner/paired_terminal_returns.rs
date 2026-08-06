use super::campaign_schedule::ActiveTerminalRefinementRollout;
use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PairedTerminalReturnSeed {
    execution_plan_sha256: Digest,
    decision_index: u64,
    source_checkpoint_sha256: Digest,
    source_state_sha256: Digest,
    source_prefix_ticks: u64,
    incumbent_ticks_to_go: u64,
    frozen_learner_snapshot_sha256: Digest,
    frozen_replay_revision: u64,
    policy_option_id: String,
    control_option_id: String,
    control_proposal_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActivePairedTerminalReturn {
    trace: NativeTacticPairedTerminalReturnTrace,
}

impl PairedTerminalReturnSeed {
    /// Select the control strictly from the pre-execution proposal order. The
    /// proposal at index zero remains the policy lineage; index one is the
    /// deterministic control. No native outcome is available to this method.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_pre_execution_proposals(
        execution_plan_sha256: Digest,
        decision_index: u64,
        source_checkpoint_sha256: Digest,
        source_state_sha256: Digest,
        acquisition: Option<&TacticFrontierAcquisition>,
        frozen_learner_snapshot_sha256: Digest,
        frozen_replay_revision: u64,
        proposals: &[SelectedTactic],
    ) -> Option<Self> {
        let acquisition = acquisition?;
        let incumbent_ticks_to_go = acquisition.exact_terminal_ticks_to_go?;
        let [policy, control, ..] = proposals else {
            return None;
        };
        Some(Self {
            execution_plan_sha256,
            decision_index,
            source_checkpoint_sha256,
            source_state_sha256,
            source_prefix_ticks: acquisition.replayed_prefix_ticks,
            incumbent_ticks_to_go,
            frozen_learner_snapshot_sha256,
            frozen_replay_revision,
            policy_option_id: policy.descriptor.option_id.clone(),
            control_option_id: control.descriptor.option_id.clone(),
            control_proposal_index: 1,
        })
    }

    pub(super) fn control_proposal_index(&self) -> usize {
        self.control_proposal_index
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn admit_first_steps(
        self,
        policy_terminal: bool,
        policy_route_ticks: u64,
        control_target_checkpoint_sha256: Digest,
        control_target_state_sha256: Digest,
        control_first_step_ticks: u32,
        control_terminal: bool,
        control_route_ticks: u64,
    ) -> Result<
        (
            Option<ActivePairedTerminalReturn>,
            NativeTacticPairedTerminalReturnTrace,
        ),
        NativeTacticRouteRunError,
    > {
        if self.execution_plan_sha256 == Digest::ZERO
            || self.source_checkpoint_sha256 == Digest::ZERO
            || self.source_state_sha256 == Digest::ZERO
            || self.frozen_learner_snapshot_sha256 == Digest::ZERO
            || control_target_checkpoint_sha256 == Digest::ZERO
            || control_target_state_sha256 == Digest::ZERO
            || self.policy_option_id.is_empty()
            || self.control_option_id.is_empty()
            || self.policy_option_id == self.control_option_id
            || self.incumbent_ticks_to_go == 0
            || policy_route_ticks < self.source_prefix_ticks
            || control_route_ticks < self.source_prefix_ticks
            || control_route_ticks.saturating_sub(self.source_prefix_ticks)
                != u64::from(control_first_step_ticks)
        {
            return Err(route_message("paired terminal-return seed is invalid"));
        }
        let pair_sha256 = paired_terminal_return_identity(
            self.execution_plan_sha256,
            self.decision_index,
            self.source_checkpoint_sha256,
            self.source_state_sha256,
            &self.policy_option_id,
            &self.control_option_id,
        );
        let policy_done = policy_terminal
            || policy_route_ticks.saturating_sub(self.source_prefix_ticks)
                >= self.incumbent_ticks_to_go;
        let control_done = control_terminal
            || control_route_ticks.saturating_sub(self.source_prefix_ticks)
                >= self.incumbent_ticks_to_go;
        let status_after_decision = if policy_done && control_done {
            NativeTacticPairedTerminalReturnStatus::Complete
        } else if policy_done {
            NativeTacticPairedTerminalReturnStatus::ControlPending
        } else {
            NativeTacticPairedTerminalReturnStatus::PolicyInProgress
        };
        let trace = NativeTacticPairedTerminalReturnTrace {
            schema: NATIVE_TACTIC_PAIRED_TERMINAL_RETURN_SCHEMA_V1.into(),
            pair_sha256,
            source_decision_index: self.decision_index,
            source_checkpoint_sha256: self.source_checkpoint_sha256,
            source_state_sha256: self.source_state_sha256,
            source_prefix_ticks: self.source_prefix_ticks,
            incumbent_ticks_to_go: self.incumbent_ticks_to_go,
            frozen_learner_snapshot_sha256: self.frozen_learner_snapshot_sha256,
            frozen_replay_revision: self.frozen_replay_revision,
            policy_option_id: self.policy_option_id,
            control_option_id: self.control_option_id,
            control_target_checkpoint_sha256,
            control_target_state_sha256,
            control_first_step_ticks,
            control_first_step_terminal: control_terminal,
            role: NativeTacticPairedTerminalReturnRole::Policy,
            status_after_decision,
            policy_terminal_supported: policy_terminal,
            control_terminal_supported: control_terminal,
        };
        validate_paired_terminal_return_trace(&trace)?;
        let active = (status_after_decision != NativeTacticPairedTerminalReturnStatus::Complete)
            .then(|| ActivePairedTerminalReturn {
                trace: trace.clone(),
            });
        Ok((active, trace))
    }
}

impl ActivePairedTerminalReturn {
    pub(super) fn recover(
        trace: &[NativeTacticDecisionTrace],
    ) -> Result<Option<Self>, NativeTacticRouteRunError> {
        let mut latest = BTreeMap::<Digest, NativeTacticPairedTerminalReturnTrace>::new();
        for decision in trace {
            let Some(current) = decision.paired_terminal_return.as_ref() else {
                continue;
            };
            validate_paired_terminal_return_trace(current)?;
            if decision.learner_snapshot_sha256 != current.frozen_learner_snapshot_sha256 {
                return Err(route_message(
                    "paired terminal-return decision changed learner authority",
                ));
            }
            if let Some(previous) = latest.get(&current.pair_sha256) {
                validate_paired_terminal_return_successor(previous, current)?;
            } else if current.role != NativeTacticPairedTerminalReturnRole::Policy
                || current.status_after_decision
                    == NativeTacticPairedTerminalReturnStatus::ControlInProgress
            {
                return Err(route_message(
                    "paired terminal-return trace starts outside its policy lineage",
                ));
            }
            latest.insert(current.pair_sha256, current.clone());
        }
        let mut active = latest
            .into_values()
            .filter(|paired| {
                paired.status_after_decision != NativeTacticPairedTerminalReturnStatus::Complete
            })
            .collect::<Vec<_>>();
        if active.len() > 1 {
            return Err(route_message(
                "multiple paired terminal-return comparisons remain active",
            ));
        }
        Ok(active.pop().map(|trace| Self { trace }))
    }

    pub(super) fn freezes_policy(&self) -> bool {
        true
    }

    pub(super) fn frozen_learner_snapshot_sha256(&self) -> Digest {
        self.trace.frozen_learner_snapshot_sha256
    }

    pub(super) fn frozen_replay_revision(&self) -> u64 {
        self.trace.frozen_replay_revision
    }

    pub(super) fn rollout(&self) -> ActiveTerminalRefinementRollout {
        ActiveTerminalRefinementRollout::new(
            self.trace.source_prefix_ticks,
            Some(self.trace.incumbent_ticks_to_go),
        )
        .expect("validated paired return has a continuation budget")
    }

    pub(super) fn control_pending(&self) -> bool {
        self.trace.status_after_decision == NativeTacticPairedTerminalReturnStatus::ControlPending
    }

    pub(super) fn control_target(&self) -> crate::state_graph::ExactStateId {
        crate::state_graph::ExactStateId {
            route_checkpoint_sha256: self.trace.control_target_checkpoint_sha256,
            state_sha256: self.trace.control_target_state_sha256,
        }
    }

    pub(super) fn begin_control(&mut self) {
        debug_assert!(self.control_pending());
        self.trace.role = NativeTacticPairedTerminalReturnRole::Control;
        self.trace.status_after_decision =
            NativeTacticPairedTerminalReturnStatus::ControlInProgress;
    }

    pub(super) fn record_decision(
        &mut self,
        terminal: bool,
        current_route_ticks: u64,
    ) -> NativeTacticPairedTerminalReturnTrace {
        let budget_consumed = !self.rollout().has_remaining_budget(current_route_ticks);
        match self.trace.role {
            NativeTacticPairedTerminalReturnRole::Policy => {
                self.trace.policy_terminal_supported |= terminal;
                if terminal || budget_consumed {
                    let control_first_step_done = self.trace.control_first_step_terminal
                        || u64::from(self.trace.control_first_step_ticks)
                            >= self.trace.incumbent_ticks_to_go;
                    self.trace.status_after_decision = if control_first_step_done {
                        NativeTacticPairedTerminalReturnStatus::Complete
                    } else {
                        NativeTacticPairedTerminalReturnStatus::ControlPending
                    };
                } else {
                    self.trace.status_after_decision =
                        NativeTacticPairedTerminalReturnStatus::PolicyInProgress;
                }
            }
            NativeTacticPairedTerminalReturnRole::Control => {
                self.trace.control_terminal_supported |= terminal;
                self.trace.status_after_decision = if terminal || budget_consumed {
                    NativeTacticPairedTerminalReturnStatus::Complete
                } else {
                    NativeTacticPairedTerminalReturnStatus::ControlInProgress
                };
            }
        }
        self.trace.clone()
    }

    pub(super) fn complete(&self) -> bool {
        self.trace.status_after_decision == NativeTacticPairedTerminalReturnStatus::Complete
    }
}

pub(super) fn validate_paired_terminal_return_trace(
    trace: &NativeTacticPairedTerminalReturnTrace,
) -> Result<(), NativeTacticRouteRunError> {
    if trace.schema != NATIVE_TACTIC_PAIRED_TERMINAL_RETURN_SCHEMA_V1
        || trace.pair_sha256 == Digest::ZERO
        || trace.source_checkpoint_sha256 == Digest::ZERO
        || trace.source_state_sha256 == Digest::ZERO
        || trace.frozen_learner_snapshot_sha256 == Digest::ZERO
        || trace.control_target_checkpoint_sha256 == Digest::ZERO
        || trace.control_target_state_sha256 == Digest::ZERO
        || trace.incumbent_ticks_to_go == 0
        || trace.policy_option_id.is_empty()
        || trace.control_option_id.is_empty()
        || trace.policy_option_id == trace.control_option_id
        || (trace.role == NativeTacticPairedTerminalReturnRole::Policy
            && trace.status_after_decision
                == NativeTacticPairedTerminalReturnStatus::ControlInProgress)
        || (trace.role == NativeTacticPairedTerminalReturnRole::Control
            && matches!(
                trace.status_after_decision,
                NativeTacticPairedTerminalReturnStatus::PolicyInProgress
                    | NativeTacticPairedTerminalReturnStatus::ControlPending
            ))
    {
        return Err(route_message("paired terminal-return trace is invalid"));
    }
    Ok(())
}

fn validate_paired_terminal_return_successor(
    previous: &NativeTacticPairedTerminalReturnTrace,
    current: &NativeTacticPairedTerminalReturnTrace,
) -> Result<(), NativeTacticRouteRunError> {
    let static_authority_matches = previous.schema == current.schema
        && previous.pair_sha256 == current.pair_sha256
        && previous.source_decision_index == current.source_decision_index
        && previous.source_checkpoint_sha256 == current.source_checkpoint_sha256
        && previous.source_state_sha256 == current.source_state_sha256
        && previous.source_prefix_ticks == current.source_prefix_ticks
        && previous.incumbent_ticks_to_go == current.incumbent_ticks_to_go
        && previous.frozen_learner_snapshot_sha256 == current.frozen_learner_snapshot_sha256
        && previous.frozen_replay_revision == current.frozen_replay_revision
        && previous.policy_option_id == current.policy_option_id
        && previous.control_option_id == current.control_option_id
        && previous.control_target_checkpoint_sha256 == current.control_target_checkpoint_sha256
        && previous.control_target_state_sha256 == current.control_target_state_sha256
        && previous.control_first_step_ticks == current.control_first_step_ticks
        && previous.control_first_step_terminal == current.control_first_step_terminal
        && (!previous.policy_terminal_supported || current.policy_terminal_supported)
        && (!previous.control_terminal_supported || current.control_terminal_supported);
    let valid_transition = match previous.status_after_decision {
        NativeTacticPairedTerminalReturnStatus::PolicyInProgress => {
            current.role == NativeTacticPairedTerminalReturnRole::Policy
                && matches!(
                    current.status_after_decision,
                    NativeTacticPairedTerminalReturnStatus::PolicyInProgress
                        | NativeTacticPairedTerminalReturnStatus::ControlPending
                        | NativeTacticPairedTerminalReturnStatus::Complete
                )
        }
        NativeTacticPairedTerminalReturnStatus::ControlPending => {
            current.role == NativeTacticPairedTerminalReturnRole::Control
                && matches!(
                    current.status_after_decision,
                    NativeTacticPairedTerminalReturnStatus::ControlInProgress
                        | NativeTacticPairedTerminalReturnStatus::Complete
                )
        }
        NativeTacticPairedTerminalReturnStatus::ControlInProgress => {
            current.role == NativeTacticPairedTerminalReturnRole::Control
                && matches!(
                    current.status_after_decision,
                    NativeTacticPairedTerminalReturnStatus::ControlInProgress
                        | NativeTacticPairedTerminalReturnStatus::Complete
                )
        }
        NativeTacticPairedTerminalReturnStatus::Complete => false,
    };
    if !static_authority_matches || !valid_transition {
        return Err(route_message(
            "paired terminal-return trace changed authority or phase",
        ));
    }
    Ok(())
}

pub(super) fn paired_terminal_return_identity(
    execution_plan_sha256: Digest,
    decision_index: u64,
    source_checkpoint_sha256: Digest,
    source_state_sha256: Digest,
    policy_option_id: &str,
    control_option_id: &str,
) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(NATIVE_TACTIC_PAIRED_TERMINAL_RETURN_SCHEMA_V1.as_bytes());
    hasher.update(execution_plan_sha256.0);
    hasher.update(decision_index.to_le_bytes());
    hasher.update(source_checkpoint_sha256.0);
    hasher.update(source_state_sha256.0);
    for value in [policy_option_id, control_option_id] {
        hasher.update((value.len() as u64).to_le_bytes());
        hasher.update(value.as_bytes());
    }
    Digest(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_control::option_execution::OptionType;

    fn selected(option_id: &str) -> SelectedTactic {
        SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            descriptor: OptionActionDescriptor {
                option_id: option_id.into(),
                option_type: OptionType::Custom("paired-test".into()),
                parameters: BTreeMap::new(),
            },
            reason: TacticSelectionReason::Greedy,
            decision_index: 3,
            learner_snapshot_sha256: Digest([9; 32]),
            exploration_draw: 0,
        }
    }

    fn seed() -> PairedTerminalReturnSeed {
        PairedTerminalReturnSeed::from_pre_execution_proposals(
            Digest([1; 32]),
            3,
            Digest([2; 32]),
            Digest([3; 32]),
            Some(&TacticFrontierAcquisition {
                expansion_count: 1,
                terminal: false,
                terminal_value_supported: true,
                achieved_goal_value_supported: false,
                goal_reachability_supported: false,
                goal_reachability_evidence_available: false,
                reward: 0.0,
                best_mean_q: None,
                best_goal_progress_per_tick: None,
                predicted_terminal_ticks_to_go: None,
                predicted_total_terminal_ticks: None,
                exact_terminal_ticks_to_go: Some(10),
                exact_total_terminal_ticks: Some(30),
                maximum_ensemble_variance: None,
                generalized_nearest_distance: None,
                discovery_spatial_novelty: None,
                novelty_rank: 0,
                replayed_prefix_ticks: 20,
            }),
            Digest([4; 32]),
            12,
            &[selected("policy"), selected("control")],
        )
        .unwrap()
    }

    #[test]
    fn control_is_fixed_before_outcomes_and_both_receive_equal_budget() {
        let seed = seed();
        assert_eq!(seed.control_proposal_index(), 1);
        let (active, first) = seed
            .admit_first_steps(false, 21, Digest([5; 32]), Digest([6; 32]), 1, false, 21)
            .unwrap();
        assert_eq!(
            first.status_after_decision,
            NativeTacticPairedTerminalReturnStatus::PolicyInProgress
        );
        let mut active = active.unwrap();
        let policy_done = active.record_decision(false, 30);
        assert_eq!(
            policy_done.status_after_decision,
            NativeTacticPairedTerminalReturnStatus::ControlPending
        );
        active.begin_control();
        let control_done = active.record_decision(true, 24);
        assert_eq!(
            control_done.status_after_decision,
            NativeTacticPairedTerminalReturnStatus::Complete
        );
        assert!(!control_done.policy_terminal_supported);
        assert!(control_done.control_terminal_supported);
    }

    #[test]
    fn completed_pair_does_not_resume_after_recovery() {
        let (active, _) = seed()
            .admit_first_steps(true, 21, Digest([5; 32]), Digest([6; 32]), 10, false, 30)
            .unwrap();
        assert!(active.is_none());
    }
}
