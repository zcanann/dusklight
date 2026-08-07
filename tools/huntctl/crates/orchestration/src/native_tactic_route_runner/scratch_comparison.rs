use super::scratch_discovery::route_report_sha256;
use super::*;

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

pub const NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V2: &str =
    "dusklight-native-tactic-scratch-comparison/v2";
pub const NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V3: &str =
    "dusklight-native-tactic-scratch-comparison/v3";
pub const NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V4: &str =
    "dusklight-native-tactic-scratch-comparison/v4";
pub const NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V5: &str =
    "dusklight-native-tactic-scratch-comparison/v5";
pub const NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V6: &str =
    "dusklight-native-tactic-scratch-comparison/v6";
pub const NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V7: &str =
    "dusklight-native-tactic-scratch-comparison/v7";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTacticScratchTreatment {
    LearnedRanking,
    FrozenPolicy,
    RandomValidRanking,
}

impl NativeTacticScratchTreatment {
    fn from_policy(policy: TacticProposalPolicy) -> Result<Self, NativeTacticRouteRunError> {
        match policy {
            TacticProposalPolicy::Learned => Ok(Self::LearnedRanking),
            TacticProposalPolicy::FrozenPolicy => Ok(Self::FrozenPolicy),
            TacticProposalPolicy::RandomValid => Ok(Self::RandomValidRanking),
            TacticProposalPolicy::StructuredNonLearning => Err(route_message(
                "structured non-learning is diagnostic, not the frozen-policy control",
            )),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchEfficiencyMetrics {
    pub seed_count: u64,
    pub terminal_seed_count: u64,
    pub terminal_rate_per_million: u64,
    pub best_authenticated_tick: Option<u64>,
    pub median_time_to_first_terminal_micros: Option<u64>,
    pub median_useful_graph_expansions_to_first_terminal: Option<u64>,
    pub median_proposal_expansions_to_first_terminal: Option<u64>,
    pub median_time_to_best_terminal_micros: Option<u64>,
    pub median_useful_graph_expansions_to_best_terminal: Option<u64>,
    pub median_proposal_expansions_to_best_terminal: Option<u64>,
    pub sample_efficiency_timeline_complete: bool,
    pub terminal_improvement_timing_complete: bool,
    /// False means action availability/support was not retained for at least
    /// one decision, so policy-quality differences are not fully auditable.
    #[serde(default)]
    pub action_surface_timeline_complete: bool,
    pub total_proposal_dispatches: u64,
    pub completed_lease_attempts: u64,
    pub retryable_lease_attempts: u64,
    pub cancelled_lease_attempts: u64,
    pub failed_lease_attempts: u64,
    pub total_useful_graph_expansions: u64,
    pub total_observed_interior_segments: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub duplicate_transpositions: u64,
    pub useful_graph_expansions_per_second_millionths: u64,
    pub search_native_ticks: u64,
    pub non_search_native_ticks: u64,
    pub simulated_ticks_per_useful_expansion_millionths: u64,
    /// Actual campaign critical-path wall. Phase fields below are additive
    /// occupancy and therefore do not use this as an attribution denominator.
    pub coordinator_wall_micros: u64,
    /// Legacy field. V4 binds this to `critical_path.unattributed_micros`.
    pub coordinator_unattributed_micros: u64,
    /// Native worker occupancy divided by coordinator wait capacity
    /// (`tactic_execution_micros * workers`), scaled by one million.
    pub native_worker_occupancy_per_million: u64,
    pub process_launch_micros: u64,
    pub tactic_selection_micros: u64,
    pub checkpoint_branching_micros: u64,
    pub tactic_execution_micros: u64,
    pub native_simulation_micros: u64,
    pub ipc_and_result_transport_micros: u64,
    pub native_observation_capture_micros: u64,
    pub native_corpus_encoding_micros: u64,
    pub rust_state_extraction_micros: u64,
    pub tactic_preparation_and_fact_extraction_micros: u64,
    pub graph_admission_micros: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign_admission_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign_admission_breakdown: Option<NativeTacticCampaignAdmissionTiming>,
    pub model_update_micros: u64,
    pub evidence_projection_micros: u64,
    pub persistence_micros: u64,
    pub orchestration_micros: u64,
    pub learner_updates: u64,
    pub restore_accounting: NativeTacticRestoreAccounting,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub route_quality_over_time: Vec<NativeTacticScratchRouteProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_utilization: Option<NativeTacticWorkerUtilization>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<NativeTacticCampaignWorkSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<NativeTacticCampaignResourceAudit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistence_breakdown: Option<NativeTacticPersistenceTiming>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orchestration_breakdown: Option<NativeTacticOrchestrationTiming>,
    /// Critical-path wall attribution, distinct from the additive phase
    /// occupancy fields retained above for work and utilization analysis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical_path: Option<NativeTacticScratchCriticalPathTiming>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchRouteProgress {
    pub seed: u64,
    pub terminal_improvements: Vec<NativeTacticScratchTerminalImprovementAudit>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchCriticalPathTiming {
    pub campaign_wall_micros: u64,
    pub coordinator_lane_occupancy_micros: u64,
    pub generation_critical_path_micros: u64,
    pub campaign_overhead_micros: u64,
    pub process_launch_micros: u64,
    pub tactic_execution_micros: u64,
    pub model_update_micros: u64,
    pub evidence_projection_micros: u64,
    pub persistence_micros: u64,
    pub orchestration_micros: u64,
    #[serde(default)]
    pub report_build_micros: u64,
    #[serde(default)]
    pub fleet_shutdown_micros: u64,
    #[serde(default)]
    pub final_artifact_persistence_micros: u64,
    #[serde(default)]
    pub campaign_completion_coordination_micros: u64,
    pub unattributed_micros: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchComparisonCell {
    pub treatment: NativeTacticScratchTreatment,
    pub proposal_policy: TacticProposalPolicy,
    pub route_report_sha256: Digest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign_completion_sha256: Option<Digest>,
    pub campaign_summary_sha256: Digest,
    pub causal_chain: NativeTacticCampaignCausalSummary,
    pub campaign_audit_sha256: Digest,
    pub execution_plan_sha256: Digest,
    pub replay_sharing: NativeTacticReplaySharingPlan,
    pub metrics: NativeTacticScratchEfficiencyMetrics,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchComparisonReport {
    pub schema: String,
    pub content_sha256: Digest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orchestrator_executable_sha256: Option<Digest>,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub objective_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub execution_strategy: NativeGenericExecutionStrategy,
    pub seeds: Vec<u64>,
    pub workers: usize,
    pub decisions_per_seed: u64,
    pub proposal_width_per_decision: usize,
    pub resource_budgets: NativeTacticPlanBudgets,
    pub cells: Vec<NativeTacticScratchComparisonCell>,
}

impl NativeTacticScratchComparisonReport {
    pub fn build(
        repository_root: &Path,
        routes: Vec<NativeTacticRouteReport>,
    ) -> Result<Self, NativeTacticRouteRunError> {
        if routes.len() != 3 {
            return Err(route_message(
                "scratch comparison requires exactly three route reports",
            ));
        }
        let repository_root = repository_root.canonicalize().map_err(route_error)?;
        let first = routes
            .first()
            .ok_or_else(|| route_message("scratch comparison has no routes"))?;
        let comparison_schema = match first.schema.as_str() {
            NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V43 => NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V6,
            NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V44 | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V45 => {
                NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V7
            }
            _ => {
                return Err(route_message(
                    "scratch comparison requires complete v43, v44, or v45 campaign evidence",
                ));
            }
        };
        let first_plan = read_plan(&repository_root, first)?;
        let mut cells = Vec::with_capacity(routes.len());
        for route in &routes {
            if route.schema != first.schema
                || route
                    .seeds
                    .iter()
                    .any(|seed| !seed.timing.seed_wall_attribution_is_exact())
                || !route.timing.orchestration_attribution_is_valid()
                || !route.timing.persistence_attribution_is_valid()
                || route
                    .orchestrator_executable_sha256
                    .is_none_or(|sha256| sha256 == Digest::ZERO)
                || route
                    .worker_utilization
                    .as_ref()
                    .is_none_or(|utilization| !utilization.validate())
                || route.timing.orchestration_breakdown.is_none()
                || route.timing.persistence_breakdown.is_none()
            {
                return Err(route_message(
                    "scratch comparison requires one matched campaign evidence version",
                ));
            }
            let plan = read_plan(&repository_root, route)?;
            let completion = read_completion(&repository_root, route)?;
            if completion.execution_plan_sha256 != route.execution_plan_sha256 {
                return Err(route_message(
                    "scratch comparison completion marker differs from its execution plan",
                ));
            }
            validate_matched_route(first, route)?;
            validate_matched_plan(&first_plan, &plan)?;
            let audit = NativeTacticScratchCampaignAudit::build(&repository_root, route)?;
            let summary = NativeTacticCampaignSummary::build(route, &plan)?;
            cells.push(NativeTacticScratchComparisonCell {
                treatment: NativeTacticScratchTreatment::from_policy(route.proposal_policy)?,
                proposal_policy: route.proposal_policy,
                route_report_sha256: route_report_sha256(route)?,
                campaign_completion_sha256: Some(completion.content_sha256),
                campaign_summary_sha256: summary.content_sha256,
                causal_chain: summary.causal_chain.clone(),
                campaign_audit_sha256: audit.content_sha256,
                execution_plan_sha256: route.execution_plan_sha256,
                replay_sharing: plan.replay_sharing,
                metrics: efficiency_metrics(route, &audit, &plan, &completion, &summary)?,
            });
        }
        cells.sort_by_key(|cell| cell.treatment);
        if cells
            .iter()
            .map(|cell| cell.treatment)
            .collect::<BTreeSet<_>>()
            != BTreeSet::from([
                NativeTacticScratchTreatment::LearnedRanking,
                NativeTacticScratchTreatment::FrozenPolicy,
                NativeTacticScratchTreatment::RandomValidRanking,
            ])
        {
            return Err(route_message(
                "scratch comparison lacks one required ranking treatment",
            ));
        }
        let mut report = Self {
            schema: comparison_schema.into(),
            content_sha256: Digest::ZERO,
            orchestrator_executable_sha256: first.orchestrator_executable_sha256,
            optimization_request_sha256: first.optimization_request_sha256,
            execution_binding_sha256: first.execution_binding_sha256,
            objective_sha256: first.objective_sha256,
            feature_schema_sha256: first.feature_schema_sha256,
            action_schema_sha256: first.action_schema_sha256,
            execution_strategy: first.execution_strategy,
            seeds: first.exploration_seeds.clone(),
            workers: first.workers,
            decisions_per_seed: first.decisions_per_seed,
            proposal_width_per_decision: first_plan.proposal_width_per_decision,
            resource_budgets: first.resource_budgets,
            cells,
        };
        report.content_sha256 = report.compute_content_sha256()?;
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        let treatments = self
            .cells
            .iter()
            .map(|cell| cell.treatment)
            .collect::<BTreeSet<_>>();
        let critical_path_version = match self.schema.as_str() {
            NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V3 => 0,
            NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V4 => 4,
            NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V5 => 5,
            NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V6 => 6,
            NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V7 => 7,
            _ => return Err(route_message("scratch comparison schema is unsupported")),
        };
        if self.content_sha256 == Digest::ZERO
            || self.optimization_request_sha256 == Digest::ZERO
            || self.execution_binding_sha256 == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || self.feature_schema_sha256 == Digest::ZERO
            || self.action_schema_sha256 == Digest::ZERO
            || (critical_path_version >= 6
                && self
                    .orchestrator_executable_sha256
                    .is_none_or(|sha256| sha256 == Digest::ZERO))
            || self.seeds.is_empty()
            || self.workers == 0
            || self.decisions_per_seed == 0
            || self.proposal_width_per_decision == 0
            || self.cells.len() != 3
            || treatments.len() != 3
            || self.cells.iter().any(|cell| {
                cell.route_report_sha256 == Digest::ZERO
                    || (critical_path_version >= 6
                        && cell
                            .campaign_completion_sha256
                            .is_none_or(|sha256| sha256 == Digest::ZERO))
                    || cell.campaign_summary_sha256 == Digest::ZERO
                    || cell.campaign_audit_sha256 == Digest::ZERO
                    || cell.execution_plan_sha256 == Digest::ZERO
                    || NativeTacticScratchTreatment::from_policy(cell.proposal_policy)
                        .map_or(true, |treatment| cell.treatment != treatment)
                    || !treatment_causal_chain_valid(cell, critical_path_version)
                    || !metrics_valid(&cell.metrics, self.workers, critical_path_version)
            })
            || self.compute_content_sha256()? != self.content_sha256
        {
            return Err(route_message("scratch comparison report is invalid"));
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

fn treatment_causal_chain_valid(
    cell: &NativeTacticScratchComparisonCell,
    comparison_version: u8,
) -> bool {
    policy_causal_chain_valid(cell.proposal_policy, &cell.causal_chain, comparison_version)
}

fn policy_causal_chain_valid(
    proposal_policy: TacticProposalPolicy,
    causal_chain: &NativeTacticCampaignCausalSummary,
    comparison_version: u8,
) -> bool {
    let controlled_policy_effect_valid = comparison_version < 7
        || (causal_chain.causal_chain_ready_for_matched_evaluation
            && causal_chain.policy_update_probes > 0
            && causal_chain.valid_policy_update_probes == causal_chain.policy_update_probes
            && causal_chain.selected_action_changes_from_policy_update > 0);
    match proposal_policy {
        TacticProposalPolicy::Learned => {
            causal_chain.learning_expected && controlled_policy_effect_valid
        }
        TacticProposalPolicy::FrozenPolicy => {
            !causal_chain.learning_expected
                && causal_chain.distinct_model_snapshots_consumed == 1
                && causal_chain.post_update_policy_decisions == 0
                && causal_chain.policy_update_probes == 0
        }
        TacticProposalPolicy::RandomValid => {
            !causal_chain.learning_expected && causal_chain.policy_update_probes == 0
        }
        TacticProposalPolicy::StructuredNonLearning => false,
    }
}

#[cfg(test)]
mod causal_control_tests {
    use super::*;

    fn learned_chain() -> NativeTacticCampaignCausalSummary {
        NativeTacticCampaignCausalSummary {
            learning_expected: true,
            decisions_with_observed_state: 2,
            decisions_with_complete_action_surface: 2,
            decisions_with_native_proposals: 2,
            realized_native_proposals: 4,
            newly_published_training_rows: 4,
            final_training_replay_rows: 4,
            learner_updates: 1,
            model_snapshots_published: 2,
            model_snapshots_consumed: 2,
            distinct_model_snapshots_consumed: 2,
            post_update_policy_decisions: 1,
            policy_update_probes: 1,
            valid_policy_update_probes: 1,
            selected_action_changes_from_policy_update: 1,
            selected_action_changes_at_model_change: 1,
            paired_terminal_return_pairs_started: 0,
            paired_terminal_return_pairs_completed: 0,
            paired_terminal_return_supported_comparisons: 0,
            paired_terminal_return_policy_only_completions: 0,
            paired_terminal_return_control_only_completions: 0,
            paired_terminal_return_neither_terminal_completions: 0,
            paired_terminal_return_invalid_completions: 0,
            paired_terminal_return_matched_policy_wins: 0,
            paired_terminal_return_matched_control_wins: 0,
            paired_terminal_return_matched_ties: 0,
            paired_terminal_return_matched_unresolved: 0,
            paired_terminal_return_censored_comparisons: 0,
            paired_terminal_return_in_progress: 0,
            paired_terminal_return_exact_outcomes: 0,
            paired_terminal_return_policy_wins: 0,
            paired_terminal_return_control_wins: 0,
            paired_terminal_return_ties: 0,
            paired_terminal_return_policy_ticks_to_terminal: 0,
            paired_terminal_return_control_ticks_to_terminal: 0,
            paired_terminal_return_authority_violations: 0,
            causal_chain_ready_for_matched_evaluation: true,
            first_incomplete_link: None,
            outcome_effect_requires_matched_control: true,
        }
    }

    #[test]
    fn v7_requires_a_controlled_policy_effect_only_for_learning() {
        let learned = learned_chain();
        assert!(policy_causal_chain_valid(
            TacticProposalPolicy::Learned,
            &learned,
            7
        ));

        let mut frozen = learned.clone();
        frozen.learning_expected = false;
        frozen.distinct_model_snapshots_consumed = 1;
        frozen.post_update_policy_decisions = 0;
        frozen.policy_update_probes = 0;
        frozen.valid_policy_update_probes = 0;
        frozen.selected_action_changes_from_policy_update = 0;
        frozen.causal_chain_ready_for_matched_evaluation = true;
        frozen.outcome_effect_requires_matched_control = false;
        assert!(policy_causal_chain_valid(
            TacticProposalPolicy::FrozenPolicy,
            &frozen,
            7
        ));
        assert!(policy_causal_chain_valid(
            TacticProposalPolicy::RandomValid,
            &frozen,
            7
        ));

        frozen.policy_update_probes = 1;
        assert!(!policy_causal_chain_valid(
            TacticProposalPolicy::FrozenPolicy,
            &frozen,
            7
        ));
        assert!(!policy_causal_chain_valid(
            TacticProposalPolicy::RandomValid,
            &frozen,
            7
        ));
    }
}

fn validate_matched_route(
    expected: &NativeTacticRouteReport,
    actual: &NativeTacticRouteReport,
) -> Result<(), NativeTacticRouteRunError> {
    if expected.optimization_request_sha256 != actual.optimization_request_sha256
        || expected.execution_binding_sha256 != actual.execution_binding_sha256
        || expected.orchestrator_executable_sha256 != actual.orchestrator_executable_sha256
        || expected.objective_sha256 != actual.objective_sha256
        || expected.feature_schema_sha256 != actual.feature_schema_sha256
        || expected.action_schema_sha256 != actual.action_schema_sha256
        || expected.execution_strategy != actual.execution_strategy
        || expected.exploration_seeds != actual.exploration_seeds
        || expected.workers != actual.workers
        || expected.decisions_per_seed != actual.decisions_per_seed
        || expected.resource_budgets != actual.resource_budgets
        || expected.refit_every_decisions != actual.refit_every_decisions
        || expected.value_treatment != actual.value_treatment
        || expected.demonstration_transitions != 0
        || actual.demonstration_transitions != 0
        || expected.demonstration.is_some()
        || actual.demonstration.is_some()
        || expected.imported_promoted_tactics.is_some()
        || actual.imported_promoted_tactics.is_some()
        || serde_json::to_vec(&expected.goal_target).map_err(route_error)?
            != serde_json::to_vec(&actual.goal_target).map_err(route_error)?
        || expected.reward_spec != actual.reward_spec
    {
        return Err(route_message(
            "scratch comparison route reports do not share one matched condition",
        ));
    }
    Ok(())
}

fn validate_matched_plan(
    expected: &NativeTacticExecutionPlan,
    actual: &NativeTacticExecutionPlan,
) -> Result<(), NativeTacticRouteRunError> {
    if expected.seeds != actual.seeds
        || expected.value_treatment != actual.value_treatment
        || expected.execution_strategy != actual.execution_strategy
        || expected.promoted_tactic_registry_sha256 != actual.promoted_tactic_registry_sha256
        || expected.proposal_width_per_decision != actual.proposal_width_per_decision
        || expected.branch_every_decisions != actual.branch_every_decisions
        || expected.refit_every_decisions != actual.refit_every_decisions
        || expected.root_refresh_cadence != actual.root_refresh_cadence
        || expected.demonstration_chunk_ticks != actual.demonstration_chunk_ticks
        || expected.checkpoint != actual.checkpoint
        || expected.budgets != actual.budgets
        || expected.generations != actual.generations
        || expected.lanes != actual.lanes
    {
        return Err(route_message(
            "scratch comparison execution plans differ outside ranking treatment",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct TopLevelPhaseTiming {
    tactic_execution_micros: u64,
    model_update_micros: u64,
    evidence_projection_micros: u64,
    persistence_micros: u64,
    orchestration_micros: u64,
}

impl TopLevelPhaseTiming {
    fn from_route_timing(timing: &NativeTacticRouteTiming) -> Self {
        Self {
            tactic_execution_micros: timing.tactic_execution_micros,
            model_update_micros: timing.model_update_micros,
            evidence_projection_micros: timing.evidence_projection_micros,
            persistence_micros: timing.persistence_micros,
            orchestration_micros: timing.orchestration_micros,
        }
    }

    fn checked_add(self, other: Self) -> Result<Self, NativeTacticRouteRunError> {
        Ok(Self {
            tactic_execution_micros: checked_timing_add(
                self.tactic_execution_micros,
                other.tactic_execution_micros,
            )?,
            model_update_micros: checked_timing_add(
                self.model_update_micros,
                other.model_update_micros,
            )?,
            evidence_projection_micros: checked_timing_add(
                self.evidence_projection_micros,
                other.evidence_projection_micros,
            )?,
            persistence_micros: checked_timing_add(
                self.persistence_micros,
                other.persistence_micros,
            )?,
            orchestration_micros: checked_timing_add(
                self.orchestration_micros,
                other.orchestration_micros,
            )?,
        })
    }

    fn checked_sub(self, other: Self) -> Result<Self, NativeTacticRouteRunError> {
        Ok(Self {
            tactic_execution_micros: checked_timing_sub(
                self.tactic_execution_micros,
                other.tactic_execution_micros,
            )?,
            model_update_micros: checked_timing_sub(
                self.model_update_micros,
                other.model_update_micros,
            )?,
            evidence_projection_micros: checked_timing_sub(
                self.evidence_projection_micros,
                other.evidence_projection_micros,
            )?,
            persistence_micros: checked_timing_sub(
                self.persistence_micros,
                other.persistence_micros,
            )?,
            orchestration_micros: checked_timing_sub(
                self.orchestration_micros,
                other.orchestration_micros,
            )?,
        })
    }

    fn total(self) -> Result<u64, NativeTacticRouteRunError> {
        [
            self.tactic_execution_micros,
            self.model_update_micros,
            self.evidence_projection_micros,
            self.persistence_micros,
            self.orchestration_micros,
        ]
        .into_iter()
        .try_fold(0_u64, checked_timing_add)
    }
}

impl NativeTacticScratchCriticalPathTiming {
    fn validate(&self) -> bool {
        self.generation_critical_path_micros <= self.campaign_wall_micros
            && self.campaign_overhead_micros
                == self
                    .campaign_wall_micros
                    .checked_sub(self.generation_critical_path_micros)
                    .unwrap_or(u64::MAX)
            && [
                self.process_launch_micros,
                self.tactic_execution_micros,
                self.model_update_micros,
                self.evidence_projection_micros,
                self.persistence_micros,
                self.orchestration_micros,
                self.report_build_micros,
                self.fleet_shutdown_micros,
                self.final_artifact_persistence_micros,
                self.campaign_completion_coordination_micros,
                self.unattributed_micros,
            ]
            .into_iter()
            .try_fold(0_u64, u64::checked_add)
                == Some(self.campaign_wall_micros)
    }
}

fn critical_path_timing(
    route: &NativeTacticRouteReport,
    plan: &NativeTacticExecutionPlan,
    completion: &NativeTacticCampaignCompletion,
) -> Result<NativeTacticScratchCriticalPathTiming, NativeTacticRouteRunError> {
    if route.seeds.len() != plan.lanes.len() || route.seeds.len() != plan.seeds.len() {
        return Err(route_message(
            "scratch comparison timing lanes differ from the execution plan",
        ));
    }
    let seed_occupancy = route
        .seeds
        .iter()
        .try_fold(TopLevelPhaseTiming::default(), |total, seed| {
            total.checked_add(TopLevelPhaseTiming::from_route_timing(&seed.timing))
        })?;
    let route_occupancy = TopLevelPhaseTiming::from_route_timing(&route.timing);
    let mut critical_phases = route_occupancy.checked_sub(seed_occupancy)?;
    let coordinator_lane_occupancy_micros = route.seeds.iter().try_fold(0_u64, |total, seed| {
        checked_timing_add(total, seed.timing.wall_micros)
    })?;
    let mut generation_critical_path_micros = 0_u64;
    for generation in &plan.generations {
        let critical_lane_index = generation
            .lane_indices
            .iter()
            .copied()
            .try_fold(None::<usize>, |current, lane_index| {
                let lane = route.seeds.get(lane_index).ok_or_else(|| {
                    route_message("scratch comparison generation names an absent timing lane")
                })?;
                let replace = current.is_none_or(|current_index| {
                    let current_lane = &route.seeds[current_index];
                    lane.timing.wall_micros > current_lane.timing.wall_micros
                        || lane.timing.wall_micros == current_lane.timing.wall_micros
                            && lane_index < current_index
                });
                Ok::<_, NativeTacticRouteRunError>(replace.then_some(lane_index).or(current))
            })?
            .ok_or_else(|| route_message("scratch comparison generation has no timing lane"))?;
        let critical_lane = &route.seeds[critical_lane_index];
        generation_critical_path_micros = checked_timing_add(
            generation_critical_path_micros,
            critical_lane.timing.wall_micros,
        )?;
        critical_phases = critical_phases.checked_add(TopLevelPhaseTiming::from_route_timing(
            &critical_lane.timing,
        ))?;
    }
    if completion.route_cutoff_wall_micros != route.timing.wall_micros {
        return Err(route_message(
            "scratch comparison completion marker differs from route cutoff wall",
        ));
    }
    let campaign_wall_micros = completion.campaign_wall_micros;
    let campaign_overhead_micros = campaign_wall_micros
        .checked_sub(generation_critical_path_micros)
        .ok_or_else(|| route_message("scratch comparison critical path exceeds campaign wall"))?;
    let accounted_micros = [
        route.timing.process_launch_micros,
        critical_phases.total()?,
        completion.report_build_micros,
        completion.fleet_shutdown_micros,
        completion.final_artifact_persistence_micros,
        completion.campaign_completion_coordination_micros,
    ]
    .into_iter()
    .try_fold(0_u64, checked_timing_add)?;
    let unattributed_micros = campaign_wall_micros
        .checked_sub(accounted_micros)
        .ok_or_else(|| {
            route_message("scratch comparison critical-path phases exceed campaign wall")
        })?;
    let critical = NativeTacticScratchCriticalPathTiming {
        campaign_wall_micros,
        coordinator_lane_occupancy_micros,
        generation_critical_path_micros,
        campaign_overhead_micros,
        process_launch_micros: route.timing.process_launch_micros,
        tactic_execution_micros: critical_phases.tactic_execution_micros,
        model_update_micros: critical_phases.model_update_micros,
        evidence_projection_micros: critical_phases.evidence_projection_micros,
        persistence_micros: critical_phases.persistence_micros,
        orchestration_micros: critical_phases.orchestration_micros,
        report_build_micros: completion.report_build_micros,
        fleet_shutdown_micros: completion.fleet_shutdown_micros,
        final_artifact_persistence_micros: completion.final_artifact_persistence_micros,
        campaign_completion_coordination_micros: completion.campaign_completion_coordination_micros,
        unattributed_micros,
    };
    if !critical.validate() {
        return Err(route_message(
            "scratch comparison critical-path timing does not reconcile",
        ));
    }
    Ok(critical)
}

fn checked_timing_add(left: u64, right: u64) -> Result<u64, NativeTacticRouteRunError> {
    left.checked_add(right)
        .ok_or_else(|| route_message("scratch comparison timing overflowed"))
}

fn checked_timing_sub(left: u64, right: u64) -> Result<u64, NativeTacticRouteRunError> {
    left.checked_sub(right)
        .ok_or_else(|| route_message("scratch comparison timing aggregate is detached"))
}

fn efficiency_metrics(
    route: &NativeTacticRouteReport,
    audit: &NativeTacticScratchCampaignAudit,
    plan: &NativeTacticExecutionPlan,
    completion: &NativeTacticCampaignCompletion,
    summary: &NativeTacticCampaignSummary,
) -> Result<NativeTacticScratchEfficiencyMetrics, NativeTacticRouteRunError> {
    let first_useful = audit
        .seeds
        .iter()
        .filter_map(|seed| seed.useful_graph_expansions_to_first_terminal)
        .collect::<Vec<_>>();
    let first_proposals = audit
        .seeds
        .iter()
        .filter_map(|seed| seed.proposal_expansions_to_first_terminal)
        .collect::<Vec<_>>();
    let best_useful = audit
        .seeds
        .iter()
        .filter_map(|seed| seed.useful_graph_expansions_to_best_terminal)
        .collect::<Vec<_>>();
    let best_proposals = audit
        .seeds
        .iter()
        .filter_map(|seed| seed.proposal_expansions_to_best_terminal)
        .collect::<Vec<_>>();
    let best_wall = audit
        .seeds
        .iter()
        .filter_map(|seed| {
            seed.time_to_best_terminal_micros
                .map(|wall| wall.saturating_add(route.timing.process_launch_micros))
        })
        .collect::<Vec<_>>();
    let first_wall = audit
        .seeds
        .iter()
        .filter_map(|seed| {
            seed.time_to_first_terminal_micros
                .map(|wall| wall.saturating_add(route.timing.process_launch_micros))
        })
        .collect::<Vec<_>>();
    let terminal_count = route.terminal_seeds;
    let seed_count = route.seeds.len() as u64;
    let audited_terminal_count = audit
        .seeds
        .iter()
        .filter(|seed| seed.terminal_discovered)
        .count() as u64;
    let audited_best = audit
        .seeds
        .iter()
        .filter_map(|seed| seed.best_authenticated_tick)
        .min();
    let audited_useful_graph_expansions = audit.unique_useful_graph_expansions;
    let mut reported_seeds = route.seeds.iter().map(|seed| seed.seed).collect::<Vec<_>>();
    reported_seeds.sort_unstable();
    let mut first_wall_sorted = first_wall.clone();
    first_wall_sorted.sort_unstable();
    if seed_count == 0
        || route.exploration_seeds.len() != route.seeds.len()
        || reported_seeds != route.exploration_seeds
        || terminal_count != audited_terminal_count
        || route.best_authenticated_tick != audited_best
        || route.unique_useful_graph_expansions != audited_useful_graph_expansions
        || route.median_time_to_first_terminal_micros != median(first_wall)
        || route.worst_time_to_first_terminal_micros != first_wall_sorted.last().copied()
    {
        return Err(route_message(
            "scratch comparison route aggregate differs from its campaign audit",
        ));
    }
    let search_native_ticks = audit
        .seeds
        .iter()
        .map(|seed| seed.native_ticks)
        .sum::<u64>();
    if route.total_native_ticks < search_native_ticks {
        return Err(route_message(
            "scratch comparison reports fewer total ticks than seed search",
        ));
    }
    let sample_efficiency_timeline_complete = audit
        .seeds
        .iter()
        .all(|seed| seed.graph_expansion_timeline_complete)
        && first_useful.len() as u64 == terminal_count;
    let terminal_improvement_timing_complete = audit
        .seeds
        .iter()
        .all(|seed| seed.terminal_improvement_timing_complete)
        && best_useful.len() as u64 == terminal_count;
    let action_surface_timeline_complete = audit
        .seeds
        .iter()
        .all(|seed| seed.action_surface_timeline_complete);
    let timing = &route.timing;
    let lease_accounting = route.seeds.iter().try_fold(
        NativeTacticLeaseAccounting::default(),
        |mut total, seed| {
            let metrics = seed
                .graph_metrics
                .as_ref()
                .ok_or_else(|| route_message("scratch comparison seed has no graph metrics"))?;
            total.proposal_dispatches = total
                .proposal_dispatches
                .saturating_add(metrics.lease_accounting.proposal_dispatches);
            total.completed_leases = total
                .completed_leases
                .saturating_add(metrics.lease_accounting.completed_leases);
            total.retryable_leases = total
                .retryable_leases
                .saturating_add(metrics.lease_accounting.retryable_leases);
            total.cancelled_leases = total
                .cancelled_leases
                .saturating_add(metrics.lease_accounting.cancelled_leases);
            total.failed_leases = total
                .failed_leases
                .saturating_add(metrics.lease_accounting.failed_leases);
            total.unresolved_leases = total
                .unresolved_leases
                .saturating_add(metrics.lease_accounting.unresolved_leases);
            Ok::<_, NativeTacticRouteRunError>(total)
        },
    )?;
    if lease_accounting.unresolved_leases != 0 {
        return Err(route_message(
            "scratch comparison contains unresolved native proposal leases",
        ));
    }
    let total_observed_interior_segments = route
        .seeds
        .iter()
        .filter_map(|seed| seed.graph_metrics.as_ref())
        .map(|metrics| metrics.graph.observed_segments)
        .sum();
    let duplicate_transpositions = route
        .seeds
        .iter()
        .filter_map(|seed| seed.graph_metrics.as_ref())
        .map(|metrics| metrics.duplicate_transpositions)
        .sum();
    let route_quality_over_time = audit
        .seeds
        .iter()
        .map(|seed| {
            let mut terminal_improvements = seed.terminal_improvements.clone();
            for improvement in &mut terminal_improvements {
                improvement.cumulative_wall_micros = improvement
                    .cumulative_wall_micros
                    .checked_add(route.timing.process_launch_micros)
                    .ok_or_else(|| {
                        route_message("scratch comparison progress wall timing overflowed")
                    })?;
            }
            Ok::<_, NativeTacticRouteRunError>(NativeTacticScratchRouteProgress {
                seed: seed.seed,
                terminal_improvements,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let critical_path = critical_path_timing(route, plan, completion)?;
    Ok(NativeTacticScratchEfficiencyMetrics {
        seed_count,
        terminal_seed_count: terminal_count,
        terminal_rate_per_million: ratio_per_million(terminal_count, seed_count),
        best_authenticated_tick: route.best_authenticated_tick,
        median_time_to_first_terminal_micros: route.median_time_to_first_terminal_micros,
        median_useful_graph_expansions_to_first_terminal: median(first_useful),
        median_proposal_expansions_to_first_terminal: median(first_proposals),
        median_time_to_best_terminal_micros: median(best_wall),
        median_useful_graph_expansions_to_best_terminal: median(best_useful),
        median_proposal_expansions_to_best_terminal: median(best_proposals),
        sample_efficiency_timeline_complete,
        terminal_improvement_timing_complete,
        action_surface_timeline_complete,
        total_proposal_dispatches: lease_accounting.proposal_dispatches,
        completed_lease_attempts: lease_accounting.completed_leases,
        retryable_lease_attempts: lease_accounting.retryable_leases,
        cancelled_lease_attempts: lease_accounting.cancelled_leases,
        failed_lease_attempts: lease_accounting.failed_leases,
        total_useful_graph_expansions: route.unique_useful_graph_expansions,
        total_observed_interior_segments,
        duplicate_transpositions,
        useful_graph_expansions_per_second_millionths: per_second_millionths(
            route.unique_useful_graph_expansions,
            critical_path.campaign_wall_micros,
        ),
        search_native_ticks,
        non_search_native_ticks: route.total_native_ticks.saturating_sub(search_native_ticks),
        simulated_ticks_per_useful_expansion_millionths: ratio_per_million(
            search_native_ticks,
            route.unique_useful_graph_expansions,
        ),
        coordinator_wall_micros: critical_path.campaign_wall_micros,
        coordinator_unattributed_micros: critical_path.unattributed_micros,
        native_worker_occupancy_per_million: ratio_per_million(
            timing.native_simulation_micros,
            timing
                .tactic_execution_micros
                .saturating_mul(route.workers as u64),
        ),
        process_launch_micros: timing.process_launch_micros,
        tactic_selection_micros: timing.tactic_selection_micros,
        checkpoint_branching_micros: timing.checkpoint_branching_micros,
        tactic_execution_micros: timing.tactic_execution_micros,
        native_simulation_micros: timing.native_simulation_micros,
        ipc_and_result_transport_micros: timing.ipc_and_result_transport_micros,
        native_observation_capture_micros: timing.native_observation_capture_micros,
        native_corpus_encoding_micros: timing.native_corpus_encoding_micros,
        rust_state_extraction_micros: timing.rust_state_extraction_micros,
        tactic_preparation_and_fact_extraction_micros: timing
            .tactic_preparation_and_fact_extraction_micros,
        graph_admission_micros: timing.graph_admission_micros,
        campaign_admission_micros: Some(timing.campaign_admission_micros),
        campaign_admission_breakdown: timing.campaign_admission_breakdown,
        model_update_micros: timing.model_update_micros,
        evidence_projection_micros: timing.evidence_projection_micros,
        persistence_micros: timing.persistence_micros,
        orchestration_micros: timing.orchestration_micros,
        learner_updates: route.learner_updates,
        restore_accounting: route.native_restore_accounting.clone(),
        route_quality_over_time,
        worker_utilization: route.worker_utilization.clone(),
        work: Some(summary.work.clone()),
        resources: Some(audit.resources.clone()),
        persistence_breakdown: timing.persistence_breakdown,
        orchestration_breakdown: timing.orchestration_breakdown,
        critical_path: Some(critical_path),
    })
}

fn metrics_valid(
    metrics: &NativeTacticScratchEfficiencyMetrics,
    workers: usize,
    critical_path_version: u8,
) -> bool {
    let wall_attribution_valid = if critical_path_version > 0 {
        metrics.critical_path.as_ref().is_some_and(|critical| {
            critical.validate()
                && critical.campaign_wall_micros == metrics.coordinator_wall_micros
                && critical.process_launch_micros == metrics.process_launch_micros
                && critical.unattributed_micros == metrics.coordinator_unattributed_micros
                && (critical_path_version < 5
                    || critical.report_build_micros > 0
                        && critical.final_artifact_persistence_micros > 0)
        })
    } else {
        metrics.critical_path.is_none()
            && metrics.coordinator_unattributed_micros
                == metrics.coordinator_wall_micros.saturating_sub(
                    metrics
                        .tactic_execution_micros
                        .saturating_add(metrics.model_update_micros)
                        .saturating_add(metrics.evidence_projection_micros)
                        .saturating_add(metrics.persistence_micros)
                        .saturating_add(metrics.orchestration_micros),
                )
    };
    metrics.seed_count > 0
        && wall_attribution_valid
        && (critical_path_version < 6 || canonical_metrics_valid(metrics, workers))
        && metrics.terminal_seed_count <= metrics.seed_count
        && metrics.terminal_rate_per_million
            == ratio_per_million(metrics.terminal_seed_count, metrics.seed_count)
        && metrics.total_proposal_dispatches
            == metrics
                .completed_lease_attempts
                .checked_add(metrics.retryable_lease_attempts)
                .and_then(|total| total.checked_add(metrics.cancelled_lease_attempts))
                .and_then(|total| total.checked_add(metrics.failed_lease_attempts))
                .unwrap_or(u64::MAX)
        && metrics.total_useful_graph_expansions <= metrics.completed_lease_attempts
        && metrics.simulated_ticks_per_useful_expansion_millionths
            == ratio_per_million(
                metrics.search_native_ticks,
                metrics.total_useful_graph_expansions,
            )
        && metrics.useful_graph_expansions_per_second_millionths
            == per_second_millionths(
                metrics.total_useful_graph_expansions,
                metrics.coordinator_wall_micros,
            )
        && metrics.native_worker_occupancy_per_million
            == ratio_per_million(
                metrics.native_simulation_micros,
                metrics
                    .tactic_execution_micros
                    .saturating_mul(workers as u64),
            )
        && metrics
            .campaign_admission_breakdown
            .is_none_or(|breakdown| {
                metrics.campaign_admission_micros == Some(breakdown.total_micros())
            })
        && (metrics.terminal_seed_count != 0 || metrics.best_authenticated_tick.is_none())
        && (metrics.terminal_seed_count != 0
            || (metrics.median_time_to_first_terminal_micros.is_none()
                && metrics
                    .median_useful_graph_expansions_to_first_terminal
                    .is_none()
                && metrics
                    .median_proposal_expansions_to_first_terminal
                    .is_none()
                && metrics.median_time_to_best_terminal_micros.is_none()
                && metrics
                    .median_useful_graph_expansions_to_best_terminal
                    .is_none()
                && metrics
                    .median_proposal_expansions_to_best_terminal
                    .is_none()))
}

fn canonical_metrics_valid(metrics: &NativeTacticScratchEfficiencyMetrics, workers: usize) -> bool {
    let Some(utilization) = metrics.worker_utilization.as_ref() else {
        return false;
    };
    let Some(work) = metrics.work.as_ref() else {
        return false;
    };
    let Some(resources) = metrics.resources.as_ref() else {
        return false;
    };
    let Some(persistence) = metrics.persistence_breakdown else {
        return false;
    };
    let Some(orchestration) = metrics.orchestration_breakdown else {
        return false;
    };
    utilization.validate()
        && utilization.worker_processes == workers as u64
        && utilization.proposal_jobs > 0
        && utilization.native_process_cpu_micros.is_some()
        && work.lease_accounting_complete
        && work.unresolved_leases == 0
        && work.proposal_dispatches == metrics.total_proposal_dispatches
        && work.completed_leases == metrics.completed_lease_attempts
        && work.retryable_leases == metrics.retryable_lease_attempts
        && work.cancelled_leases == metrics.cancelled_lease_attempts
        && work.failed_leases == metrics.failed_lease_attempts
        && resources.declared_memory_bound_bytes.is_some()
        && resources.memory_bound_satisfied
        && resources.passed
        && persistence.checked_total_micros() == Some(metrics.persistence_micros)
        && orchestration.checked_total_micros() == Some(metrics.orchestration_micros)
        && metrics.sample_efficiency_timeline_complete
        && metrics.terminal_improvement_timing_complete
        && metrics.action_surface_timeline_complete
        && route_quality_timeline_valid(metrics)
}

fn route_quality_timeline_valid(metrics: &NativeTacticScratchEfficiencyMetrics) -> bool {
    if metrics.route_quality_over_time.len() as u64 != metrics.seed_count
        || metrics
            .route_quality_over_time
            .iter()
            .map(|seed| seed.seed)
            .collect::<BTreeSet<_>>()
            .len()
            != metrics.route_quality_over_time.len()
    {
        return false;
    }
    let terminal_timelines = metrics
        .route_quality_over_time
        .iter()
        .filter(|seed| !seed.terminal_improvements.is_empty())
        .collect::<Vec<_>>();
    if terminal_timelines.len() as u64 != metrics.terminal_seed_count
        || terminal_timelines.iter().any(|seed| {
            seed.terminal_improvements.windows(2).any(|pair| {
                pair[0].authenticated_tick <= pair[1].authenticated_tick
                    || pair[0].decision_index > pair[1].decision_index
                    || pair[0].cumulative_wall_micros > pair[1].cumulative_wall_micros
                    || pair[0].cumulative_proposal_expansions
                        > pair[1].cumulative_proposal_expansions
                    || pair[0].cumulative_useful_graph_expansions
                        > pair[1].cumulative_useful_graph_expansions
            })
        })
    {
        return false;
    }
    let first = terminal_timelines
        .iter()
        .filter_map(|seed| seed.terminal_improvements.first())
        .collect::<Vec<_>>();
    let best = terminal_timelines
        .iter()
        .filter_map(|seed| seed.terminal_improvements.last())
        .collect::<Vec<_>>();
    metrics.best_authenticated_tick == best.iter().map(|point| point.authenticated_tick).min()
        && metrics.median_time_to_first_terminal_micros
            == median(
                first
                    .iter()
                    .map(|point| point.cumulative_wall_micros)
                    .collect(),
            )
        && metrics.median_useful_graph_expansions_to_first_terminal
            == median(
                first
                    .iter()
                    .map(|point| point.cumulative_useful_graph_expansions)
                    .collect(),
            )
        && metrics.median_proposal_expansions_to_first_terminal
            == median(
                first
                    .iter()
                    .map(|point| point.cumulative_proposal_expansions)
                    .collect(),
            )
        && metrics.median_time_to_best_terminal_micros
            == median(
                best.iter()
                    .map(|point| point.cumulative_wall_micros)
                    .collect(),
            )
        && metrics.median_useful_graph_expansions_to_best_terminal
            == median(
                best.iter()
                    .map(|point| point.cumulative_useful_graph_expansions)
                    .collect(),
            )
        && metrics.median_proposal_expansions_to_best_terminal
            == median(
                best.iter()
                    .map(|point| point.cumulative_proposal_expansions)
                    .collect(),
            )
}

fn median(mut values: Vec<u64>) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(values[middle])
    } else {
        let lower = values[middle - 1];
        let difference = values[middle] - lower;
        Some(lower + difference / 2 + difference % 2)
    }
}

fn read_plan(
    repository_root: &Path,
    route: &NativeTacticRouteReport,
) -> Result<NativeTacticExecutionPlan, NativeTacticRouteRunError> {
    let declared = Path::new(&route.execution_plan_path);
    let candidate = if declared.is_absolute() {
        declared.to_path_buf()
    } else {
        repository_root.join(declared)
    };
    let resolved = candidate.canonicalize().map_err(route_error)?;
    if !resolved.starts_with(repository_root) || !resolved.is_file() {
        return Err(route_message(
            "scratch comparison execution plan is outside the repository or absent",
        ));
    }
    let plan = NativeTacticExecutionPlan::read(&resolved)?;
    if plan.identity()? != route.execution_plan_sha256
        || plan.proposal_policy != route.proposal_policy
    {
        return Err(route_message(
            "scratch comparison execution plan differs from its route report",
        ));
    }
    Ok(plan)
}

fn read_completion(
    repository_root: &Path,
    route: &NativeTacticRouteReport,
) -> Result<NativeTacticCampaignCompletion, NativeTacticRouteRunError> {
    let declared = Path::new(&route.execution_plan_path);
    let candidate = if declared.is_absolute() {
        declared.to_path_buf()
    } else {
        repository_root.join(declared)
    };
    let plan_path = candidate.canonicalize().map_err(route_error)?;
    let output_root = plan_path
        .parent()
        .ok_or_else(|| route_message("scratch comparison execution plan has no campaign root"))?;
    let completion_path = output_root.join(NATIVE_TACTIC_CAMPAIGN_COMPLETION_FILE);
    let report_path = output_root.join("report.json");
    let summary_path = output_root.join(NATIVE_TACTIC_CAMPAIGN_SUMMARY_FILE);
    let completion = NativeTacticCampaignCompletion::read(&completion_path)?;
    completion.validate_files(&report_path, &summary_path)?;
    Ok(completion)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_learning::tactic_value_treatment::TacticValueTreatment;

    fn plan(
        proposal_policy: TacticProposalPolicy,
        replay_sharing: NativeTacticReplaySharingPlan,
    ) -> NativeTacticExecutionPlan {
        NativeTacticExecutionPlan::build(NativeTacticExecutionPlanRequest {
            seeds: vec![11, 22, 33, 44],
            proposal_policy,
            value_treatment: TacticValueTreatment::GoalRelabeledFittedQKnnV2,
            execution_strategy: NativeGenericExecutionStrategy::NativeController,
            promoted_tactic_registry_sha256: None,
            lanes_per_generation: 1,
            proposal_width_per_decision: 4,
            branch_every_decisions: 8,
            refit_every_decisions: 4,
            root_refresh_cadence: 4,
            epsilon_per_million: 350_000,
            demonstration_chunk_ticks: None,
            paired_terminal_return_evaluation: false,
            replay_sharing,
            budgets: NativeTacticPlanBudgets {
                decisions_per_lane: 256,
                native_ticks: NativeTacticResourceLimit::Bounded(100_000),
                memory_bytes: NativeTacticResourceLimit::Bounded(1_000_000_000),
                wall_micros: NativeTacticResourceLimit::Bounded(900_000_000),
            },
        })
        .unwrap()
    }

    #[test]
    fn median_is_overflow_safe_and_rounds_half_up() {
        assert_eq!(median(Vec::new()), None);
        assert_eq!(median(vec![9, 3, 5]), Some(5));
        assert_eq!(median(vec![4, 9]), Some(7));
        assert_eq!(median(vec![u64::MAX, u64::MAX]), Some(u64::MAX));
    }

    #[test]
    fn critical_path_timing_requires_exact_wall_reconciliation() {
        let timing = NativeTacticScratchCriticalPathTiming {
            campaign_wall_micros: 1_000,
            coordinator_lane_occupancy_micros: 1_500,
            generation_critical_path_micros: 800,
            campaign_overhead_micros: 200,
            process_launch_micros: 100,
            tactic_execution_micros: 300,
            model_update_micros: 100,
            evidence_projection_micros: 10,
            persistence_micros: 100,
            orchestration_micros: 90,
            report_build_micros: 0,
            fleet_shutdown_micros: 0,
            final_artifact_persistence_micros: 0,
            campaign_completion_coordination_micros: 0,
            unattributed_micros: 300,
        };
        assert!(timing.validate());

        let mut detached = timing.clone();
        detached.unattributed_micros += 1;
        assert!(!detached.validate());
        let mut detached = timing;
        detached.campaign_overhead_micros += 1;
        assert!(!detached.validate());
    }

    #[test]
    fn phase_occupancy_never_uses_saturating_subtraction() {
        let route = TopLevelPhaseTiming {
            tactic_execution_micros: 10,
            ..TopLevelPhaseTiming::default()
        };
        let detached_seed_sum = TopLevelPhaseTiming {
            tactic_execution_micros: 11,
            ..TopLevelPhaseTiming::default()
        };
        assert!(route.checked_sub(detached_seed_sum).is_err());
    }

    #[test]
    fn every_policy_maps_to_a_distinct_required_treatment() {
        let treatments = [
            TacticProposalPolicy::Learned,
            TacticProposalPolicy::FrozenPolicy,
            TacticProposalPolicy::RandomValid,
        ]
        .map(|policy| NativeTacticScratchTreatment::from_policy(policy).unwrap())
        .into_iter()
        .collect::<BTreeSet<_>>();
        assert_eq!(treatments.len(), 3);
        assert!(
            NativeTacticScratchTreatment::from_policy(TacticProposalPolicy::StructuredNonLearning)
                .is_err()
        );
    }

    #[test]
    fn matched_plan_check_allows_only_ranking_and_replay_treatment() {
        let learned = plan(
            TacticProposalPolicy::Learned,
            NativeTacticReplaySharingPlan::BoundedStaleness {
                maximum_stale_replay_revisions: 16,
            },
        );
        let random = plan(
            TacticProposalPolicy::RandomValid,
            NativeTacticReplaySharingPlan::GenerationBarrier,
        );
        let frozen = plan(
            TacticProposalPolicy::FrozenPolicy,
            NativeTacticReplaySharingPlan::GenerationBarrier,
        );
        validate_matched_plan(&learned, &frozen).unwrap();
        validate_matched_plan(&learned, &random).unwrap();

        let mut budget_drift = random.clone();
        budget_drift.budgets.native_ticks = NativeTacticResourceLimit::Bounded(99_999);
        assert!(validate_matched_plan(&learned, &budget_drift).is_err());

        let mut acquisition_drift = random;
        acquisition_drift.lanes[0].acquisition =
            NativeTacticAcquisitionPlan::FixedRank { rank: 99 };
        assert!(validate_matched_plan(&learned, &acquisition_drift).is_err());
    }
}
