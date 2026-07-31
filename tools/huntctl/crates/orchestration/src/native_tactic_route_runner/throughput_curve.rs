use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

pub const NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V1: &str =
    "dusklight-native-tactic-throughput-curve/v1";
pub const NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V2: &str =
    "dusklight-native-tactic-throughput-curve/v2";
pub const NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V3: &str =
    "dusklight-native-tactic-throughput-curve/v3";
pub const NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V4: &str =
    "dusklight-native-tactic-throughput-curve/v4";
pub const NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS: [usize; 5] = [1, 2, 4, 8, 16];
const MINIMUM_LONG_WORK_DECISIONS: u64 = 16;
const MINIMUM_REPETITIONS: u32 = 2;
const MAXIMUM_REPETITIONS: u32 = 6;
const REPORT_FILE: &str = "throughput-curve.json";
const PROGRESS_DIRECTORY: &str = "throughput-progress";
const RUN_COMMIT_FILE: &str = "run.dtcr";
const RUN_COMMIT_SCHEMA_V1: &str = "dusklight-native-tactic-throughput-run-commit/v1";
const SAMPLE_COMMIT_SCHEMA_V1: &str = "dusklight-native-tactic-throughput-sample-commit/v1";
const FLEET_LAUNCH_COMMIT_SCHEMA_V1: &str =
    "dusklight-native-tactic-throughput-fleet-launch-commit/v1";
const MAXIMUM_PROGRESS_RECORD_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct NativeTacticThroughputCurveConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub execution_plan: &'a NativeTacticExecutionPlan,
    pub output_root: &'a Path,
    pub repetitions: u32,
    pub resume: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum NativeTacticThroughputCurveRun {
    Complete {
        report: Box<NativeTacticThroughputCurveReport>,
    },
    StoppedAfterSample {
        completed_samples: u32,
        total_samples: u32,
        last_sample: NativeTacticThroughputCurveSample,
    },
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
    pub learner_updates: u64,
    pub model_snapshots_published: u64,
    pub training_replay_rows: u64,
    pub restore_samples: u64,
    pub non_root_restore_requests: u64,
    pub direct_restore_fallback_replays: u64,
    pub cache_evictions: u64,
    pub tactic_selection_micros: u64,
    pub checkpoint_branching_micros: u64,
    pub tactic_execution_micros: u64,
    pub native_simulation_micros: u64,
    pub ipc_and_result_transport_micros: u64,
    pub tactic_preparation_and_fact_extraction_micros: u64,
    pub model_update_micros: u64,
    pub evidence_projection_micros: u64,
    pub persistence_micros: u64,
    pub orchestration_micros: u64,
    pub result_validation_and_fact_extraction_micros: u64,
    pub graph_admission_micros: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign_admission_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign_admission_breakdown: Option<NativeTacticCampaignAdmissionTiming>,
    pub native_worker_occupancy_per_million: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeTacticThroughputRunCommit {
    schema: String,
    content_sha256: Digest,
    optimization_request_sha256: Digest,
    execution_binding_sha256: Digest,
    execution_plan_sha256: Digest,
    repetitions: u32,
    worker_counts: Vec<usize>,
}

impl NativeTacticThroughputRunCommit {
    fn new(
        config: &NativeTacticThroughputCurveConfig<'_>,
        execution_plan_sha256: Digest,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let mut commit = Self {
            schema: RUN_COMMIT_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: config.optimization.content_sha256,
            execution_binding_sha256: config.execution.content_sha256,
            execution_plan_sha256,
            repetitions: config.repetitions,
            worker_counts: NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS.to_vec(),
        };
        commit.content_sha256 = commit.compute_content_sha256()?;
        Ok(commit)
    }

    fn validate(
        &self,
        config: &NativeTacticThroughputCurveConfig<'_>,
        execution_plan_sha256: Digest,
    ) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != RUN_COMMIT_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
            || self.optimization_request_sha256 != config.optimization.content_sha256
            || self.execution_binding_sha256 != config.execution.content_sha256
            || self.execution_plan_sha256 != execution_plan_sha256
            || self.repetitions != config.repetitions
            || self.worker_counts != NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS
        {
            return Err(route_message(
                "native tactic throughput run commit is detached",
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeTacticThroughputSampleCommit {
    schema: String,
    content_sha256: Digest,
    execution_plan_sha256: Digest,
    sample: NativeTacticThroughputCurveSample,
}

impl NativeTacticThroughputSampleCommit {
    fn new(
        execution_plan_sha256: Digest,
        sample: NativeTacticThroughputCurveSample,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let mut commit = Self {
            schema: SAMPLE_COMMIT_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            execution_plan_sha256,
            sample,
        };
        commit.content_sha256 = commit.compute_content_sha256()?;
        Ok(commit)
    }

    fn validate(
        &self,
        execution_plan_sha256: Digest,
        ordinal: u32,
        repetition: u32,
        workers: usize,
        route_report_path: &Path,
    ) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != SAMPLE_COMMIT_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
            || self.execution_plan_sha256 != execution_plan_sha256
            || self.sample.ordinal != ordinal
            || self.sample.repetition != repetition
            || self.sample.workers != workers
            || self.sample.route_report_path != path_text(route_report_path)
        {
            return Err(route_message(
                "native tactic throughput sample commit is detached",
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeTacticThroughputFleetLaunchCommit {
    schema: String,
    content_sha256: Digest,
    execution_plan_sha256: Digest,
    fleet_workers: usize,
    launch_micros: u64,
}

impl NativeTacticThroughputFleetLaunchCommit {
    fn new(
        execution_plan_sha256: Digest,
        fleet_workers: usize,
        launch_micros: u64,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let mut commit = Self {
            schema: FLEET_LAUNCH_COMMIT_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            execution_plan_sha256,
            fleet_workers,
            launch_micros,
        };
        commit.content_sha256 = commit.compute_content_sha256()?;
        Ok(commit)
    }

    fn validate(
        &self,
        execution_plan_sha256: Digest,
        fleet_workers: usize,
    ) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != FLEET_LAUNCH_COMMIT_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
            || self.execution_plan_sha256 != execution_plan_sha256
            || self.fleet_workers != fleet_workers
            || self.launch_micros == 0
        {
            return Err(route_message(
                "native tactic throughput fleet launch commit is detached",
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
    pub long_work_exercised: bool,
    pub strictly_increasing_throughput: bool,
    pub passed: bool,
}

impl NativeTacticThroughputCurveReport {
    pub fn read_and_validate(path: &Path) -> Result<Self, NativeTacticRouteRunError> {
        let report: Self = serde_json::from_slice(&read_bounded_regular_file(
            path,
            MAX_RESUME_JSON_BYTES,
            "native tactic throughput curve report",
        )?)
        .map_err(route_error)?;
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V4
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
            || self.fixed_completed_decisions < MINIMUM_LONG_WORK_DECISIONS
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
                || sample.native_worker_occupancy_per_million
                    != ratio_per_million(
                        sample.native_simulation_micros,
                        sample.wall_micros.saturating_mul(sample.workers as u64),
                    )
                || !sample_admission_timing_is_valid(sample)
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
            || self.long_work_exercised != derived.long_work_exercised
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
        hasher.update(b"dusklight.native-tactic-throughput-curve/v4\0");
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
    long_work_exercised: bool,
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
        let long_work_exercised = fixed_decisions >= MINIMUM_LONG_WORK_DECISIONS
            && samples.iter().all(|sample| {
                sample.learner_updates >= 2
                    && sample.model_snapshots_published >= 2
                    && sample.training_replay_rows > 0
                    && sample.restore_samples > sample.completed_decisions
                    && sample.non_root_restore_requests > 0
                    && sample.cache_evictions > 0
                    && sample.model_update_micros > 0
                    && sample.evidence_projection_micros > 0
                    && sample.persistence_micros > 0
                    && sample.graph_admission_micros > 0
            });
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
            long_work_exercised,
            strictly_increasing_throughput,
        })
    }

    fn passed(&self) -> bool {
        self.fixed_work_satisfied
            && self.identical_useful_expansion_evidence_satisfied
            && self.memory_bound_satisfied
            && self.learner_staleness_bound_satisfied
            && self.long_work_exercised
            && self.strictly_increasing_throughput
    }
}

pub fn run_native_tactic_throughput_curve(
    config: &NativeTacticThroughputCurveConfig<'_>,
) -> Result<NativeTacticThroughputCurveReport, NativeTacticRouteRunError> {
    match run_native_tactic_throughput_curve_controlled(config, None)? {
        NativeTacticThroughputCurveRun::Complete { report } => Ok(*report),
        NativeTacticThroughputCurveRun::StoppedAfterSample { .. } => Err(route_message(
            "unbounded native tactic throughput curve stopped unexpectedly",
        )),
    }
}

pub fn run_native_tactic_throughput_curve_controlled(
    config: &NativeTacticThroughputCurveConfig<'_>,
    stop_after_sample: Option<u32>,
) -> Result<NativeTacticThroughputCurveRun, NativeTacticRouteRunError> {
    validate_curve_config(config)?;
    let execution_plan_sha256 = config.execution_plan.identity()?;
    let report_path = config.output_root.join(REPORT_FILE);
    prepare_curve_output(config)?;
    let report_exists = regular_file_presence(
        &report_path,
        "native tactic throughput curve aggregate report",
    )?;
    let progress_root = config.output_root.join(PROGRESS_DIRECTORY);
    prepare_curve_run_commit(config, &progress_root, execution_plan_sha256)?;
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
    let schedule = throughput_sample_schedule(config.repetitions)?;
    validate_stop_after_sample(stop_after_sample, schedule.len())?;
    reject_detached_sample_roots(config.output_root, &schedule)?;
    let mut execution_order = Vec::new();
    let mut first_unfinished = schedule.len();
    for (index, &(ordinal, repetition, workers)) in schedule.iter().enumerate() {
        match load_or_commit_completed_sample(
            config,
            &progress_root,
            execution_plan_sha256,
            ordinal,
            repetition,
            workers,
        )? {
            Some(sample) => execution_order.push(sample),
            None => {
                first_unfinished = index;
                break;
            }
        }
    }
    reject_non_prefix_progress(
        config.output_root,
        &progress_root,
        &schedule,
        first_unfinished,
    )?;
    if report_exists {
        if first_unfinished != schedule.len() {
            return Err(route_message(
                "completed native tactic throughput curve has incomplete sample evidence",
            ));
        }
        let fleet_launch_micros =
            read_fleet_launch_micros(&progress_root, execution_plan_sha256, fleet_workers)?;
        let report = NativeTacticThroughputCurveReport::read_and_validate(&report_path)?;
        validate_resumed_curve_report(
            config,
            execution_plan_sha256,
            fleet_launch_micros,
            &execution_order,
            &report,
        )?;
        return Ok(NativeTacticThroughputCurveRun::Complete {
            report: Box::new(report),
        });
    }
    if stop_after_sample.is_some_and(|limit| execution_order.len() >= limit as usize) {
        return stopped_curve_run(&schedule, &execution_order);
    }

    if let Some(first_unfinished) = curve_fleet_launch_start(first_unfinished, schedule.len())? {
        let fleet_root = config.output_root.join("worker-fleet");
        let fleet_config = NativeTacticRouteRunConfig {
            repository_root: config.repository_root,
            optimization: config.optimization,
            execution: config.execution,
            execution_plan: config.execution_plan,
            promoted_tactic_registry: None,
            output_root: &fleet_root,
            checkpoint_capacity_workers: fleet_workers,
            workers: fleet_workers,
            cancellation: None,
            fault_injection: None,
            resume: config.resume,
        };
        let fleet = launch_native_tactic_worker_fleet(&fleet_config, &fleet_root, fleet_workers)?;
        append_fleet_launch_commit(
            &progress_root,
            execution_plan_sha256,
            fleet_workers,
            fleet.launch_micros(),
        )?;
        for &(ordinal, repetition, workers) in &schedule[first_unfinished..] {
            let sample_root = sample_root(config.output_root, ordinal, repetition, workers);
            let resume_sample = sample_root.exists();
            let route_report = run_native_tactic_route_with_fleet(
                &NativeTacticRouteRunConfig {
                    repository_root: config.repository_root,
                    optimization: config.optimization,
                    execution: config.execution,
                    execution_plan: config.execution_plan,
                    promoted_tactic_registry: None,
                    output_root: &sample_root,
                    checkpoint_capacity_workers: fleet_workers,
                    workers,
                    cancellation: None,
                    fault_injection: None,
                    resume: resume_sample,
                },
                &fleet,
            )?;
            let route_report_path = sample_root.join("report.json");
            let route_report_bytes = fs::read(&route_report_path).map_err(route_error)?;
            let route_report_sha256 = Digest(Sha256::digest(route_report_bytes).into());
            let sample = sample_from_route_report(
                ordinal,
                repetition,
                workers,
                path_text(&route_report_path),
                route_report_sha256,
                &route_report,
            );
            validate_completed_sample(config, execution_plan_sha256, &sample, &route_report)?;
            write_sample_commit(&progress_root, execution_plan_sha256, sample.clone())?;
            execution_order.push(sample);
            if stop_after_sample.is_some_and(|limit| execution_order.len() >= limit as usize) {
                fleet.shutdown()?;
                return stopped_curve_run(&schedule, &execution_order);
            }
        }
        fleet.shutdown()?;
    }
    let fleet_launch_micros =
        read_fleet_launch_micros(&progress_root, execution_plan_sha256, fleet_workers)?;
    let derived = DerivedCurve::from_samples(
        &execution_order,
        fixed_completed_decisions,
        fixed_unique_useful_graph_expansions,
        checkpoint_pool_memory_bound_bytes,
        maximum_allowed_stale_revisions,
    )?;
    let passed = derived.passed();
    let mut report = NativeTacticThroughputCurveReport {
        schema: NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V4.into(),
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
        long_work_exercised: derived.long_work_exercised,
        strictly_increasing_throughput: derived.strictly_increasing_throughput,
        passed,
    };
    report.refresh_content_sha256()?;
    report.validate()?;
    publish_curve_report(&report_path, &report.to_pretty_json()?)?;
    Ok(NativeTacticThroughputCurveRun::Complete {
        report: Box::new(report),
    })
}

fn validate_stop_after_sample(
    stop_after_sample: Option<u32>,
    sample_count: usize,
) -> Result<(), NativeTacticRouteRunError> {
    if stop_after_sample.is_some_and(|ordinal| ordinal == 0 || ordinal as usize > sample_count) {
        return Err(route_message(
            "native tactic throughput stop sample is outside its sample schedule",
        ));
    }
    Ok(())
}

fn stopped_curve_run(
    schedule: &[(u32, u32, usize)],
    execution_order: &[NativeTacticThroughputCurveSample],
) -> Result<NativeTacticThroughputCurveRun, NativeTacticRouteRunError> {
    let last_sample = execution_order
        .last()
        .cloned()
        .ok_or_else(|| route_message("native tactic throughput stop has no committed sample"))?;
    Ok(NativeTacticThroughputCurveRun::StoppedAfterSample {
        completed_samples: execution_order.len().try_into().map_err(route_error)?,
        total_samples: schedule.len().try_into().map_err(route_error)?,
        last_sample,
    })
}

fn publish_curve_report(report_path: &Path, bytes: &[u8]) -> Result<(), NativeTacticRouteRunError> {
    let parent = report_path
        .parent()
        .ok_or_else(|| route_message("native tactic throughput report has no parent"))?;
    let file_name = report_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| route_message("native tactic throughput report filename is invalid"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(route_error)?
        .as_nanos();
    let partial = parent.join(format!(
        ".{file_name}.{}.{}.partial",
        std::process::id(),
        nonce
    ));
    write_new(&partial, bytes)?;
    fs::rename(&partial, report_path).map_err(route_error)?;
    sync_curve_report_parent(parent)
}

#[cfg(not(windows))]
fn sync_curve_report_parent(parent: &Path) -> Result<(), NativeTacticRouteRunError> {
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(route_error)
}

#[cfg(windows)]
fn sync_curve_report_parent(parent: &Path) -> Result<(), NativeTacticRouteRunError> {
    // Standard Windows file opening rejects directory handles. The report
    // file itself is flushed before the atomic rename; validate the parent
    // instead of converting a successful publication into ERROR_ACCESS_DENIED.
    if !parent.is_dir() {
        return Err(route_message(
            "native tactic throughput report parent is not a directory",
        ));
    }
    Ok(())
}

fn prepare_curve_output(
    config: &NativeTacticThroughputCurveConfig<'_>,
) -> Result<(), NativeTacticRouteRunError> {
    if path_exists_without_following(config.output_root)? {
        if !config.resume {
            return Err(route_message(
                "native tactic throughput curve output already exists; pass --resume",
            ));
        }
        let metadata = fs::symlink_metadata(config.output_root).map_err(route_error)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(route_message(
                "native tactic throughput curve output is not a physical directory",
            ));
        }
        return Ok(());
    }
    if config.resume {
        return Err(route_message(
            "native tactic throughput curve resume output does not exist",
        ));
    }
    if let Some(parent) = config
        .output_root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(route_error)?;
    }
    fs::create_dir(config.output_root).map_err(route_error)
}

fn prepare_curve_run_commit(
    config: &NativeTacticThroughputCurveConfig<'_>,
    progress_root: &Path,
    execution_plan_sha256: Digest,
) -> Result<(), NativeTacticRouteRunError> {
    let path = progress_root.join(RUN_COMMIT_FILE);
    let expected = NativeTacticThroughputRunCommit::new(config, execution_plan_sha256)?;
    if path_exists_without_following(progress_root)? {
        let metadata = fs::symlink_metadata(progress_root).map_err(route_error)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(route_message(
                "native tactic throughput progress root is not a physical directory",
            ));
        }
    }
    if regular_file_presence(&path, "native tactic throughput run commit")? {
        let actual: NativeTacticThroughputRunCommit = read_progress_record(&path)?;
        actual.validate(config, execution_plan_sha256)?;
        if actual != expected {
            return Err(route_message(
                "native tactic throughput run commit differs from its request",
            ));
        }
    } else {
        let output_has_artifacts = fs::read_dir(config.output_root)
            .map_err(route_error)?
            .next()
            .transpose()
            .map_err(route_error)?
            .is_some();
        if config.resume && output_has_artifacts {
            return Err(route_message(
                "legacy native tactic throughput progress has no authenticated run commit",
            ));
        }
        fs::create_dir_all(progress_root).map_err(route_error)?;
        write_progress_record(&path, &expected)?;
    }
    Ok(())
}

fn validate_resumed_curve_report(
    config: &NativeTacticThroughputCurveConfig<'_>,
    execution_plan_sha256: Digest,
    fleet_launch_micros: u64,
    execution_order: &[NativeTacticThroughputCurveSample],
    report: &NativeTacticThroughputCurveReport,
) -> Result<(), NativeTacticRouteRunError> {
    if report.optimization_request_sha256 != config.optimization.content_sha256
        || report.execution_binding_sha256 != config.execution.content_sha256
        || report.execution_plan_sha256 != execution_plan_sha256
        || report.repetitions != config.repetitions
        || report.worker_counts != NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS
        || report.fleet_launch_micros != fleet_launch_micros
        || report.execution_order != execution_order
    {
        return Err(route_message(
            "completed native tactic throughput curve is detached from resume",
        ));
    }
    Ok(())
}

fn throughput_sample_schedule(
    repetitions: u32,
) -> Result<Vec<(u32, u32, usize)>, NativeTacticRouteRunError> {
    let mut schedule = Vec::new();
    for repetition in 1..=repetitions {
        let workers = if repetition % 2 == 1 {
            NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS.to_vec()
        } else {
            NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS
                .iter()
                .rev()
                .copied()
                .collect()
        };
        for workers in workers {
            schedule.push((
                u32::try_from(schedule.len() + 1).map_err(route_error)?,
                repetition,
                workers,
            ));
        }
    }
    Ok(schedule)
}

fn sample_root(output_root: &Path, ordinal: u32, repetition: u32, workers: usize) -> PathBuf {
    output_root.join(format!("sample-{ordinal:02}-r{repetition}-w{workers}"))
}

fn sample_commit_path(progress_root: &Path, ordinal: u32) -> PathBuf {
    progress_root.join(format!("sample-{ordinal:02}.dtcs"))
}

fn reject_detached_sample_roots(
    output_root: &Path,
    schedule: &[(u32, u32, usize)],
) -> Result<(), NativeTacticRouteRunError> {
    for entry in fs::read_dir(output_root).map_err(route_error)? {
        let entry = entry.map_err(route_error)?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if !file_name.starts_with("sample-") {
            continue;
        }
        let path = entry.path();
        let expected = schedule.iter().any(|&(ordinal, repetition, workers)| {
            sample_root(output_root, ordinal, repetition, workers) == path
        });
        if !expected || !entry.file_type().map_err(route_error)?.is_dir() {
            return Err(route_message(
                "native tactic throughput output contains a detached sample root",
            ));
        }
    }
    Ok(())
}

fn load_or_commit_completed_sample(
    config: &NativeTacticThroughputCurveConfig<'_>,
    progress_root: &Path,
    execution_plan_sha256: Digest,
    ordinal: u32,
    repetition: u32,
    workers: usize,
) -> Result<Option<NativeTacticThroughputCurveSample>, NativeTacticRouteRunError> {
    let report_path =
        sample_root(config.output_root, ordinal, repetition, workers).join("report.json");
    let commit_path = sample_commit_path(progress_root, ordinal);
    let report_exists =
        regular_file_presence(&report_path, "native tactic throughput sample route report")?;
    let commit_exists =
        regular_file_presence(&commit_path, "native tactic throughput sample commit")?;
    if commit_exists && !report_exists {
        return Err(route_message(
            "native tactic throughput sample commit has no route report",
        ));
    }
    if !report_exists {
        return Ok(None);
    }
    let sample = read_completed_sample(
        config,
        execution_plan_sha256,
        ordinal,
        repetition,
        workers,
        &report_path,
    )?;
    if commit_exists {
        let commit: NativeTacticThroughputSampleCommit = read_progress_record(&commit_path)?;
        commit.validate(
            execution_plan_sha256,
            ordinal,
            repetition,
            workers,
            &report_path,
        )?;
        if commit.sample != sample {
            return Err(route_message(
                "native tactic throughput sample differs from its durable commit",
            ));
        }
    } else {
        write_sample_commit(progress_root, execution_plan_sha256, sample.clone())?;
    }
    Ok(Some(sample))
}

fn read_completed_sample(
    config: &NativeTacticThroughputCurveConfig<'_>,
    execution_plan_sha256: Digest,
    ordinal: u32,
    repetition: u32,
    workers: usize,
    report_path: &Path,
) -> Result<NativeTacticThroughputCurveSample, NativeTacticRouteRunError> {
    let bytes = read_bounded_regular_file(
        report_path,
        MAX_RESUME_JSON_BYTES,
        "native tactic throughput sample route report",
    )?;
    let route: NativeTacticRouteReport = serde_json::from_slice(&bytes).map_err(route_error)?;
    let sample = sample_from_route_report(
        ordinal,
        repetition,
        workers,
        path_text(report_path),
        Digest(Sha256::digest(bytes).into()),
        &route,
    );
    validate_completed_sample(config, execution_plan_sha256, &sample, &route)?;
    Ok(sample)
}

fn validate_completed_sample(
    config: &NativeTacticThroughputCurveConfig<'_>,
    execution_plan_sha256: Digest,
    sample: &NativeTacticThroughputCurveSample,
    route: &NativeTacticRouteReport,
) -> Result<(), NativeTacticRouteRunError> {
    if route.schema != NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V37
        || route.optimization_request_sha256 != config.optimization.content_sha256
        || route.execution_binding_sha256 != config.execution.content_sha256
        || route.execution_plan_sha256 != execution_plan_sha256
        || route.workers != sample.workers
        || route.timing.process_launch_micros != 0
        || !route.timing.persistence_attribution_is_valid()
        || route
            .timing
            .campaign_admission_breakdown
            .is_some_and(|breakdown| {
                breakdown.total_micros() != route.timing.campaign_admission_micros
            })
    {
        return Err(route_message(
            "native tactic throughput route sample is detached",
        ));
    }
    let audit = NativeTacticScratchCampaignAudit::build(config.repository_root, route)?;
    audit.validate_resource_binding(route, config.execution_plan)?;
    Ok(())
}

fn write_sample_commit(
    progress_root: &Path,
    execution_plan_sha256: Digest,
    sample: NativeTacticThroughputCurveSample,
) -> Result<(), NativeTacticRouteRunError> {
    let path = sample_commit_path(progress_root, sample.ordinal);
    write_progress_record(
        &path,
        &NativeTacticThroughputSampleCommit::new(execution_plan_sha256, sample)?,
    )
}

fn reject_non_prefix_progress(
    output_root: &Path,
    progress_root: &Path,
    schedule: &[(u32, u32, usize)],
    first_unfinished: usize,
) -> Result<(), NativeTacticRouteRunError> {
    for entry in fs::read_dir(progress_root).map_err(route_error)? {
        let path = entry.map_err(route_error)?.path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("dtcs")
            && !schedule
                .iter()
                .any(|(ordinal, _, _)| sample_commit_path(progress_root, *ordinal) == path)
        {
            return Err(route_message(
                "native tactic throughput progress contains an unexpected sample commit",
            ));
        }
    }
    for &(ordinal, repetition, workers) in schedule.iter().skip(first_unfinished + 1) {
        if path_exists_without_following(&sample_commit_path(progress_root, ordinal))?
            || path_exists_without_following(&sample_root(
                output_root,
                ordinal,
                repetition,
                workers,
            ))?
        {
            return Err(route_message(
                "native tactic throughput progress is not a contiguous sample prefix",
            ));
        }
    }
    Ok(())
}

fn append_fleet_launch_commit(
    progress_root: &Path,
    execution_plan_sha256: Digest,
    fleet_workers: usize,
    launch_micros: u64,
) -> Result<(), NativeTacticRouteRunError> {
    let existing = fleet_launch_commit_paths(progress_root)?;
    let ordinal = existing
        .len()
        .checked_add(1)
        .ok_or_else(|| route_message("native tactic fleet launch ordinal overflows"))?;
    let path = progress_root.join(format!("fleet-launch-{ordinal:04}.dtfl"));
    write_progress_record(
        &path,
        &NativeTacticThroughputFleetLaunchCommit::new(
            execution_plan_sha256,
            fleet_workers,
            launch_micros,
        )?,
    )
}

fn read_fleet_launch_micros(
    progress_root: &Path,
    execution_plan_sha256: Digest,
    fleet_workers: usize,
) -> Result<u64, NativeTacticRouteRunError> {
    let paths = fleet_launch_commit_paths(progress_root)?;
    if paths.is_empty() {
        return Err(route_message(
            "native tactic throughput progress has no fleet launch",
        ));
    }
    let mut total = 0_u64;
    for (index, path) in paths.iter().enumerate() {
        let expected_name = format!("fleet-launch-{:04}.dtfl", index + 1);
        if path.file_name().and_then(|name| name.to_str()) != Some(expected_name.as_str()) {
            return Err(route_message(
                "native tactic throughput fleet launch commits are not contiguous",
            ));
        }
        let commit: NativeTacticThroughputFleetLaunchCommit = read_progress_record(path)?;
        commit.validate(execution_plan_sha256, fleet_workers)?;
        total = total
            .checked_add(commit.launch_micros)
            .ok_or_else(|| route_message("native tactic fleet launch time overflows"))?;
    }
    Ok(total)
}

fn fleet_launch_commit_paths(
    progress_root: &Path,
) -> Result<Vec<PathBuf>, NativeTacticRouteRunError> {
    let mut paths = fs::read_dir(progress_root)
        .map_err(route_error)?
        .map(|entry| entry.map_err(route_error).map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| path.extension().and_then(|extension| extension.to_str()) == Some("dtfl"));
    paths.sort();
    Ok(paths)
}

fn path_exists_without_following(path: &Path) -> Result<bool, NativeTacticRouteRunError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(route_error(error)),
    }
}

fn regular_file_presence(
    path: &Path,
    description: &str,
) -> Result<bool, NativeTacticRouteRunError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            Ok(true)
        }
        Ok(_) => Err(route_message(format!(
            "{description} is not a physical regular file: {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(route_error(error)),
    }
}

fn read_bounded_regular_file(
    path: &Path,
    maximum_bytes: u64,
    description: &str,
) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    let metadata = fs::symlink_metadata(path).map_err(route_error)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > maximum_bytes
    {
        return Err(route_message(format!(
            "{description} is invalid or oversized: {}",
            path.display()
        )));
    }
    let bytes = fs::read(path).map_err(route_error)?;
    if bytes.is_empty()
        || u64::try_from(bytes.len()).map_err(route_error)? != metadata.len()
        || u64::try_from(bytes.len()).map_err(route_error)? > maximum_bytes
    {
        return Err(route_message(format!(
            "{description} changed while it was read: {}",
            path.display()
        )));
    }
    Ok(bytes)
}

fn read_progress_record<T>(path: &Path) -> Result<T, NativeTacticRouteRunError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_cbor::from_slice(&read_bounded_regular_file(
        path,
        MAXIMUM_PROGRESS_RECORD_BYTES,
        "native tactic throughput progress record",
    )?)
    .map_err(route_error)
}

fn write_progress_record<T: Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), NativeTacticRouteRunError> {
    let bytes = serde_cbor::to_vec(value).map_err(route_error)?;
    if bytes.is_empty()
        || u64::try_from(bytes.len()).map_err(route_error)? > MAXIMUM_PROGRESS_RECORD_BYTES
    {
        return Err(route_message(
            "native tactic throughput progress record size is invalid",
        ));
    }
    write_new(path, &bytes)
}

pub(super) fn sample_from_route_report(
    ordinal: u32,
    repetition: u32,
    workers: usize,
    route_report_path: String,
    route_report_sha256: Digest,
    route_report: &NativeTacticRouteReport,
) -> NativeTacticThroughputCurveSample {
    let peak_worker_checkpoint_resident_bytes =
        route_report.native_restore_accounting.peak_resident_bytes;
    NativeTacticThroughputCurveSample {
        ordinal,
        repetition,
        workers,
        route_report_path,
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
        learner_updates: route_report.learner_updates,
        model_snapshots_published: route_report.learner_authority.model_snapshots_published,
        training_replay_rows: route_report.training_replay_rows,
        restore_samples: route_report.native_restore_accounting.restore_samples,
        non_root_restore_requests: route_report
            .native_restore_accounting
            .direct_process_local_restore_requests
            .saturating_add(
                route_report
                    .native_restore_accounting
                    .direct_process_local_continuation_requests,
            ),
        direct_restore_fallback_replays: route_report
            .native_restore_accounting
            .direct_restore_fallback_replays,
        cache_evictions: route_report.native_restore_accounting.cache_evictions,
        tactic_selection_micros: route_report.timing.tactic_selection_micros,
        checkpoint_branching_micros: route_report.timing.checkpoint_branching_micros,
        tactic_execution_micros: route_report.timing.tactic_execution_micros,
        native_simulation_micros: route_report.timing.native_simulation_micros,
        ipc_and_result_transport_micros: route_report.timing.ipc_and_result_transport_micros,
        tactic_preparation_and_fact_extraction_micros: route_report
            .timing
            .tactic_preparation_and_fact_extraction_micros,
        model_update_micros: route_report.timing.model_update_micros,
        evidence_projection_micros: route_report.timing.evidence_projection_micros,
        persistence_micros: route_report.timing.persistence_micros,
        orchestration_micros: route_report.timing.orchestration_micros,
        result_validation_and_fact_extraction_micros: route_report
            .timing
            .result_validation_and_fact_extraction_micros,
        graph_admission_micros: route_report.timing.graph_admission_micros,
        campaign_admission_micros: Some(route_report.timing.campaign_admission_micros),
        campaign_admission_breakdown: route_report.timing.campaign_admission_breakdown,
        native_worker_occupancy_per_million: ratio_per_million(
            route_report.timing.native_simulation_micros,
            route_report
                .timing
                .wall_micros
                .saturating_mul(workers as u64),
        ),
    }
}

fn sample_admission_timing_is_valid(sample: &NativeTacticThroughputCurveSample) -> bool {
    match (
        sample.campaign_admission_micros,
        sample.campaign_admission_breakdown,
    ) {
        (None, None) => true,
        (Some(total), Some(breakdown)) => total > 0 && breakdown.total_micros() == total,
        _ => false,
    }
}

fn validate_curve_config(
    config: &NativeTacticThroughputCurveConfig<'_>,
) -> Result<(), NativeTacticRouteRunError> {
    config.execution_plan.validate()?;
    let output_exists = path_exists_without_following(config.output_root)?;
    if !(MINIMUM_REPETITIONS..=MAXIMUM_REPETITIONS).contains(&config.repetitions)
        || !curve_output_mode_is_valid(output_exists, config.resume)
        || config.execution_plan.lanes.len() != 1
        || config.execution_plan.generations.len() != 1
        || config.execution_plan.proposal_width_per_decision
            != NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS
                .last()
                .copied()
                .unwrap_or(0)
        || config.execution_plan.demonstration_chunk_ticks.is_some()
        || config.execution_plan.budgets.decisions_per_lane < MINIMUM_LONG_WORK_DECISIONS
        || config.execution_plan.budgets.decisions_per_lane
            < config
                .execution_plan
                .branch_every_decisions
                .saturating_mul(2)
        || config.execution_plan.budgets.decisions_per_lane
            < config
                .execution_plan
                .refit_every_decisions
                .saturating_mul(2)
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

fn curve_output_mode_is_valid(output_exists: bool, resume: bool) -> bool {
    output_exists == resume
}

fn curve_fleet_launch_start(
    first_unfinished: usize,
    sample_count: usize,
) -> Result<Option<usize>, NativeTacticRouteRunError> {
    if first_unfinished > sample_count {
        return Err(route_message(
            "native tactic throughput resume cursor exceeds its sample schedule",
        ));
    }
    Ok((first_unfinished < sample_count).then_some(first_unfinished))
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
#[path = "throughput_curve/tests.rs"]
mod tests;
