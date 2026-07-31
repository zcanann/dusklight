use super::fault_recovery_audit::semantic_trace_sha256_v2;
use super::*;

pub const NATIVE_TACTIC_THROUGHPUT_TREATMENT_AUDIT_SCHEMA_V1: &str =
    "dusklight-native-tactic-throughput-treatment-audit/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticThroughputTreatmentMetrics {
    pub control_wall_micros: u64,
    pub treatment_wall_micros: u64,
    pub wall_speedup_millionths: u64,
    pub control_tactic_execution_micros: u64,
    pub treatment_tactic_execution_micros: u64,
    pub tactic_execution_speedup_millionths: u64,
    pub control_useful_expansions_per_second_millionths: u64,
    pub treatment_useful_expansions_per_second_millionths: u64,
    pub useful_expansion_throughput_speedup_millionths: u64,
    pub control_prefix_materializations: u64,
    pub treatment_prefix_materializations: u64,
    pub control_cache_evictions: u64,
    pub treatment_cache_evictions: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticThroughputTreatmentAudit {
    pub schema: String,
    pub content_sha256: Digest,
    pub control_report_sha256: Digest,
    pub treatment_report_sha256: Digest,
    pub execution_plan_sha256: Digest,
    pub platform_os: String,
    pub platform_arch: String,
    pub control_semantic_trace_sha256: Digest,
    pub treatment_semantic_trace_sha256: Digest,
    pub semantic_trace_equal: bool,
    pub campaign_identity_equal: bool,
    pub replay_authority_equal: bool,
    pub learner_authority_equal: bool,
    pub useful_expansion_set_equal: bool,
    pub fixed_work_equal: bool,
    pub resource_contract_equal: bool,
    pub graph_shape_equal: bool,
    pub terminal_result_equal: bool,
    pub metrics: NativeTacticThroughputTreatmentMetrics,
    pub passed: bool,
}

impl NativeTacticThroughputTreatmentAudit {
    pub fn build(
        control_bytes: &[u8],
        treatment_bytes: &[u8],
    ) -> Result<Self, NativeTacticRouteRunError> {
        let control: NativeTacticRouteReport =
            serde_json::from_slice(control_bytes).map_err(route_error)?;
        let treatment: NativeTacticRouteReport =
            serde_json::from_slice(treatment_bytes).map_err(route_error)?;
        if control.schema != NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V37
            || treatment.schema != NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V37
            || control.seeds.len() != 1
            || treatment.seeds.len() != 1
            || control.seeds[0].seed != treatment.seeds[0].seed
            || !control.timing.persistence_attribution_is_valid()
            || !treatment.timing.persistence_attribution_is_valid()
        {
            return Err(route_message(
                "throughput treatment requires matched single-seed v37 native tactic reports",
            ));
        }

        let control_seed = &control.seeds[0];
        let treatment_seed = &treatment.seeds[0];
        let control_semantic_trace_sha256 = semantic_trace_sha256_v2(&control_seed.trace)?;
        let treatment_semantic_trace_sha256 = semantic_trace_sha256_v2(&treatment_seed.trace)?;
        let semantic_trace_equal = control_semantic_trace_sha256 == treatment_semantic_trace_sha256;
        let campaign_identity_equal = campaign_identity_equal(&control, &treatment);
        // Attempts and duplicates are invocation telemetry, not replay
        // authority. The snapshot hash is also run-local because its chain
        // authenticates publisher-lane metadata. Cross-run semantic equality
        // is established independently below by the trace, learner authority,
        // replay row counts, useful expansion set, and graph shape.
        let replay_authority_equal = control.replay_revision == treatment.replay_revision
            && control.replay_admission.admitted == treatment.replay_admission.admitted;
        let learner_authority_equal = control.learner_authority == treatment.learner_authority;
        let useful_expansion_set_equal = control_seed.useful_graph_expansion_set_sha256
            == treatment_seed.useful_graph_expansion_set_sha256;
        let fixed_work_equal = fixed_work_equal(&control, &treatment);
        let resource_contract_equal = control.workers == treatment.workers
            && control.checkpoint_cache_capacity_per_worker_bytes
                == treatment.checkpoint_cache_capacity_per_worker_bytes
            && control.resource_budgets == treatment.resource_budgets;
        let graph_shape_equal = graph_shape_equal(control_seed, treatment_seed)?;
        let terminal_result_equal = terminal_result_equal(control_seed, treatment_seed);
        let metrics = treatment_metrics(&control, &treatment);
        let performance_improved = metrics.wall_speedup_millionths > 1_000_000
            && metrics.tactic_execution_speedup_millionths > 1_000_000
            && metrics.useful_expansion_throughput_speedup_millionths > 1_000_000
            && metrics.treatment_prefix_materializations < metrics.control_prefix_materializations
            && metrics.treatment_cache_evictions < metrics.control_cache_evictions;
        let passed = semantic_trace_equal
            && campaign_identity_equal
            && replay_authority_equal
            && learner_authority_equal
            && useful_expansion_set_equal
            && fixed_work_equal
            && resource_contract_equal
            && graph_shape_equal
            && terminal_result_equal
            && performance_improved;
        if !passed {
            return Err(route_message(format!(
                "native tactic throughput treatment differs: semantic_trace={semantic_trace_equal}, \
                 campaign_identity={campaign_identity_equal}, replay_authority={replay_authority_equal}, \
                 learner_authority={learner_authority_equal}, useful_expansion_set={useful_expansion_set_equal}, \
                 fixed_work={fixed_work_equal}, resource_contract={resource_contract_equal}, \
                 graph_shape={graph_shape_equal}, terminal_result={terminal_result_equal}, \
                 performance_improved={performance_improved}"
            )));
        }
        let mut audit = Self {
            schema: NATIVE_TACTIC_THROUGHPUT_TREATMENT_AUDIT_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            control_report_sha256: bytes_sha256(control_bytes),
            treatment_report_sha256: bytes_sha256(treatment_bytes),
            execution_plan_sha256: control.execution_plan_sha256,
            platform_os: std::env::consts::OS.into(),
            platform_arch: std::env::consts::ARCH.into(),
            control_semantic_trace_sha256,
            treatment_semantic_trace_sha256,
            semantic_trace_equal,
            campaign_identity_equal,
            replay_authority_equal,
            learner_authority_equal,
            useful_expansion_set_equal,
            fixed_work_equal,
            resource_contract_equal,
            graph_shape_equal,
            terminal_result_equal,
            metrics,
            passed,
        };
        audit.content_sha256 = audit.compute_content_sha256()?;
        audit.validate()?;
        Ok(audit)
    }

    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        let metrics = &self.metrics;
        if self.schema != NATIVE_TACTIC_THROUGHPUT_TREATMENT_AUDIT_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.control_report_sha256 == Digest::ZERO
            || self.treatment_report_sha256 == Digest::ZERO
            || self.execution_plan_sha256 == Digest::ZERO
            || !self.semantic_trace_equal
            || !self.campaign_identity_equal
            || !self.replay_authority_equal
            || !self.learner_authority_equal
            || !self.useful_expansion_set_equal
            || !self.fixed_work_equal
            || !self.resource_contract_equal
            || !self.graph_shape_equal
            || !self.terminal_result_equal
            || metrics.wall_speedup_millionths <= 1_000_000
            || metrics.tactic_execution_speedup_millionths <= 1_000_000
            || metrics.useful_expansion_throughput_speedup_millionths <= 1_000_000
            || metrics.treatment_prefix_materializations >= metrics.control_prefix_materializations
            || metrics.treatment_cache_evictions >= metrics.control_cache_evictions
            || !self.passed
            || self.content_sha256 != self.compute_content_sha256()?
        {
            return Err(route_message(
                "native tactic throughput treatment audit is invalid",
            ));
        }
        Ok(())
    }

    fn compute_content_sha256(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        Ok(Digest(
            Sha256::digest(serde_cbor::to_vec(&unsigned).map_err(route_error)?).into(),
        ))
    }
}

fn campaign_identity_equal(
    control: &NativeTacticRouteReport,
    treatment: &NativeTacticRouteReport,
) -> bool {
    control.optimization_request_sha256 == treatment.optimization_request_sha256
        && control.execution_binding_sha256 == treatment.execution_binding_sha256
        && control.execution_plan_sha256 == treatment.execution_plan_sha256
        && control.objective_sha256 == treatment.objective_sha256
        && control.feature_schema_sha256 == treatment.feature_schema_sha256
        && control.action_schema_sha256 == treatment.action_schema_sha256
        && control.goal_target == treatment.goal_target
        && control.reward_spec == treatment.reward_spec
        && control.demonstration_transitions == treatment.demonstration_transitions
        && control.exploration_seeds == treatment.exploration_seeds
        && control.proposal_policy == treatment.proposal_policy
        && control.value_treatment == treatment.value_treatment
        && control.execution_strategy == treatment.execution_strategy
        && control.decisions_per_seed == treatment.decisions_per_seed
        && control.refit_every_decisions == treatment.refit_every_decisions
}

fn fixed_work_equal(
    control: &NativeTacticRouteReport,
    treatment: &NativeTacticRouteReport,
) -> bool {
    let control_seed = &control.seeds[0];
    let treatment_seed = &treatment.seeds[0];
    control.total_native_ticks == treatment.total_native_ticks
        && control.total_decisions == treatment.total_decisions
        && control.useful_decisions == treatment.useful_decisions
        && control.unique_useful_graph_expansions == treatment.unique_useful_graph_expansions
        && control.training_replay_rows == treatment.training_replay_rows
        && control.shared_training_replay_rows == treatment.shared_training_replay_rows
        && control.duplicate_training_transitions == treatment.duplicate_training_transitions
        && control.censored_training_transitions == treatment.censored_training_transitions
        && control_seed.decisions == treatment_seed.decisions
        && control_seed.episodes == treatment_seed.episodes
        && control_seed.native_ticks == treatment_seed.native_ticks
        && control_seed.training_replay_rows == treatment_seed.training_replay_rows
        && control_seed.visited_states == treatment_seed.visited_states
        && control_seed.unique_useful_graph_expansions
            == treatment_seed.unique_useful_graph_expansions
}

fn graph_shape_equal(
    control: &NativeTacticSeedResult,
    treatment: &NativeTacticSeedResult,
) -> Result<bool, NativeTacticRouteRunError> {
    let control = control
        .graph_metrics
        .as_ref()
        .ok_or_else(|| route_message("control throughput report lacks graph metrics"))?;
    let treatment = treatment
        .graph_metrics
        .as_ref()
        .ok_or_else(|| route_message("treatment throughput report lacks graph metrics"))?;
    Ok(control.graph.nodes == treatment.graph.nodes
        && control.graph.observed_segments == treatment.graph.observed_segments
        && control.graph.untried_expansions == treatment.graph.untried_expansions
        && control.graph.leased_expansions == treatment.graph.leased_expansions
        && control.graph.retryable_expansions == treatment.graph.retryable_expansions
        && control.graph.completed_expansions == treatment.graph.completed_expansions
        && control.graph.failed_validation_expansions
            == treatment.graph.failed_validation_expansions
        && control.graph.best_terminal == treatment.graph.best_terminal
        && control.lease_accounting == treatment.lease_accounting)
}

fn terminal_result_equal(
    control: &NativeTacticSeedResult,
    treatment: &NativeTacticSeedResult,
) -> bool {
    control.terminal_discovered == treatment.terminal_discovered
        && control.best_authenticated_tick == treatment.best_authenticated_tick
        && control.best_terminal_state_sha256 == treatment.best_terminal_state_sha256
        && control.best_terminal_route_checkpoint_sha256
            == treatment.best_terminal_route_checkpoint_sha256
        && control.best_terminal_tape == treatment.best_terminal_tape
        && control.successful_tape == treatment.successful_tape
}

fn treatment_metrics(
    control: &NativeTacticRouteReport,
    treatment: &NativeTacticRouteReport,
) -> NativeTacticThroughputTreatmentMetrics {
    NativeTacticThroughputTreatmentMetrics {
        control_wall_micros: control.timing.wall_micros,
        treatment_wall_micros: treatment.timing.wall_micros,
        wall_speedup_millionths: ratio_millionths(
            control.timing.wall_micros,
            treatment.timing.wall_micros,
        ),
        control_tactic_execution_micros: control.timing.tactic_execution_micros,
        treatment_tactic_execution_micros: treatment.timing.tactic_execution_micros,
        tactic_execution_speedup_millionths: ratio_millionths(
            control.timing.tactic_execution_micros,
            treatment.timing.tactic_execution_micros,
        ),
        control_useful_expansions_per_second_millionths: control
            .timing
            .unique_useful_graph_expansions_per_second_millionths,
        treatment_useful_expansions_per_second_millionths: treatment
            .timing
            .unique_useful_graph_expansions_per_second_millionths,
        useful_expansion_throughput_speedup_millionths: ratio_millionths(
            treatment
                .timing
                .unique_useful_graph_expansions_per_second_millionths,
            control
                .timing
                .unique_useful_graph_expansions_per_second_millionths,
        ),
        control_prefix_materializations: control.native_restore_accounting.prefix_materializations,
        treatment_prefix_materializations: treatment
            .native_restore_accounting
            .prefix_materializations,
        control_cache_evictions: control.native_restore_accounting.cache_evictions,
        treatment_cache_evictions: treatment.native_restore_accounting.cache_evictions,
    }
}

fn ratio_millionths(numerator: u64, denominator: u64) -> u64 {
    if denominator == 0 {
        return 0;
    }
    u64::try_from(
        u128::from(numerator)
            .saturating_mul(1_000_000)
            .checked_div(u128::from(denominator))
            .unwrap_or(0)
            .min(u128::from(u64::MAX)),
    )
    .unwrap_or(u64::MAX)
}

fn bytes_sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_point_ratio_is_bounded_and_directional() {
        assert_eq!(ratio_millionths(200, 100), 2_000_000);
        assert_eq!(ratio_millionths(100, 200), 500_000);
        assert_eq!(ratio_millionths(100, 0), 0);
    }
}
