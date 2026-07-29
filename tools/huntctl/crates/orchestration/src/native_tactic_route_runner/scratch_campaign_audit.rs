use super::scratch_discovery::route_report_sha256;
use super::*;
use crate::state_graph::StateGraph;
use crate::tactic_q_campaign::TacticQCampaign;
use dusklight_learning::tactic_exploration::TacticSelectionReason;
use std::collections::{BTreeMap, BTreeSet};

pub const NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V1: &str =
    "dusklight-native-tactic-scratch-campaign-audit/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTacticScratchStopReason {
    DecisionBudgetExhausted,
    SimulatedTickBudgetExhausted,
    NativeTickBudgetExhausted,
    WallBudgetExhausted,
    LegacyUnreportedBudget,
}

impl From<NativeTacticSeedStopReason> for NativeTacticScratchStopReason {
    fn from(value: NativeTacticSeedStopReason) -> Self {
        match value {
            NativeTacticSeedStopReason::DecisionBudgetReached => Self::DecisionBudgetExhausted,
            NativeTacticSeedStopReason::SimulatedTickBudgetReached => {
                Self::SimulatedTickBudgetExhausted
            }
            NativeTacticSeedStopReason::NativeTickBudgetReached => Self::NativeTickBudgetExhausted,
            NativeTacticSeedStopReason::WallBudgetReached => Self::WallBudgetExhausted,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchTerminalImprovementAudit {
    pub decision_index: u64,
    pub cumulative_wall_micros: u64,
    pub cumulative_proposal_expansions: u64,
    pub cumulative_useful_graph_expansions: u64,
    pub authenticated_tick: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchDecisionAudit {
    pub decision_index: u64,
    pub cumulative_wall_micros: u64,
    pub learner_snapshot_sha256: Digest,
    pub replay_rows_at_decision: u64,
    pub replay_generation: u64,
    pub acquisition_rank: u64,
    pub frontier_identity: Digest,
    pub source_route_ticks: u64,
    pub checkpoint_owner_worker_slot: Option<usize>,
    pub proposal_worker_slots: Vec<usize>,
    pub restore_source: Option<NativeTacticRestoreSource>,
    pub selected_option_id: String,
    pub selection_reason: TacticSelectionReason,
    /// Exact applicable action surface and fitted value support visible to the
    /// policy before native execution. Legacy reports omitted this evidence.
    #[serde(default)]
    pub applicable_tactics: Vec<NativeTacticValueTrace>,
    /// Exact graph/model-bound action queue that produced this selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler_decision: Option<TacticSchedulerDecisionTrace>,
    pub branch_acquisition: Option<TacticFrontierAcquisition>,
    pub proposal_count: u64,
    pub terminal_proposal_count: u64,
    pub retained_proposal_count: u64,
    pub completed_executable_graph_expansions: u64,
    pub terminal: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchSeedAudit {
    pub seed: u64,
    pub stop_reasons: Vec<NativeTacticScratchStopReason>,
    pub terminal_discovered: bool,
    pub best_authenticated_tick: Option<u64>,
    pub first_terminal_decision_index: Option<u64>,
    pub time_to_first_terminal_micros: Option<u64>,
    pub proposal_expansions_to_first_terminal: Option<u64>,
    pub useful_graph_expansions_to_first_terminal: Option<u64>,
    /// False for legacy reports written before terminal proposals carried
    /// their exact root-derived route length and cumulative graph work.
    pub terminal_improvement_timing_complete: bool,
    pub terminal_improvements: Vec<NativeTacticScratchTerminalImprovementAudit>,
    pub best_terminal_decision_index: Option<u64>,
    pub time_to_best_terminal_micros: Option<u64>,
    pub proposal_expansions_to_best_terminal: Option<u64>,
    pub useful_graph_expansions_to_best_terminal: Option<u64>,
    pub total_proposal_expansions: u64,
    pub native_ticks: u64,
    pub unique_useful_graph_expansions: u64,
    pub graph_expansion_timeline_complete: bool,
    /// True only when every decision retained a non-empty, duplicate-free
    /// action surface containing exactly one selected action.
    #[serde(default)]
    pub action_surface_timeline_complete: bool,
    /// True only when every decision retains a valid scheduler queue bound to
    /// the same learner revision reported for the decision.
    #[serde(default)]
    pub scheduler_timeline_complete: bool,
    /// Number of decisions on which each action was applicable.
    #[serde(default)]
    pub action_availability_counts: BTreeMap<String, u64>,
    /// Applicable decisions for which the learner had no fitted estimate.
    #[serde(default)]
    pub unsupported_action_availability_counts: BTreeMap<String, u64>,
    pub proposal_dispatches: u64,
    pub completed_graph_leases: u64,
    pub retryable_graph_leases: u64,
    pub cancelled_graph_leases: u64,
    pub failed_graph_leases: u64,
    pub unresolved_graph_leases: u64,
    pub terminal_path_ticks: Vec<u64>,
    pub selection_counts: BTreeMap<String, u64>,
    pub proposal_selection_counts: BTreeMap<String, u64>,
    pub learner_snapshots_consumed: Vec<Digest>,
    pub native_restore_accounting: NativeTacticRestoreAccounting,
    pub timing: NativeTacticRouteTiming,
    pub decisions: Vec<NativeTacticScratchDecisionAudit>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchCampaignAudit {
    pub schema: String,
    pub content_sha256: Digest,
    pub route_report_sha256: Digest,
    pub execution_plan_sha256: Digest,
    pub objective_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub workers: usize,
    pub proposal_policy: TacticProposalPolicy,
    pub value_treatment: TacticValueTreatment,
    pub seeds: Vec<NativeTacticScratchSeedAudit>,
}

impl NativeTacticScratchCampaignAudit {
    pub fn build(
        repository_root: &Path,
        route: &NativeTacticRouteReport,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let repository_root = repository_root.canonicalize().map_err(route_error)?;
        let mut seeds = Vec::with_capacity(route.seeds.len());
        for seed in &route.seeds {
            let checkpoint_path =
                confined_checkpoint(&repository_root, Path::new(&seed.final_checkpoint))?;
            let checkpoint =
                TacticQCampaign::read_checkpoint_payload(&checkpoint_path).map_err(route_error)?;
            if checkpoint
                .state_graph
                .content_sha256()
                .map_err(route_error)?
                != seed.state_graph_sha256
            {
                return Err(route_message(
                    "scratch campaign audit checkpoint graph identity differs",
                ));
            }
            seeds.push(seed_audit(route, seed, &checkpoint.state_graph)?);
        }
        seeds.sort_by_key(|seed| seed.seed);
        let mut audit = Self {
            schema: NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            route_report_sha256: route_report_sha256(route)?,
            execution_plan_sha256: route.execution_plan_sha256,
            objective_sha256: route.objective_sha256,
            execution_binding_sha256: route.execution_binding_sha256,
            workers: route.workers,
            proposal_policy: route.proposal_policy,
            value_treatment: route.value_treatment,
            seeds,
        };
        audit.content_sha256 = audit.compute_content_sha256()?;
        audit.validate()?;
        Ok(audit)
    }

    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.route_report_sha256 == Digest::ZERO
            || self.execution_plan_sha256 == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || self.execution_binding_sha256 == Digest::ZERO
            || self.workers == 0
            || self.seeds.is_empty()
            || !self
                .seeds
                .windows(2)
                .all(|pair| pair[0].seed < pair[1].seed)
            || self.seeds.iter().any(|seed| !seed_is_valid(seed))
            || self.compute_content_sha256()? != self.content_sha256
        {
            return Err(route_message("scratch campaign audit is invalid"));
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, NativeTacticRouteRunError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(route_error)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn compute_content_sha256(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        Ok(Digest(
            Sha256::digest(serde_json::to_vec(&unsigned).map_err(route_error)?).into(),
        ))
    }
}

fn seed_audit(
    route: &NativeTacticRouteReport,
    seed: &NativeTacticSeedResult,
    graph: &StateGraph,
) -> Result<NativeTacticScratchSeedAudit, NativeTacticRouteRunError> {
    let mut proposal_expansions = 0_u64;
    let mut proposal_expansions_to_first_terminal = None;
    let mut useful_graph_expansions_to_first_terminal = None;
    let mut proposal_selection_counts = BTreeMap::<String, u64>::new();
    let mut learner_snapshots = BTreeSet::new();
    let mut action_availability_counts = BTreeMap::<String, u64>::new();
    let mut unsupported_action_availability_counts = BTreeMap::<String, u64>::new();
    let mut action_surface_timeline_complete = true;
    let mut scheduler_timeline_complete = true;
    let mut terminal_improvements = Vec::new();
    let mut terminal_improvement_timing_complete = true;
    let mut best_observed_terminal_tick = None;
    let mut decisions = Vec::with_capacity(seed.trace.len());
    for trace in &seed.trace {
        let unique_action_ids = trace
            .applicable_tactics
            .iter()
            .map(|tactic| tactic.option_id.as_str())
            .collect::<BTreeSet<_>>();
        action_surface_timeline_complete &= !trace.applicable_tactics.is_empty()
            && unique_action_ids.len() == trace.applicable_tactics.len()
            && trace
                .applicable_tactics
                .iter()
                .filter(|tactic| tactic.selected)
                .count()
                == 1
            && trace
                .applicable_tactics
                .iter()
                .any(|tactic| tactic.selected && tactic.option_id == trace.selected_option_id);
        scheduler_timeline_complete &= trace.scheduler_decision.as_ref().is_some_and(|scheduler| {
            scheduler.learner_model_sha256 == trace.learner_snapshot_sha256
                && scheduler.validate().is_ok()
        });
        for tactic in &trace.applicable_tactics {
            *action_availability_counts
                .entry(tactic.option_id.clone())
                .or_default() += 1;
            if tactic.mean_q.is_none() {
                *unsupported_action_availability_counts
                    .entry(tactic.option_id.clone())
                    .or_default() += 1;
            }
        }
        let proposal_count = u64::try_from(trace.proposal_batch.len()).map_err(route_error)?;
        proposal_expansions = proposal_expansions.saturating_add(proposal_count);
        if Some(trace.decision_index) == seed.first_terminal_decision_index {
            proposal_expansions_to_first_terminal = Some(proposal_expansions);
            useful_graph_expansions_to_first_terminal =
                (trace.completed_executable_graph_expansions != 0)
                    .then_some(trace.completed_executable_graph_expansions);
        }
        if trace.learner_snapshot_sha256 != Digest::ZERO {
            learner_snapshots.insert(trace.learner_snapshot_sha256);
        }
        for proposal in &trace.proposal_batch {
            *proposal_selection_counts
                .entry(selection_reason_key(proposal.selection_reason)?)
                .or_default() += 1;
            if proposal.terminal {
                let Some(authenticated_tick) = proposal.root_route_ticks.checked_sub(1) else {
                    terminal_improvement_timing_complete = false;
                    continue;
                };
                if trace.completed_executable_graph_expansions == 0 {
                    terminal_improvement_timing_complete = false;
                    continue;
                }
                if best_observed_terminal_tick
                    .is_none_or(|incumbent| authenticated_tick < incumbent)
                {
                    best_observed_terminal_tick = Some(authenticated_tick);
                    terminal_improvements.push(NativeTacticScratchTerminalImprovementAudit {
                        decision_index: trace.decision_index,
                        cumulative_wall_micros: trace.cumulative_wall_micros,
                        cumulative_proposal_expansions: proposal_expansions,
                        cumulative_useful_graph_expansions: trace
                            .completed_executable_graph_expansions,
                        authenticated_tick,
                    });
                }
            }
        }
        decisions.push(NativeTacticScratchDecisionAudit {
            decision_index: trace.decision_index,
            cumulative_wall_micros: trace.cumulative_wall_micros,
            learner_snapshot_sha256: trace.learner_snapshot_sha256,
            replay_rows_at_decision: trace.replay_rows_at_decision,
            replay_generation: trace.replay_generation,
            acquisition_rank: trace.acquisition_rank,
            frontier_identity: trace.frontier_identity,
            source_route_ticks: trace.source_route_ticks,
            checkpoint_owner_worker_slot: trace.checkpoint_owner_worker_slot,
            proposal_worker_slots: trace.proposal_worker_slots.clone(),
            restore_source: trace.restore_source,
            selected_option_id: trace.selected_option_id.clone(),
            selection_reason: trace.selection_reason,
            applicable_tactics: trace.applicable_tactics.clone(),
            scheduler_decision: trace.scheduler_decision.clone(),
            branch_acquisition: trace.branch_acquisition.clone(),
            proposal_count,
            terminal_proposal_count: trace
                .proposal_batch
                .iter()
                .filter(|proposal| proposal.terminal)
                .count() as u64,
            retained_proposal_count: trace
                .proposal_batch
                .iter()
                .filter(|proposal| proposal.retained)
                .count() as u64,
            completed_executable_graph_expansions: trace.completed_executable_graph_expansions,
            terminal: trace.terminal,
        });
    }
    let mut terminal_path_ticks = graph
        .nodes()
        .filter(|node| node.terminal && node.restoration.executable)
        .map(|node| node.root_ticks.saturating_sub(1))
        .collect::<Vec<_>>();
    terminal_path_ticks.sort_unstable();
    let graph_metrics = seed
        .graph_metrics
        .as_ref()
        .ok_or_else(|| route_message("scratch campaign audit seed lacks graph metrics"))?;
    if terminal_path_ticks.len() as u64 != graph_metrics.terminal_paths
        || terminal_path_ticks.first().copied() != seed.best_authenticated_tick
        || seed.execution_plan_sha256 != route.execution_plan_sha256
    {
        return Err(route_message(
            "scratch campaign audit terminal paths differ from seed report",
        ));
    }
    let graph_expansion_timeline_complete = decisions
        .iter()
        .all(|decision| decision.completed_executable_graph_expansions != 0)
        && decisions.windows(2).all(|pair| {
            pair[0].completed_executable_graph_expansions
                <= pair[1].completed_executable_graph_expansions
        })
        && decisions
            .last()
            .map(|decision| decision.completed_executable_graph_expansions)
            == Some(seed.unique_useful_graph_expansions);
    if terminal_improvement_timing_complete
        && best_observed_terminal_tick != seed.best_authenticated_tick
    {
        return Err(route_message(
            "scratch campaign terminal improvement trace differs from final graph",
        ));
    }
    let best_terminal = terminal_improvements.last();
    Ok(NativeTacticScratchSeedAudit {
        seed: seed.seed,
        stop_reasons: stop_reasons(route, seed),
        terminal_discovered: seed.terminal_discovered,
        best_authenticated_tick: seed.best_authenticated_tick,
        first_terminal_decision_index: seed.first_terminal_decision_index,
        time_to_first_terminal_micros: seed.time_to_first_terminal_micros,
        proposal_expansions_to_first_terminal,
        useful_graph_expansions_to_first_terminal,
        terminal_improvement_timing_complete,
        terminal_improvements: terminal_improvements.clone(),
        best_terminal_decision_index: best_terminal.map(|row| row.decision_index),
        time_to_best_terminal_micros: best_terminal.map(|row| row.cumulative_wall_micros),
        proposal_expansions_to_best_terminal: best_terminal
            .map(|row| row.cumulative_proposal_expansions),
        useful_graph_expansions_to_best_terminal: best_terminal
            .map(|row| row.cumulative_useful_graph_expansions),
        total_proposal_expansions: proposal_expansions,
        native_ticks: seed.native_ticks,
        unique_useful_graph_expansions: seed.unique_useful_graph_expansions,
        graph_expansion_timeline_complete,
        action_surface_timeline_complete,
        scheduler_timeline_complete,
        action_availability_counts,
        unsupported_action_availability_counts,
        proposal_dispatches: graph_metrics.lease_accounting.proposal_dispatches,
        completed_graph_leases: graph_metrics.lease_accounting.completed_leases,
        retryable_graph_leases: graph_metrics.lease_accounting.retryable_leases,
        cancelled_graph_leases: graph_metrics.lease_accounting.cancelled_leases,
        failed_graph_leases: graph_metrics.lease_accounting.failed_leases,
        unresolved_graph_leases: graph_metrics.lease_accounting.unresolved_leases,
        terminal_path_ticks,
        selection_counts: seed.selection_counts.clone(),
        proposal_selection_counts,
        learner_snapshots_consumed: learner_snapshots.into_iter().collect(),
        native_restore_accounting: seed.native_restore_accounting.clone(),
        timing: seed.timing.clone(),
        decisions,
    })
}

fn stop_reasons(
    route: &NativeTacticRouteReport,
    seed: &NativeTacticSeedResult,
) -> Vec<NativeTacticScratchStopReason> {
    if !seed.stop_reasons.is_empty() {
        return seed.stop_reasons.iter().copied().map(Into::into).collect();
    }
    let mut reasons = Vec::new();
    if seed.decisions >= route.decisions_per_seed {
        reasons.push(NativeTacticScratchStopReason::DecisionBudgetExhausted);
    }
    if route
        .resource_budgets
        .native_ticks
        .reached(seed.native_ticks)
    {
        reasons.push(NativeTacticScratchStopReason::NativeTickBudgetExhausted);
    }
    if seed.wall_budget_reached {
        reasons.push(NativeTacticScratchStopReason::WallBudgetExhausted);
    }
    if reasons.is_empty() {
        reasons.push(NativeTacticScratchStopReason::LegacyUnreportedBudget);
    }
    reasons
}

fn seed_is_valid(seed: &NativeTacticScratchSeedAudit) -> bool {
    let total_proposals = seed
        .decisions
        .iter()
        .map(|decision| decision.proposal_count)
        .sum::<u64>();
    let first_terminal_valid = match (
        seed.terminal_discovered,
        seed.first_terminal_decision_index,
        seed.time_to_first_terminal_micros,
        seed.proposal_expansions_to_first_terminal,
        seed.best_authenticated_tick,
    ) {
        (true, Some(decision), Some(_), Some(expansions), Some(_)) => {
            expansions > 0
                && seed
                    .decisions
                    .iter()
                    .any(|row| row.decision_index == decision && row.terminal_proposal_count > 0)
        }
        (false, None, None, None, None) => true,
        _ => false,
    };
    seed.decisions
        .windows(2)
        .all(|pair| pair[0].decision_index < pair[1].decision_index)
        && !seed.stop_reasons.is_empty()
        && total_proposals == seed.total_proposal_expansions
        && seed.completed_graph_leases == total_proposals
        && seed.proposal_dispatches
            == seed
                .completed_graph_leases
                .saturating_add(seed.retryable_graph_leases)
                .saturating_add(seed.cancelled_graph_leases)
                .saturating_add(seed.failed_graph_leases)
                .saturating_add(seed.unresolved_graph_leases)
        && seed.unresolved_graph_leases == 0
        && (!seed.action_surface_timeline_complete
            || seed.decisions.iter().all(|decision| {
                !decision.applicable_tactics.is_empty()
                    && decision
                        .applicable_tactics
                        .iter()
                        .filter(|tactic| tactic.selected)
                        .count()
                        == 1
                    && decision.applicable_tactics.iter().any(|tactic| {
                        tactic.selected && tactic.option_id == decision.selected_option_id
                    })
            }))
        && (!seed.scheduler_timeline_complete
            || seed.decisions.iter().all(|decision| {
                decision
                    .scheduler_decision
                    .as_ref()
                    .is_some_and(|scheduler| {
                        scheduler.learner_model_sha256 == decision.learner_snapshot_sha256
                            && scheduler.validate().is_ok()
                    })
            }))
        && first_terminal_valid
        && seed.terminal_discovered == !seed.terminal_path_ticks.is_empty()
        && seed.best_authenticated_tick == seed.terminal_path_ticks.first().copied()
        && (!seed.terminal_improvement_timing_complete
            || (seed.best_terminal_decision_index.is_some() == seed.terminal_discovered
                && seed.time_to_best_terminal_micros.is_some() == seed.terminal_discovered
                && seed.proposal_expansions_to_best_terminal.is_some() == seed.terminal_discovered
                && seed.useful_graph_expansions_to_best_terminal.is_some()
                    == seed.terminal_discovered
                && seed
                    .terminal_improvements
                    .last()
                    .map(|row| row.authenticated_tick)
                    == seed.best_authenticated_tick))
}

fn selection_reason_key(
    reason: TacticSelectionReason,
) -> Result<String, NativeTacticRouteRunError> {
    serde_json::to_value(reason)
        .map_err(route_error)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| route_message("tactic selection reason is not a string"))
}

fn confined_checkpoint(
    repository_root: &Path,
    declared: &Path,
) -> Result<PathBuf, NativeTacticRouteRunError> {
    let candidate = if declared.is_absolute() {
        declared.to_path_buf()
    } else {
        repository_root.join(declared)
    };
    let resolved = candidate.canonicalize().map_err(route_error)?;
    if !resolved.starts_with(repository_root) || !resolved.is_file() {
        return Err(route_message(
            "scratch campaign checkpoint is outside the repository or absent",
        ));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_reason_keys_use_the_stable_wire_names() {
        assert_eq!(
            selection_reason_key(TacticSelectionReason::GoalReachability).unwrap(),
            "goal_reachability"
        );
        assert_eq!(
            selection_reason_key(TacticSelectionReason::RandomBaseline).unwrap(),
            "random_baseline"
        );
    }

    #[test]
    fn exact_stop_reasons_have_stable_audit_names() {
        assert_eq!(
            NativeTacticScratchStopReason::from(
                NativeTacticSeedStopReason::SimulatedTickBudgetReached
            ),
            NativeTacticScratchStopReason::SimulatedTickBudgetExhausted
        );
        assert_eq!(
            NativeTacticScratchStopReason::from(NativeTacticSeedStopReason::WallBudgetReached),
            NativeTacticScratchStopReason::WallBudgetExhausted
        );
    }
}
