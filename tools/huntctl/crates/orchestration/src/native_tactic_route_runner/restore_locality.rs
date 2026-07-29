use super::*;
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

pub const NATIVE_TACTIC_RESTORE_LOCALITY_SCHEMA_V1: &str =
    "dusklight-native-tactic-restore-locality/v1";
const MINIMUM_REPETITIONS: u32 = 2;
const MAXIMUM_REPETITIONS: u32 = 6;
const REPORT_FILE: &str = "restore-locality.json";

#[derive(Clone, Debug)]
pub struct NativeTacticRestoreLocalityConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub execution_plan: &'a NativeTacticExecutionPlan,
    pub output_root: &'a Path,
    pub workers: usize,
    pub repetitions: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTacticRestoreLocalityTreatment {
    AuthenticatedReplayControl,
    CheckpointOwnerLocal,
}

impl NativeTacticRestoreLocalityTreatment {
    fn direct_restore_enabled(self) -> bool {
        self == Self::CheckpointOwnerLocal
    }

    fn slug(self) -> &'static str {
        match self {
            Self::AuthenticatedReplayControl => "authenticated-replay-control",
            Self::CheckpointOwnerLocal => "checkpoint-owner-local",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticRestoreLocalitySample {
    pub ordinal: u32,
    pub repetition: u32,
    pub treatment: NativeTacticRestoreLocalityTreatment,
    pub execution_plan_sha256: Digest,
    pub route_report_path: String,
    pub route_report_sha256: Digest,
    pub completed_decisions: u64,
    pub unique_useful_graph_expansions: u64,
    pub exploration_evidence_sha256: Digest,
    pub total_proposal_dispatches: u64,
    pub minimum_distinct_worker_slots_per_decision: usize,
    pub checkpoint_owner_available_decisions: u64,
    pub checkpoint_owner_local_decisions: u64,
    pub misrouted_owner_local_decisions: u64,
    pub direct_process_local_restore_requests: u64,
    pub direct_process_local_continuation_requests: u64,
    pub direct_process_local_restore_micros: u64,
    pub prefix_materializations: u64,
    pub replayed_prefix_ticks: u64,
    pub replay_restore_micros: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_hit_rate_per_million: u64,
    pub maximum_observed_stale_revisions: u64,
    pub wall_micros: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticRestoreLocalityPair {
    pub repetition: u32,
    pub replay_control_ordinal: u32,
    pub owner_local_ordinal: u32,
    pub fixed_work_satisfied: bool,
    pub exploration_evidence_parity_satisfied: bool,
    pub dispatch_diversity_satisfied: bool,
    pub owner_local_scheduling_satisfied: bool,
    pub replay_reduction_satisfied: bool,
    pub prefix_materialization_reduction: u64,
    pub replayed_prefix_tick_reduction: u64,
    pub replay_restore_micros_reduction: u64,
    pub passed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticRestoreLocalityReport {
    pub schema: String,
    pub content_sha256: Digest,
    pub recorded_unix_millis: u64,
    pub operating_system: String,
    pub architecture: String,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub replay_control_execution_plan_sha256: Digest,
    pub owner_local_execution_plan_sha256: Digest,
    pub workers: usize,
    pub repetitions: u32,
    pub fixed_completed_decisions: u64,
    pub fixed_unique_useful_graph_expansions: u64,
    pub proposal_width_per_decision: usize,
    pub minimum_required_distinct_worker_slots_per_decision: usize,
    pub fleet_launch_micros: u64,
    pub samples: Vec<NativeTacticRestoreLocalitySample>,
    pub pairs: Vec<NativeTacticRestoreLocalityPair>,
    pub fixed_work_satisfied: bool,
    pub exploration_evidence_parity_satisfied: bool,
    pub dispatch_diversity_satisfied: bool,
    pub owner_local_scheduling_satisfied: bool,
    pub replay_reduction_satisfied: bool,
    pub passed: bool,
}

impl NativeTacticRestoreLocalityReport {
    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_RESTORE_LOCALITY_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.recorded_unix_millis == 0
            || self.operating_system.is_empty()
            || self.architecture.is_empty()
            || self.optimization_request_sha256 == Digest::ZERO
            || self.execution_binding_sha256 == Digest::ZERO
            || self.replay_control_execution_plan_sha256 == Digest::ZERO
            || self.owner_local_execution_plan_sha256 == Digest::ZERO
            || self.replay_control_execution_plan_sha256 == self.owner_local_execution_plan_sha256
            || self.workers < 2
            || !(MINIMUM_REPETITIONS..=MAXIMUM_REPETITIONS).contains(&self.repetitions)
            || self.fixed_completed_decisions < 2
            || self.fixed_unique_useful_graph_expansions == 0
            || self.proposal_width_per_decision < 2
            || self.minimum_required_distinct_worker_slots_per_decision
                != self.workers.min(self.proposal_width_per_decision)
            || self.fleet_launch_micros == 0
        {
            return Err(route_message(
                "native tactic restore locality report identity is invalid",
            ));
        }
        let expected_samples = usize::try_from(self.repetitions)
            .map_err(route_error)?
            .checked_mul(2)
            .ok_or_else(|| route_message("restore locality sample count overflows"))?;
        if self.samples.len() != expected_samples
            || self.pairs.len() != usize::try_from(self.repetitions).map_err(route_error)?
        {
            return Err(route_message(
                "native tactic restore locality sample count is invalid",
            ));
        }
        let expected_dispatches = self
            .fixed_completed_decisions
            .checked_mul(self.proposal_width_per_decision as u64)
            .ok_or_else(|| route_message("restore locality dispatch count overflows"))?;
        for (index, sample) in self.samples.iter().enumerate() {
            let expected_ordinal = u32::try_from(index + 1).map_err(route_error)?;
            let expected_repetition = u32::try_from(index / 2 + 1).map_err(route_error)?;
            let within = index % 2;
            let expected_treatment = if expected_repetition % 2 == 1 {
                [
                    NativeTacticRestoreLocalityTreatment::AuthenticatedReplayControl,
                    NativeTacticRestoreLocalityTreatment::CheckpointOwnerLocal,
                ][within]
            } else {
                [
                    NativeTacticRestoreLocalityTreatment::CheckpointOwnerLocal,
                    NativeTacticRestoreLocalityTreatment::AuthenticatedReplayControl,
                ][within]
            };
            let expected_plan = if expected_treatment.direct_restore_enabled() {
                self.owner_local_execution_plan_sha256
            } else {
                self.replay_control_execution_plan_sha256
            };
            if sample.ordinal != expected_ordinal
                || sample.repetition != expected_repetition
                || sample.treatment != expected_treatment
                || sample.execution_plan_sha256 != expected_plan
                || sample.route_report_path.is_empty()
                || sample.route_report_sha256 == Digest::ZERO
                || sample.exploration_evidence_sha256 == Digest::ZERO
                || sample.completed_decisions != self.fixed_completed_decisions
                || sample.unique_useful_graph_expansions
                    != self.fixed_unique_useful_graph_expansions
                || sample.total_proposal_dispatches != expected_dispatches
                || sample.minimum_distinct_worker_slots_per_decision
                    < self.minimum_required_distinct_worker_slots_per_decision
                || sample.misrouted_owner_local_decisions != 0
                || sample.wall_micros == 0
            {
                return Err(route_message(
                    "native tactic restore locality sample is invalid",
                ));
            }
        }
        let derived = derive_pairs(
            &self.samples,
            self.repetitions,
            self.fixed_completed_decisions,
            self.fixed_unique_useful_graph_expansions,
            expected_dispatches,
            self.minimum_required_distinct_worker_slots_per_decision,
        )?;
        let conclusions = conclusions(&derived);
        if self.pairs != derived
            || self.fixed_work_satisfied != conclusions.fixed_work
            || self.exploration_evidence_parity_satisfied != conclusions.evidence_parity
            || self.dispatch_diversity_satisfied != conclusions.dispatch_diversity
            || self.owner_local_scheduling_satisfied != conclusions.owner_local_scheduling
            || self.replay_reduction_satisfied != conclusions.replay_reduction
            || self.passed != conclusions.passed()
        {
            return Err(route_message(
                "native tactic restore locality conclusions are stale",
            ));
        }
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        if self.content_sha256 != report_digest(&canonical)? {
            return Err(route_message(
                "native tactic restore locality content digest is stale",
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
}

pub fn run_native_tactic_restore_locality(
    config: &NativeTacticRestoreLocalityConfig<'_>,
) -> Result<NativeTacticRestoreLocalityReport, NativeTacticRouteRunError> {
    validate_config(config)?;
    if let Some(parent) = config
        .output_root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(route_error)?;
    }
    fs::create_dir(config.output_root).map_err(route_error)?;
    let mut replay_control_plan = config.execution_plan.clone();
    replay_control_plan.checkpoint.cross_decision_direct_restore = false;
    replay_control_plan.validate()?;
    let mut owner_local_plan = config.execution_plan.clone();
    owner_local_plan.checkpoint.cross_decision_direct_restore = true;
    owner_local_plan.validate()?;
    let replay_control_execution_plan_sha256 = replay_control_plan.identity()?;
    let owner_local_execution_plan_sha256 = owner_local_plan.identity()?;
    if replay_control_execution_plan_sha256 == owner_local_execution_plan_sha256 {
        return Err(route_message(
            "restore locality treatments do not have distinct execution plans",
        ));
    }

    let fixed_completed_decisions = config
        .execution_plan
        .budgets
        .decisions_per_lane
        .checked_mul(config.execution_plan.lanes.len() as u64)
        .ok_or_else(|| route_message("restore locality decision target overflows"))?;
    let fixed_unique_useful_graph_expansions = fixed_completed_decisions
        .checked_mul(config.execution_plan.proposal_width_per_decision as u64)
        .ok_or_else(|| route_message("restore locality expansion target overflows"))?;
    let fleet_root = config.output_root.join("worker-fleet");
    let fleet_config = NativeTacticRouteRunConfig {
        repository_root: config.repository_root,
        optimization: config.optimization,
        execution: config.execution,
        execution_plan: &owner_local_plan,
        promoted_tactic_registry: None,
        output_root: &fleet_root,
        workers: config.workers,
        cancellation: None,
        fault_injection: None,
        resume: false,
    };
    let fleet = launch_native_tactic_worker_fleet(&fleet_config, &fleet_root, config.workers)?;
    let fleet_launch_micros = fleet.launch_micros();
    let mut samples = Vec::new();
    for repetition in 1..=config.repetitions {
        let treatments = if repetition % 2 == 1 {
            [
                NativeTacticRestoreLocalityTreatment::AuthenticatedReplayControl,
                NativeTacticRestoreLocalityTreatment::CheckpointOwnerLocal,
            ]
        } else {
            [
                NativeTacticRestoreLocalityTreatment::CheckpointOwnerLocal,
                NativeTacticRestoreLocalityTreatment::AuthenticatedReplayControl,
            ]
        };
        for treatment in treatments {
            let ordinal = u32::try_from(samples.len() + 1).map_err(route_error)?;
            let plan = if treatment.direct_restore_enabled() {
                &owner_local_plan
            } else {
                &replay_control_plan
            };
            let sample_root = config.output_root.join(format!(
                "sample-{ordinal:02}-r{repetition}-{}",
                treatment.slug()
            ));
            let route_report = run_native_tactic_route_with_fleet(
                &NativeTacticRouteRunConfig {
                    repository_root: config.repository_root,
                    optimization: config.optimization,
                    execution: config.execution,
                    execution_plan: plan,
                    promoted_tactic_registry: None,
                    output_root: &sample_root,
                    workers: config.workers,
                    cancellation: None,
                    fault_injection: None,
                    resume: false,
                },
                &fleet,
            )?;
            samples.push(sample_from_report(
                ordinal,
                repetition,
                treatment,
                &sample_root.join("report.json"),
                &route_report,
            )?);
        }
    }
    fleet.shutdown()?;
    let expected_dispatches = fixed_completed_decisions
        .checked_mul(config.execution_plan.proposal_width_per_decision as u64)
        .ok_or_else(|| route_message("restore locality dispatch target overflows"))?;
    let minimum_required_distinct_worker_slots_per_decision = config
        .workers
        .min(config.execution_plan.proposal_width_per_decision);
    let pairs = derive_pairs(
        &samples,
        config.repetitions,
        fixed_completed_decisions,
        fixed_unique_useful_graph_expansions,
        expected_dispatches,
        minimum_required_distinct_worker_slots_per_decision,
    )?;
    let conclusions = conclusions(&pairs);
    let mut report = NativeTacticRestoreLocalityReport {
        schema: NATIVE_TACTIC_RESTORE_LOCALITY_SCHEMA_V1.into(),
        content_sha256: Digest::ZERO,
        recorded_unix_millis: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(route_error)?
            .as_millis()
            .try_into()
            .map_err(route_error)?,
        operating_system: std::env::consts::OS.into(),
        architecture: std::env::consts::ARCH.into(),
        optimization_request_sha256: config.optimization.content_sha256,
        execution_binding_sha256: config.execution.content_sha256,
        replay_control_execution_plan_sha256,
        owner_local_execution_plan_sha256,
        workers: config.workers,
        repetitions: config.repetitions,
        fixed_completed_decisions,
        fixed_unique_useful_graph_expansions,
        proposal_width_per_decision: config.execution_plan.proposal_width_per_decision,
        minimum_required_distinct_worker_slots_per_decision,
        fleet_launch_micros,
        samples,
        pairs,
        fixed_work_satisfied: conclusions.fixed_work,
        exploration_evidence_parity_satisfied: conclusions.evidence_parity,
        dispatch_diversity_satisfied: conclusions.dispatch_diversity,
        owner_local_scheduling_satisfied: conclusions.owner_local_scheduling,
        replay_reduction_satisfied: conclusions.replay_reduction,
        passed: conclusions.passed(),
    };
    report.content_sha256 = report_digest(&report)?;
    report.validate()?;
    write_new(
        &config.output_root.join(REPORT_FILE),
        &report.to_pretty_json()?,
    )?;
    Ok(report)
}

fn sample_from_report(
    ordinal: u32,
    repetition: u32,
    treatment: NativeTacticRestoreLocalityTreatment,
    route_report_path: &Path,
    report: &NativeTacticRouteReport,
) -> Result<NativeTacticRestoreLocalitySample, NativeTacticRouteRunError> {
    let route_report_sha256 =
        Digest(Sha256::digest(fs::read(route_report_path).map_err(route_error)?).into());
    let traces = report
        .seeds
        .iter()
        .flat_map(|seed| seed.trace.iter())
        .collect::<Vec<_>>();
    let total_proposal_dispatches = traces.iter().try_fold(0_u64, |total, trace| {
        total
            .checked_add(u64::try_from(trace.proposal_worker_slots.len()).map_err(route_error)?)
            .ok_or_else(|| route_message("restore locality dispatch count overflows"))
    })?;
    let minimum_distinct_worker_slots_per_decision = traces
        .iter()
        .map(|trace| {
            trace
                .proposal_worker_slots
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
        })
        .min()
        .unwrap_or(0);
    let checkpoint_owner_available_decisions = traces
        .iter()
        .filter(|trace| trace.checkpoint_owner_worker_slot.is_some())
        .count() as u64;
    let checkpoint_owner_local_decisions = traces
        .iter()
        .filter(|trace| {
            trace.restore_source == Some(NativeTacticRestoreSource::ProcessLocalCheckpoint)
                && trace.proposal_worker_slots.first().copied()
                    == trace.checkpoint_owner_worker_slot
        })
        .count() as u64;
    let misrouted_owner_local_decisions = traces
        .iter()
        .filter(|trace| {
            trace.restore_source == Some(NativeTacticRestoreSource::ProcessLocalCheckpoint)
                && trace.proposal_worker_slots.first().copied()
                    != trace.checkpoint_owner_worker_slot
        })
        .count() as u64;
    let restore = &report.native_restore_accounting;
    Ok(NativeTacticRestoreLocalitySample {
        ordinal,
        repetition,
        treatment,
        execution_plan_sha256: report.execution_plan_sha256,
        route_report_path: route_report_path.to_string_lossy().into_owned(),
        route_report_sha256,
        completed_decisions: report.total_decisions,
        unique_useful_graph_expansions: report.unique_useful_graph_expansions,
        exploration_evidence_sha256: exploration_evidence_digest(report)?,
        total_proposal_dispatches,
        minimum_distinct_worker_slots_per_decision,
        checkpoint_owner_available_decisions,
        checkpoint_owner_local_decisions,
        misrouted_owner_local_decisions,
        direct_process_local_restore_requests: restore.direct_process_local_restore_requests,
        direct_process_local_continuation_requests: restore
            .direct_process_local_continuation_requests,
        direct_process_local_restore_micros: restore.direct_process_local_restore_micros,
        prefix_materializations: restore.prefix_materializations,
        replayed_prefix_ticks: restore.replayed_prefix_ticks,
        replay_restore_micros: restore.replay_restore_micros,
        cache_hits: restore.cache_hits,
        cache_misses: restore.cache_misses,
        cache_hit_rate_per_million: restore.cache_hit_rate_per_million,
        maximum_observed_stale_revisions: report.replay_sharing.maximum_observed_stale_revisions,
        wall_micros: report.timing.wall_micros,
    })
}

fn exploration_evidence_digest(
    report: &NativeTacticRouteReport,
) -> Result<Digest, NativeTacticRouteRunError> {
    let projection = report
        .seeds
        .iter()
        .map(|seed| {
            (
                seed.seed,
                seed.terminal_discovered,
                seed.best_authenticated_tick,
                seed.decisions,
                seed.unique_useful_graph_expansions,
                seed.trace
                    .iter()
                    .map(|trace| {
                        (
                            trace.decision_index,
                            trace.frontier_identity,
                            &trace.selected_option_id,
                            trace.route_suffix_ticks,
                            trace.before.snapshot_sha256,
                            trace.after.snapshot_sha256,
                            trace.terminal,
                            trace
                                .proposal_batch
                                .iter()
                                .map(|proposal| {
                                    (
                                        &proposal.option_id,
                                        proposal.emitted_tape_sha256,
                                        proposal.after_snapshot_sha256,
                                        proposal.realized_ticks,
                                        proposal.terminal,
                                    )
                                })
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&projection).map_err(route_error)?;
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.native-tactic-restore-locality-exploration/v1\0");
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

fn derive_pairs(
    samples: &[NativeTacticRestoreLocalitySample],
    repetitions: u32,
    fixed_decisions: u64,
    fixed_expansions: u64,
    expected_dispatches: u64,
    minimum_distinct_workers: usize,
) -> Result<Vec<NativeTacticRestoreLocalityPair>, NativeTacticRouteRunError> {
    (1..=repetitions)
        .map(|repetition| {
            let replay = sample_for(
                samples,
                repetition,
                NativeTacticRestoreLocalityTreatment::AuthenticatedReplayControl,
            )?;
            let owner = sample_for(
                samples,
                repetition,
                NativeTacticRestoreLocalityTreatment::CheckpointOwnerLocal,
            )?;
            let fixed_work_satisfied = replay.completed_decisions == fixed_decisions
                && owner.completed_decisions == fixed_decisions
                && replay.unique_useful_graph_expansions == fixed_expansions
                && owner.unique_useful_graph_expansions == fixed_expansions;
            let exploration_evidence_parity_satisfied =
                replay.exploration_evidence_sha256 == owner.exploration_evidence_sha256;
            let dispatch_diversity_satisfied = replay.total_proposal_dispatches
                == expected_dispatches
                && owner.total_proposal_dispatches == expected_dispatches
                && replay.minimum_distinct_worker_slots_per_decision >= minimum_distinct_workers
                && owner.minimum_distinct_worker_slots_per_decision >= minimum_distinct_workers;
            let owner_local_scheduling_satisfied = replay.checkpoint_owner_local_decisions == 0
                && replay.direct_process_local_continuation_requests == 0
                && owner.checkpoint_owner_available_decisions > 0
                && owner.checkpoint_owner_local_decisions
                    == owner.checkpoint_owner_available_decisions
                && owner.misrouted_owner_local_decisions == 0
                && owner.direct_process_local_continuation_requests
                    == owner.checkpoint_owner_local_decisions
                && owner.direct_process_local_restore_requests
                    < replay.direct_process_local_restore_requests;
            let replay_reduction_satisfied = owner.prefix_materializations
                < replay.prefix_materializations
                && owner.replayed_prefix_ticks < replay.replayed_prefix_ticks;
            let passed = fixed_work_satisfied
                && exploration_evidence_parity_satisfied
                && dispatch_diversity_satisfied
                && owner_local_scheduling_satisfied
                && replay_reduction_satisfied;
            Ok(NativeTacticRestoreLocalityPair {
                repetition,
                replay_control_ordinal: replay.ordinal,
                owner_local_ordinal: owner.ordinal,
                fixed_work_satisfied,
                exploration_evidence_parity_satisfied,
                dispatch_diversity_satisfied,
                owner_local_scheduling_satisfied,
                replay_reduction_satisfied,
                prefix_materialization_reduction: replay
                    .prefix_materializations
                    .saturating_sub(owner.prefix_materializations),
                replayed_prefix_tick_reduction: replay
                    .replayed_prefix_ticks
                    .saturating_sub(owner.replayed_prefix_ticks),
                replay_restore_micros_reduction: replay
                    .replay_restore_micros
                    .saturating_sub(owner.replay_restore_micros),
                passed,
            })
        })
        .collect()
}

fn sample_for(
    samples: &[NativeTacticRestoreLocalitySample],
    repetition: u32,
    treatment: NativeTacticRestoreLocalityTreatment,
) -> Result<&NativeTacticRestoreLocalitySample, NativeTacticRouteRunError> {
    let mut matching = samples
        .iter()
        .filter(|sample| sample.repetition == repetition && sample.treatment == treatment);
    let sample = matching
        .next()
        .ok_or_else(|| route_message("restore locality treatment sample is absent"))?;
    if matching.next().is_some() {
        return Err(route_message(
            "restore locality treatment sample is duplicated",
        ));
    }
    Ok(sample)
}

struct Conclusions {
    fixed_work: bool,
    evidence_parity: bool,
    dispatch_diversity: bool,
    owner_local_scheduling: bool,
    replay_reduction: bool,
}

impl Conclusions {
    fn passed(&self) -> bool {
        self.fixed_work
            && self.evidence_parity
            && self.dispatch_diversity
            && self.owner_local_scheduling
            && self.replay_reduction
    }
}

fn conclusions(pairs: &[NativeTacticRestoreLocalityPair]) -> Conclusions {
    Conclusions {
        fixed_work: pairs.iter().all(|pair| pair.fixed_work_satisfied),
        evidence_parity: pairs
            .iter()
            .all(|pair| pair.exploration_evidence_parity_satisfied),
        dispatch_diversity: pairs.iter().all(|pair| pair.dispatch_diversity_satisfied),
        owner_local_scheduling: pairs
            .iter()
            .all(|pair| pair.owner_local_scheduling_satisfied),
        replay_reduction: pairs.iter().all(|pair| pair.replay_reduction_satisfied),
    }
}

fn validate_config(
    config: &NativeTacticRestoreLocalityConfig<'_>,
) -> Result<(), NativeTacticRouteRunError> {
    config.execution_plan.validate()?;
    if config.output_root.exists()
        || config.workers < 2
        || config.workers > MAX_ROUTE_WORKERS
        || !(MINIMUM_REPETITIONS..=MAXIMUM_REPETITIONS).contains(&config.repetitions)
        || config.execution_plan.lanes.len() != 1
        || config.execution_plan.generations.len() != 1
        || config.execution_plan.budgets.decisions_per_lane < 2
        || config.execution_plan.proposal_width_per_decision < 2
        || config.execution_plan.proposal_width_per_decision > MAX_TACTIC_PROPOSALS_PER_DECISION
        || config.execution_plan.demonstration_chunk_ticks.is_some()
        || config
            .execution_plan
            .promoted_tactic_registry_sha256
            .is_some()
    {
        return Err(route_message(
            "native tactic restore locality config is invalid",
        ));
    }
    Ok(())
}

fn report_digest(
    report: &NativeTacticRestoreLocalityReport,
) -> Result<Digest, NativeTacticRouteRunError> {
    let bytes = serde_json::to_vec(report).map_err(route_error)?;
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.native-tactic-restore-locality/v1\0");
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(
        ordinal: u32,
        repetition: u32,
        treatment: NativeTacticRestoreLocalityTreatment,
    ) -> NativeTacticRestoreLocalitySample {
        let owner = treatment == NativeTacticRestoreLocalityTreatment::CheckpointOwnerLocal;
        NativeTacticRestoreLocalitySample {
            ordinal,
            repetition,
            treatment,
            execution_plan_sha256: Digest([if owner { 2 } else { 1 }; 32]),
            route_report_path: format!("sample-{ordinal}/report.json"),
            route_report_sha256: Digest([3; 32]),
            completed_decisions: 4,
            unique_useful_graph_expansions: 16,
            exploration_evidence_sha256: Digest([4; 32]),
            total_proposal_dispatches: 16,
            minimum_distinct_worker_slots_per_decision: 4,
            checkpoint_owner_available_decisions: 3,
            checkpoint_owner_local_decisions: u64::from(owner) * 3,
            misrouted_owner_local_decisions: 0,
            direct_process_local_restore_requests: if owner { 9 } else { 12 },
            direct_process_local_continuation_requests: u64::from(owner) * 3,
            direct_process_local_restore_micros: u64::from(owner) * 30,
            prefix_materializations: if owner { 9 } else { 12 },
            replayed_prefix_ticks: if owner { 90 } else { 120 },
            replay_restore_micros: if owner { 900 } else { 1_200 },
            cache_hits: u64::from(owner) * 3,
            cache_misses: 0,
            cache_hit_rate_per_million: if owner { 1_000_000 } else { 0 },
            maximum_observed_stale_revisions: 0,
            wall_micros: 10_000,
        }
    }

    #[test]
    fn paired_derivation_requires_locality_replay_reduction_and_diversity() {
        let samples = vec![
            sample(
                1,
                1,
                NativeTacticRestoreLocalityTreatment::AuthenticatedReplayControl,
            ),
            sample(
                2,
                1,
                NativeTacticRestoreLocalityTreatment::CheckpointOwnerLocal,
            ),
        ];
        let pairs = derive_pairs(&samples, 1, 4, 16, 16, 4).unwrap();
        assert!(pairs[0].passed);
        assert_eq!(pairs[0].prefix_materialization_reduction, 3);
        assert_eq!(pairs[0].replayed_prefix_tick_reduction, 30);

        let mut mislabeled = samples.clone();
        mislabeled[1].direct_process_local_continuation_requests = 0;
        assert!(
            !derive_pairs(&mislabeled, 1, 4, 16, 16, 4).unwrap()[0]
                .owner_local_scheduling_satisfied
        );

        let mut collapsed = samples;
        collapsed[1].minimum_distinct_worker_slots_per_decision = 1;
        assert!(
            !derive_pairs(&collapsed, 1, 4, 16, 16, 4).unwrap()[0].dispatch_diversity_satisfied
        );
    }
}
