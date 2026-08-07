use super::*;
use std::time::Instant;

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

impl TacticQCampaign {
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
