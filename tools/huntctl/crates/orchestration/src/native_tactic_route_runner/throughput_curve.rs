use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

pub const NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V1: &str =
    "dusklight-native-tactic-throughput-curve/v1";
pub const NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V2: &str =
    "dusklight-native-tactic-throughput-curve/v2";
pub const NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V3: &str =
    "dusklight-native-tactic-throughput-curve/v3";
pub const NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS: [usize; 5] = [1, 2, 4, 8, 16];
const MINIMUM_REPETITIONS: u32 = 2;
const MAXIMUM_REPETITIONS: u32 = 6;
const REPORT_FILE: &str = "throughput-curve.json";

#[derive(Clone, Debug)]
pub struct NativeTacticThroughputCurveConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub execution_plan: &'a NativeTacticExecutionPlan,
    pub output_root: &'a Path,
    pub repetitions: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticThroughputCurveSample {
    pub ordinal: u32,
    pub repetition: u32,
    pub workers: usize,
    pub route_report_path: String,
    pub route_report_sha256: Digest,
    pub state_graph_sha256s: Vec<Digest>,
    pub useful_graph_expansion_set_sha256s: Vec<Digest>,
    pub completed_decisions: u64,
    pub unique_useful_graph_expansions: u64,
    pub wall_micros: u64,
    pub process_launch_micros: u64,
    pub unique_useful_graph_expansions_per_second_millionths: u64,
    pub peak_worker_checkpoint_resident_bytes: u64,
    pub checkpoint_pool_resident_bytes_upper_bound: u64,
    pub maximum_model_replay_lag_revisions: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticThroughputCurveCell {
    pub workers: usize,
    pub samples: u32,
    pub median_wall_micros: u64,
    pub unique_useful_graph_expansions_per_second_millionths: u64,
    pub speedup_over_one_worker_millionths: u64,
    pub parallel_efficiency_millionths: u64,
    pub peak_checkpoint_pool_resident_bytes_upper_bound: u64,
    pub maximum_model_replay_lag_revisions: u64,
    pub throughput_increased_from_prior: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticThroughputCurveReport {
    pub schema: String,
    pub content_sha256: Digest,
    pub recorded_unix_millis: u128,
    pub operating_system: String,
    pub architecture: String,
    pub logical_cpu_count: usize,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub execution_plan_sha256: Digest,
    pub fleet_workers: usize,
    pub fleet_launch_micros: u64,
    pub repetitions: u32,
    pub worker_counts: Vec<usize>,
    pub fixed_completed_decisions: u64,
    pub fixed_unique_useful_graph_expansions: u64,
    pub checkpoint_pool_memory_bound_bytes: u64,
    pub maximum_allowed_stale_revisions: u64,
    pub execution_order: Vec<NativeTacticThroughputCurveSample>,
    pub curve: Vec<NativeTacticThroughputCurveCell>,
    pub fixed_work_satisfied: bool,
    pub identical_useful_expansion_evidence_satisfied: bool,
    pub memory_bound_satisfied: bool,
    pub learner_staleness_bound_satisfied: bool,
    pub strictly_increasing_throughput: bool,
    pub passed: bool,
}

impl NativeTacticThroughputCurveReport {
    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V3
            || self.recorded_unix_millis == 0
            || self.operating_system.is_empty()
            || self.architecture.is_empty()
            || self.logical_cpu_count == 0
            || self.optimization_request_sha256 == Digest::ZERO
            || self.execution_binding_sha256 == Digest::ZERO
            || self.execution_plan_sha256 == Digest::ZERO
            || self.fleet_workers
                != NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS
                    .last()
                    .copied()
                    .unwrap_or(0)
            || self.fleet_launch_micros == 0
            || !(MINIMUM_REPETITIONS..=MAXIMUM_REPETITIONS).contains(&self.repetitions)
            || self.worker_counts != NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS
            || self.fixed_completed_decisions == 0
            || self.fixed_unique_useful_graph_expansions == 0
            || self.checkpoint_pool_memory_bound_bytes == 0
        {
            return Err(route_message(
                "native tactic throughput curve identity is invalid",
            ));
        }
        let expected_samples = usize::try_from(self.repetitions)
            .map_err(route_error)?
            .checked_mul(NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS.len())
            .ok_or_else(|| route_message("throughput curve sample count overflows"))?;
        if self.execution_order.len() != expected_samples {
            return Err(route_message(
                "native tactic throughput curve sample count is invalid",
            ));
        }
        for (index, sample) in self.execution_order.iter().enumerate() {
            let expected_ordinal = u32::try_from(index + 1).map_err(route_error)?;
            let repetition =
                u32::try_from(index / NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS.len() + 1)
                    .map_err(route_error)?;
            let within = index % NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS.len();
            let expected_workers = if repetition % 2 == 1 {
                NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS[within]
            } else {
                NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS
                    [NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS.len() - 1 - within]
            };
            if sample.ordinal != expected_ordinal
                || sample.repetition != repetition
                || sample.workers != expected_workers
                || sample.route_report_path.is_empty()
                || sample.route_report_sha256 == Digest::ZERO
                || sample.state_graph_sha256s.is_empty()
                || sample.state_graph_sha256s.contains(&Digest::ZERO)
                || sample.useful_graph_expansion_set_sha256s.is_empty()
                || sample
                    .useful_graph_expansion_set_sha256s
                    .contains(&Digest::ZERO)
                || sample.wall_micros == 0
                || sample.process_launch_micros != 0
                || sample.unique_useful_graph_expansions_per_second_millionths
                    != per_second_millionths(
                        sample.unique_useful_graph_expansions,
                        sample.wall_micros,
                    )
                || sample.checkpoint_pool_resident_bytes_upper_bound
                    != sample
                        .peak_worker_checkpoint_resident_bytes
                        .checked_mul(sample.workers as u64)
                        .ok_or_else(|| {
                            route_message("throughput curve checkpoint memory overflows")
                        })?
            {
                return Err(route_message(
                    "native tactic throughput curve sample is invalid",
                ));
            }
        }
        let derived = DerivedCurve::from_samples(
            &self.execution_order,
            self.fixed_completed_decisions,
            self.fixed_unique_useful_graph_expansions,
            self.checkpoint_pool_memory_bound_bytes,
            self.maximum_allowed_stale_revisions,
        )?;
        if self.curve != derived.curve
            || self.fixed_work_satisfied != derived.fixed_work_satisfied
            || self.identical_useful_expansion_evidence_satisfied
                != derived.identical_useful_expansion_evidence_satisfied
            || self.memory_bound_satisfied != derived.memory_bound_satisfied
            || self.learner_staleness_bound_satisfied != derived.learner_staleness_bound_satisfied
            || self.strictly_increasing_throughput != derived.strictly_increasing_throughput
            || self.passed != derived.passed()
        {
            return Err(route_message(
                "native tactic throughput curve conclusions are stale",
            ));
        }
        if self.content_sha256 != self.compute_content_sha256()? {
            return Err(route_message(
                "native tactic throughput curve content digest is stale",
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

    fn refresh_content_sha256(&mut self) -> Result<(), NativeTacticRouteRunError> {
        self.content_sha256 = self.compute_content_sha256()?;
        Ok(())
    }

    fn compute_content_sha256(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut identity = self.clone();
        identity.content_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&identity).map_err(route_error)?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-tactic-throughput-curve/v3\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

struct DerivedCurve {
    curve: Vec<NativeTacticThroughputCurveCell>,
    fixed_work_satisfied: bool,
    identical_useful_expansion_evidence_satisfied: bool,
    memory_bound_satisfied: bool,
    learner_staleness_bound_satisfied: bool,
    strictly_increasing_throughput: bool,
}

impl DerivedCurve {
    fn from_samples(
        samples: &[NativeTacticThroughputCurveSample],
        fixed_decisions: u64,
        fixed_expansions: u64,
        memory_bound: u64,
        maximum_staleness: u64,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let baseline_expansion_sets = samples
            .first()
            .map(|sample| &sample.useful_graph_expansion_set_sha256s)
            .ok_or_else(|| route_message("throughput curve has no baseline sample"))?;
        let fixed_work_satisfied = samples.iter().all(|sample| {
            sample.completed_decisions == fixed_decisions
                && sample.unique_useful_graph_expansions == fixed_expansions
        });
        let identical_useful_expansion_evidence_satisfied = samples
            .iter()
            .all(|sample| sample.useful_graph_expansion_set_sha256s == *baseline_expansion_sets);
        let memory_bound_satisfied = samples
            .iter()
            .all(|sample| sample.checkpoint_pool_resident_bytes_upper_bound <= memory_bound);
        let learner_staleness_bound_satisfied = samples
            .iter()
            .all(|sample| sample.maximum_model_replay_lag_revisions <= maximum_staleness);
        let mut curve = Vec::with_capacity(NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS.len());
        for workers in NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS {
            let selected = samples
                .iter()
                .filter(|sample| sample.workers == workers)
                .collect::<Vec<_>>();
            if selected.is_empty() {
                return Err(route_message("throughput curve worker cell is absent"));
            }
            let mut wall = selected
                .iter()
                .map(|sample| sample.wall_micros)
                .collect::<Vec<_>>();
            wall.sort_unstable();
            let median_wall_micros = median(&wall)?;
            curve.push(NativeTacticThroughputCurveCell {
                workers,
                samples: u32::try_from(selected.len()).map_err(route_error)?,
                median_wall_micros,
                unique_useful_graph_expansions_per_second_millionths: per_second_millionths(
                    fixed_expansions,
                    median_wall_micros,
                ),
                speedup_over_one_worker_millionths: 0,
                parallel_efficiency_millionths: 0,
                peak_checkpoint_pool_resident_bytes_upper_bound: selected
                    .iter()
                    .map(|sample| sample.checkpoint_pool_resident_bytes_upper_bound)
                    .max()
                    .unwrap_or(0),
                maximum_model_replay_lag_revisions: selected
                    .iter()
                    .map(|sample| sample.maximum_model_replay_lag_revisions)
                    .max()
                    .unwrap_or(0),
                throughput_increased_from_prior: None,
            });
        }
        let baseline_wall = curve[0].median_wall_micros;
        for index in 0..curve.len() {
            let speedup = ratio_per_million(baseline_wall, curve[index].median_wall_micros);
            curve[index].speedup_over_one_worker_millionths = speedup;
            curve[index].parallel_efficiency_millionths =
                speedup / u64::try_from(curve[index].workers).map_err(route_error)?;
            if index != 0 {
                curve[index].throughput_increased_from_prior = Some(
                    curve[index].unique_useful_graph_expansions_per_second_millionths
                        > curve[index - 1].unique_useful_graph_expansions_per_second_millionths,
                );
            }
        }
        let strictly_increasing_throughput = curve
            .iter()
            .skip(1)
            .all(|cell| cell.throughput_increased_from_prior == Some(true));
        Ok(Self {
            curve,
            fixed_work_satisfied,
            identical_useful_expansion_evidence_satisfied,
            memory_bound_satisfied,
            learner_staleness_bound_satisfied,
            strictly_increasing_throughput,
        })
    }

    fn passed(&self) -> bool {
        self.fixed_work_satisfied
            && self.identical_useful_expansion_evidence_satisfied
            && self.memory_bound_satisfied
            && self.learner_staleness_bound_satisfied
            && self.strictly_increasing_throughput
    }
}

pub fn run_native_tactic_throughput_curve(
    config: &NativeTacticThroughputCurveConfig<'_>,
) -> Result<NativeTacticThroughputCurveReport, NativeTacticRouteRunError> {
    validate_curve_config(config)?;
    if let Some(parent) = config
        .output_root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(route_error)?;
    }
    fs::create_dir(config.output_root).map_err(route_error)?;
    let execution_plan_sha256 = config.execution_plan.identity()?;
    let fixed_completed_decisions = config
        .execution_plan
        .budgets
        .decisions_per_lane
        .checked_mul(config.execution_plan.lanes.len() as u64)
        .ok_or_else(|| route_message("fixed throughput decision target overflows"))?;
    let fixed_unique_useful_graph_expansions = fixed_completed_decisions
        .checked_mul(config.execution_plan.proposal_width_per_decision as u64)
        .ok_or_else(|| route_message("fixed throughput expansion target overflows"))?;
    let checkpoint_pool_memory_bound_bytes = match config.execution_plan.budgets.memory_bytes {
        NativeTacticResourceLimit::Bounded(bytes) => bytes,
        NativeTacticResourceLimit::Unbounded => {
            return Err(route_message(
                "throughput curve requires a bounded checkpoint memory budget",
            ));
        }
    };
    let maximum_allowed_stale_revisions = match config.execution_plan.replay_sharing {
        NativeTacticReplaySharingPlan::BoundedStaleness {
            maximum_stale_replay_revisions,
        } => maximum_stale_replay_revisions,
        NativeTacticReplaySharingPlan::GenerationBarrier => 0,
    };
    let fleet_workers = NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS
        .last()
        .copied()
        .ok_or_else(|| route_message("throughput curve worker topology is empty"))?;
    let fleet_root = config.output_root.join("worker-fleet");
    let fleet_config = NativeTacticRouteRunConfig {
        repository_root: config.repository_root,
        optimization: config.optimization,
        execution: config.execution,
        execution_plan: config.execution_plan,
        promoted_tactic_registry: None,
        output_root: &fleet_root,
        workers: fleet_workers,
        cancellation: None,
        fault_injection: None,
        resume: false,
    };
    let fleet = launch_native_tactic_worker_fleet(&fleet_config, &fleet_root, fleet_workers)?;
    let fleet_launch_micros = fleet.launch_micros();
    let mut execution_order = Vec::new();
    for repetition in 1..=config.repetitions {
        let worker_order = if repetition % 2 == 1 {
            NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS.to_vec()
        } else {
            NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS
                .iter()
                .rev()
                .copied()
                .collect()
        };
        for workers in worker_order {
            let ordinal = u32::try_from(execution_order.len() + 1).map_err(route_error)?;
            let sample_root = config
                .output_root
                .join(format!("sample-{ordinal:02}-r{repetition}-w{workers}"));
            let route_report = run_native_tactic_route_with_fleet(
                &NativeTacticRouteRunConfig {
                    repository_root: config.repository_root,
                    optimization: config.optimization,
                    execution: config.execution,
                    execution_plan: config.execution_plan,
                    promoted_tactic_registry: None,
                    output_root: &sample_root,
                    workers,
                    cancellation: None,
                    fault_injection: None,
                    resume: false,
                },
                &fleet,
            )?;
            let route_report_path = sample_root.join("report.json");
            let route_report_bytes = fs::read(&route_report_path).map_err(route_error)?;
            let route_report_sha256 = Digest(Sha256::digest(route_report_bytes).into());
            let peak_worker_checkpoint_resident_bytes =
                route_report.native_restore_accounting.peak_resident_bytes;
            execution_order.push(NativeTacticThroughputCurveSample {
                ordinal,
                repetition,
                workers,
                route_report_path: path_text(&route_report_path),
                route_report_sha256,
                state_graph_sha256s: route_report
                    .seeds
                    .iter()
                    .map(|seed| seed.state_graph_sha256)
                    .collect(),
                useful_graph_expansion_set_sha256s: route_report
                    .seeds
                    .iter()
                    .map(|seed| seed.useful_graph_expansion_set_sha256)
                    .collect(),
                completed_decisions: route_report.total_decisions,
                unique_useful_graph_expansions: route_report.unique_useful_graph_expansions,
                wall_micros: route_report.timing.wall_micros,
                process_launch_micros: route_report.timing.process_launch_micros,
                unique_useful_graph_expansions_per_second_millionths: route_report
                    .timing
                    .unique_useful_graph_expansions_per_second_millionths,
                peak_worker_checkpoint_resident_bytes,
                checkpoint_pool_resident_bytes_upper_bound: peak_worker_checkpoint_resident_bytes
                    .saturating_mul(workers as u64),
                maximum_model_replay_lag_revisions: route_report
                    .replay_sharing
                    .maximum_model_replay_lag_revisions,
            });
        }
    }
    fleet.shutdown()?;
    let derived = DerivedCurve::from_samples(
        &execution_order,
        fixed_completed_decisions,
        fixed_unique_useful_graph_expansions,
        checkpoint_pool_memory_bound_bytes,
        maximum_allowed_stale_revisions,
    )?;
    let passed = derived.passed();
    let mut report = NativeTacticThroughputCurveReport {
        schema: NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V3.into(),
        content_sha256: Digest::ZERO,
        recorded_unix_millis: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(route_error)?
            .as_millis(),
        operating_system: std::env::consts::OS.into(),
        architecture: std::env::consts::ARCH.into(),
        logical_cpu_count: std::thread::available_parallelism()
            .map_err(route_error)?
            .get(),
        optimization_request_sha256: config.optimization.content_sha256,
        execution_binding_sha256: config.execution.content_sha256,
        execution_plan_sha256,
        fleet_workers,
        fleet_launch_micros,
        repetitions: config.repetitions,
        worker_counts: NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS.to_vec(),
        fixed_completed_decisions,
        fixed_unique_useful_graph_expansions,
        checkpoint_pool_memory_bound_bytes,
        maximum_allowed_stale_revisions,
        execution_order,
        curve: derived.curve,
        fixed_work_satisfied: derived.fixed_work_satisfied,
        identical_useful_expansion_evidence_satisfied: derived
            .identical_useful_expansion_evidence_satisfied,
        memory_bound_satisfied: derived.memory_bound_satisfied,
        learner_staleness_bound_satisfied: derived.learner_staleness_bound_satisfied,
        strictly_increasing_throughput: derived.strictly_increasing_throughput,
        passed,
    };
    report.refresh_content_sha256()?;
    report.validate()?;
    write_new(
        &config.output_root.join(REPORT_FILE),
        &report.to_pretty_json()?,
    )?;
    Ok(report)
}

fn validate_curve_config(
    config: &NativeTacticThroughputCurveConfig<'_>,
) -> Result<(), NativeTacticRouteRunError> {
    config.execution_plan.validate()?;
    if !(MINIMUM_REPETITIONS..=MAXIMUM_REPETITIONS).contains(&config.repetitions)
        || config.output_root.exists()
        || config.execution_plan.lanes.len() != 1
        || config.execution_plan.generations.len() != 1
        || config.execution_plan.proposal_width_per_decision
            != NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS
                .last()
                .copied()
                .unwrap_or(0)
        || config.execution_plan.demonstration_chunk_ticks.is_some()
        || config
            .execution_plan
            .promoted_tactic_registry_sha256
            .is_some()
        || !matches!(
            config.execution_plan.budgets.memory_bytes,
            NativeTacticResourceLimit::Bounded(bytes) if bytes > 0
        )
    {
        return Err(route_message(
            "native tactic throughput curve configuration is invalid",
        ));
    }
    Ok(())
}

fn median(sorted: &[u64]) -> Result<u64, NativeTacticRouteRunError> {
    if sorted.is_empty() {
        return Err(route_message("throughput curve median has no samples"));
    }
    let midpoint = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        Ok(sorted[midpoint])
    } else {
        let lower = sorted[midpoint - 1];
        Ok(lower.saturating_add(sorted[midpoint].saturating_sub(lower) / 2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(
        ordinal: u32,
        repetition: u32,
        workers: usize,
        wall_micros: u64,
    ) -> NativeTacticThroughputCurveSample {
        NativeTacticThroughputCurveSample {
            ordinal,
            repetition,
            workers,
            route_report_path: format!("sample-{ordinal}/report.json"),
            route_report_sha256: Digest([ordinal as u8; 32]),
            state_graph_sha256s: vec![Digest([9; 32])],
            useful_graph_expansion_set_sha256s: vec![Digest([8; 32])],
            completed_decisions: 4,
            unique_useful_graph_expansions: 64,
            wall_micros,
            process_launch_micros: 0,
            unique_useful_graph_expansions_per_second_millionths: per_second_millionths(
                64,
                wall_micros,
            ),
            peak_worker_checkpoint_resident_bytes: 100,
            checkpoint_pool_resident_bytes_upper_bound: 100 * workers as u64,
            maximum_model_replay_lag_revisions: 2,
        }
    }

    #[test]
    fn fixed_graph_work_builds_a_strict_scaling_curve() {
        let order = [1, 2, 4, 8, 16, 16, 8, 4, 2, 1];
        let samples = order
            .into_iter()
            .enumerate()
            .map(|(index, workers)| {
                let repetition = if index < 5 { 1 } else { 2 };
                sample(
                    index as u32 + 1,
                    repetition,
                    workers,
                    16_000_000 / workers as u64,
                )
            })
            .collect::<Vec<_>>();
        let derived = DerivedCurve::from_samples(&samples, 4, 64, 2_000, 2).unwrap();
        assert!(derived.fixed_work_satisfied);
        assert!(derived.identical_useful_expansion_evidence_satisfied);
        assert!(derived.memory_bound_satisfied);
        assert!(derived.learner_staleness_bound_satisfied);
        assert!(derived.strictly_increasing_throughput);
        assert!(derived.passed());
        assert_eq!(
            derived.curve[4].speedup_over_one_worker_millionths,
            16_000_000
        );
        assert_eq!(derived.curve[4].parallel_efficiency_millionths, 1_000_000);
    }

    #[test]
    fn raw_tick_speed_cannot_hide_non_scaling_useful_work() {
        let order = [1, 2, 4, 8, 16, 16, 8, 4, 2, 1];
        let mut samples = order
            .into_iter()
            .enumerate()
            .map(|(index, workers)| {
                let repetition = if index < 5 { 1 } else { 2 };
                sample(index as u32 + 1, repetition, workers, 1_000_000)
            })
            .collect::<Vec<_>>();
        samples[4].unique_useful_graph_expansions = 63;
        samples[4].unique_useful_graph_expansions_per_second_millionths =
            per_second_millionths(63, samples[4].wall_micros);
        let derived = DerivedCurve::from_samples(&samples, 4, 64, 2_000, 2).unwrap();
        assert!(!derived.fixed_work_satisfied);
        assert!(!derived.strictly_increasing_throughput);
        assert!(!derived.passed());
    }

    #[test]
    fn persistent_curve_rejects_per_sample_process_relaunch() {
        let order = [1, 2, 4, 8, 16, 16, 8, 4, 2, 1];
        let samples = order
            .into_iter()
            .enumerate()
            .map(|(index, workers)| {
                sample(
                    index as u32 + 1,
                    if index < 5 { 1 } else { 2 },
                    workers,
                    16_000_000 / workers as u64,
                )
            })
            .collect::<Vec<_>>();
        let derived = DerivedCurve::from_samples(&samples, 4, 64, 2_000, 2).unwrap();
        let passed = derived.passed();
        let mut report = NativeTacticThroughputCurveReport {
            schema: NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V3.into(),
            content_sha256: Digest::ZERO,
            recorded_unix_millis: 1,
            operating_system: "test".into(),
            architecture: "test".into(),
            logical_cpu_count: 16,
            optimization_request_sha256: Digest([1; 32]),
            execution_binding_sha256: Digest([2; 32]),
            execution_plan_sha256: Digest([3; 32]),
            fleet_workers: 16,
            fleet_launch_micros: 10,
            repetitions: 2,
            worker_counts: NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS.to_vec(),
            fixed_completed_decisions: 4,
            fixed_unique_useful_graph_expansions: 64,
            checkpoint_pool_memory_bound_bytes: 2_000,
            maximum_allowed_stale_revisions: 2,
            execution_order: samples,
            curve: derived.curve,
            fixed_work_satisfied: derived.fixed_work_satisfied,
            identical_useful_expansion_evidence_satisfied: derived
                .identical_useful_expansion_evidence_satisfied,
            memory_bound_satisfied: derived.memory_bound_satisfied,
            learner_staleness_bound_satisfied: derived.learner_staleness_bound_satisfied,
            strictly_increasing_throughput: derived.strictly_increasing_throughput,
            passed,
        };
        report.refresh_content_sha256().unwrap();
        report.validate().unwrap();

        report.execution_order[0].process_launch_micros = 1;
        report.refresh_content_sha256().unwrap();
        assert!(report.validate().is_err());
    }
}
