use super::scratch_discovery::route_report_sha256;
use super::*;

pub const NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V2: &str =
    "dusklight-native-tactic-scratch-comparison/v2";
pub const NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V3: &str =
    "dusklight-native-tactic-scratch-comparison/v3";

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
    pub useful_graph_expansions_per_second_millionths: u64,
    pub search_native_ticks: u64,
    pub non_search_native_ticks: u64,
    pub simulated_ticks_per_useful_expansion_millionths: u64,
    /// Sum of per-seed coordinator wall time. Concurrent lanes intentionally
    /// remain additive so phase occupancy has the same denominator.
    pub coordinator_wall_micros: u64,
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
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchComparisonCell {
    pub treatment: NativeTacticScratchTreatment,
    pub proposal_policy: TacticProposalPolicy,
    pub route_report_sha256: Digest,
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
        let first_plan = read_plan(&repository_root, first)?;
        let mut cells = Vec::with_capacity(routes.len());
        for route in &routes {
            let plan = read_plan(&repository_root, route)?;
            validate_matched_route(first, route)?;
            validate_matched_plan(&first_plan, &plan)?;
            let audit = NativeTacticScratchCampaignAudit::build(&repository_root, route)?;
            let summary = NativeTacticCampaignSummary::build(route, &plan)?;
            cells.push(NativeTacticScratchComparisonCell {
                treatment: NativeTacticScratchTreatment::from_policy(route.proposal_policy)?,
                proposal_policy: route.proposal_policy,
                route_report_sha256: route_report_sha256(route)?,
                campaign_summary_sha256: summary.content_sha256,
                causal_chain: summary.causal_chain,
                campaign_audit_sha256: audit.content_sha256,
                execution_plan_sha256: route.execution_plan_sha256,
                replay_sharing: plan.replay_sharing,
                metrics: efficiency_metrics(route, &audit)?,
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
            schema: NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V3.into(),
            content_sha256: Digest::ZERO,
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
        if self.schema != NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V3
            || self.content_sha256 == Digest::ZERO
            || self.optimization_request_sha256 == Digest::ZERO
            || self.execution_binding_sha256 == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || self.feature_schema_sha256 == Digest::ZERO
            || self.action_schema_sha256 == Digest::ZERO
            || self.seeds.is_empty()
            || self.workers == 0
            || self.decisions_per_seed == 0
            || self.proposal_width_per_decision == 0
            || self.cells.len() != 3
            || treatments.len() != 3
            || self.cells.iter().any(|cell| {
                cell.route_report_sha256 == Digest::ZERO
                    || cell.campaign_summary_sha256 == Digest::ZERO
                    || cell.campaign_audit_sha256 == Digest::ZERO
                    || cell.execution_plan_sha256 == Digest::ZERO
                    || NativeTacticScratchTreatment::from_policy(cell.proposal_policy)
                        .map_or(true, |treatment| cell.treatment != treatment)
                    || !treatment_causal_chain_valid(cell)
                    || !metrics_valid(&cell.metrics, self.workers)
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

fn treatment_causal_chain_valid(cell: &NativeTacticScratchComparisonCell) -> bool {
    match cell.proposal_policy {
        TacticProposalPolicy::Learned => cell.causal_chain.learning_expected,
        TacticProposalPolicy::FrozenPolicy => {
            !cell.causal_chain.learning_expected
                && cell.causal_chain.distinct_model_snapshots_consumed == 1
                && cell.causal_chain.post_update_policy_decisions == 0
        }
        TacticProposalPolicy::RandomValid => !cell.causal_chain.learning_expected,
        TacticProposalPolicy::StructuredNonLearning => false,
    }
}

fn validate_matched_route(
    expected: &NativeTacticRouteReport,
    actual: &NativeTacticRouteReport,
) -> Result<(), NativeTacticRouteRunError> {
    if expected.optimization_request_sha256 != actual.optimization_request_sha256
        || expected.execution_binding_sha256 != actual.execution_binding_sha256
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

fn efficiency_metrics(
    route: &NativeTacticRouteReport,
    audit: &NativeTacticScratchCampaignAudit,
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
    let accounted = timing
        .tactic_execution_micros
        .saturating_add(timing.model_update_micros)
        .saturating_add(timing.evidence_projection_micros)
        .saturating_add(timing.persistence_micros)
        .saturating_add(timing.orchestration_micros);
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
        useful_graph_expansions_per_second_millionths: timing
            .unique_useful_graph_expansions_per_second_millionths,
        search_native_ticks,
        non_search_native_ticks: route.total_native_ticks.saturating_sub(search_native_ticks),
        simulated_ticks_per_useful_expansion_millionths: ratio_per_million(
            search_native_ticks,
            route.unique_useful_graph_expansions,
        ),
        coordinator_wall_micros: timing.wall_micros,
        coordinator_unattributed_micros: timing.wall_micros.saturating_sub(accounted),
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
    })
}

fn metrics_valid(metrics: &NativeTacticScratchEfficiencyMetrics, workers: usize) -> bool {
    metrics.seed_count > 0
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
        && metrics.coordinator_unattributed_micros
            == metrics.coordinator_wall_micros.saturating_sub(
                metrics
                    .tactic_execution_micros
                    .saturating_add(metrics.model_update_micros)
                    .saturating_add(metrics.evidence_projection_micros)
                    .saturating_add(metrics.persistence_micros)
                    .saturating_add(metrics.orchestration_micros),
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
            lanes_per_generation: 4,
            proposal_width_per_decision: 4,
            branch_every_decisions: 8,
            refit_every_decisions: 4,
            root_refresh_cadence: 4,
            epsilon_per_million: 350_000,
            demonstration_chunk_ticks: None,
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
