use super::scratch_discovery::route_report_sha256;
use super::*;
use crate::state_graph::{ActionExpansionStatus, ExpansionEvidenceAuthority, StateGraph};
use crate::tactic_q_campaign::TacticQCampaign;
use dusklight_learning::tactic_exploration::TacticSelectionReason;
use std::collections::{BTreeMap, BTreeSet};

pub const NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V2: &str =
    "dusklight-native-tactic-scratch-campaign-audit/v2";
pub const NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V3: &str =
    "dusklight-native-tactic-scratch-campaign-audit/v3";

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignResourceAudit {
    pub completed_decisions: u64,
    pub declared_memory_bound_bytes: Option<u64>,
    pub configured_checkpoint_cache_capacity_per_worker_bytes: u64,
    pub configured_checkpoint_pool_capacity_bytes: u64,
    pub observed_peak_worker_resident_bytes: u64,
    pub observed_checkpoint_pool_resident_upper_bound_bytes: u64,
    pub memory_bound_satisfied: bool,
    pub maximum_allowed_stale_replay_revisions: u64,
    pub maximum_model_replay_lag_revisions: u64,
    pub maximum_lane_refresh_gap_revisions: u64,
    pub learner_staleness_bound_satisfied: bool,
    pub direct_restore_fallback_replays: u64,
    pub prefix_materializations: u64,
    pub fallback_rate_per_million_decisions: u64,
    pub fallback_bound_satisfied: bool,
    pub checkpoint_owner_available_decisions: u64,
    pub checkpoint_owner_local_decisions: u64,
    pub misrouted_owner_local_decisions: u64,
    pub checkpoint_owner_counts_by_worker: Vec<u64>,
    pub checkpoint_owner_assignment_skew: u64,
    pub passed: bool,
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
    pub resources: NativeTacticCampaignResourceAudit,
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
        let plan = NativeTacticExecutionPlan::read(Path::new(&route.execution_plan_path))?;
        if plan.identity()? != route.execution_plan_sha256
            || plan.budgets != route.resource_budgets
            || plan.seeds != route.exploration_seeds
        {
            return Err(route_message(
                "scratch campaign resource audit is detached from its execution plan",
            ));
        }
        let resources = resource_audit(route, &plan)?;
        let mut audit = Self {
            schema: NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V3.into(),
            content_sha256: Digest::ZERO,
            route_report_sha256: route_report_sha256(route)?,
            execution_plan_sha256: route.execution_plan_sha256,
            objective_sha256: route.objective_sha256,
            execution_binding_sha256: route.execution_binding_sha256,
            workers: route.workers,
            proposal_policy: route.proposal_policy,
            value_treatment: route.value_treatment,
            resources,
            seeds,
        };
        audit.content_sha256 = audit.compute_content_sha256()?;
        audit.validate()?;
        Ok(audit)
    }

    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V3
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
            || !resource_audit_is_valid(&self.resources, self.workers, &self.seeds)
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

    pub fn validate_resource_binding(
        &self,
        route: &NativeTacticRouteReport,
        plan: &NativeTacticExecutionPlan,
    ) -> Result<(), NativeTacticRouteRunError> {
        self.validate()?;
        if self.route_report_sha256 != route_report_sha256(route)?
            || self.execution_plan_sha256 != plan.identity()?
            || self.resources != resource_audit(route, plan)?
        {
            return Err(route_message(
                "scratch campaign resource audit is detached from its route",
            ));
        }
        Ok(())
    }

    fn compute_content_sha256(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        Ok(Digest(
            Sha256::digest(serde_json::to_vec(&unsigned).map_err(route_error)?).into(),
        ))
    }
}

pub(super) fn resource_audit(
    route: &NativeTacticRouteReport,
    plan: &NativeTacticExecutionPlan,
) -> Result<NativeTacticCampaignResourceAudit, NativeTacticRouteRunError> {
    let workers = u64::try_from(route.workers).map_err(route_error)?;
    let maximum_per_worker = u64::try_from(tactic_checkpoint_cache_capacity_per_worker(
        plan.budgets.memory_bytes,
        route.workers,
    )?)
    .map_err(route_error)?;
    let configured_per_worker = route.checkpoint_cache_capacity_per_worker_bytes;
    if configured_per_worker == 0 || configured_per_worker > maximum_per_worker {
        return Err(route_message(
            "native tactic reported checkpoint capacity exceeds its execution plan",
        ));
    }
    let configured_pool = configured_per_worker
        .checked_mul(workers)
        .ok_or_else(|| route_message("native tactic configured checkpoint memory overflows"))?;
    let observed_peak_worker = route.native_restore_accounting.peak_resident_bytes;
    let observed_pool = observed_peak_worker
        .checked_mul(workers)
        .ok_or_else(|| route_message("native tactic observed checkpoint memory overflows"))?;
    let declared_memory_bound = match plan.budgets.memory_bytes {
        NativeTacticResourceLimit::Bounded(bytes) => Some(bytes),
        NativeTacticResourceLimit::Unbounded => None,
    };
    let memory_bound_satisfied = observed_pool <= configured_pool
        && declared_memory_bound.is_some_and(|bound| configured_pool <= bound);
    let maximum_allowed_staleness = match plan.replay_sharing {
        NativeTacticReplaySharingPlan::GenerationBarrier => 0,
        NativeTacticReplaySharingPlan::BoundedStaleness {
            maximum_stale_replay_revisions,
        } => maximum_stale_replay_revisions,
    };
    let maximum_model_replay_lag = route.replay_sharing.maximum_model_replay_lag_revisions;
    let learner_staleness_bound_satisfied = maximum_model_replay_lag <= maximum_allowed_staleness;
    let fallback_replays = route
        .native_restore_accounting
        .direct_restore_fallback_replays;
    let prefix_materializations = route.native_restore_accounting.prefix_materializations;
    let fallback_bound_satisfied = fallback_replays <= prefix_materializations;
    let mut checkpoint_owner_counts_by_worker = vec![0_u64; route.workers];
    let mut checkpoint_owner_available_decisions = 0_u64;
    let mut checkpoint_owner_local_decisions = 0_u64;
    let mut misrouted_owner_local_decisions = 0_u64;
    for trace in route.seeds.iter().flat_map(|seed| &seed.trace) {
        if let Some(owner) = trace.checkpoint_owner_worker_slot {
            let count = checkpoint_owner_counts_by_worker
                .get_mut(owner)
                .ok_or_else(|| route_message("native tactic checkpoint owner is not a worker"))?;
            *count = count.saturating_add(1);
            checkpoint_owner_available_decisions =
                checkpoint_owner_available_decisions.saturating_add(1);
        }
        if trace.restore_source == Some(NativeTacticRestoreSource::ProcessLocalCheckpoint) {
            if trace.proposal_worker_slots.first().copied() == trace.checkpoint_owner_worker_slot
                && trace.checkpoint_owner_worker_slot.is_some()
            {
                checkpoint_owner_local_decisions =
                    checkpoint_owner_local_decisions.saturating_add(1);
            } else {
                misrouted_owner_local_decisions = misrouted_owner_local_decisions.saturating_add(1);
            }
        }
    }
    let minimum_owner_assignments = checkpoint_owner_counts_by_worker
        .iter()
        .copied()
        .min()
        .unwrap_or(0);
    let maximum_owner_assignments = checkpoint_owner_counts_by_worker
        .iter()
        .copied()
        .max()
        .unwrap_or(0);
    let checkpoint_owner_assignment_skew =
        maximum_owner_assignments.saturating_sub(minimum_owner_assignments);
    let fallback_rate_per_million_decisions =
        ratio_per_million(fallback_replays, route.total_decisions);
    let passed = memory_bound_satisfied
        && learner_staleness_bound_satisfied
        && fallback_bound_satisfied
        && misrouted_owner_local_decisions == 0;
    Ok(NativeTacticCampaignResourceAudit {
        completed_decisions: route.total_decisions,
        declared_memory_bound_bytes: declared_memory_bound,
        configured_checkpoint_cache_capacity_per_worker_bytes: configured_per_worker,
        configured_checkpoint_pool_capacity_bytes: configured_pool,
        observed_peak_worker_resident_bytes: observed_peak_worker,
        observed_checkpoint_pool_resident_upper_bound_bytes: observed_pool,
        memory_bound_satisfied,
        maximum_allowed_stale_replay_revisions: maximum_allowed_staleness,
        maximum_model_replay_lag_revisions: maximum_model_replay_lag,
        maximum_lane_refresh_gap_revisions: route.replay_sharing.maximum_observed_stale_revisions,
        learner_staleness_bound_satisfied,
        direct_restore_fallback_replays: fallback_replays,
        prefix_materializations,
        fallback_rate_per_million_decisions,
        fallback_bound_satisfied,
        checkpoint_owner_available_decisions,
        checkpoint_owner_local_decisions,
        misrouted_owner_local_decisions,
        checkpoint_owner_counts_by_worker,
        checkpoint_owner_assignment_skew,
        passed,
    })
}

fn resource_audit_is_valid(
    resources: &NativeTacticCampaignResourceAudit,
    workers: usize,
    seeds: &[NativeTacticScratchSeedAudit],
) -> bool {
    let total_decisions = seeds
        .iter()
        .map(|seed| seed.decisions.len() as u64)
        .sum::<u64>();
    let configured_pool = resources
        .configured_checkpoint_cache_capacity_per_worker_bytes
        .checked_mul(workers as u64);
    let observed_pool = resources
        .observed_peak_worker_resident_bytes
        .checked_mul(workers as u64);
    let mut derived_owner_counts = vec![0_u64; workers];
    let mut derived_owner_available = 0_u64;
    let mut derived_owner_local = 0_u64;
    let mut derived_owner_misrouted = 0_u64;
    for decision in seeds.iter().flat_map(|seed| &seed.decisions) {
        if let Some(owner) = decision.checkpoint_owner_worker_slot {
            let Some(count) = derived_owner_counts.get_mut(owner) else {
                return false;
            };
            *count = count.saturating_add(1);
            derived_owner_available = derived_owner_available.saturating_add(1);
        }
        if decision.restore_source == Some(NativeTacticRestoreSource::ProcessLocalCheckpoint) {
            if decision.proposal_worker_slots.first().copied()
                == decision.checkpoint_owner_worker_slot
                && decision.checkpoint_owner_worker_slot.is_some()
            {
                derived_owner_local = derived_owner_local.saturating_add(1);
            } else {
                derived_owner_misrouted = derived_owner_misrouted.saturating_add(1);
            }
        }
    }
    let minimum_owner_assignments = resources
        .checkpoint_owner_counts_by_worker
        .iter()
        .copied()
        .min()
        .unwrap_or(0);
    let maximum_owner_assignments = resources
        .checkpoint_owner_counts_by_worker
        .iter()
        .copied()
        .max()
        .unwrap_or(0);
    let memory_bound_satisfied = observed_pool
        == Some(resources.observed_checkpoint_pool_resident_upper_bound_bytes)
        && configured_pool == Some(resources.configured_checkpoint_pool_capacity_bytes)
        && observed_pool.is_some_and(|observed| {
            observed <= resources.configured_checkpoint_pool_capacity_bytes
        })
        && resources
            .declared_memory_bound_bytes
            .is_some_and(|bound| resources.configured_checkpoint_pool_capacity_bytes <= bound);
    let learner_staleness_bound_satisfied = resources.maximum_model_replay_lag_revisions
        <= resources.maximum_allowed_stale_replay_revisions;
    let fallback_bound_satisfied =
        resources.direct_restore_fallback_replays <= resources.prefix_materializations;
    let passed = memory_bound_satisfied
        && learner_staleness_bound_satisfied
        && fallback_bound_satisfied
        && resources.misrouted_owner_local_decisions == 0;
    workers > 0
        && resources.completed_decisions == total_decisions
        && resources.configured_checkpoint_cache_capacity_per_worker_bytes > 0
        && resources.configured_checkpoint_cache_capacity_per_worker_bytes
            <= TACTIC_CHECKPOINT_CACHE_BYTES as u64
        && resources.checkpoint_owner_counts_by_worker.len() == workers
        && resources.checkpoint_owner_counts_by_worker == derived_owner_counts
        && resources.checkpoint_owner_available_decisions == derived_owner_available
        && resources.checkpoint_owner_local_decisions == derived_owner_local
        && resources.misrouted_owner_local_decisions == derived_owner_misrouted
        && resources.fallback_rate_per_million_decisions
            == ratio_per_million(resources.direct_restore_fallback_replays, total_decisions)
        && resources.checkpoint_owner_assignment_skew
            == maximum_owner_assignments.saturating_sub(minimum_owner_assignments)
        && resources.memory_bound_satisfied == memory_bound_satisfied
        && resources.learner_staleness_bound_satisfied == learner_staleness_bound_satisfied
        && resources.fallback_bound_satisfied == fallback_bound_satisfied
        && resources.passed == passed
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
    let source_frame = graph
        .node(graph.root())
        .map(|root| root.restoration.route.tape_frames)
        .ok_or_else(|| route_message("scratch campaign audit graph root is absent"))?;
    let mut decisions = Vec::with_capacity(seed.trace.len());
    for trace in &seed.trace {
        if trace.source_route_ticks != trace.before.tape_frame {
            return Err(route_message(
                "scratch campaign decision source route differs from its native boundary",
            ));
        }
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
                && scheduler.evaluated_expansion_sha256.len() == trace.proposal_batch.len()
                && trace
                    .proposal_batch
                    .iter()
                    .enumerate()
                    .all(|(index, proposal)| {
                        proposal_matches_graph_expansion(graph, scheduler, index, proposal)
                    })
        });
        for tactic in &trace.applicable_tactics {
            let availability = action_availability_counts
                .entry(tactic.option_id.clone())
                .or_default();
            *availability = availability
                .checked_add(1)
                .ok_or_else(|| route_message("scratch action availability count overflows"))?;
            if tactic.mean_q.is_none() {
                let unsupported = unsupported_action_availability_counts
                    .entry(tactic.option_id.clone())
                    .or_default();
                *unsupported = unsupported.checked_add(1).ok_or_else(|| {
                    route_message("scratch unsupported action availability count overflows")
                })?;
            }
        }
        let proposal_count = u64::try_from(trace.proposal_batch.len()).map_err(route_error)?;
        proposal_expansions = proposal_expansions
            .checked_add(proposal_count)
            .ok_or_else(|| route_message("scratch proposal expansion count overflows"))?;
        if Some(trace.decision_index) == seed.first_terminal_decision_index {
            proposal_expansions_to_first_terminal = Some(proposal_expansions);
            useful_graph_expansions_to_first_terminal =
                (trace.completed_executable_graph_expansions != 0)
                    .then_some(trace.completed_executable_graph_expansions);
        }
        if trace.learner_snapshot_sha256 != Digest::ZERO {
            learner_snapshots.insert(trace.learner_snapshot_sha256);
        }
        for (proposal_index, proposal) in trace.proposal_batch.iter().enumerate() {
            let selected = proposal_selection_counts
                .entry(selection_reason_key(proposal.selection_reason)?)
                .or_default();
            *selected = selected
                .checked_add(1)
                .ok_or_else(|| route_message("scratch proposal selection count overflows"))?;
            if proposal.terminal {
                let graph_authenticated =
                    trace.scheduler_decision.as_ref().is_some_and(|scheduler| {
                        proposal_matches_graph_expansion(graph, scheduler, proposal_index, proposal)
                    });
                if !graph_authenticated {
                    terminal_improvement_timing_complete = false;
                    continue;
                }
                let Some(authenticated_tick) =
                    route_frames_first_hit_tick(proposal.root_route_ticks, source_frame)
                else {
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
        .map(|node| {
            node.root_ticks.checked_sub(1).ok_or_else(|| {
                route_message("scratch campaign terminal node precedes its first native tick")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
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
    let Some(total_proposals) = seed.decisions.iter().try_fold(0_u64, |total, decision| {
        total.checked_add(decision.proposal_count)
    }) else {
        return false;
    };
    let first_terminal_valid = first_terminal_evidence_is_valid(seed);
    seed.decisions.windows(2).all(|pair| {
        pair[0].decision_index < pair[1].decision_index
            && pair[0].cumulative_wall_micros <= pair[1].cumulative_wall_micros
    }) && !seed.stop_reasons.is_empty()
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
                let unique = decision
                    .applicable_tactics
                    .iter()
                    .map(|tactic| tactic.option_id.as_str())
                    .collect::<BTreeSet<_>>();
                !decision.applicable_tactics.is_empty()
                    && unique.len() == decision.applicable_tactics.len()
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
                            && usize::try_from(decision.proposal_count).ok()
                                == Some(scheduler.evaluated_expansion_sha256.len())
                    })
            }))
        && first_terminal_valid
        && seed.terminal_discovered == !seed.terminal_path_ticks.is_empty()
        && seed
            .terminal_path_ticks
            .windows(2)
            .all(|pair| pair[0] <= pair[1])
        && seed.best_authenticated_tick == seed.terminal_path_ticks.first().copied()
        && terminal_improvement_timeline_is_valid(seed)
}

fn first_terminal_evidence_is_valid(seed: &NativeTacticScratchSeedAudit) -> bool {
    match (
        seed.terminal_discovered,
        seed.first_terminal_decision_index,
        seed.time_to_first_terminal_micros,
        seed.proposal_expansions_to_first_terminal,
        seed.useful_graph_expansions_to_first_terminal,
        seed.best_authenticated_tick,
    ) {
        (
            true,
            Some(decision_index),
            Some(wall_micros),
            Some(proposal_expansions),
            Some(useful_expansions),
            Some(_),
        ) => {
            let Some((position, decision)) = seed
                .decisions
                .iter()
                .enumerate()
                .find(|(_, row)| row.decision_index == decision_index)
            else {
                return false;
            };
            let expected_proposals = seed.decisions[..=position]
                .iter()
                .try_fold(0_u64, |total, row| total.checked_add(row.proposal_count));
            wall_micros == decision.cumulative_wall_micros
                && expected_proposals == Some(proposal_expansions)
                && proposal_expansions > 0
                && useful_expansions == decision.completed_executable_graph_expansions
                && useful_expansions > 0
                && decision.terminal_proposal_count > 0
        }
        (false, None, None, None, None, None) => true,
        _ => false,
    }
}

fn terminal_improvement_timeline_is_valid(seed: &NativeTacticScratchSeedAudit) -> bool {
    if !seed.terminal_discovered {
        return seed.terminal_improvements.is_empty()
            && seed.best_terminal_decision_index.is_none()
            && seed.time_to_best_terminal_micros.is_none()
            && seed.proposal_expansions_to_best_terminal.is_none()
            && seed.useful_graph_expansions_to_best_terminal.is_none();
    }
    if !seed.terminal_improvement_timing_complete {
        return true;
    }
    let Some(first) = seed.terminal_improvements.first() else {
        return false;
    };
    let Some(last) = seed.terminal_improvements.last() else {
        return false;
    };
    if Some(first.decision_index) != seed.first_terminal_decision_index
        || Some(first.cumulative_wall_micros) != seed.time_to_first_terminal_micros
        || Some(first.cumulative_proposal_expansions) != seed.proposal_expansions_to_first_terminal
        || Some(first.cumulative_useful_graph_expansions)
            != seed.useful_graph_expansions_to_first_terminal
        || Some(last.decision_index) != seed.best_terminal_decision_index
        || Some(last.cumulative_wall_micros) != seed.time_to_best_terminal_micros
        || Some(last.cumulative_proposal_expansions) != seed.proposal_expansions_to_best_terminal
        || Some(last.cumulative_useful_graph_expansions)
            != seed.useful_graph_expansions_to_best_terminal
        || Some(last.authenticated_tick) != seed.best_authenticated_tick
        || seed.terminal_improvements.windows(2).any(|pair| {
            pair[0].authenticated_tick <= pair[1].authenticated_tick
                || pair[0].decision_index > pair[1].decision_index
                || pair[0].cumulative_wall_micros > pair[1].cumulative_wall_micros
                || pair[0].cumulative_proposal_expansions > pair[1].cumulative_proposal_expansions
                || pair[0].cumulative_useful_graph_expansions
                    > pair[1].cumulative_useful_graph_expansions
        })
    {
        return false;
    }
    seed.terminal_improvements.iter().all(|improvement| {
        let Some((position, decision)) = seed
            .decisions
            .iter()
            .enumerate()
            .find(|(_, row)| row.decision_index == improvement.decision_index)
        else {
            return false;
        };
        let expected_proposals = seed.decisions[..=position]
            .iter()
            .try_fold(0_u64, |total, row| total.checked_add(row.proposal_count));
        improvement.cumulative_wall_micros == decision.cumulative_wall_micros
            && expected_proposals == Some(improvement.cumulative_proposal_expansions)
            && improvement.cumulative_useful_graph_expansions
                == decision.completed_executable_graph_expansions
            && improvement.cumulative_proposal_expansions > 0
            && improvement.cumulative_useful_graph_expansions > 0
            && decision.terminal_proposal_count > 0
    })
}

fn proposal_matches_graph_expansion(
    graph: &StateGraph,
    scheduler: &crate::tactic_q_campaign::TacticSchedulerDecisionTrace,
    proposal_index: usize,
    proposal: &NativeTacticProposalTrace,
) -> bool {
    let Some(expansion_sha256) = scheduler
        .evaluated_expansion_sha256
        .get(proposal_index)
        .copied()
    else {
        return false;
    };
    let Some(expansion) = graph.expansion(expansion_sha256) else {
        return false;
    };
    let (
        ActionExpansionStatus::Completed {
            authority: ExpansionEvidenceAuthority::Executable,
            evidence,
            ..
        },
        Some(target),
        Some(execution),
    ) = (
        &expansion.status,
        expansion.target,
        expansion.execution.as_ref(),
    )
    else {
        return false;
    };
    let Some(source_node) = graph.node(expansion.source) else {
        return false;
    };
    let Some(target_node) = graph.node(target) else {
        return false;
    };
    expansion.action.option_id == proposal.option_id
        && execution.option_id == expansion.action.option_id
        && execution.option_type == expansion.action.option_type
        && execution.parameters == expansion.action.parameters
        && execution.duration.realized_ticks == proposal.realized_ticks
        && execution.tape_sha256 == proposal.emitted_tape_sha256
        && execution.realized_tape_range.start_frame == source_node.restoration.route.tape_frames
        && execution.realized_tape_range.end_frame_exclusive
            == target_node.restoration.route.tape_frames
        && source_node
            .restoration
            .route
            .tape_frames
            .checked_add(u64::from(proposal.realized_ticks))
            == Some(proposal.root_route_ticks)
        && target_node.terminal == proposal.terminal
        && target_node.id.state_sha256 == proposal.after_snapshot_sha256
        && target_node.restoration.route.tape_frames == proposal.root_route_ticks
        && evidence.values().any(|row| {
            row.authority == ExpansionEvidenceAuthority::Executable
                && row.transition.after_state_sha256 == proposal.after_snapshot_sha256
                && row.transition.value_sample.action == expansion.action
                && row.transition.value_sample.realized_tape_sha256 == proposal.emitted_tape_sha256
                && row.transition.value_sample.reward.to_bits() == proposal.reward.to_bits()
                && row.transition.value_sample.terminal == proposal.terminal
        })
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

    #[test]
    fn resource_audit_recomputes_every_bound_and_rejects_stale_conclusions() {
        let mut resources = NativeTacticCampaignResourceAudit {
            completed_decisions: 0,
            declared_memory_bound_bytes: Some(1_000),
            configured_checkpoint_cache_capacity_per_worker_bytes: 400,
            configured_checkpoint_pool_capacity_bytes: 800,
            observed_peak_worker_resident_bytes: 300,
            observed_checkpoint_pool_resident_upper_bound_bytes: 600,
            memory_bound_satisfied: true,
            maximum_allowed_stale_replay_revisions: 2,
            maximum_model_replay_lag_revisions: 1,
            maximum_lane_refresh_gap_revisions: 7,
            learner_staleness_bound_satisfied: true,
            direct_restore_fallback_replays: 0,
            prefix_materializations: 0,
            fallback_rate_per_million_decisions: 0,
            fallback_bound_satisfied: true,
            checkpoint_owner_available_decisions: 0,
            checkpoint_owner_local_decisions: 0,
            misrouted_owner_local_decisions: 0,
            checkpoint_owner_counts_by_worker: vec![0, 0],
            checkpoint_owner_assignment_skew: 0,
            passed: true,
        };
        assert!(resource_audit_is_valid(&resources, 2, &[]));

        resources.maximum_model_replay_lag_revisions = 3;
        assert!(!resource_audit_is_valid(&resources, 2, &[]));
        resources.maximum_model_replay_lag_revisions = 1;
        resources.observed_checkpoint_pool_resident_upper_bound_bytes = 599;
        assert!(!resource_audit_is_valid(&resources, 2, &[]));
    }
}
