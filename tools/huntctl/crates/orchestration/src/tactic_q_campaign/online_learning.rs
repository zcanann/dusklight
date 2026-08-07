use super::*;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TacticQOnlineFrontierStrategy {
    Graph,
    LearnedRanked {
        demonstration_curriculum: bool,
        goal_distance_feature: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TacticQOnlineBranchRequest {
    pub seed: u64,
    pub round: u64,
    pub acquisition_rank: u64,
    pub maximum_route_frames: usize,
    pub prefer_root: bool,
    pub strategy: TacticQOnlineFrontierStrategy,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TacticQOnlineBranchSelection {
    pub branch: TacticCampaignBranch,
    pub selected_root: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TacticQOnlineLeaseMode {
    Exploration,
    PolicyEvaluation {
        proposal_policy: TacticProposalPolicy,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct TacticQOnlineProposalLease {
    pub batch: TacticQProposalBatch,
    pub leases: Vec<TacticExpansionLease>,
    pub scheduler_decision: Option<TacticSchedulerDecisionTrace>,
    pub policy_evaluation_decision: Option<TacticPolicyEvaluationDecisionTrace>,
    pub timing: TacticGraphSchedulingTiming,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TacticQOnlineDecisionRequest {
    pub suffix_ticks: u64,
    pub horizon: u64,
    pub maximum_proposals: usize,
    pub learner_model_sha256: Digest,
    pub lease_mode: TacticQOnlineLeaseMode,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TacticQOnlineDecisionPlan {
    Execute(TacticQOnlineProposalLease),
    RestoreCheckpoint { selected_maximum_ticks: u32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct TacticQOnlineActionSurface {
    pub catalog: TacticAssetCatalog,
    pub blueprints: Vec<TacticBlueprint>,
    pub applicable_actions: Vec<OptionActionDescriptor>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TacticQOnlineContinuationRequest {
    pub force_branch: bool,
    pub terminal_restart: bool,
    pub native_terminal_supported: bool,
    pub next_acquisition_rank: u64,
    pub demonstration_coverage_pending: bool,
    pub terminal_refinement_in_progress: bool,
    pub terminal_refinement_completed: bool,
    pub root_refresh_due: bool,
    pub goal_relabeling_enabled: bool,
    pub terminal_frontier_action_value_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TacticQOnlineContinuationPlan {
    pub acquisition_rank: u64,
    pub terminal_support: bool,
    pub demonstration: bool,
    pub prefer_root: bool,
    pub use_learned_frontier: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TacticQOnlineContinuationSelectionRequest {
    pub continuation: TacticQOnlineContinuationRequest,
    pub seed: u64,
    pub round: u64,
    pub maximum_route_frames: usize,
    pub goal_distance_feature: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TacticQOnlineContinuationSelection {
    pub continuation: TacticQOnlineContinuationPlan,
    pub branch: TacticCampaignBranch,
    pub selected_root: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TacticQOnlineRolloutRequest {
    pub force_branch: bool,
    pub next_acquisition_rank: u64,
    pub demonstration_coverage_pending: bool,
    pub terminal_refinement_in_progress: bool,
    pub terminal_refinement_completed: bool,
    pub root_refresh_due: bool,
    pub goal_relabeling_enabled: bool,
    pub terminal_frontier_action_value_enabled: bool,
    pub seed: u64,
    pub round: u64,
    pub episode_group: u64,
    pub maximum_route_frames: usize,
    pub goal_distance_feature: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TacticQOnlineHorizonPlan {
    Execute(TacticQProposalBatch),
    RestoreCheckpoint { selected_maximum_ticks: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TacticQOnlinePolicyUpdate {
    Adaptive { refit_model: bool },
    Frozen,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TacticQOnlineAdmissionTiming {
    pub terminal_projection_micros: u64,
    pub graph_admission_micros: u64,
    pub selected_outcome_retention_micros: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TacticQOnlineAdmission {
    pub step: RewardedTacticQCampaignStep,
    pub terminal_candidates: Vec<TacticQFinalResult>,
    pub newly_admitted_training_rows: usize,
    pub duplicate_training_transitions: usize,
    pub evaluated_native_ticks: u64,
    pub best_authenticated_terminal_ticks: Option<u64>,
    pub timing: TacticQOnlineAdmissionTiming,
}

pub fn plan_online_continuation(
    request: TacticQOnlineContinuationRequest,
) -> Result<Option<TacticQOnlineContinuationPlan>, TacticQCampaignError> {
    let terminal_support = request.native_terminal_supported && request.next_acquisition_rank == 0;
    let scheduled_branch =
        request.demonstration_coverage_pending || request.terminal_restart || terminal_support;
    let should_restore = request.force_branch
        || request.terminal_restart
        || request.terminal_refinement_completed
        || (!request.terminal_refinement_in_progress && scheduled_branch);
    if !should_restore {
        return Ok(None);
    }

    // Reaching a terminal forces a restore, not a change of acquisition
    // partition. Otherwise every successful rollout restarts from rank zero
    // and can permanently starve the broad-discovery ranks.
    let acquisition_rank = request.next_acquisition_rank;
    let demonstration =
        request.demonstration_coverage_pending && !request.terminal_restart && !terminal_support;
    let learned_exploitation = acquisition_rank == 0;
    let force_scheduled_frontier = terminal_support
        || (!request.native_terminal_supported
            && learned_exploitation
            && request.goal_relabeling_enabled);
    let use_learned_frontier = demonstration
        || (!request.native_terminal_supported
            && learned_exploitation
            && request.goal_relabeling_enabled)
        || (terminal_support && request.terminal_frontier_action_value_enabled);
    Ok(Some(TacticQOnlineContinuationPlan {
        acquisition_rank,
        terminal_support,
        demonstration,
        prefer_root: !force_scheduled_frontier && request.root_refresh_due,
        use_learned_frontier,
    }))
}

pub fn online_tactic_fits_horizon(
    suffix_ticks: u64,
    selected_maximum_ticks: u32,
    horizon: u64,
) -> bool {
    suffix_ticks.saturating_add(u64::from(selected_maximum_ticks)) <= horizon
}

pub fn plan_online_horizon(
    mut batch: TacticQProposalBatch,
    suffix_ticks: u64,
    horizon: u64,
) -> Result<TacticQOnlineHorizonPlan, TacticQCampaignError> {
    let primary = batch
        .proposals
        .first()
        .ok_or(TacticQCampaignError::InvalidState(
            "online proposal batch is empty",
        ))?;
    let selected_maximum_ticks = batch
        .ranking
        .choices
        .iter()
        .find(|choice| choice.choice_id == primary.descriptor.option_id)
        .ok_or(TacticQCampaignError::InvalidState(
            "selected online tactic is absent from its action surface",
        ))?
        .duration
        .maximum_ticks;
    if !online_tactic_fits_horizon(suffix_ticks, selected_maximum_ticks, horizon) {
        return Ok(TacticQOnlineHorizonPlan::RestoreCheckpoint {
            selected_maximum_ticks,
        });
    }
    batch.proposals.retain(|proposal| {
        batch
            .ranking
            .choices
            .iter()
            .find(|choice| choice.choice_id == proposal.descriptor.option_id)
            .is_some_and(|choice| {
                online_tactic_fits_horizon(suffix_ticks, choice.duration.maximum_ticks, horizon)
            })
    });
    Ok(TacticQOnlineHorizonPlan::Execute(batch))
}

impl TacticQCampaign {
    /// Apply the shared rollout horizon to the policy-selected batch and lease
    /// every executable alternative that can finish inside that horizon. This
    /// is the complete pre-execution decision boundary used by both native and
    /// deterministic environments.
    pub fn prepare_online_decision(
        &mut self,
        batch: TacticQProposalBatch,
        request: TacticQOnlineDecisionRequest,
    ) -> Result<TacticQOnlineDecisionPlan, TacticQCampaignError> {
        let batch = match plan_online_horizon(batch, request.suffix_ticks, request.horizon)? {
            TacticQOnlineHorizonPlan::Execute(batch) => batch,
            TacticQOnlineHorizonPlan::RestoreCheckpoint {
                selected_maximum_ticks,
            } => {
                return Ok(TacticQOnlineDecisionPlan::RestoreCheckpoint {
                    selected_maximum_ticks,
                });
            }
        };
        let eligible = batch
            .ranking
            .choices
            .iter()
            .filter(|choice| {
                choice.applicable
                    && online_tactic_fits_horizon(
                        request.suffix_ticks,
                        choice.duration.maximum_ticks,
                        request.horizon,
                    )
            })
            .map(|choice| choice.descriptor.clone())
            .collect::<Vec<_>>();
        Ok(TacticQOnlineDecisionPlan::Execute(
            self.lease_online_batch(
                batch,
                &eligible,
                request.maximum_proposals,
                request.learner_model_sha256,
                request.lease_mode,
            )?,
        ))
    }

    /// Select and logically restore the next checkpoint through one shared
    /// environment-independent operation. The environment supplies only the
    /// executable action surface for a state; it does not choose or reinterpret
    /// the frontier selected by the learning loop.
    #[allow(clippy::too_many_arguments)]
    pub fn restore_online_continuation<E, F, A>(
        &mut self,
        request: TacticQOnlineContinuationSelectionRequest,
        episode_group: u64,
        registry: &FactRegistry,
        reference: &[TacticEndpointDescriptor],
        encode: &F,
        action_surface: &A,
    ) -> Result<Option<TacticQOnlineContinuationSelection>, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(
            &TacticQCampaign,
            &FactSnapshot,
        ) -> Result<TacticQOnlineActionSurface, TacticQCampaignError>,
    {
        let selection = self.select_online_continuation(request, reference, encode, &|state| {
            Ok::<_, TacticQCampaignError>(action_surface(self, state)?.applicable_actions)
        })?;
        let Some(selection) = selection else {
            return Ok(None);
        };
        let surface = action_surface(self, &selection.branch.state)?;
        self.restore_branch(
            &selection.branch,
            episode_group,
            registry,
            &surface.catalog,
            &surface.blueprints,
            |_| true,
        )?;
        Ok(Some(selection))
    }

    /// Continue the current rollout or restore the next scheduled checkpoint.
    /// Terminal state and terminal-support availability are derived from the
    /// authoritative campaign rather than repeated by environment adapters.
    pub fn continue_online_rollout<E, F, A>(
        &mut self,
        request: TacticQOnlineRolloutRequest,
        registry: &FactRegistry,
        reference: &[TacticEndpointDescriptor],
        encode: &F,
        action_surface: &A,
    ) -> Result<Option<TacticQOnlineContinuationSelection>, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(
            &TacticQCampaign,
            &FactSnapshot,
        ) -> Result<TacticQOnlineActionSurface, TacticQCampaignError>,
    {
        self.restore_online_continuation(
            TacticQOnlineContinuationSelectionRequest {
                continuation: TacticQOnlineContinuationRequest {
                    force_branch: request.force_branch,
                    terminal_restart: self.current.snapshot.terminal.reached == Some(true),
                    native_terminal_supported: self.native_terminal_supported(),
                    next_acquisition_rank: request.next_acquisition_rank,
                    demonstration_coverage_pending: request.demonstration_coverage_pending,
                    terminal_refinement_in_progress: request.terminal_refinement_in_progress,
                    terminal_refinement_completed: request.terminal_refinement_completed,
                    root_refresh_due: request.root_refresh_due,
                    goal_relabeling_enabled: request.goal_relabeling_enabled,
                    terminal_frontier_action_value_enabled: request
                        .terminal_frontier_action_value_enabled,
                },
                seed: request.seed,
                round: request.round,
                maximum_route_frames: request.maximum_route_frames,
                goal_distance_feature: request.goal_distance_feature,
            },
            request.episode_group,
            registry,
            reference,
            encode,
            action_surface,
        )
    }

    /// Decide whether the current rollout must branch and, when it must,
    /// select the exact executable frontier through the matching acquisition
    /// partition. Environments restore the returned branch; they do not
    /// reinterpret terminal, curriculum, exploitation, or discovery policy.
    pub fn select_online_continuation<E, AE, F, A>(
        &self,
        request: TacticQOnlineContinuationSelectionRequest,
        reference: &[TacticEndpointDescriptor],
        encode: &F,
        applicable_actions: &A,
    ) -> Result<Option<TacticQOnlineContinuationSelection>, TacticQCampaignError>
    where
        E: fmt::Display,
        AE: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(&FactSnapshot) -> Result<Vec<OptionActionDescriptor>, AE>,
    {
        let Some(continuation) = plan_online_continuation(request.continuation)? else {
            return Ok(None);
        };
        let strategy = if continuation.use_learned_frontier {
            TacticQOnlineFrontierStrategy::LearnedRanked {
                demonstration_curriculum: continuation.demonstration,
                goal_distance_feature: request.goal_distance_feature,
            }
        } else {
            TacticQOnlineFrontierStrategy::Graph
        };
        let selection = self.select_online_branch(
            TacticQOnlineBranchRequest {
                seed: request.seed,
                round: request.round,
                acquisition_rank: continuation.acquisition_rank,
                maximum_route_frames: request.maximum_route_frames,
                prefer_root: continuation.prefer_root,
                strategy,
            },
            reference,
            encode,
            applicable_actions,
        )?;
        Ok(Some(TacticQOnlineContinuationSelection {
            continuation,
            branch: selection.branch,
            selected_root: selection.selected_root,
        }))
    }

    /// Select the next executable checkpoint through the same acquisition
    /// policy regardless of which environment will restore and execute it.
    /// The action-surface callback prevents a fixed or prompted action library
    /// from turning an already exhausted state into an infinite retry loop.
    pub fn select_online_branch<E, AE, F, A>(
        &self,
        request: TacticQOnlineBranchRequest,
        reference: &[TacticEndpointDescriptor],
        encode: &F,
        applicable_actions: &A,
    ) -> Result<TacticQOnlineBranchSelection, TacticQCampaignError>
    where
        E: fmt::Display,
        AE: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(&FactSnapshot) -> Result<Vec<OptionActionDescriptor>, AE>,
    {
        let [root, frontier] = match request.strategy {
            TacticQOnlineFrontierStrategy::Graph => self
                .graph_scheduled_root_and_frontier_with_action_surface(
                    request.seed,
                    request.round,
                    request.acquisition_rank,
                    request.maximum_route_frames,
                    applicable_actions,
                )?,
            TacticQOnlineFrontierStrategy::LearnedRanked {
                demonstration_curriculum,
                goal_distance_feature,
            } => self.sample_root_and_ranked_frontier(
                request.seed,
                request.round,
                reference,
                request.maximum_route_frames,
                demonstration_curriculum,
                goal_distance_feature,
                encode,
                applicable_actions,
            )?,
        };
        Ok(TacticQOnlineBranchSelection {
            branch: if request.prefer_root { root } else { frontier },
            selected_root: request.prefer_root,
        })
    }

    /// Seal the exact proposals that an environment may execute. Ordinary
    /// learning leases unique graph expansions; explicit causal evaluation can
    /// repeat a policy-ranked action without changing exploration ownership.
    pub fn lease_online_batch(
        &mut self,
        batch: TacticQProposalBatch,
        eligible: &[OptionActionDescriptor],
        maximum_proposals: usize,
        learner_model_sha256: Digest,
        mode: TacticQOnlineLeaseMode,
    ) -> Result<TacticQOnlineProposalLease, TacticQCampaignError> {
        match mode {
            TacticQOnlineLeaseMode::Exploration => {
                let leased = self.lease_current_parameterized_batch(
                    batch,
                    eligible,
                    maximum_proposals,
                    learner_model_sha256,
                )?;
                Ok(TacticQOnlineProposalLease {
                    batch: leased.batch,
                    leases: leased.leases,
                    scheduler_decision: Some(leased.scheduler_decision),
                    policy_evaluation_decision: None,
                    timing: leased.timing,
                })
            }
            TacticQOnlineLeaseMode::PolicyEvaluation { proposal_policy } => {
                let evaluated = self.authorize_current_policy_evaluation_batch(
                    batch,
                    eligible,
                    maximum_proposals,
                    learner_model_sha256,
                    proposal_policy,
                )?;
                Ok(TacticQOnlineProposalLease {
                    batch: evaluated.batch,
                    leases: evaluated.leases,
                    scheduler_decision: None,
                    policy_evaluation_decision: Some(evaluated.evaluation_decision),
                    timing: evaluated.timing,
                })
            }
        }
    }

    /// Commit one policy-selected online decision and all of its native
    /// counterfactuals. The first proposal remains authoritative; sibling
    /// outcomes teach the graph and policy but cannot replace it after their
    /// results are observed.
    #[allow(clippy::too_many_arguments)]
    pub fn admit_online_batch<E, F, A>(
        &mut self,
        batch: &TacticQProposalBatch,
        evaluated: &[EvaluatedRewardedTacticOutcome],
        episode_groups: &[u64],
        leases: &[TacticExpansionLease],
        next_catalog: &TacticAssetCatalog,
        next_blueprints: &[TacticBlueprint],
        registry: &FactRegistry,
        encode: &F,
        entry_applicable: A,
        reward_spec: &TacticRewardSpec,
        policy_update: TacticQOnlinePolicyUpdate,
    ) -> Result<TacticQOnlineAdmission, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(&TacticAssetDescription) -> bool,
    {
        let winner = evaluated.first().ok_or(TacticQCampaignError::InvalidState(
            "online admission requires at least one evaluated proposal",
        ))?;
        if batch.proposals.first() != Some(&winner.outcome.selected)
            || batch.proposals.len() != evaluated.len()
        {
            return Err(TacticQCampaignError::InvalidState(
                "online admission outcomes differ from the policy-selected batch",
            ));
        }

        let terminal_projection_started = Instant::now();
        let terminal_candidates = evaluated
            .iter()
            .filter(|proposal| proposal.outcome.terminal)
            .map(|proposal| self.final_result_from_evaluated_terminal(proposal))
            .collect::<Result<Vec<_>, _>>()?;
        let terminal_projection_micros = elapsed_micros(terminal_projection_started);

        let winning_outcome = winner.outcome.clone();
        let expected_transition = winner.transition.clone();
        let expected_reward = winner.reward.clone();
        let decision = TacticQDecision {
            ranking: batch.ranking.clone(),
            selected: winning_outcome.selected.clone(),
        };
        let evaluated_native_ticks = evaluated.iter().fold(0_u64, |total, proposal| {
            total.saturating_add(u64::from(
                proposal.outcome.execution.duration.realized_ticks,
            ))
        });

        let graph_admission_started = Instant::now();
        let newly_admitted_training_rows =
            self.admit_leased_evaluated_replay(evaluated, episode_groups, leases)?;
        let graph_admission_micros = elapsed_micros(graph_admission_started);

        let selected_outcome_retention_started = Instant::now();
        let step = match policy_update {
            TacticQOnlinePolicyUpdate::Adaptive { refit_model } => self.retain_and_refit_rewarded(
                decision,
                winning_outcome,
                next_catalog,
                next_blueprints,
                registry,
                encode,
                entry_applicable,
                reward_spec,
                refit_model,
            )?,
            TacticQOnlinePolicyUpdate::Frozen => self.retain_rewarded_without_policy_update(
                decision,
                winning_outcome,
                next_catalog,
                next_blueprints,
                registry,
                encode,
                entry_applicable,
                reward_spec,
            )?,
        };
        let selected_outcome_retention_micros = elapsed_micros(selected_outcome_retention_started);
        if step.step.transition != *expected_transition || step.reward != expected_reward {
            return Err(TacticQCampaignError::InvalidState(
                "retained online proposal differs from its pre-admission evaluation",
            ));
        }

        Ok(TacticQOnlineAdmission {
            step,
            terminal_candidates,
            newly_admitted_training_rows,
            duplicate_training_transitions: evaluated
                .len()
                .saturating_sub(newly_admitted_training_rows),
            evaluated_native_ticks,
            best_authenticated_terminal_ticks: self
                .best_graph_terminal_path()?
                .map(|path| path.root_to_terminal_ticks),
            timing: TacticQOnlineAdmissionTiming {
                terminal_projection_micros,
                graph_admission_micros,
                selected_outcome_retention_micros,
            },
        })
    }
}

fn elapsed_micros(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod continuation_tests {
    use super::*;

    fn request() -> TacticQOnlineContinuationRequest {
        TacticQOnlineContinuationRequest {
            force_branch: false,
            terminal_restart: false,
            native_terminal_supported: false,
            next_acquisition_rank: 1,
            demonstration_coverage_pending: false,
            terminal_refinement_in_progress: false,
            terminal_refinement_completed: false,
            root_refresh_due: false,
            goal_relabeling_enabled: false,
            terminal_frontier_action_value_enabled: false,
        }
    }

    #[test]
    fn unforced_nonterminal_rollout_continues_until_terminal_or_horizon() {
        assert_eq!(plan_online_continuation(request()).unwrap(), None);
        let mut refinement = request();
        refinement.force_branch = true;
        refinement.terminal_refinement_in_progress = true;
        assert!(plan_online_continuation(refinement).unwrap().is_some());
    }

    #[test]
    fn terminal_support_owns_rank_zero_and_cannot_be_replaced_by_root_refresh() {
        let mut terminal = request();
        terminal.native_terminal_supported = true;
        terminal.next_acquisition_rank = 0;
        terminal.root_refresh_due = true;
        terminal.terminal_frontier_action_value_enabled = true;
        assert_eq!(
            plan_online_continuation(terminal).unwrap().unwrap(),
            TacticQOnlineContinuationPlan {
                acquisition_rank: 0,
                terminal_support: true,
                demonstration: false,
                prefer_root: false,
                use_learned_frontier: true,
            }
        );
    }

    #[test]
    fn terminal_restart_preserves_the_scheduled_discovery_partition() {
        let mut terminal = request();
        terminal.terminal_restart = true;
        terminal.native_terminal_supported = true;
        terminal.next_acquisition_rank = 2;
        terminal.root_refresh_due = true;
        terminal.terminal_frontier_action_value_enabled = true;
        assert_eq!(
            plan_online_continuation(terminal).unwrap().unwrap(),
            TacticQOnlineContinuationPlan {
                acquisition_rank: 2,
                terminal_support: false,
                demonstration: false,
                prefer_root: true,
                use_learned_frontier: false,
            }
        );
    }

    #[test]
    fn forced_horizon_branch_still_preserves_discovery_partition() {
        let mut forced = request();
        forced.force_branch = true;
        forced.next_acquisition_rank = 3;
        forced.root_refresh_due = true;
        assert_eq!(
            plan_online_continuation(forced).unwrap().unwrap(),
            TacticQOnlineContinuationPlan {
                acquisition_rank: 3,
                terminal_support: false,
                demonstration: false,
                prefer_root: true,
                use_learned_frontier: false,
            }
        );
    }

    #[test]
    fn learned_frontiers_are_limited_to_curriculum_and_exploitation_partitions() {
        let mut preterminal = request();
        preterminal.force_branch = true;
        preterminal.next_acquisition_rank = 0;
        preterminal.goal_relabeling_enabled = true;
        preterminal.root_refresh_due = true;
        let preterminal = plan_online_continuation(preterminal).unwrap().unwrap();
        assert!(preterminal.use_learned_frontier);
        assert!(!preterminal.prefer_root);

        let mut curriculum = request();
        curriculum.demonstration_coverage_pending = true;
        let curriculum = plan_online_continuation(curriculum).unwrap().unwrap();
        assert!(curriculum.demonstration);
        assert!(curriculum.use_learned_frontier);

        let mut broad = request();
        broad.force_branch = true;
        broad.native_terminal_supported = true;
        broad.next_acquisition_rank = 2;
        broad.terminal_frontier_action_value_enabled = true;
        assert!(
            !plan_online_continuation(broad)
                .unwrap()
                .unwrap()
                .use_learned_frontier
        );
    }
}
