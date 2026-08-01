use super::scratch_discovery::route_report_sha256;
use super::*;

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

pub const NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V1: &str =
    "dusklight-native-tactic-campaign-summary/v1";
pub const NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V2: &str =
    "dusklight-native-tactic-campaign-summary/v2";
pub const NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V3: &str =
    "dusklight-native-tactic-campaign-summary/v3";
pub const NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V4: &str =
    "dusklight-native-tactic-campaign-summary/v4";
pub const NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V5: &str =
    "dusklight-native-tactic-campaign-summary/v5";
pub const NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V6: &str =
    "dusklight-native-tactic-campaign-summary/v6";
pub const NATIVE_TACTIC_CAMPAIGN_SUMMARY_FILE: &str = "campaign-summary.json";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTacticCausalLink {
    ObservedState,
    LegalActionSurface,
    NativeExploration,
    ExperiencePublication,
    LearnerUpdate,
    PolicyDeployment,
    PolicyEffect,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignIdentities {
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub execution_plan_sha256: Digest,
    pub objective_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub replay_snapshot_sha256: Digest,
    pub learner_snapshot_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignTreatmentSummary {
    pub proposal_policy: TacticProposalPolicy,
    pub value_treatment: TacticValueTreatment,
    pub execution_strategy: NativeGenericExecutionStrategy,
    pub seeds: Vec<u64>,
    pub workers: usize,
    pub proposal_width_per_decision: usize,
    pub decisions_per_seed: u64,
    pub branch_every_decisions: u64,
    pub refit_every_decisions: u64,
    pub root_refresh_cadence: u32,
    pub epsilon_per_million: Vec<u32>,
    pub demonstration_chunk_ticks: Option<u32>,
    pub demonstration_transitions: u64,
    pub resource_budgets: NativeTacticPlanBudgets,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignOutcomeSummary {
    /// Seeds with at least one authenticated terminal among all evaluated
    /// proposals, whether or not the terminating proposal was retained.
    pub terminal_seeds: u64,
    /// Authenticated terminal proposals evaluated across the campaign.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub terminal_proposals: u64,
    /// Decisions whose retained proposal terminated. This is policy adoption,
    /// not by itself causal proof that learning produced the choice.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub selected_terminal_decisions: u64,
    /// Seeds containing at least one selected terminal decision.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub selected_terminal_seeds: u64,
    pub seed_count: u64,
    pub best_authenticated_tick: Option<u64>,
    pub median_time_to_first_terminal_micros: Option<u64>,
    pub worst_time_to_first_terminal_micros: Option<u64>,
    pub total_decisions: u64,
    pub total_proposals: u64,
    pub total_native_ticks: u64,
    pub useful_decisions: u64,
    pub unique_useful_graph_expansions: u64,
    pub stop_reasons: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignCausalSummary {
    pub learning_expected: bool,
    pub decisions_with_observed_state: u64,
    pub decisions_with_complete_action_surface: u64,
    pub decisions_with_native_proposals: u64,
    pub realized_native_proposals: u64,
    pub newly_published_training_rows: u64,
    pub final_training_replay_rows: u64,
    pub learner_updates: u64,
    pub model_snapshots_published: u64,
    pub model_snapshots_consumed: u64,
    pub distinct_model_snapshots_consumed: u64,
    pub post_update_policy_decisions: u64,
    /// Controlled reassessments with state and legal action surface held fixed.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub policy_update_probes: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub valid_policy_update_probes: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub selected_action_changes_from_policy_update: u64,
    /// Legacy adjacent-decision diagnostic. State changes between adjacent
    /// decisions, so this is not causal proof of a policy effect.
    pub selected_action_changes_at_model_change: u64,
    pub causal_chain_ready_for_matched_evaluation: bool,
    pub first_incomplete_link: Option<NativeTacticCausalLink>,
    /// A single treatment can prove that an updated policy was deployed, but
    /// only the matched learned/control comparison can attribute an outcome
    /// change to learning.
    pub outcome_effect_requires_matched_control: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignGoalReachabilitySummary {
    pub calibration_authority_enforced: bool,
    pub calibration_decisions: u64,
    pub deployment_ready_decisions: u64,
    pub deployment_blocked_decisions: u64,
    pub action_evidence_decisions: u64,
    pub action_policy_authorized_decisions: u64,
    pub action_policy_blocked_decisions: u64,
    pub unproven_action_policy_deployments: u64,
    pub frontier_evidence_decisions: u64,
    pub frontier_policy_authorized_decisions: u64,
    pub frontier_policy_blocked_decisions: u64,
    pub unproven_frontier_policy_deployments: u64,
    pub reachability_primary_decisions: u64,
    pub unproven_reachability_primary_decisions: u64,
    pub most_mature_source_transitions: u64,
    pub most_mature_source_state_groups: u64,
    pub most_mature_evaluated_action_predictions: u64,
    pub most_mature_comparable_state_groups: u64,
    pub most_mature_ranking_wins: u64,
    pub most_mature_ranking_win_rate_millionths: Option<u64>,
    pub most_mature_chance_win_rate_millionths: Option<u64>,
    pub most_mature_wilson_lower_bound_millionths: Option<u64>,
    pub most_mature_mean_observed_regret_millionths: Option<u64>,
    pub most_mature_mean_absolute_progress_error_millionths: Option<u64>,
    pub most_mature_progress_sign_accuracy_millionths: Option<u64>,
    pub most_mature_deployment_ready: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignEfficiencySummary {
    pub useful_expansions_per_second_millionths: u64,
    pub native_ticks_per_second_millionths: u64,
    pub learner_updates_per_second_millionths: u64,
    pub native_worker_utilization_per_million: u64,
    pub maximum_model_replay_lag_revisions: u64,
    pub maximum_observed_stale_revisions: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignTimingSummary {
    pub wall_micros: u64,
    pub process_launch_micros: u64,
    pub tactic_selection_micros: u64,
    pub checkpoint_branching_micros: u64,
    pub native_wait_micros: u64,
    pub native_simulation_occupancy_micros: u64,
    pub ipc_and_result_transport_micros: u64,
    pub native_observation_capture_micros: u64,
    pub native_corpus_encoding_micros: u64,
    pub rust_state_extraction_micros: u64,
    pub model_update_micros: u64,
    pub evidence_projection_micros: u64,
    pub persistence_micros: u64,
    pub orchestration_micros: u64,
    pub reporting_micros: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignWorkSummary {
    pub lease_accounting_complete: bool,
    pub proposal_dispatches: u64,
    pub completed_leases: u64,
    pub retryable_leases: u64,
    pub cancelled_leases: u64,
    pub failed_leases: u64,
    pub unresolved_leases: u64,
    pub discarded_proposals: u64,
    pub replay_duplicate_admissions: u64,
    pub duplicate_training_transitions: u64,
    pub censored_training_transitions: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignResourceSummary {
    pub memory_budget_bytes: Option<u64>,
    pub checkpoint_cache_capacity_per_worker_bytes: u64,
    pub peak_worker_resident_bytes: u64,
    pub peak_checkpoint_bytes: u64,
    pub peak_host_snapshot_bytes: u64,
    pub peak_live_endpoint_entries: u64,
    pub peak_live_endpoint_host_snapshot_bytes: u64,
    pub memory_bound_satisfied: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticCampaignSummary {
    pub schema: String,
    pub content_sha256: Digest,
    pub route_report_sha256: Digest,
    pub identities: NativeTacticCampaignIdentities,
    pub treatment: NativeTacticCampaignTreatmentSummary,
    pub outcome: NativeTacticCampaignOutcomeSummary,
    pub causal_chain: NativeTacticCampaignCausalSummary,
    pub goal_reachability: NativeTacticCampaignGoalReachabilitySummary,
    pub efficiency: NativeTacticCampaignEfficiencySummary,
    pub timing: NativeTacticCampaignTimingSummary,
    pub work: NativeTacticCampaignWorkSummary,
    pub resources: NativeTacticCampaignResourceSummary,
}

impl NativeTacticCampaignSummary {
    pub fn build(
        route: &NativeTacticRouteReport,
        plan: &NativeTacticExecutionPlan,
    ) -> Result<Self, NativeTacticRouteRunError> {
        if plan.identity()? != route.execution_plan_sha256
            || plan.seeds != route.exploration_seeds
            || plan.proposal_policy != route.proposal_policy
            || plan.value_treatment != route.value_treatment
            || plan.execution_strategy != route.execution_strategy
            || plan.budgets != route.resource_budgets
        {
            return Err(route_message(
                "campaign summary execution plan is detached from its route report",
            ));
        }

        let mut epsilon_per_million = plan
            .lanes
            .iter()
            .map(|lane| lane.epsilon_per_million)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        epsilon_per_million.sort_unstable();

        let total_proposals = route
            .seeds
            .iter()
            .flat_map(|seed| &seed.trace)
            .map(|decision| decision.proposal_batch.len() as u64)
            .sum();
        let (terminal_proposals, selected_terminal_decisions, selected_terminal_seeds) =
            if route.schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V44 {
                terminal_outcome_counts(route)
            } else {
                (0, 0, 0)
            };
        let mut stop_reasons = BTreeMap::new();
        for reason in route
            .seeds
            .iter()
            .flat_map(|seed| seed.stop_reasons.iter().copied())
        {
            let name = match reason {
                NativeTacticSeedStopReason::DecisionBudgetReached => "decision_budget_reached",
                NativeTacticSeedStopReason::SimulatedTickBudgetReached => {
                    "simulated_tick_budget_reached"
                }
                NativeTacticSeedStopReason::NativeTickBudgetReached => "native_tick_budget_reached",
                NativeTacticSeedStopReason::WallBudgetReached => "wall_budget_reached",
            };
            *stop_reasons.entry(name.into()).or_default() += 1;
        }

        let causal_chain = causal_summary(route);
        let goal_reachability = goal_reachability_summary(route);
        let work = work_summary(route, total_proposals);
        let worker_capacity_micros = route
            .timing
            .tactic_execution_micros
            .saturating_mul(route.workers as u64);
        let native_worker_utilization_per_million = ratio_per_million(
            route.timing.native_simulation_micros,
            worker_capacity_micros,
        )
        .min(1_000_000);
        let memory_budget_bytes = match route.resource_budgets.memory_bytes {
            NativeTacticResourceLimit::Bounded(value) => Some(value),
            NativeTacticResourceLimit::Unbounded => None,
        };
        let peak_worker_resident_bytes = route.native_restore_accounting.peak_resident_bytes;

        let mut summary = Self {
            schema: if route.schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V44 {
                NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V6.into()
            } else {
                NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V4.into()
            },
            content_sha256: Digest::ZERO,
            route_report_sha256: route_report_sha256(route)?,
            identities: NativeTacticCampaignIdentities {
                optimization_request_sha256: route.optimization_request_sha256,
                execution_binding_sha256: route.execution_binding_sha256,
                execution_plan_sha256: route.execution_plan_sha256,
                objective_sha256: route.objective_sha256,
                feature_schema_sha256: route.feature_schema_sha256,
                action_schema_sha256: route.action_schema_sha256,
                replay_snapshot_sha256: route.replay_snapshot_sha256,
                learner_snapshot_sha256: route.learner_authority.latest_model_snapshot_sha256,
            },
            treatment: NativeTacticCampaignTreatmentSummary {
                proposal_policy: route.proposal_policy,
                value_treatment: route.value_treatment,
                execution_strategy: route.execution_strategy,
                seeds: route.exploration_seeds.clone(),
                workers: route.workers,
                proposal_width_per_decision: plan.proposal_width_per_decision,
                decisions_per_seed: route.decisions_per_seed,
                branch_every_decisions: plan.branch_every_decisions,
                refit_every_decisions: route.refit_every_decisions,
                root_refresh_cadence: plan.root_refresh_cadence,
                epsilon_per_million,
                demonstration_chunk_ticks: plan.demonstration_chunk_ticks,
                demonstration_transitions: route.demonstration_transitions,
                resource_budgets: route.resource_budgets,
            },
            outcome: NativeTacticCampaignOutcomeSummary {
                terminal_seeds: route.terminal_seeds,
                terminal_proposals,
                selected_terminal_decisions,
                selected_terminal_seeds,
                seed_count: route.seeds.len() as u64,
                best_authenticated_tick: route.best_authenticated_tick,
                median_time_to_first_terminal_micros: route.median_time_to_first_terminal_micros,
                worst_time_to_first_terminal_micros: route.worst_time_to_first_terminal_micros,
                total_decisions: route.total_decisions,
                total_proposals,
                total_native_ticks: route.total_native_ticks,
                useful_decisions: route.useful_decisions,
                unique_useful_graph_expansions: route.unique_useful_graph_expansions,
                stop_reasons,
            },
            causal_chain,
            goal_reachability,
            efficiency: NativeTacticCampaignEfficiencySummary {
                useful_expansions_per_second_millionths: route
                    .timing
                    .unique_useful_graph_expansions_per_second_millionths,
                native_ticks_per_second_millionths: route.timing.native_ticks_per_second_millionths,
                learner_updates_per_second_millionths: route.learner_updates_per_second_millionths,
                native_worker_utilization_per_million,
                maximum_model_replay_lag_revisions: route
                    .replay_sharing
                    .maximum_model_replay_lag_revisions,
                maximum_observed_stale_revisions: route
                    .replay_sharing
                    .maximum_observed_stale_revisions,
            },
            timing: NativeTacticCampaignTimingSummary {
                wall_micros: route.timing.wall_micros,
                process_launch_micros: route.timing.process_launch_micros,
                tactic_selection_micros: route.timing.tactic_selection_micros,
                checkpoint_branching_micros: route.timing.checkpoint_branching_micros,
                native_wait_micros: route.timing.tactic_execution_micros,
                native_simulation_occupancy_micros: route.timing.native_simulation_micros,
                ipc_and_result_transport_micros: route.timing.ipc_and_result_transport_micros,
                native_observation_capture_micros: route.timing.native_observation_capture_micros,
                native_corpus_encoding_micros: route.timing.native_corpus_encoding_micros,
                rust_state_extraction_micros: route.timing.rust_state_extraction_micros,
                model_update_micros: route.timing.model_update_micros,
                evidence_projection_micros: route.timing.evidence_projection_micros,
                persistence_micros: route.timing.persistence_micros,
                orchestration_micros: route.timing.orchestration_micros,
                reporting_micros: route.timing.reporting_micros,
            },
            work,
            resources: NativeTacticCampaignResourceSummary {
                memory_budget_bytes,
                checkpoint_cache_capacity_per_worker_bytes: route
                    .checkpoint_cache_capacity_per_worker_bytes,
                peak_worker_resident_bytes,
                peak_checkpoint_bytes: route
                    .native_restore_accounting
                    .peak_resident_checkpoint_bytes,
                peak_host_snapshot_bytes: route
                    .native_restore_accounting
                    .peak_resident_host_snapshot_bytes,
                peak_live_endpoint_entries: route
                    .native_restore_accounting
                    .peak_live_endpoint_entries,
                peak_live_endpoint_host_snapshot_bytes: route
                    .native_restore_accounting
                    .peak_live_endpoint_host_snapshot_bytes,
                memory_bound_satisfied: memory_budget_bytes
                    .is_none_or(|bound| peak_worker_resident_bytes <= bound),
            },
        };
        summary.content_sha256 = summary.compute_content_sha256()?;
        summary.validate()?;
        Ok(summary)
    }

    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        if !matches!(
            self.schema.as_str(),
            NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V4
                | NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V5
                | NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V6
        ) || self.content_sha256 == Digest::ZERO
            || self.route_report_sha256 == Digest::ZERO
            || self.identities.optimization_request_sha256 == Digest::ZERO
            || self.identities.execution_binding_sha256 == Digest::ZERO
            || self.identities.execution_plan_sha256 == Digest::ZERO
            || self.identities.objective_sha256 == Digest::ZERO
            || self.identities.feature_schema_sha256 == Digest::ZERO
            || self.identities.action_schema_sha256 == Digest::ZERO
            || self.identities.replay_snapshot_sha256 == Digest::ZERO
            || self.identities.learner_snapshot_sha256 == Digest::ZERO
            || self.treatment.seeds.is_empty()
            || self.treatment.workers == 0
            || self.treatment.proposal_width_per_decision == 0
            || self.treatment.decisions_per_seed == 0
            || self.outcome.seed_count != self.treatment.seeds.len() as u64
            || self.outcome.selected_terminal_seeds > self.outcome.terminal_seeds
            || self.outcome.selected_terminal_decisions > self.outcome.terminal_proposals
            || self.outcome.selected_terminal_seeds > self.outcome.selected_terminal_decisions
            || self.outcome.selected_terminal_decisions > self.outcome.total_decisions
            || (self.schema != NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V6
                && (self.outcome.terminal_proposals != 0
                    || self.outcome.selected_terminal_decisions != 0
                    || self.outcome.selected_terminal_seeds != 0))
            || (self.schema == NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V6
                && (self.outcome.terminal_seeds > self.outcome.terminal_proposals
                    || (self.outcome.terminal_seeds == 0)
                        != (self.outcome.terminal_proposals == 0)))
            || self.outcome.total_decisions < self.outcome.useful_decisions
            || self.efficiency.native_worker_utilization_per_million > 1_000_000
            || self.work.proposal_dispatches
                != self
                    .work
                    .completed_leases
                    .saturating_add(self.work.retryable_leases)
                    .saturating_add(self.work.cancelled_leases)
                    .saturating_add(self.work.failed_leases)
                    .saturating_add(self.work.unresolved_leases)
            || self.causal_chain.causal_chain_ready_for_matched_evaluation
                == self.causal_chain.first_incomplete_link.is_some()
            || self.causal_chain.valid_policy_update_probes > self.causal_chain.policy_update_probes
            || self.causal_chain.selected_action_changes_from_policy_update
                > self.causal_chain.valid_policy_update_probes
            || (!self.causal_chain.learning_expected && self.causal_chain.policy_update_probes != 0)
            || (matches!(
                self.schema.as_str(),
                NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V5 | NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V6
            ) && self.causal_chain.learning_expected
                && self.causal_chain.causal_chain_ready_for_matched_evaluation
                && (self.causal_chain.policy_update_probes == 0
                    || self.causal_chain.valid_policy_update_probes
                        != self.causal_chain.policy_update_probes
                    || self.causal_chain.selected_action_changes_from_policy_update == 0))
            || self.goal_reachability.calibration_decisions
                != self
                    .goal_reachability
                    .deployment_ready_decisions
                    .saturating_add(self.goal_reachability.deployment_blocked_decisions)
            || self.goal_reachability.action_evidence_decisions
                != self
                    .goal_reachability
                    .action_policy_authorized_decisions
                    .saturating_add(self.goal_reachability.action_policy_blocked_decisions)
            || self.goal_reachability.frontier_evidence_decisions
                != self
                    .goal_reachability
                    .frontier_policy_authorized_decisions
                    .saturating_add(self.goal_reachability.frontier_policy_blocked_decisions)
            || (self.goal_reachability.calibration_authority_enforced
                && (self
                    .goal_reachability
                    .unproven_reachability_primary_decisions
                    != 0
                    || self.goal_reachability.unproven_action_policy_deployments != 0
                    || self.goal_reachability.unproven_frontier_policy_deployments != 0))
            || self.compute_content_sha256()? != self.content_sha256
        {
            return Err(route_message("native tactic campaign summary is invalid"));
        }
        Ok(())
    }

    pub fn validate_against(
        &self,
        route: &NativeTacticRouteReport,
        plan: &NativeTacticExecutionPlan,
    ) -> Result<(), NativeTacticRouteRunError> {
        self.validate()?;
        if self != &Self::build(route, plan)? {
            return Err(route_message(
                "native tactic campaign summary is detached from its report or plan",
            ));
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
        let mut identity = self.clone();
        identity.content_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&identity).map_err(route_error)?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight-native-tactic-campaign-summary/v4\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn terminal_outcome_counts(route: &NativeTacticRouteReport) -> (u64, u64, u64) {
    let terminal_proposals = route
        .seeds
        .iter()
        .flat_map(|seed| &seed.trace)
        .flat_map(|decision| &decision.proposal_batch)
        .filter(|proposal| proposal.terminal)
        .count() as u64;
    let selected_terminal_decisions = route
        .seeds
        .iter()
        .flat_map(|seed| &seed.trace)
        .filter(|decision| decision.terminal)
        .count() as u64;
    let selected_terminal_seeds = route
        .seeds
        .iter()
        .filter(|seed| seed.trace.iter().any(|decision| decision.terminal))
        .count() as u64;
    (
        terminal_proposals,
        selected_terminal_decisions,
        selected_terminal_seeds,
    )
}

fn goal_reachability_summary(
    route: &NativeTacticRouteReport,
) -> NativeTacticCampaignGoalReachabilitySummary {
    let traces = route.seeds.iter().flat_map(|seed| seed.trace.iter());
    let mut calibration_decisions = 0_u64;
    let mut deployment_ready_decisions = 0_u64;
    let mut deployment_blocked_decisions = 0_u64;
    let mut action_evidence_decisions = 0_u64;
    let mut action_policy_authorized_decisions = 0_u64;
    let mut action_policy_blocked_decisions = 0_u64;
    let mut unproven_action_policy_deployments = 0_u64;
    let mut frontier_evidence_decisions = 0_u64;
    let mut frontier_policy_authorized_decisions = 0_u64;
    let mut frontier_policy_blocked_decisions = 0_u64;
    let mut unproven_frontier_policy_deployments = 0_u64;
    let mut reachability_primary_decisions = 0_u64;
    let mut unproven_reachability_primary_decisions = 0_u64;
    let mut most_mature: Option<&GoalReachabilityCalibration> = None;

    for decision in traces {
        let deployment_ready = decision
            .goal_reachability_calibration
            .as_ref()
            .is_some_and(|calibration| calibration.deployment_ready);
        if let Some(calibration) = &decision.goal_reachability_calibration {
            calibration_decisions = calibration_decisions.saturating_add(1);
            if calibration.deployment_ready {
                deployment_ready_decisions = deployment_ready_decisions.saturating_add(1);
            } else {
                deployment_blocked_decisions = deployment_blocked_decisions.saturating_add(1);
            }
            if most_mature.is_none_or(|current| {
                (
                    calibration.source_transitions,
                    calibration.source_state_groups,
                    calibration.comparable_state_groups,
                    calibration.evaluated_action_predictions,
                ) > (
                    current.source_transitions,
                    current.source_state_groups,
                    current.comparable_state_groups,
                    current.evaluated_action_predictions,
                )
            }) {
                most_mature = Some(calibration);
            }
        }
        let action_evidence_available = decision
            .proposal_batch
            .iter()
            .any(|proposal| proposal.predicted_goal_progress_per_tick.is_some());
        if action_evidence_available {
            action_evidence_decisions = action_evidence_decisions.saturating_add(1);
            if deployment_ready {
                action_policy_authorized_decisions =
                    action_policy_authorized_decisions.saturating_add(1);
            } else {
                action_policy_blocked_decisions = action_policy_blocked_decisions.saturating_add(1);
            }
        }
        let reachability_action_deployed = decision.selection_reason
            == TacticSelectionReason::GoalReachability
            || decision.proposal_batch.iter().any(|proposal| {
                proposal.selection_reason == TacticSelectionReason::GoalReachability
            });
        if reachability_action_deployed && (!deployment_ready || !action_evidence_available) {
            unproven_action_policy_deployments =
                unproven_action_policy_deployments.saturating_add(1);
        }
        if let Some(acquisition) = &decision.branch_acquisition {
            if acquisition.goal_reachability_evidence_available {
                frontier_evidence_decisions = frontier_evidence_decisions.saturating_add(1);
                if acquisition.goal_reachability_supported {
                    frontier_policy_authorized_decisions =
                        frontier_policy_authorized_decisions.saturating_add(1);
                } else {
                    frontier_policy_blocked_decisions =
                        frontier_policy_blocked_decisions.saturating_add(1);
                }
            }
            if acquisition.goal_reachability_supported
                && (!deployment_ready || !acquisition.goal_reachability_evidence_available)
            {
                unproven_frontier_policy_deployments =
                    unproven_frontier_policy_deployments.saturating_add(1);
            }
        }
        if decision.selection_reason == TacticSelectionReason::GoalReachability {
            reachability_primary_decisions = reachability_primary_decisions.saturating_add(1);
            if decision
                .goal_reachability_calibration
                .as_ref()
                .is_none_or(|calibration| !calibration.deployment_ready)
            {
                unproven_reachability_primary_decisions =
                    unproven_reachability_primary_decisions.saturating_add(1);
            }
        }
    }

    NativeTacticCampaignGoalReachabilitySummary {
        calibration_authority_enforced: supports_current_route_report_schema(&route.schema),
        calibration_decisions,
        deployment_ready_decisions,
        deployment_blocked_decisions,
        action_evidence_decisions,
        action_policy_authorized_decisions,
        action_policy_blocked_decisions,
        unproven_action_policy_deployments,
        frontier_evidence_decisions,
        frontier_policy_authorized_decisions,
        frontier_policy_blocked_decisions,
        unproven_frontier_policy_deployments,
        reachability_primary_decisions,
        unproven_reachability_primary_decisions,
        most_mature_source_transitions: most_mature
            .map_or(0, |value| value.source_transitions as u64),
        most_mature_source_state_groups: most_mature
            .map_or(0, |value| value.source_state_groups as u64),
        most_mature_evaluated_action_predictions: most_mature
            .map_or(0, |value| value.evaluated_action_predictions as u64),
        most_mature_comparable_state_groups: most_mature
            .map_or(0, |value| value.comparable_state_groups as u64),
        most_mature_ranking_wins: most_mature.map_or(0, |value| value.ranking_wins as u64),
        most_mature_ranking_win_rate_millionths: most_mature
            .and_then(|value| millionths(value.ranking_win_rate)),
        most_mature_chance_win_rate_millionths: most_mature
            .and_then(|value| millionths(value.chance_win_rate)),
        most_mature_wilson_lower_bound_millionths: most_mature
            .and_then(|value| millionths(value.ranking_win_rate_wilson_lower_bound)),
        most_mature_mean_observed_regret_millionths: most_mature
            .and_then(|value| millionths(value.mean_observed_regret)),
        most_mature_mean_absolute_progress_error_millionths: most_mature
            .and_then(|value| millionths(value.mean_absolute_progress_error)),
        most_mature_progress_sign_accuracy_millionths: most_mature
            .and_then(|value| millionths(value.progress_sign_accuracy)),
        most_mature_deployment_ready: most_mature.is_some_and(|value| value.deployment_ready),
    }
}

fn millionths(value: Option<f64>) -> Option<u64> {
    value.map(|value| (value * 1_000_000.0).round() as u64)
}

fn causal_summary(route: &NativeTacticRouteReport) -> NativeTacticCampaignCausalSummary {
    let traces = route
        .seeds
        .iter()
        .flat_map(|seed| seed.trace.iter())
        .collect::<Vec<_>>();
    let decisions_with_observed_state = traces
        .iter()
        .filter(|decision| {
            decision.before.snapshot_sha256 != Digest::ZERO
                && decision.after.snapshot_sha256 != Digest::ZERO
        })
        .count() as u64;
    let decisions_with_complete_action_surface = traces
        .iter()
        .filter(|decision| {
            !decision.applicable_tactics.is_empty()
                && decision
                    .applicable_tactics
                    .iter()
                    .all(|action| action.applicable && action.descriptor.is_some())
                && decision
                    .applicable_tactics
                    .iter()
                    .filter(|action| action.selected)
                    .count()
                    == 1
                && decision.applicable_tactics.iter().any(|action| {
                    action.selected && action.option_id == decision.selected_option_id
                })
        })
        .count() as u64;
    let decisions_with_native_proposals = traces
        .iter()
        .filter(|decision| !decision.proposal_batch.is_empty())
        .count() as u64;
    let realized_native_proposals = traces
        .iter()
        .map(|decision| decision.proposal_batch.len() as u64)
        .sum();
    let newly_published_training_rows = traces
        .iter()
        .map(|decision| decision.newly_admitted_training_rows)
        .sum();
    let consumed = traces
        .iter()
        .map(|decision| decision.learner_snapshot_sha256)
        .filter(|sha256| *sha256 != Digest::ZERO)
        .collect::<BTreeSet<_>>();
    let mut post_update_policy_decisions = 0_u64;
    let mut selected_action_changes_at_model_change = 0_u64;
    let policy_update_probes = traces
        .iter()
        .flat_map(|decision| &decision.policy_update_probes)
        .collect::<Vec<_>>();
    let valid_policy_update_probes = traces
        .iter()
        .filter(|decision| policy_update_probe_chain_valid(decision))
        .map(|decision| decision.policy_update_probes.len() as u64)
        .sum();
    let selected_action_changes_from_policy_update = traces
        .iter()
        .filter(|decision| policy_update_probe_chain_valid(decision))
        .flat_map(|decision| &decision.policy_update_probes)
        .filter(|probe| probe.selected_action_changed)
        .count() as u64;
    for seed in &route.seeds {
        let Some(first) = seed.trace.first() else {
            continue;
        };
        for pair in seed.trace.windows(2) {
            if pair[1].learner_snapshot_sha256 != pair[0].learner_snapshot_sha256 {
                post_update_policy_decisions = post_update_policy_decisions.saturating_add(1);
                if pair[1].selected_option_id != pair[0].selected_option_id {
                    selected_action_changes_at_model_change =
                        selected_action_changes_at_model_change.saturating_add(1);
                }
            } else if pair[1].learner_snapshot_sha256 != first.learner_snapshot_sha256 {
                post_update_policy_decisions = post_update_policy_decisions.saturating_add(1);
            }
        }
    }
    let learning_expected = route.proposal_policy.deploys_policy_updates();
    let decision_count = traces.len() as u64;
    let first_incomplete_link = if decision_count == 0
        || decisions_with_observed_state != decision_count
    {
        Some(NativeTacticCausalLink::ObservedState)
    } else if decisions_with_complete_action_surface != decision_count {
        Some(NativeTacticCausalLink::LegalActionSurface)
    } else if decisions_with_native_proposals != decision_count || realized_native_proposals == 0 {
        Some(NativeTacticCausalLink::NativeExploration)
    } else if learning_expected && newly_published_training_rows == 0 {
        Some(NativeTacticCausalLink::ExperiencePublication)
    } else if learning_expected && route.learner_updates == 0 {
        Some(NativeTacticCausalLink::LearnerUpdate)
    } else if learning_expected
        && (route.learner_authority.declared_model_snapshots_consumed == 0
            || consumed.len() < 2
            || post_update_policy_decisions == 0)
    {
        Some(NativeTacticCausalLink::PolicyDeployment)
    } else if learning_expected
        && route.schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V44
        && (policy_update_probes.is_empty()
            || valid_policy_update_probes != policy_update_probes.len() as u64
            || selected_action_changes_from_policy_update == 0)
    {
        Some(NativeTacticCausalLink::PolicyEffect)
    } else {
        None
    };

    NativeTacticCampaignCausalSummary {
        learning_expected,
        decisions_with_observed_state,
        decisions_with_complete_action_surface,
        decisions_with_native_proposals,
        realized_native_proposals,
        newly_published_training_rows,
        final_training_replay_rows: route.learner_authority.latest_training_replay_rows,
        learner_updates: route.learner_updates,
        model_snapshots_published: route.learner_authority.model_snapshots_published,
        model_snapshots_consumed: route.learner_authority.declared_model_snapshots_consumed,
        distinct_model_snapshots_consumed: consumed.len() as u64,
        post_update_policy_decisions,
        policy_update_probes: policy_update_probes.len() as u64,
        valid_policy_update_probes,
        selected_action_changes_from_policy_update,
        selected_action_changes_at_model_change,
        causal_chain_ready_for_matched_evaluation: first_incomplete_link.is_none(),
        first_incomplete_link,
        outcome_effect_requires_matched_control: learning_expected,
    }
}

fn policy_update_probe_chain_valid(decision: &NativeTacticDecisionTrace) -> bool {
    decision
        .policy_update_probes
        .iter()
        .all(|probe| probe.validate().is_ok())
        && decision.policy_update_probes.last().is_none_or(|probe| {
            probe.after_learner_snapshot_sha256 == decision.learner_snapshot_sha256
        })
        && decision.policy_update_probes.windows(2).all(|pair| {
            pair[0].after_learner_snapshot_sha256 == pair[1].before_learner_snapshot_sha256
                && pair[0].after_replay_rows == pair[1].before_replay_rows
                && pair[0].after_model_revision == pair[1].before_model_revision
        })
}

fn work_summary(
    route: &NativeTacticRouteReport,
    total_proposals: u64,
) -> NativeTacticCampaignWorkSummary {
    let lease_accounting_complete = route.seeds.iter().all(|seed| seed.graph_metrics.is_some());
    let (
        proposal_dispatches,
        completed_leases,
        retryable_leases,
        cancelled_leases,
        failed_leases,
        unresolved_leases,
    ) = if lease_accounting_complete {
        route
            .seeds
            .iter()
            .filter_map(|seed| seed.graph_metrics.as_ref())
            .fold(
                (0_u64, 0_u64, 0_u64, 0_u64, 0_u64, 0_u64),
                |totals, metrics| {
                    let leases = &metrics.lease_accounting;
                    (
                        totals.0.saturating_add(leases.proposal_dispatches),
                        totals.1.saturating_add(leases.completed_leases),
                        totals.2.saturating_add(leases.retryable_leases),
                        totals.3.saturating_add(leases.cancelled_leases),
                        totals.4.saturating_add(leases.failed_leases),
                        totals.5.saturating_add(leases.unresolved_leases),
                    )
                },
            )
    } else {
        (total_proposals, total_proposals, 0, 0, 0, 0)
    };
    NativeTacticCampaignWorkSummary {
        lease_accounting_complete,
        proposal_dispatches,
        completed_leases,
        retryable_leases,
        cancelled_leases,
        failed_leases,
        unresolved_leases,
        discarded_proposals: route
            .seeds
            .iter()
            .flat_map(|seed| &seed.trace)
            .flat_map(|decision| &decision.proposal_batch)
            .filter(|proposal| !proposal.retained)
            .count() as u64,
        replay_duplicate_admissions: route.replay_admission.duplicates,
        duplicate_training_transitions: route.duplicate_training_transitions,
        censored_training_transitions: route.censored_training_transitions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retained_report_and_plan() -> (Vec<u8>, NativeTacticRouteReport, NativeTacticExecutionPlan) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let evidence = root.join(
            "benchmarks/native-tactic-fault-recovery/win32-x86_64/\
             after-recovery-point-commit-portable-boundary-v1/blobs/sha256",
        );
        let compressed = fs::read(
            evidence.join("c9/80f212588942ea67ed0b48bb019913298233642ea884a4fc56a4fb1de7e3e9"),
        )
        .unwrap();
        let raw = zstd::stream::decode_all(compressed.as_slice()).unwrap();
        let route = serde_json::from_slice(&raw).unwrap();
        let plan = NativeTacticExecutionPlan::read(
            &evidence.join("86/ad166401846662dfbc6604acbeae522e09420e3369e8f7d087cde63cd8f742"),
        )
        .unwrap();
        (raw, route, plan)
    }

    #[test]
    fn compact_summary_projects_the_complete_learning_chain_and_authorities() {
        let (raw, route, plan) = retained_report_and_plan();
        let summary = NativeTacticCampaignSummary::build(&route, &plan).unwrap();
        summary.validate_against(&route, &plan).unwrap();
        let encoded = summary.to_pretty_json().unwrap();
        let reparsed_route: NativeTacticRouteReport =
            serde_json::from_slice(&serde_json::to_vec_pretty(&route).unwrap()).unwrap();
        let reparsed_summary: NativeTacticCampaignSummary =
            serde_json::from_slice(&encoded).unwrap();
        reparsed_summary
            .validate_against(&reparsed_route, &plan)
            .unwrap();

        assert!(
            summary
                .causal_chain
                .causal_chain_ready_for_matched_evaluation
        );
        assert_eq!(summary.causal_chain.first_incomplete_link, None);
        assert_eq!(summary.causal_chain.decisions_with_observed_state, 2);
        assert_eq!(
            summary.causal_chain.decisions_with_complete_action_surface,
            2
        );
        assert_eq!(summary.causal_chain.realized_native_proposals, 4);
        assert_eq!(summary.causal_chain.newly_published_training_rows, 4);
        assert_eq!(summary.causal_chain.learner_updates, 2);
        assert_eq!(summary.causal_chain.post_update_policy_decisions, 1);
        assert_eq!(summary.goal_reachability.calibration_decisions, 0);
        assert!(!summary.goal_reachability.calibration_authority_enforced);
        assert_eq!(summary.work.proposal_dispatches, 4);
        assert_eq!(summary.work.unresolved_leases, 0);
        assert!(summary.resources.memory_bound_satisfied);
        assert!(encoded.len() < 16 * 1024);
        assert!(raw.len() > encoded.len() * 40);
    }

    #[test]
    fn v44_summary_requires_a_same_state_policy_effect() {
        let (_, mut route, _) = retained_report_and_plan();
        route.schema = NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V44.into();
        let snapshots = route
            .seeds
            .iter()
            .flat_map(|seed| &seed.trace)
            .map(|decision| decision.learner_snapshot_sha256)
            .collect::<Vec<_>>();
        let decision = route
            .seeds
            .iter_mut()
            .flat_map(|seed| &mut seed.trace)
            .nth(1)
            .unwrap();
        decision.policy_update_probes = vec![NativeTacticPolicyUpdateProbe {
            state_sha256: Digest([21; 32]),
            before_action_surface_sha256: Digest([22; 32]),
            after_action_surface_sha256: Digest([22; 32]),
            before_learner_snapshot_sha256: snapshots[0],
            after_learner_snapshot_sha256: decision.learner_snapshot_sha256,
            before_replay_rows: 1,
            after_replay_rows: 2,
            before_model_revision: 1,
            after_model_revision: 2,
            before_selected_option_id: "before".into(),
            after_selected_option_id: "after".into(),
            before_selection_reason: TacticSelectionReason::Greedy,
            after_selection_reason: TacticSelectionReason::Greedy,
            selected_action_changed: true,
        }];

        let summary = causal_summary(&route);
        assert!(summary.causal_chain_ready_for_matched_evaluation);
        assert_eq!(summary.policy_update_probes, 1);
        assert_eq!(summary.valid_policy_update_probes, 1);
        assert_eq!(summary.selected_action_changes_from_policy_update, 1);

        route
            .seeds
            .iter_mut()
            .flat_map(|seed| &mut seed.trace)
            .for_each(|decision| decision.policy_update_probes.clear());
        let incomplete = causal_summary(&route);
        assert!(!incomplete.causal_chain_ready_for_matched_evaluation);
        assert_eq!(
            incomplete.first_incomplete_link,
            Some(NativeTacticCausalLink::PolicyEffect)
        );
    }

    #[test]
    fn terminal_outcomes_separate_discovery_from_policy_adoption() {
        let (_, mut route, _) = retained_report_and_plan();
        route.seeds[0].trace[0]
            .proposal_batch
            .iter_mut()
            .find(|proposal| !proposal.retained)
            .unwrap()
            .terminal = true;
        assert_eq!(terminal_outcome_counts(&route), (1, 0, 0));

        route.seeds[0].trace[0].terminal = true;
        route.seeds[0].trace[0]
            .proposal_batch
            .iter_mut()
            .find(|proposal| proposal.retained)
            .unwrap()
            .terminal = true;
        assert_eq!(terminal_outcome_counts(&route), (2, 1, 1));
    }

    #[test]
    fn summary_rejects_tampering_and_a_detached_plan() {
        let (_, route, plan) = retained_report_and_plan();
        let original = NativeTacticCampaignSummary::build(&route, &plan).unwrap();

        let mut tampered = original.clone();
        tampered.work.unresolved_leases = 1;
        assert!(tampered.validate().is_err());

        tampered = original;
        tampered.identities.objective_sha256.0[0] ^= 1;
        assert!(tampered.validate().is_err());

        let mut detached = plan;
        detached.proposal_width_per_decision += 1;
        assert!(NativeTacticCampaignSummary::build(&route, &detached).is_err());
    }

    #[test]
    fn summary_rejects_reachability_policy_without_calibration_authority() {
        let (_, mut route, plan) = retained_report_and_plan();
        route.schema = NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V39.into();
        for decision in route.seeds.iter_mut().flat_map(|seed| &mut seed.trace) {
            decision.selection_reason = TacticSelectionReason::UnsupportedBootstrap;
            for proposal in &mut decision.proposal_batch {
                proposal.selection_reason = TacticSelectionReason::UnsupportedBootstrap;
                proposal.predicted_goal_progress_per_tick = None;
            }
        }
        route.seeds[0].trace[0].goal_reachability_calibration = Some(
            dusklight_learning::goal_reachability_calibration::calibrate_goal_reachability(&[], 0)
                .unwrap(),
        );
        route.seeds[0].trace[0].proposal_batch[0].predicted_goal_progress_per_tick = Some(1.0);
        route.seeds[0].trace[0].branch_acquisition = Some(TacticFrontierAcquisition {
            expansion_count: 0,
            terminal: false,
            terminal_value_supported: false,
            achieved_goal_value_supported: false,
            goal_reachability_supported: false,
            goal_reachability_evidence_available: true,
            reward: 0.0,
            best_mean_q: None,
            best_goal_progress_per_tick: Some(1.0),
            predicted_terminal_ticks_to_go: None,
            predicted_total_terminal_ticks: None,
            exact_terminal_ticks_to_go: None,
            exact_total_terminal_ticks: None,
            maximum_ensemble_variance: None,
            generalized_nearest_distance: None,
            discovery_spatial_novelty: None,
            novelty_rank: 0,
            replayed_prefix_ticks: 0,
        });
        let acquisition = route.seeds[0].trace[0].branch_acquisition.as_mut().unwrap();
        acquisition.goal_reachability_evidence_available = true;
        acquisition.goal_reachability_supported = false;
        let blocked = NativeTacticCampaignSummary::build(&route, &plan).unwrap();
        assert_eq!(blocked.goal_reachability.calibration_decisions, 1);
        assert_eq!(blocked.goal_reachability.deployment_blocked_decisions, 1);
        assert_eq!(blocked.goal_reachability.action_evidence_decisions, 1);
        assert_eq!(blocked.goal_reachability.action_policy_blocked_decisions, 1);
        assert_eq!(blocked.goal_reachability.frontier_evidence_decisions, 1);
        assert_eq!(
            blocked.goal_reachability.frontier_policy_blocked_decisions,
            1
        );
        assert!(!blocked.goal_reachability.most_mature_deployment_ready);

        route.seeds[0].trace[0].proposal_batch[0].selection_reason =
            TacticSelectionReason::GoalReachability;
        assert!(NativeTacticCampaignSummary::build(&route, &plan).is_err());
        route.seeds[0].trace[0].proposal_batch[0].selection_reason =
            TacticSelectionReason::UnsupportedBootstrap;
        route.seeds[0].trace[0]
            .branch_acquisition
            .as_mut()
            .unwrap()
            .goal_reachability_supported = true;
        assert!(NativeTacticCampaignSummary::build(&route, &plan).is_err());
    }
}
