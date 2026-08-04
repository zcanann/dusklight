//! Representative checkpoint-path benchmark for persistent native learning.

use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::native_suffix_result::{
    NativeRetainedCheckpointResult, NativeSuffixBatchResult, NativeTerminalBinding,
    ValidatedNativeSuffixBatch,
};
use crate::native_suffix_worker::{
    NativeSuffixPrevalidatedFileIdentities, NativeSuffixWorkerLaunch,
    NativeSuffixWorkerLaunchTiming, NativeSuffixWorkerSession,
};
use crate::native_tactic_worker::{
    TACTIC_CHECKPOINT_CACHE_BYTES, TACTIC_CHECKPOINT_CACHE_ENTRIES, pad_runs,
};
use crate::optimization_request::OptimizationRequest;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_evidence::native_episode_shard::{NativeEpisodeShard, NativeObservationPhase};
use dusklight_learning::fact_snapshot::FactSnapshot;
use dusklight_search::suffix_batch::{
    NATIVE_CACHED_SUFFIX_BATCH_SCHEMA, NativeCheckpointCacheRequest, NativeCheckpointValidation,
    NativeSuffixBatch, NativeSuffixCandidate,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V1: &str = "dusklight-native-checkpoint-benchmark/v1";
pub const NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V2: &str = "dusklight-native-checkpoint-benchmark/v2";
pub const NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V3: &str = "dusklight-native-checkpoint-benchmark/v3";
pub const NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V4: &str = "dusklight-native-checkpoint-benchmark/v4";

pub struct NativeCheckpointBenchmarkConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub output_root: &'a Path,
    pub frontier_ticks: &'a [usize],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCheckpointBenchmarkReport {
    pub schema: String,
    pub optimization_request_sha256: Digest,
    pub execution_sha256: Digest,
    pub executable_sha256: Digest,
    pub game_data_sha256: Digest,
    pub platform_os: String,
    pub platform_arch: String,
    pub source_frame: u64,
    pub cache_capacity_bytes: u64,
    pub cache_capacity_entries: u64,
    pub launch: NativeCheckpointLaunchMeasurement,
    pub frontiers: Vec<NativeCheckpointFrontierMeasurement>,
    pub throughput: NativeCheckpointThroughputMeasurement,
    pub passed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCheckpointLaunchMeasurement {
    pub phases: NativeSuffixWorkerLaunchTiming,
    pub initial_batch_native_wall_micros: u64,
    pub initial_batch_native_simulation_micros: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCheckpointFrontierMeasurement {
    pub label: String,
    pub route_ticks: u64,
    pub authenticated_root_replay: NativeCheckpointBatchMeasurement,
    #[serde(alias = "process_local_restore")]
    pub process_local_follow_up: NativeCheckpointBatchMeasurement,
    #[serde(alias = "portable_reconstruction")]
    pub authenticated_replay_fallback: NativeCheckpointBatchMeasurement,
    #[serde(alias = "checkpoint_capture")]
    pub endpoint_retention: NativeCheckpointCaptureMeasurement,
    pub evidence_projection: NativeEvidenceProjectionMeasurement,
    pub parity: NativeCheckpointParityMeasurement,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCheckpointBatchMeasurement {
    pub host_wall_micros: u64,
    pub native_batch_wall_micros: u64,
    pub native_simulation_micros: u64,
    pub native_restore_micros: u64,
    pub simulated_ticks: u64,
    pub source_kind: String,
    #[serde(default)]
    pub cpu_draw_traversal_micros: u64,
    #[serde(default)]
    pub cpu_renderer_submission_micros: u64,
    #[serde(default)]
    pub audio_emulation_micros: u64,
    #[serde(default)]
    pub game_audio_update_micros: u64,
    #[serde(default)]
    pub headless_audit: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCheckpointCaptureMeasurement {
    #[serde(default)]
    pub storage_kind: String,
    pub checkpoint_bytes: u64,
    pub host_snapshot_bytes: u64,
    pub machine_capture_micros: u64,
    pub host_snapshot_transfer_kind: String,
    pub host_snapshot_capture_nanos: u64,
    pub total_capture_micros: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEvidenceProjectionMeasurement {
    pub episode_decode_micros: u64,
    pub fact_extraction_micros: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCheckpointParityMeasurement {
    pub source_state_exact: bool,
    pub transition_exact: bool,
    pub checkpoint_wide_semantic_digest_scope: String,
    pub semantic_state_digest_exact: bool,
    #[serde(default)]
    pub checkpoint_entry_count: u64,
    #[serde(default)]
    pub divergent_checkpoint_entries: Vec<String>,
    pub terminal_evidence_bytes_exact: bool,
    pub terminal_boundary_exact: bool,
    pub passed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCheckpointThroughputMeasurement {
    pub useful_transition_definition: String,
    pub useful_transitions: u64,
    pub non_root_expansion_requests: u64,
    pub direct_restore_requests: u64,
    pub direct_restore_rate_millionths: u64,
    pub useful_transitions_per_direct_restore_millionths: u64,
    pub useful_transitions_per_native_sim_second_millionths: u64,
    pub useful_transitions_per_wall_second_millionths: u64,
    pub measured_wall_micros: u64,
    pub measured_native_simulation_micros: u64,
}

impl NativeCheckpointBenchmarkReport {
    pub fn to_pretty_json(&self) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn validate(&self) -> Result<(), Box<dyn Error>> {
        let labels = ["early", "middle", "late"];
        if !matches!(
            self.schema.as_str(),
            NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V1
                | NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V2
                | NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V3
                | NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V4
        ) || self.optimization_request_sha256 == Digest::ZERO
            || self.execution_sha256 == Digest::ZERO
            || self.executable_sha256 == Digest::ZERO
            || self.game_data_sha256 == Digest::ZERO
            || self.platform_os.is_empty()
            || self.platform_arch.is_empty()
            || self.cache_capacity_bytes == 0
            || self.cache_capacity_entries == 0
            || self.launch.phases.total_micros == 0
            || self.launch.initial_batch_native_wall_micros == 0
            || self.frontiers.len() != labels.len()
        {
            return Err("native checkpoint benchmark report is incomplete".into());
        }
        for (index, frontier) in self.frontiers.iter().enumerate() {
            let parity_passed = frontier.parity.source_state_exact
                && frontier.parity.transition_exact
                && frontier.parity.terminal_evidence_bytes_exact
                && frontier.parity.terminal_boundary_exact
                && (self.schema == NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V1
                    || frontier.parity.semantic_state_digest_exact);
            let v2_or_later = self.schema != NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V1;
            let headless_audit_valid = matches!(
                self.schema.as_str(),
                NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V1 | NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V2
            ) || [
                &frontier.authenticated_root_replay,
                &frontier.process_local_follow_up,
                &frontier.authenticated_replay_fallback,
            ]
            .into_iter()
            .all(validate_headless_measurement);
            let live_endpoint = self.schema == NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V4;
            let checks = [
                ("label", frontier.label == labels[index]),
                ("route_ticks", frontier.route_ticks != 0),
                (
                    "authenticated_root_source",
                    frontier.authenticated_root_replay.source_kind == "authenticated_root_restore",
                ),
                (
                    "process_local_source",
                    frontier.process_local_follow_up.source_kind
                        == if live_endpoint {
                            "direct_process_local_continuation"
                        } else {
                            "direct_process_local_restore"
                        },
                ),
                (
                    "fallback_source",
                    frontier.authenticated_replay_fallback.source_kind
                        == "authenticated_root_restore",
                ),
                (
                    "process_local_tick_count",
                    frontier.process_local_follow_up.simulated_ticks == 1,
                ),
                (
                    "storage_kind",
                    if live_endpoint {
                        frontier.endpoint_retention.storage_kind == "live_endpoint"
                    } else {
                        frontier.endpoint_retention.storage_kind.is_empty()
                            || frontier.endpoint_retention.storage_kind == "portable_image"
                    },
                ),
                (
                    "checkpoint_bytes",
                    (frontier.endpoint_retention.checkpoint_bytes == 0) == live_endpoint,
                ),
                (
                    "host_snapshot_bytes",
                    frontier.endpoint_retention.host_snapshot_bytes != 0,
                ),
                (
                    "machine_capture_time",
                    (frontier.endpoint_retention.machine_capture_micros == 0) == live_endpoint,
                ),
                (
                    "host_snapshot_transfer",
                    frontier.endpoint_retention.host_snapshot_transfer_kind
                        == if live_endpoint {
                            "process_local_live_endpoint"
                        } else {
                            "in_process_capture_and_move_into_resident_cache"
                        },
                ),
                (
                    "host_snapshot_capture_time",
                    frontier.endpoint_retention.host_snapshot_capture_nanos != 0,
                ),
                (
                    "total_retention_time",
                    frontier.endpoint_retention.total_capture_micros != 0,
                ),
                (
                    "semantic_scope",
                    !frontier
                        .parity
                        .checkpoint_wide_semantic_digest_scope
                        .is_empty(),
                ),
                (
                    "checkpoint_entry_count",
                    !v2_or_later || frontier.parity.checkpoint_entry_count != 0,
                ),
                (
                    "checkpoint_entry_consistency",
                    !v2_or_later
                        || frontier.parity.semantic_state_digest_exact
                            == frontier.parity.divergent_checkpoint_entries.is_empty()
                            && frontier.parity.divergent_checkpoint_entries.len() as u64
                                <= frontier.parity.checkpoint_entry_count,
                ),
                ("headless_audit", headless_audit_valid),
                ("parity_pass_bit", frontier.parity.passed == parity_passed),
            ];
            if let Some((failed, _)) = checks.into_iter().find(|(_, passed)| !passed) {
                return Err(format!(
                    "native checkpoint benchmark frontier {:?} failed {failed}",
                    frontier.label,
                )
                .into());
            }
        }
        if self
            .frontiers
            .windows(2)
            .any(|pair| pair[0].route_ticks >= pair[1].route_ticks)
            || self.throughput.useful_transitions != self.frontiers.len() as u64
            || self.throughput.non_root_expansion_requests
                != self.throughput.direct_restore_requests
            || self.throughput.direct_restore_rate_millionths != 1_000_000
            || self.passed != self.frontiers.iter().all(|frontier| frontier.parity.passed)
        {
            return Err("native checkpoint benchmark throughput accounting is invalid".into());
        }
        Ok(())
    }
}

pub fn run_native_checkpoint_benchmark(
    config: &NativeCheckpointBenchmarkConfig<'_>,
) -> Result<NativeCheckpointBenchmarkReport, Box<dyn Error>> {
    validate_config(config)?;
    let measured_started = Instant::now();
    let root = config.repository_root.canonicalize()?;
    config
        .execution
        .validate_files(&root, config.optimization)?;
    fs::create_dir_all(config.output_root)?;
    let output_root = config.output_root.canonicalize()?;
    if fs::read_dir(&output_root)?.next().is_some() {
        return Err(format!(
            "native checkpoint benchmark output must be empty: {}",
            output_root.display()
        )
        .into());
    }

    let process_tape_path = root.join(&config.execution.process_boot_tape.path);
    let process_tape = InputTape::decode(&fs::read(&process_tape_path)?)?.tape;
    let source_frame = usize::try_from(config.optimization.route.source_boundary_index)?;
    let required_end = source_frame
        .checked_add(config.frontier_ticks.last().copied().unwrap_or(0))
        .and_then(|value| value.checked_add(1))
        .ok_or("checkpoint benchmark tape range overflowed")?;
    if required_end > process_tape.frames.len() {
        return Err("checkpoint benchmark frontiers exceed the process tape".into());
    }

    let initial_batch = batch(
        config,
        &process_tape,
        0,
        1,
        "launch-probe",
        None,
        None,
        false,
        false,
    )?;
    let initial_batch_path = output_root.join("launch.batch.json");
    let initial_result_path = output_root.join("launch.result.json");
    write_new_json(&initial_batch_path, &initial_batch)?;
    let terminal = NativeTerminalBinding {
        goal: config.optimization.terminal_predicate.goal.clone(),
        program_sha256: config.optimization.terminal_predicate.program_sha256,
        definition_sha256: config.optimization.terminal_predicate.definition_sha256,
    };
    let launch = NativeSuffixWorkerLaunch {
        executable: root.join(&config.execution.executable.path),
        game_data: root.join(&config.execution.game_data.path),
        input_tape: process_tape_path,
        milestone_program: root.join(&config.execution.milestone_program.path),
        card_fixture: config
            .execution
            .card_fixture_root(&root, config.optimization)?,
        card_fixture_sha256: config.execution.card_fixture_manifest.sha256,
        working_directory: root,
        state_root: output_root.join("native-state"),
        world_context_sha256: config.execution.world_context.sha256,
        terminal,
        initial_batch: initial_batch_path,
        initial_result: initial_result_path.clone(),
        initial_winner_tape: None,
    };
    let (mut worker, _, launch_timing) =
        NativeSuffixWorkerSession::launch_profiled_with_prevalidated_files(
            &launch,
            NativeSuffixPrevalidatedFileIdentities {
                executable_sha256: config.execution.executable.sha256,
                game_data_sha256: config.execution.game_data.sha256,
            },
        )?;
    let initial_raw = read_result(&initial_result_path)?;
    let launch_measurement = NativeCheckpointLaunchMeasurement {
        phases: launch_timing,
        initial_batch_native_wall_micros: initial_raw.timing.batch_wall_micros,
        initial_batch_native_simulation_micros: phase_micros(&initial_raw, "simulation")?,
    };

    let mut frontiers = Vec::with_capacity(config.frontier_ticks.len());
    let frontier_result = (|| -> Result<(), Box<dyn Error>> {
        for (index, &route_ticks) in config.frontier_ticks.iter().enumerate() {
            frontiers.push(measure_frontier(
                config,
                &process_tape,
                &output_root,
                &mut worker,
                index,
                route_ticks,
            )?);
        }
        Ok(())
    })();
    let shutdown_result = worker.shutdown();
    frontier_result?;
    shutdown_result?;

    let measured_wall_micros = elapsed_micros(measured_started);
    let measured_native_simulation_micros = frontiers
        .iter()
        .map(|frontier| frontier.process_local_follow_up.native_simulation_micros)
        .sum();
    let useful_transitions = u64::try_from(frontiers.len())?;
    let direct_restore_requests = useful_transitions;
    let non_root_expansion_requests = useful_transitions;
    let throughput = NativeCheckpointThroughputMeasurement {
        useful_transition_definition:
            "one parity-verified continuation transition per representative frontier".into(),
        useful_transitions,
        non_root_expansion_requests,
        direct_restore_requests,
        direct_restore_rate_millionths: ratio_millionths(
            direct_restore_requests,
            non_root_expansion_requests,
        ),
        useful_transitions_per_direct_restore_millionths: ratio_millionths(
            useful_transitions,
            direct_restore_requests,
        ),
        useful_transitions_per_native_sim_second_millionths: per_second_millionths(
            useful_transitions,
            measured_native_simulation_micros,
        ),
        useful_transitions_per_wall_second_millionths: per_second_millionths(
            useful_transitions,
            measured_wall_micros,
        ),
        measured_wall_micros,
        measured_native_simulation_micros,
    };
    let passed = frontiers.iter().all(|frontier| frontier.parity.passed)
        && throughput.direct_restore_rate_millionths == 1_000_000;
    let report = NativeCheckpointBenchmarkReport {
        schema: NATIVE_CHECKPOINT_BENCHMARK_SCHEMA_V4.into(),
        optimization_request_sha256: config.optimization.content_sha256,
        execution_sha256: config.execution.content_sha256,
        executable_sha256: config.execution.executable.sha256,
        game_data_sha256: config.execution.game_data.sha256,
        platform_os: std::env::consts::OS.into(),
        platform_arch: std::env::consts::ARCH.into(),
        source_frame: config.optimization.route.source_boundary_index,
        cache_capacity_bytes: TACTIC_CHECKPOINT_CACHE_BYTES as u64,
        cache_capacity_entries: TACTIC_CHECKPOINT_CACHE_ENTRIES as u64,
        launch: launch_measurement,
        frontiers,
        throughput,
        passed,
    };
    report.validate()?;
    Ok(report)
}

fn measure_frontier(
    config: &NativeCheckpointBenchmarkConfig<'_>,
    process_tape: &InputTape,
    output_root: &Path,
    worker: &mut NativeSuffixWorkerSession,
    index: usize,
    route_ticks: usize,
) -> Result<NativeCheckpointFrontierMeasurement, Box<dyn Error>> {
    let prefix = format!("frontier-{index}-{route_ticks}");
    let materialize_batch = batch(
        config,
        process_tape,
        0,
        route_ticks,
        &format!("{prefix}-materialize"),
        None,
        None,
        false,
        true,
    )?;
    let (materialized, materialized_raw, materialized_wall) = run_batch(
        worker,
        output_root,
        &prefix,
        "materialize",
        &materialize_batch,
    )?;
    let retained = materialized
        .candidates
        .first()
        .and_then(|candidate| candidate.retained_checkpoint.clone())
        .ok_or("frontier materialization did not retain its checkpoint")?;
    let retained_boundary_fingerprint = materialized
        .candidates
        .first()
        .map(|candidate| candidate.terminal_boundary_fingerprint.as_str())
        .ok_or("frontier materialization did not return its boundary fingerprint")?;
    let materialized_shard_path = PathBuf::from(&materialized.episode_shard_path);
    let decode_started = Instant::now();
    let materialized_shard = NativeEpisodeShard::read(&materialized_shard_path)?;
    let episode_decode_micros = elapsed_micros(decode_started);
    let endpoint = materialized_shard
        .episodes
        .first()
        .and_then(|episode| episode.steps.last())
        .ok_or("materialized frontier episode has no endpoint")?;
    let mut next_boundary = endpoint.post_simulation.clone();
    next_boundary.phase = NativeObservationPhase::PreInput;
    next_boundary.simulation_tick = next_boundary
        .simulation_tick
        .checked_add(1)
        .ok_or("frontier simulation tick overflowed")?;
    next_boundary.tape_frame = next_boundary
        .tape_frame
        .checked_add(1)
        .ok_or("frontier tape frame overflowed")?;
    let fact_started = Instant::now();
    let facts = FactSnapshot::from_native_learning(&next_boundary, &[], None, Vec::new())?;
    let fact_extraction_micros = elapsed_micros(fact_started);
    if facts.tape_frame
        != config
            .optimization
            .route
            .source_boundary_index
            .saturating_add(route_ticks as u64)
    {
        return Err("fact extraction produced the wrong frontier boundary".into());
    }

    let direct_batch = batch(
        config,
        process_tape,
        route_ticks,
        1,
        &format!("{prefix}-direct"),
        Some(&retained),
        Some(retained_boundary_fingerprint),
        false,
        false,
    )?;
    let (direct, direct_raw, direct_wall) =
        run_batch(worker, output_root, &prefix, "direct", &direct_batch)?;
    let replay_batch = batch(
        config,
        process_tape,
        0,
        route_ticks + 1,
        &format!("{prefix}-authenticated-replay-fallback"),
        None,
        None,
        false,
        false,
    )?;
    let (replay, replay_raw, replay_wall) = run_batch(
        worker,
        output_root,
        &prefix,
        "authenticated-replay-fallback",
        &replay_batch,
    )?;
    let direct_shard = NativeEpisodeShard::read(Path::new(&direct.episode_shard_path))?;
    let replay_shard = NativeEpisodeShard::read(Path::new(&replay.episode_shard_path))?;
    let direct_step = direct_shard
        .episodes
        .first()
        .and_then(|episode| episode.steps.first())
        .ok_or("direct-restore episode has no transition")?;
    let replay_step = replay_shard
        .episodes
        .first()
        .and_then(|episode| episode.steps.last())
        .ok_or("authenticated replay fallback episode has no transition")?;
    let direct_candidate = direct_raw
        .candidates
        .first()
        .ok_or("direct-restore result has no candidate")?;
    let replay_candidate = replay_raw
        .candidates
        .first()
        .ok_or("authenticated replay fallback result has no candidate")?;
    let direct_state_digest = direct_candidate
        .state_tick_digests
        .as_ref()
        .and_then(|digests| digests.last());
    let replay_state_digest = replay_candidate
        .state_tick_digests
        .as_ref()
        .and_then(|digests| digests.last());
    let (checkpoint_entry_count, divergent_checkpoint_entries) =
        compare_checkpoint_entries(direct_candidate, replay_candidate)?;
    let semantic_state_digest_exact =
        direct_state_digest.is_some() && direct_state_digest == replay_state_digest;
    if semantic_state_digest_exact != divergent_checkpoint_entries.is_empty() {
        return Err("checkpoint-wide and entry-level semantic digest comparisons disagree".into());
    }
    let direct_terminal_bytes = terminal_evidence_bytes(direct_candidate)?;
    let replay_terminal_bytes = terminal_evidence_bytes(replay_candidate)?;
    let parity = NativeCheckpointParityMeasurement {
        source_state_exact: direct_step.pre_input == replay_step.pre_input,
        transition_exact: direct_step == replay_step,
        checkpoint_wide_semantic_digest_scope:
            "registered parity-relevant checkpoint state; canonicalizes explicit host-ABI padding, JUT/VI presentation clocks, JUTProcBar, and the dPa particle heap while raw checkpoint restore remains byte-exact"
                .into(),
        semantic_state_digest_exact,
        checkpoint_entry_count,
        divergent_checkpoint_entries,
        terminal_evidence_bytes_exact: direct_terminal_bytes == replay_terminal_bytes,
        terminal_boundary_exact: direct_candidate.terminal_boundary_fingerprint
            == replay_candidate.terminal_boundary_fingerprint,
        passed: false,
    };
    let parity = NativeCheckpointParityMeasurement {
        passed: parity.source_state_exact
            && parity.transition_exact
            && parity.terminal_evidence_bytes_exact
            && parity.terminal_boundary_exact
            && parity.semantic_state_digest_exact,
        ..parity
    };
    Ok(NativeCheckpointFrontierMeasurement {
        label: frontier_label(index, config.frontier_ticks.len()).into(),
        route_ticks: route_ticks as u64,
        authenticated_root_replay: measurement(
            &materialized,
            &materialized_raw,
            materialized_wall,
        )?,
        process_local_follow_up: measurement(&direct, &direct_raw, direct_wall)?,
        authenticated_replay_fallback: measurement(&replay, &replay_raw, replay_wall)?,
        endpoint_retention: capture_measurement(&retained),
        evidence_projection: NativeEvidenceProjectionMeasurement {
            episode_decode_micros,
            fact_extraction_micros,
        },
        parity,
    })
}

fn compare_checkpoint_entries(
    direct: &crate::native_suffix_result::NativeSuffixCandidateResult,
    replay: &crate::native_suffix_result::NativeSuffixCandidateResult,
) -> Result<(u64, Vec<String>), Box<dyn Error>> {
    let direct_entries = direct
        .terminal_state_entry_digests
        .as_deref()
        .filter(|entries| !entries.is_empty())
        .ok_or("direct-restore result lacks terminal checkpoint-entry digests")?;
    let replay_entries = replay
        .terminal_state_entry_digests
        .as_deref()
        .filter(|entries| !entries.is_empty())
        .ok_or("portable-replay result lacks terminal checkpoint-entry digests")?;
    if direct_entries.len() != replay_entries.len() {
        return Err("direct and replayed checkpoint manifests have different lengths".into());
    }
    let mut divergent = Vec::new();
    for (direct_entry, replay_entry) in direct_entries.iter().zip(replay_entries) {
        if direct_entry.name != replay_entry.name
            || direct_entry.kind != replay_entry.kind
            || direct_entry.bytes != replay_entry.bytes
        {
            return Err("direct and replayed checkpoint manifests differ".into());
        }
        if direct_entry.digest != replay_entry.digest {
            divergent.push(direct_entry.name.clone());
        }
    }
    Ok((u64::try_from(direct_entries.len())?, divergent))
}

fn batch(
    config: &NativeCheckpointBenchmarkConfig<'_>,
    process_tape: &InputTape,
    source_route_ticks: usize,
    action_ticks: usize,
    id: &str,
    retained: Option<&NativeRetainedCheckpointResult>,
    retained_boundary_fingerprint: Option<&str>,
    retain_candidate_checkpoints: bool,
    retain_live_endpoint: bool,
) -> Result<NativeSuffixBatch, Box<dyn Error>> {
    if retained.is_some() != retained_boundary_fingerprint.is_some() {
        return Err("cached checkpoint identity and boundary must be supplied together".into());
    }
    let source_frame = usize::try_from(config.optimization.route.source_boundary_index)?;
    let action_start = source_frame
        .checked_add(source_route_ticks)
        .ok_or("checkpoint benchmark action range overflowed")?;
    let action_end = action_start
        .checked_add(action_ticks)
        .ok_or("checkpoint benchmark action range overflowed")?;
    let frames = process_tape
        .frames
        .get(action_start..action_end)
        .ok_or("checkpoint benchmark action range exceeds the process tape")?;
    Ok(NativeSuffixBatch {
        schema: NATIVE_CACHED_SUFFIX_BATCH_SCHEMA.into(),
        source_frame,
        source_boundary_fingerprint: retained_boundary_fingerprint.map_or_else(
            || {
                config
                    .optimization
                    .route
                    .native_source_boundary_fingerprint
                    .clone()
            },
            str::to_owned,
        ),
        checkpoint_validation: NativeCheckpointValidation {
            kind: "recorded_replay_window".into(),
            ticks: usize::try_from(config.execution.checkpoint_validation_ticks)?,
        },
        maximum_ticks: action_ticks,
        verify_state_hashes: true,
        checkpoint_cache: Some(NativeCheckpointCacheRequest {
            capacity_bytes: TACTIC_CHECKPOINT_CACHE_BYTES,
            capacity_entries: TACTIC_CHECKPOINT_CACHE_ENTRIES,
            source_identity: retained.map(|checkpoint| checkpoint.restore_identity.clone()),
            source_route_ticks,
            retain_candidate_checkpoints,
            retain_live_endpoint,
            retain_candidate_index: None,
        }),
        candidates: vec![NativeSuffixCandidate {
            id: id.into(),
            actions: pad_runs(frames)?,
            controller_program_hex: None,
            maximum_ticks: None,
            cancellation_guard: None,
        }],
    })
}

fn run_batch(
    worker: &mut NativeSuffixWorkerSession,
    output_root: &Path,
    prefix: &str,
    kind: &str,
    batch: &NativeSuffixBatch,
) -> Result<(ValidatedNativeSuffixBatch, NativeSuffixBatchResult, u64), Box<dyn Error>> {
    let batch_path = output_root.join(format!("{prefix}.{kind}.batch.json"));
    let result_path = output_root.join(format!("{prefix}.{kind}.result.json"));
    write_new_json(&batch_path, batch)?;
    let started = Instant::now();
    let validated = worker.run_batch(&batch_path, &result_path, None)?;
    let wall_micros = elapsed_micros(started);
    let raw = read_result(&result_path)?;
    Ok((validated, raw, wall_micros))
}

fn measurement(
    validated: &ValidatedNativeSuffixBatch,
    raw: &NativeSuffixBatchResult,
    host_wall_micros: u64,
) -> Result<NativeCheckpointBatchMeasurement, Box<dyn Error>> {
    Ok(NativeCheckpointBatchMeasurement {
        host_wall_micros,
        native_batch_wall_micros: raw.timing.batch_wall_micros,
        native_simulation_micros: phase_micros(raw, "simulation")?,
        native_restore_micros: validated.restore_micros.iter().copied().sum(),
        simulated_ticks: validated.simulated_ticks,
        source_kind: raw
            .checkpoint_cache
            .as_ref()
            .map(|cache| cache.source_kind.clone())
            .unwrap_or_else(|| "uncached".into()),
        cpu_draw_traversal_micros: phase_micros(raw, "cpu_draw_traversal")?,
        cpu_renderer_submission_micros: phase_micros(raw, "cpu_renderer_submission")?,
        audio_emulation_micros: phase_micros(raw, "audio_emulation")?,
        game_audio_update_micros: phase_micros(raw, "game_audio_update")?,
        headless_audit: raw.timing.headless_audit.clone(),
    })
}

fn validate_headless_measurement(measurement: &NativeCheckpointBatchMeasurement) -> bool {
    let audit = &measurement.headless_audit;
    measurement.cpu_renderer_submission_micros == 0
        && audit.get("active").and_then(Value::as_bool) == Some(true)
        && audit.get("host_pacing").and_then(Value::as_str) == Some("disabled")
        && audit.get("imgui_frame_lifecycle").and_then(Value::as_str)
            == Some("suppressed_on_candidate_ticks")
        && audit.get("host_audio_device").and_then(Value::as_str) == Some("suppressed")
        && audit
            .get("deterministic_audio_emulation")
            .and_then(Value::as_str)
            == Some("suppressed_on_candidate_ticks")
        && audit.get("game_audio_update").and_then(Value::as_str)
            == Some("suppressed_on_candidate_ticks")
        && audit.get("gameplay_draw_traversal").and_then(Value::as_str) == Some("retained")
        && audit.get("cpu_renderer_submission").and_then(Value::as_str)
            == Some("suppressed_on_candidate_ticks")
}

fn capture_measurement(
    retained: &NativeRetainedCheckpointResult,
) -> NativeCheckpointCaptureMeasurement {
    NativeCheckpointCaptureMeasurement {
        storage_kind: retained.storage_kind.clone(),
        checkpoint_bytes: retained.checkpoint_bytes,
        host_snapshot_bytes: retained.host_snapshot_bytes,
        machine_capture_micros: retained.machine_capture_micros,
        host_snapshot_transfer_kind: if retained.storage_kind == "live_endpoint" {
            "process_local_live_endpoint"
        } else {
            "in_process_capture_and_move_into_resident_cache"
        }
        .into(),
        host_snapshot_capture_nanos: retained.host_snapshot_capture_nanos,
        total_capture_micros: retained.capture_micros,
    }
}

fn terminal_evidence_bytes(
    candidate: &crate::native_suffix_result::NativeSuffixCandidateResult,
) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(serde_json::to_vec(&(
        candidate.success,
        &candidate.predicate_evidence,
        &candidate.terminal_observation,
    ))?)
}

fn phase_micros(result: &NativeSuffixBatchResult, phase: &str) -> Result<u64, Box<dyn Error>> {
    result
        .timing
        .phases
        .get(phase)
        .and_then(Value::as_object)
        .and_then(|value| value.get("micros"))
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("native timing phase {phase:?} has no measured micros").into())
}

fn read_result(path: &Path) -> Result<NativeSuffixBatchResult, Box<dyn Error>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn write_new_json(path: &Path, value: &impl Serialize) -> Result<(), Box<dyn Error>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn validate_config(config: &NativeCheckpointBenchmarkConfig<'_>) -> Result<(), Box<dyn Error>> {
    if config.output_root.exists()
        || config.frontier_ticks.len() != 3
        || config.frontier_ticks.contains(&0)
        || config
            .frontier_ticks
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || config.frontier_ticks.last().copied().unwrap_or(0) >= 4_096
    {
        return Err(
            "checkpoint benchmark requires a new output root and three increasing frontiers in 1..4096"
                .into(),
        );
    }
    Ok(())
}

fn frontier_label(index: usize, count: usize) -> &'static str {
    match (index, count) {
        (0, 3) => "early",
        (1, 3) => "middle",
        (2, 3) => "late",
        _ => "representative",
    }
}

fn ratio_millionths(numerator: u64, denominator: u64) -> u64 {
    if denominator == 0 {
        return 0;
    }
    numerator.saturating_mul(1_000_000) / denominator
}

fn per_second_millionths(transitions: u64, micros: u64) -> u64 {
    if micros == 0 {
        return 0;
    }
    transitions.saturating_mul(1_000_000_000_000) / micros
}

fn elapsed_micros(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
