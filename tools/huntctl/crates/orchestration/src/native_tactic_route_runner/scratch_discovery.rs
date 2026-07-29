use super::*;
use std::collections::BTreeSet;

pub const NATIVE_TACTIC_SCRATCH_DISCOVERY_SCHEMA_V1: &str =
    "dusklight-native-tactic-scratch-discovery/v1";
pub const ORDON_SCRATCH_DISCOVERY_GOAL: &str = "ordon_spring_load_committed";
pub const ORDON_SCRATCH_DISCOVERY_SEEDS: usize = 4;
pub const ORDON_MEDIAN_TERMINAL_WALL_LIMIT_MICROS: u64 = 5 * 60 * 1_000_000;
pub const ORDON_WORST_TERMINAL_WALL_LIMIT_MICROS: u64 = 15 * 60 * 1_000_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchCondition {
    pub id: String,
    pub passed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchTotals {
    pub graph_nodes: u64,
    pub graph_edges: u64,
    pub duplicate_transpositions: u64,
    pub proposal_dispatches: u64,
    pub completed_leases: u64,
    pub retryable_leases: u64,
    pub cancelled_leases: u64,
    pub failed_leases: u64,
    pub unresolved_leases: u64,
    pub completed_graph_expansions: u64,
    pub active_leases: u64,
    pub restore_samples: u64,
    pub simulated_ticks: u64,
    pub terminal_paths: u64,
    pub learner_updates: u64,
    pub useful_training_transitions: u64,
    pub wall_micros: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchDiscoveryReport {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub route_report_sha256: Digest,
    pub execution_plan_sha256: Digest,
    pub goal: String,
    pub seeds: Vec<u64>,
    pub exploration_horizon_ticks: u64,
    pub promotion_before_tick: u64,
    pub maximum_graph_expansions: u64,
    pub wall_budget_micros: u64,
    pub median_time_to_first_terminal_micros: Option<u64>,
    pub worst_time_to_first_terminal_micros: Option<u64>,
    pub totals: NativeTacticScratchTotals,
    pub conditions: Vec<NativeTacticScratchCondition>,
    pub passed: bool,
}

impl NativeTacticScratchDiscoveryReport {
    pub fn build(
        request: &OptimizationRequest,
        route: &NativeTacticRouteReport,
    ) -> Result<Self, NativeTacticRouteRunError> {
        request.validate().map_err(route_error)?;
        if route.schema != NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V35
            || route.optimization_request_sha256 != request.content_sha256
        {
            return Err(route_message(
                "scratch discovery route report is detached from its request",
            ));
        }
        let plan = NativeTacticExecutionPlan::read(Path::new(&route.execution_plan_path))?;
        let plan_sha256 = plan.identity()?;
        if plan_sha256 != route.execution_plan_sha256
            || plan.seeds != route.exploration_seeds
            || plan.budgets != route.resource_budgets
        {
            return Err(route_message(
                "scratch discovery route report is detached from its execution plan",
            ));
        }
        validate_seed_artifacts(route, &plan)?;

        let maximum_graph_expansions = u64::try_from(plan.seeds.len())
            .ok()
            .and_then(|seeds| plan.budgets.decisions_per_lane.checked_mul(seeds))
            .and_then(|decisions| decisions.checked_mul(plan.proposal_width_per_decision as u64))
            .ok_or_else(|| route_message("scratch graph expansion budget overflowed"))?;
        let wall_budget_micros = match plan.budgets.wall_micros {
            NativeTacticResourceLimit::Bounded(micros) => micros,
            NativeTacticResourceLimit::Unbounded => 0,
        };
        let totals = scratch_totals(route)?;
        let minimum_horizon =
            minimum_discovery_horizon_ticks(request.budgets.promotion_before_tick)
                .ok_or_else(|| route_message("scratch discovery horizon requirement overflowed"))?;
        let conditions = vec![
            condition(
                "ordon_real_load_zone",
                request.terminal_predicate.goal == ORDON_SCRATCH_DISCOVERY_GOAL
                    && route.objective_sha256 == request.terminal_predicate.definition_sha256,
            ),
            condition(
                "from_scratch_request",
                request.campaign_class == CampaignClass::FromScratchDiscovery
                    && request.incumbent.is_none(),
            ),
            condition(
                "no_assistance",
                route.proposal_policy == TacticProposalPolicy::Learned
                    && route.demonstration.is_none()
                    && route.demonstration_transitions == 0
                    && route.imported_promoted_tactics.is_none()
                    && plan.demonstration_chunk_ticks.is_none()
                    && plan.promoted_tactic_registry_sha256.is_none()
                    && !route.goal_target.authored_route_coordinates_used,
            ),
            condition(
                "four_sealed_seeds",
                request.execution.deterministic_seeds.len() == ORDON_SCRATCH_DISCOVERY_SEEDS
                    && route.exploration_seeds == request.execution.deterministic_seeds,
            ),
            condition(
                "generous_route_horizon",
                request.budgets.exploration_horizon_ticks >= minimum_horizon,
            ),
            condition(
                "bounded_graph_expansions",
                maximum_graph_expansions > 0
                    && totals.completed_graph_expansions <= maximum_graph_expansions
                    && route.total_decisions <= request.budgets.candidate_budget,
            ),
            condition(
                "bounded_wall_time",
                wall_budget_micros > 0
                    && wall_budget_micros <= ORDON_WORST_TERMINAL_WALL_LIMIT_MICROS,
            ),
            condition(
                "all_seeds_reach_terminal",
                route.terminal_seeds as usize == ORDON_SCRATCH_DISCOVERY_SEEDS
                    && route.seeds.len() == ORDON_SCRATCH_DISCOVERY_SEEDS
                    && route.seeds.iter().all(|seed| {
                        seed.terminal_discovered
                            && seed
                                .graph_metrics
                                .as_ref()
                                .is_some_and(|metrics| metrics.terminal_paths > 0)
                    }),
            ),
            condition(
                "median_terminal_within_five_minutes",
                route
                    .median_time_to_first_terminal_micros
                    .is_some_and(|micros| micros <= ORDON_MEDIAN_TERMINAL_WALL_LIMIT_MICROS),
            ),
            condition(
                "worst_terminal_within_fifteen_minutes",
                route
                    .worst_time_to_first_terminal_micros
                    .is_some_and(|micros| micros <= ORDON_WORST_TERMINAL_WALL_LIMIT_MICROS),
            ),
            condition(
                "inspectable_graph_and_learner_work",
                route.seeds.iter().all(|seed| seed.graph_metrics.is_some())
                    && totals.graph_nodes >= ORDON_SCRATCH_DISCOVERY_SEEDS as u64
                    && totals.completed_graph_expansions == route.unique_useful_graph_expansions
                    && totals.simulated_ticks == route.total_native_ticks,
            ),
            condition(
                "all_dispatched_leases_resolved",
                totals.unresolved_leases == 0
                    && totals.proposal_dispatches
                        == totals
                            .completed_leases
                            .saturating_add(totals.retryable_leases)
                            .saturating_add(totals.cancelled_leases)
                            .saturating_add(totals.failed_leases),
            ),
        ];
        let passed = conditions.iter().all(|condition| condition.passed);
        let mut report = Self {
            schema: NATIVE_TACTIC_SCRATCH_DISCOVERY_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: request.content_sha256,
            route_report_sha256: route_report_sha256(route)?,
            execution_plan_sha256: plan_sha256,
            goal: request.terminal_predicate.goal.clone(),
            seeds: route.exploration_seeds.clone(),
            exploration_horizon_ticks: request.budgets.exploration_horizon_ticks,
            promotion_before_tick: request.budgets.promotion_before_tick,
            maximum_graph_expansions,
            wall_budget_micros,
            median_time_to_first_terminal_micros: route.median_time_to_first_terminal_micros,
            worst_time_to_first_terminal_micros: route.worst_time_to_first_terminal_micros,
            totals,
            conditions,
            passed,
        };
        report.content_sha256 = report.compute_content_sha256()?;
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        let ids = self
            .conditions
            .iter()
            .map(|condition| condition.id.as_str())
            .collect::<BTreeSet<_>>();
        if self.schema != NATIVE_TACTIC_SCRATCH_DISCOVERY_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.optimization_request_sha256 == Digest::ZERO
            || self.route_report_sha256 == Digest::ZERO
            || self.execution_plan_sha256 == Digest::ZERO
            || self.goal != ORDON_SCRATCH_DISCOVERY_GOAL
            || self.seeds.len() != ORDON_SCRATCH_DISCOVERY_SEEDS
            || minimum_discovery_horizon_ticks(self.promotion_before_tick)
                .is_none_or(|minimum| self.exploration_horizon_ticks < minimum)
            || self.maximum_graph_expansions == 0
            || self.wall_budget_micros == 0
            || self.totals.unresolved_leases != 0
            || self.totals.proposal_dispatches
                != self
                    .totals
                    .completed_leases
                    .checked_add(self.totals.retryable_leases)
                    .and_then(|total| total.checked_add(self.totals.cancelled_leases))
                    .and_then(|total| total.checked_add(self.totals.failed_leases))
                    .unwrap_or(u64::MAX)
            || self.totals.completed_graph_expansions > self.totals.completed_leases
            || ids.len() != self.conditions.len()
            || self.passed != self.conditions.iter().all(|condition| condition.passed)
            || self.compute_content_sha256()? != self.content_sha256
        {
            return Err(route_message(
                "native tactic scratch discovery report is invalid",
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
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        Ok(Digest(
            Sha256::digest(serde_json::to_vec(&unsigned).map_err(route_error)?).into(),
        ))
    }
}

fn condition(id: &str, passed: bool) -> NativeTacticScratchCondition {
    NativeTacticScratchCondition {
        id: id.into(),
        passed,
    }
}

pub(super) fn route_report_sha256(
    route: &NativeTacticRouteReport,
) -> Result<Digest, NativeTacticRouteRunError> {
    Ok(Digest(
        Sha256::digest(serde_json::to_vec(route).map_err(route_error)?).into(),
    ))
}

fn validate_seed_artifacts(
    route: &NativeTacticRouteReport,
    plan: &NativeTacticExecutionPlan,
) -> Result<(), NativeTacticRouteRunError> {
    if route.seeds.len() != plan.lanes.len() {
        return Err(route_message(
            "scratch discovery seed results are detached from the plan",
        ));
    }
    for (index, reported) in route.seeds.iter().enumerate() {
        let seed_root = Path::new(&reported.final_checkpoint)
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| route_message("scratch seed checkpoint path has no seed root"))?;
        let validated = read_completed_seed_result(
            &seed_root.join("seed-result.json"),
            reported.seed,
            plan.budgets.decisions_per_lane,
            route.execution_plan_sha256,
            plan.lanes
                .get(index)
                .ok_or_else(|| route_message("scratch plan seed lane is absent"))?,
        )?;
        let reported_sha256 =
            Digest(Sha256::digest(serde_json::to_vec(reported).map_err(route_error)?).into());
        let validated_sha256 =
            Digest(Sha256::digest(serde_json::to_vec(&validated).map_err(route_error)?).into());
        if reported_sha256 != validated_sha256 {
            return Err(route_message(
                "scratch route report seed differs from validated durable evidence",
            ));
        }
    }
    Ok(())
}

fn scratch_totals(
    route: &NativeTacticRouteReport,
) -> Result<NativeTacticScratchTotals, NativeTacticRouteRunError> {
    let mut totals = NativeTacticScratchTotals {
        graph_nodes: 0,
        graph_edges: 0,
        duplicate_transpositions: 0,
        proposal_dispatches: 0,
        completed_leases: 0,
        retryable_leases: 0,
        cancelled_leases: 0,
        failed_leases: 0,
        unresolved_leases: 0,
        completed_graph_expansions: 0,
        active_leases: 0,
        restore_samples: route.native_restore_accounting.restore_samples,
        simulated_ticks: route.total_native_ticks,
        terminal_paths: 0,
        learner_updates: route.learner_updates,
        useful_training_transitions: route.useful_training_transitions,
        wall_micros: route.timing.wall_micros,
    };
    for seed in &route.seeds {
        let metrics = seed
            .graph_metrics
            .as_ref()
            .ok_or_else(|| route_message("scratch seed has no authoritative graph metrics"))?;
        totals.graph_nodes = totals.graph_nodes.saturating_add(metrics.graph.nodes);
        totals.graph_edges = totals
            .graph_edges
            .saturating_add(metrics.graph.observed_segments);
        totals.duplicate_transpositions = totals
            .duplicate_transpositions
            .saturating_add(metrics.duplicate_transpositions);
        totals.proposal_dispatches = totals
            .proposal_dispatches
            .saturating_add(metrics.lease_accounting.proposal_dispatches);
        totals.completed_leases = totals
            .completed_leases
            .saturating_add(metrics.lease_accounting.completed_leases);
        totals.retryable_leases = totals
            .retryable_leases
            .saturating_add(metrics.lease_accounting.retryable_leases);
        totals.cancelled_leases = totals
            .cancelled_leases
            .saturating_add(metrics.lease_accounting.cancelled_leases);
        totals.failed_leases = totals
            .failed_leases
            .saturating_add(metrics.lease_accounting.failed_leases);
        totals.unresolved_leases = totals
            .unresolved_leases
            .saturating_add(metrics.lease_accounting.unresolved_leases);
        totals.completed_graph_expansions = totals
            .completed_graph_expansions
            .saturating_add(metrics.graph.completed_expansions);
        totals.active_leases = totals
            .active_leases
            .saturating_add(metrics.graph.leased_expansions);
        totals.terminal_paths = totals.terminal_paths.saturating_add(metrics.terminal_paths);
    }
    Ok(totals)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> NativeTacticScratchDiscoveryReport {
        let mut report = NativeTacticScratchDiscoveryReport {
            schema: NATIVE_TACTIC_SCRATCH_DISCOVERY_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: Digest([1; 32]),
            route_report_sha256: Digest([2; 32]),
            execution_plan_sha256: Digest([3; 32]),
            goal: ORDON_SCRATCH_DISCOVERY_GOAL.into(),
            seeds: vec![11, 22, 33, 44],
            exploration_horizon_ticks: 1_024,
            promotion_before_tick: 131,
            maximum_graph_expansions: 4_096,
            wall_budget_micros: ORDON_WORST_TERMINAL_WALL_LIMIT_MICROS,
            median_time_to_first_terminal_micros: Some(100),
            worst_time_to_first_terminal_micros: Some(200),
            totals: NativeTacticScratchTotals {
                graph_nodes: 8,
                graph_edges: 4,
                duplicate_transpositions: 0,
                proposal_dispatches: 8,
                completed_leases: 8,
                retryable_leases: 0,
                cancelled_leases: 0,
                failed_leases: 0,
                unresolved_leases: 0,
                completed_graph_expansions: 4,
                active_leases: 0,
                restore_samples: 4,
                simulated_ticks: 128,
                terminal_paths: 4,
                learner_updates: 1,
                useful_training_transitions: 4,
                wall_micros: 300,
            },
            conditions: vec![condition("all_seeds_reach_terminal", true)],
            passed: true,
        };
        report.content_sha256 = report.compute_content_sha256().unwrap();
        report
    }

    #[test]
    fn acceptance_report_is_content_bound_and_rejects_resealed_failure_drift() {
        let report = report();
        assert!(report.totals.completed_leases > report.totals.completed_graph_expansions);
        report.validate().unwrap();

        let mut tampered = report.clone();
        tampered.totals.completed_leases += 1;
        assert!(tampered.validate().is_err());

        let mut resealed_lifecycle_drift = report.clone();
        resealed_lifecycle_drift.totals.proposal_dispatches += 1;
        resealed_lifecycle_drift.content_sha256 =
            resealed_lifecycle_drift.compute_content_sha256().unwrap();
        assert!(resealed_lifecycle_drift.validate().is_err());

        let mut failed = report;
        failed.conditions[0].passed = false;
        failed.content_sha256 = failed.compute_content_sha256().unwrap();
        assert!(failed.validate().is_err());
    }
}
