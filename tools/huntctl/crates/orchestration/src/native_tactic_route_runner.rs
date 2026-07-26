//! Fresh-model tactic-Q route learning on an authenticated native checkpoint.

use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::native_suffix_result::{NativeTerminalBinding, ValidatedNativeSuffixBatch};
use crate::native_suffix_worker::{
    NativeSuffixWorkerError, NativeSuffixWorkerLaunch, NativeSuffixWorkerSession,
};
use crate::native_tactic_worker::{
    NativeTacticCheckpointSource, NativeTacticWorkerError, NativeTacticWorkerOutcome,
    NativeTacticWorkerPaths, PersistentTacticBatchWorker, execute_selected_tactic,
    tactic_root_checkpoint_sha256,
};
use crate::optimization_request::OptimizationRequest;
use crate::tactic_q_campaign::{
    EvaluatedRewardedTacticOutcome, TACTIC_Q_CHECKPOINT_EXTENSION, TacticCampaignDiagnostics,
    TacticCampaignGraphProjection, TacticCampaignGraphProjectionEdge,
    TacticCampaignGraphProjectionNode, TacticQCampaign, TacticQCampaignError, TacticQDecision,
    has_no_progress_loop, route_checkpoint,
};
use crate::tactic_q_checkpoint_store::{StoredContentRef, TacticQContentStore};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::{InputTape, RawPadState};
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use dusklight_learning::default_tactic_catalog::{
    MAX_GOAL_SEEK_TARGETS, goal_conditioned_route_tactic_catalog,
};
use dusklight_learning::fact_registry::FactRegistry;
use dusklight_learning::fact_snapshot::FactSnapshot;
use dusklight_learning::fqi::FqiConfig;
use dusklight_learning::learner_state::LearnerState;
use dusklight_learning::option_values::OptionValueConfig;
use dusklight_learning::reward_shaping::{
    POTENTIAL_SHAPING_SCHEMA_V1, PotentialShapingSpec, PotentialTerm, TACTIC_REWARD_SPEC_SCHEMA_V1,
    TacticRewardBreakdown, TacticRewardSpec,
};
use dusklight_learning::tactic_exploration::{
    SelectedTactic, TacticExplorationConfig, TacticSelectionReason,
};
use dusklight_learning::tactic_features::GoalConditionedTacticFeatureEncoder;
use dusklight_objectives::milestone_dsl::{Comparison, Expression, Field, Value};
use dusklight_proposals::behavior_archive::BehaviorArchive;
use dusklight_search::search::{MacroAction, SearchPadState};
use dusklight_search::suffix_batch::{
    NATIVE_SUFFIX_BATCH_SCHEMA, NativeCheckpointValidation, NativeSuffixBatch,
    NativeSuffixCandidate,
};
use dusklight_world::world_context::WorldContext;
use dusklight_world::world_geometry::KclReconstruction;
use dusklight_world::world_inventory::WorldInventory;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V4: &str = "dusklight-native-tactic-route-report/v4";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V5: &str = "dusklight-native-tactic-route-report/v5";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V6: &str = "dusklight-native-tactic-route-report/v6";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V7: &str = "dusklight-native-tactic-route-report/v7";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V8: &str = "dusklight-native-tactic-route-report/v8";
pub const NATIVE_TACTIC_DECISION_SUMMARY_SCHEMA_V1: &str =
    "dusklight-native-tactic-decision-summary/v1";
pub const NATIVE_TACTIC_DECISION_JOURNAL_FILE: &str = "decisions.dtqj";
pub const NATIVE_TACTIC_CONTENT_STORE_DIRECTORY: &str = "objects";
const NATIVE_TACTIC_DECISION_SEGMENTS_DIRECTORY: &str = "decision-journal";
const NATIVE_TACTIC_DECISION_SEGMENT_MAGIC: &[u8; 8] = b"DSKTQJC1";
const NATIVE_TACTIC_DECISION_SEGMENT_VERSION: u16 = 1;
const NATIVE_TACTIC_DECISION_SEGMENT_HEADER_SIZE: usize = 8 + 2 + 2 + 8 + 8 + 8 + 32;
const NATIVE_TACTIC_DECISION_SEGMENT_COMPRESSION_LEVEL: i32 = 3;
const NATIVE_TACTIC_DECISION_COMPACTION_RECORDS: u64 = 256;
const NATIVE_TACTIC_DECISION_JOURNAL_MAGIC: &[u8; 8] = b"DSKTQJ01";
const NATIVE_TACTIC_DECISION_JOURNAL_VERSION: u16 = 2;
const NATIVE_TACTIC_DECISION_JOURNAL_HEADER_SIZE: usize = 8 + 2 + 2;
const NATIVE_TACTIC_DECISION_RECORD_HEADER_SIZE: usize = 4 + 32;
const MAXIMUM_TACTIC_DECISION_RECORD_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_TACTIC_DECISION_SEGMENT_BYTES: usize = 256 * 1024 * 1024;
const MAXIMUM_TACTIC_DECISION_SEGMENTS: usize =
    (MAX_ROUTE_DECISIONS as usize).div_ceil(NATIVE_TACTIC_DECISION_COMPACTION_RECORDS as usize) + 1;
const MAX_ROUTE_SEEDS: usize = 32;
const MAX_ROUTE_WORKERS: usize = 32;
const MAX_ROUTE_DECISIONS: u64 = 100_000;
const ROUTE_TACTIC_VALUE_DISCOUNT: f32 = 0.999;
const ROUTE_TACTIC_NOVELTY_REWARD: f32 = 0.05;
const TACTIC_PROPOSALS_PER_DECISION: usize = 4;
const MAX_RESUME_JSON_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ROUTE_ATTEMPTS: usize = 10_000;
const TACTIC_ROUTE_PERFORMANCE_SCHEMA_V1: &str = "dusklight-native-tactic-route-performance/v1";

#[derive(Clone, Debug)]
pub struct NativeTacticRouteRunConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub output_root: &'a Path,
    pub exploration_seeds: &'a [u64],
    pub decisions_per_seed: u64,
    pub branch_every_decisions: u64,
    pub refit_every_decisions: u64,
    pub epsilon_per_million: u32,
    pub workers: usize,
    pub cancellation: Option<&'a AtomicBool>,
    pub resume: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticRouteReport {
    pub schema: String,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub objective_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub goal_target: NativeTacticGoalTargetReport,
    pub reward_spec: TacticRewardSpec,
    pub demonstration_transitions: u64,
    pub exploration_seeds: Vec<u64>,
    pub workers: usize,
    pub decisions_per_seed: u64,
    pub refit_every_decisions: u64,
    pub successful_seeds: u64,
    pub total_native_ticks: u64,
    pub total_decisions: u64,
    pub useful_decisions: u64,
    pub frontier_availability: NativeTacticFrontierAvailability,
    pub timing: NativeTacticRouteTiming,
    pub seeds: Vec<NativeTacticSeedResult>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticFrontierAvailability {
    pub logical_frontier_records: usize,
    pub directly_restorable_native_frontiers: usize,
    pub replay_only_frontiers: usize,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticRouteTiming {
    pub wall_micros: u64,
    pub tactic_selection_micros: u64,
    pub checkpoint_branching_micros: u64,
    /// Coordinator wall time waiting for proposal batches. Concurrent seed
    /// coordinators are summed in seed-level aggregates.
    pub tactic_execution_micros: u64,
    /// Total native worker occupancy. This is work, not wall time, and may
    /// exceed `tactic_execution_micros` when proposals overlap.
    pub native_simulation_micros: u64,
    /// Total non-native preparation/fact-extraction occupancy across workers.
    pub tactic_preparation_and_fact_extraction_micros: u64,
    pub model_update_micros: u64,
    pub evidence_projection_and_persistence_micros: u64,
    /// Candidate-only artifact generation after native terminal admission.
    /// Exploration-only seeds keep this at zero; cold replay is a separate
    /// explicit command and is never charged here.
    #[serde(default)]
    pub retained_candidate_artifact_micros: u64,
    pub useful_decisions_per_second_millionths: u64,
    pub native_ticks_per_second_millionths: u64,
    pub episodes_per_second_millionths: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeTacticSeedPerformance {
    schema: String,
    decisions: u64,
    useful_decisions: u64,
    timing: NativeTacticRouteTiming,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticSeedResult {
    pub seed: u64,
    pub success: bool,
    pub decisions: u64,
    pub episodes: u64,
    pub native_ticks: u64,
    pub replay_rows: usize,
    pub visited_states: usize,
    #[serde(default)]
    pub useful_decisions: u64,
    #[serde(default)]
    pub timing: NativeTacticRouteTiming,
    pub selection_counts: BTreeMap<String, u64>,
    pub diagnostics: Option<TacticCampaignDiagnostics>,
    pub final_checkpoint: Option<String>,
    pub graph: Option<String>,
    pub successful_tape: Option<String>,
    pub final_result: Option<String>,
    pub trace: Vec<NativeTacticDecisionTrace>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticDecisionTrace {
    pub decision_index: u64,
    pub episode: u64,
    pub route_suffix_ticks: u64,
    pub selected_option_id: String,
    pub selection_reason: TacticSelectionReason,
    pub selected_q: Option<f64>,
    pub best_q: Option<f64>,
    pub reward: f32,
    pub reward_components: TacticRewardBreakdown,
    pub goal_distance_before: f32,
    pub goal_distance_after: f32,
    pub terminal: bool,
    pub frontier_cells: usize,
    #[serde(default)]
    pub logical_frontier_records: usize,
    #[serde(default)]
    pub directly_restorable_native_frontiers: usize,
    #[serde(default)]
    pub replay_only_frontiers: usize,
    pub visited_states: usize,
    pub before: NativeTacticStateTrace,
    pub after: NativeTacticStateTrace,
    pub measurements: Vec<NativeTacticMeasurementTrace>,
    pub applicable_tactics: Vec<NativeTacticValueTrace>,
    #[serde(default)]
    pub proposal_batch: Vec<NativeTacticProposalTrace>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticStateTrace {
    pub snapshot_sha256: Digest,
    pub stage: String,
    pub room: i8,
    pub layer: Option<i8>,
    pub point: Option<i16>,
    pub simulation_tick: u64,
    pub tape_frame: u64,
    pub player_position: [f32; 3],
    pub player_velocity: Option<[f32; 3]>,
    pub player_procedure: Option<u16>,
    pub player_contacts: Option<u8>,
    pub event_running: Option<bool>,
    pub event_id: Option<i16>,
    pub terminal_reached: Option<bool>,
    pub actor_count: usize,
    pub same_room_actor_count: usize,
    pub recent_option_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticMeasurementTrace {
    pub name: String,
    pub before: f32,
    pub after: f32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticValueTrace {
    pub option_id: String,
    pub mean_q: Option<f64>,
    pub ensemble_variance: Option<f64>,
    pub selected: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticProposalTrace {
    pub option_id: String,
    pub selection_reason: TacticSelectionReason,
    pub reward: f32,
    pub reward_components: TacticRewardBreakdown,
    pub realized_ticks: u32,
    pub terminal: bool,
    pub goal_distance_after: f32,
    pub after_snapshot_sha256: Digest,
    pub retained: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeTacticProposalRecord {
    trace: NativeTacticProposalTrace,
    transition: StoredContentRef,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeTacticDecisionRecord {
    decision_index: u64,
    episode: u64,
    episode_group: u64,
    route_suffix_ticks: u64,
    selection_reason: TacticSelectionReason,
    selected_q: Option<f64>,
    best_q: Option<f64>,
    reward: f32,
    reward_components: TacticRewardBreakdown,
    goal_distance_before: f32,
    goal_distance_after: f32,
    terminal: bool,
    frontier_cells: usize,
    #[serde(default)]
    logical_frontier_records: usize,
    #[serde(default)]
    directly_restorable_native_frontiers: usize,
    #[serde(default)]
    replay_only_frontiers: usize,
    visited_states: usize,
    root_checkpoint_sha256: Digest,
    root_tape: StoredContentRef,
    transition: StoredContentRef,
    #[serde(default)]
    proposal_batch: Vec<NativeTacticProposalRecord>,
}

pub fn run_native_tactic_route(
    config: &NativeTacticRouteRunConfig<'_>,
) -> Result<NativeTacticRouteReport, NativeTacticRouteRunError> {
    let campaign_started = Instant::now();
    validate_config(config)?;
    let root = config.repository_root.canonicalize().map_err(route_error)?;
    config
        .execution
        .validate_files(&root, config.optimization)
        .map_err(route_error)?;
    if config.output_root.exists() && !config.resume {
        return Err(route_message(format!(
            "tactic route output already exists: {}",
            config.output_root.display()
        )));
    }
    if !config.output_root.exists() && config.resume {
        return Err(route_message(format!(
            "tactic route output does not exist to resume: {}",
            config.output_root.display()
        )));
    }
    if config.output_root.join("report.json").exists() {
        return Err(route_message("completed tactic route cannot be resumed"));
    }
    fs::create_dir_all(config.output_root).map_err(route_error)?;

    let registry = FactRegistry::canonical();
    let process_tape = InputTape::decode(
        &fs::read(root.join(&config.execution.process_boot_tape.path)).map_err(route_error)?,
    )
    .map_err(route_error)?
    .tape;
    let source_frame =
        usize::try_from(config.optimization.route.source_boundary_index).map_err(route_error)?;
    let route_prefix = InputTape {
        boot: process_tape.boot.clone(),
        tick_rate_numerator: process_tape.tick_rate_numerator,
        tick_rate_denominator: process_tape.tick_rate_denominator,
        frames: process_tape
            .frames
            .get(..source_frame)
            .ok_or_else(|| route_message("source frame is beyond the process tape"))?
            .to_vec(),
    };
    route_prefix.validate().map_err(route_error)?;

    let initial_batch = initial_probe_batch(config)?;
    let terminal = NativeTerminalBinding {
        goal: config.optimization.terminal_predicate.goal.clone(),
        program_sha256: config.optimization.terminal_predicate.program_sha256,
        definition_sha256: config.optimization.terminal_predicate.definition_sha256,
    };
    let card_fixture = config
        .execution
        .card_fixture_root(&root, config.optimization)
        .map_err(route_error)?;
    let worker_count = config.workers;
    let attempt_roots = (0..worker_count)
        .map(|_| reserve_attempt_root(config.output_root))
        .collect::<Result<Vec<_>, _>>()?;
    let mut launched = std::thread::scope(|scope| {
        let root = &root;
        let initial_batch = &initial_batch;
        let terminal = &terminal;
        let card_fixture = &card_fixture;
        let handles = attempt_roots
            .iter()
            .enumerate()
            .map(|(worker_index, attempt_root)| {
                scope.spawn(move || {
                    launch_tactic_route_worker(
                        config,
                        root,
                        attempt_root,
                        initial_batch,
                        terminal,
                        card_fixture,
                    )
                    .map(|worker| (worker_index, worker))
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| route_message("native tactic route worker launch panicked"))?
            })
            .collect::<Result<Vec<_>, _>>()
    })?;
    launched.sort_by_key(|(worker_index, _)| *worker_index);
    let mut workers = Vec::with_capacity(worker_count);
    let mut worker_initial_facts = Vec::with_capacity(worker_count);
    let mut worker_root_checkpoints = Vec::with_capacity(worker_count);
    for (_, (worker, facts, root_checkpoint_sha256)) in launched {
        workers.push(worker);
        worker_initial_facts.push(facts);
        worker_root_checkpoints.push(root_checkpoint_sha256);
    }
    let initial_facts = worker_initial_facts
        .first()
        .cloned()
        .ok_or_else(|| route_message("tactic route worker pool is empty"))?;
    if initial_facts.tape_frame != config.optimization.route.source_boundary_index
        || initial_facts.terminal.reached != Some(false)
        || worker_initial_facts
            .iter()
            .any(|facts| facts != &initial_facts)
        || worker_root_checkpoints
            .iter()
            .any(|checkpoint| *checkpoint != worker_root_checkpoints[0])
    {
        return Err(route_message(
            "native worker pool does not share the requested source boundary",
        ));
    }
    let GoalConditionedTacticRuntime {
        catalog,
        encoder,
        report: goal_target,
    } = goal_conditioned_tactic_runtime(
        &root,
        config.optimization,
        config.execution,
        &initial_facts,
    )?;
    let root_checkpoint_sha256 = worker_root_checkpoints[0];
    let reward_spec = route_tactic_reward_spec(&encoder, &initial_facts)?;

    let mut indexed_results = std::thread::scope(|scope| {
        let mut senders = Vec::with_capacity(workers.len());
        let worker_handles = workers
            .into_iter()
            .enumerate()
            .map(|(worker_slot, worker)| {
                let (sender, receiver) = mpsc::channel();
                senders.push(sender);
                let catalog = &catalog;
                scope.spawn(move || {
                    run_tactic_proposal_worker(worker_slot, worker, catalog, receiver)
                })
            })
            .collect::<Vec<_>>();
        let pool = NativeTacticProposalPool {
            senders: Arc::new(senders),
            next_worker: Arc::new(AtomicUsize::new(0)),
            direct_restore_enabled: config.exploration_seeds.len() == 1,
        };
        let coordinator_handles = config
            .exploration_seeds
            .iter()
            .copied()
            .enumerate()
            .map(|(seed_index, seed)| {
                let pool = pool.clone();
                let catalog = &catalog;
                let registry = &registry;
                let encoder = &encoder;
                let reward_spec = &reward_spec;
                let initial_facts = &initial_facts;
                let route_prefix = &route_prefix;
                scope.spawn(move || {
                    run_seed_coordinator(
                        config,
                        &pool,
                        catalog,
                        registry,
                        encoder,
                        reward_spec,
                        initial_facts,
                        route_prefix,
                        root_checkpoint_sha256,
                        seed_index,
                        seed,
                    )
                    .map(|result| (seed_index, result))
                })
            })
            .collect::<Vec<_>>();
        let coordinator_results = coordinator_handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| route_message("native tactic route coordinator panicked"))?
            })
            .collect::<Result<Vec<_>, _>>();
        drop(pool);
        let worker_results = worker_handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| route_message("native tactic route worker thread panicked"))?
            })
            .collect::<Result<Vec<_>, _>>();
        match (coordinator_results, worker_results) {
            (Ok(results), Ok(_)) => Ok(results),
            (Err(error), _) | (Ok(_), Err(error)) => Err(error),
        }
    })?;
    indexed_results.sort_by_key(|(seed_index, _)| *seed_index);
    if indexed_results.len() != config.exploration_seeds.len()
        || indexed_results
            .iter()
            .enumerate()
            .any(|(expected, (actual, _))| expected != *actual)
    {
        return Err(route_message(
            "native tactic route worker pool returned detached seeds",
        ));
    }
    let seed_results = indexed_results
        .into_iter()
        .map(|(_, result)| result)
        .collect::<Vec<_>>();

    let useful_decisions = seed_results.iter().map(|seed| seed.useful_decisions).sum();
    let mut timing = aggregate_route_timing(&seed_results);
    timing.wall_micros = elapsed_micros(campaign_started.elapsed());
    refresh_route_throughput(&mut timing, &seed_results);
    let frontier_availability = seed_results
        .iter()
        .filter_map(|seed| seed.trace.last())
        .fold(
            NativeTacticFrontierAvailability::default(),
            |mut total, trace| {
                total.logical_frontier_records = total
                    .logical_frontier_records
                    .saturating_add(trace.logical_frontier_records);
                total.directly_restorable_native_frontiers = total
                    .directly_restorable_native_frontiers
                    .saturating_add(trace.directly_restorable_native_frontiers);
                total.replay_only_frontiers = total
                    .replay_only_frontiers
                    .saturating_add(trace.replay_only_frontiers);
                total
            },
        );
    let report = NativeTacticRouteReport {
        schema: NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V8.into(),
        optimization_request_sha256: config.optimization.content_sha256,
        execution_binding_sha256: config.execution.content_sha256,
        objective_sha256: config.optimization.terminal_predicate.definition_sha256,
        feature_schema_sha256: encoder.schema_sha256,
        action_schema_sha256: catalog.action_schema_sha256(),
        goal_target,
        reward_spec,
        demonstration_transitions: 0,
        exploration_seeds: config.exploration_seeds.to_vec(),
        workers: worker_count,
        decisions_per_seed: config.decisions_per_seed,
        refit_every_decisions: config.refit_every_decisions,
        successful_seeds: seed_results.iter().filter(|seed| seed.success).count() as u64,
        total_native_ticks: seed_results.iter().map(|seed| seed.native_ticks).sum(),
        total_decisions: seed_results.iter().map(|seed| seed.decisions).sum(),
        useful_decisions,
        frontier_availability,
        timing,
        seeds: seed_results,
    };
    write_new(
        &config.output_root.join("report.json"),
        &serde_json::to_vec_pretty(&report).map_err(route_error)?,
    )?;
    Ok(report)
}

fn launch_tactic_route_worker(
    config: &NativeTacticRouteRunConfig<'_>,
    repository_root: &Path,
    attempt_root: &Path,
    initial_batch: &NativeSuffixBatch,
    terminal: &NativeTerminalBinding,
    card_fixture: &Path,
) -> Result<(NativeSuffixWorkerSession, FactSnapshot, Digest), NativeTacticRouteRunError> {
    let initial_root = attempt_root.join("initial");
    fs::create_dir_all(&initial_root).map_err(route_error)?;
    let initial_batch_path = initial_root.join("request.json");
    write_new(
        &initial_batch_path,
        &serde_json::to_vec_pretty(initial_batch).map_err(route_error)?,
    )?;
    let launch = NativeSuffixWorkerLaunch {
        executable: repository_root.join(&config.execution.executable.path),
        game_data: repository_root.join(&config.execution.game_data.path),
        input_tape: repository_root.join(&config.execution.process_boot_tape.path),
        milestone_program: repository_root.join(&config.execution.milestone_program.path),
        card_fixture: card_fixture.to_path_buf(),
        card_fixture_sha256: config.execution.card_fixture_manifest.sha256,
        working_directory: repository_root.to_path_buf(),
        state_root: attempt_root.join("native-state"),
        world_context_sha256: config.execution.world_context.sha256,
        terminal: terminal.clone(),
        initial_batch: initial_batch_path,
        initial_result: initial_root.join("result.json"),
        initial_winner_tape: None,
    };
    let (worker, initial) = NativeSuffixWorkerSession::launch(&launch).map_err(route_error)?;
    let facts = initial_facts(&initial)?;
    let root_checkpoint_sha256 =
        tactic_root_checkpoint_sha256(worker.identity()).map_err(route_error)?;
    Ok((worker, facts, root_checkpoint_sha256))
}

#[allow(clippy::too_many_arguments)]
fn run_seed_coordinator(
    config: &NativeTacticRouteRunConfig<'_>,
    pool: &NativeTacticProposalPool,
    catalog: &dusklight_learning::tactic_asset::TacticAssetCatalog,
    registry: &FactRegistry,
    encoder: &GoalConditionedTacticFeatureEncoder,
    reward_spec: &TacticRewardSpec,
    initial_facts: &FactSnapshot,
    route_prefix: &InputTape,
    root_checkpoint_sha256: Digest,
    seed_index: usize,
    seed: u64,
) -> Result<NativeTacticSeedResult, NativeTacticRouteRunError> {
    let seed_root = config
        .output_root
        .join(format!("seed-{seed_index:03}-{seed}"));
    let seed_result_path = seed_root.join("seed-result.json");
    if seed_result_path.exists() {
        if !config.resume {
            return Err(route_message("unexpected pre-existing tactic seed result"));
        }
        read_completed_seed_result(&seed_result_path, seed, config.decisions_per_seed)
    } else {
        let result = run_seed(
            config,
            pool,
            catalog,
            registry,
            encoder,
            reward_spec,
            initial_facts,
            route_prefix,
            root_checkpoint_sha256,
            seed_index,
            seed,
        )?;
        write_new(
            &seed_result_path,
            &serde_json::to_vec_pretty(&result).map_err(route_error)?,
        )?;
        Ok(result)
    }
}

struct TimedTacticWorker<'a, W> {
    inner: &'a mut W,
    native_elapsed: Duration,
}

impl<'a, W> TimedTacticWorker<'a, W> {
    fn new(inner: &'a mut W) -> Self {
        Self {
            inner,
            native_elapsed: Duration::ZERO,
        }
    }
}

impl<W: PersistentTacticBatchWorker> PersistentTacticBatchWorker for TimedTacticWorker<'_, W> {
    fn identity(&self) -> &crate::native_suffix_worker::NativeSuffixWorkerIdentity {
        self.inner.identity()
    }

    fn run_tactic_batch(
        &mut self,
        request: &Path,
        result: &Path,
    ) -> Result<ValidatedNativeSuffixBatch, NativeTacticWorkerError> {
        let started = Instant::now();
        let response = self.inner.run_tactic_batch(request, result);
        self.native_elapsed = self.native_elapsed.saturating_add(started.elapsed());
        response
    }
}

struct NativeTacticProposalJob {
    selected: SelectedTactic,
    source_snapshot: FactSnapshot,
    source_route_tape: InputTape,
    checkpoint_source: Option<NativeTacticCheckpointSource>,
    paths: NativeTacticWorkerPaths,
    response: mpsc::SyncSender<Result<NativeTacticProposalWork, NativeTacticRouteRunError>>,
}

struct NativeTacticProposalWork {
    worker_slot: usize,
    outcome: NativeTacticWorkerOutcome,
    native_elapsed: Duration,
    preparation_elapsed: Duration,
}

#[derive(Clone)]
struct NativeTacticProposalPool {
    senders: Arc<Vec<mpsc::Sender<NativeTacticProposalJob>>>,
    next_worker: Arc<AtomicUsize>,
    direct_restore_enabled: bool,
}

#[derive(Clone, Debug)]
struct CachedTacticFrontier {
    worker_slot: usize,
    source: NativeTacticCheckpointSource,
    state_sha256: Digest,
    route_frames: usize,
}

impl NativeTacticProposalPool {
    fn execute_batch(
        &self,
        proposals: &[SelectedTactic],
        source_snapshot: &FactSnapshot,
        source_route_tape: &InputTape,
        cached_frontier: Option<&CachedTacticFrontier>,
        paths_root: &Path,
    ) -> Result<Vec<NativeTacticProposalWork>, NativeTacticRouteRunError> {
        let mut responses = Vec::with_capacity(proposals.len());
        if self.senders.is_empty() {
            return Err(route_message("native tactic proposal pool is empty"));
        }
        for (proposal_index, selected) in proposals.iter().enumerate() {
            let proposal_root = paths_root.join(format!("proposal-{proposal_index:03}"));
            fs::create_dir_all(&proposal_root).map_err(route_error)?;
            let (response, receiver) = mpsc::sync_channel(1);
            let direct = (proposal_index == 0)
                .then_some(cached_frontier)
                .flatten()
                .filter(|frontier| {
                    self.direct_restore_enabled && frontier.worker_slot < self.senders.len()
                });
            let worker_slot = direct.map_or_else(
                || self.next_worker.fetch_add(1, Ordering::Relaxed) % self.senders.len(),
                |frontier| frontier.worker_slot,
            );
            self.senders[worker_slot]
                .send(NativeTacticProposalJob {
                    selected: selected.clone(),
                    source_snapshot: source_snapshot.clone(),
                    source_route_tape: source_route_tape.clone(),
                    checkpoint_source: direct.map(|frontier| frontier.source.clone()),
                    paths: NativeTacticWorkerPaths {
                        request: proposal_root.join("request.json"),
                        result: proposal_root.join("result.json"),
                    },
                    response,
                })
                .map_err(|_| route_message("native tactic proposal pool stopped"))?;
            responses.push(receiver);
        }
        responses
            .into_iter()
            .map(|receiver| {
                receiver
                    .recv()
                    .map_err(|_| route_message("native tactic proposal worker stopped"))?
            })
            .collect()
    }
}

fn run_tactic_proposal_worker(
    worker_slot: usize,
    mut worker: NativeSuffixWorkerSession,
    catalog: &dusklight_learning::tactic_asset::TacticAssetCatalog,
    receiver: mpsc::Receiver<NativeTacticProposalJob>,
) -> Result<(), NativeTacticRouteRunError> {
    loop {
        let job = receiver.recv();
        let Ok(job) = job else {
            break;
        };
        let execution_started = Instant::now();
        let mut timed_worker = TimedTacticWorker::new(&mut worker);
        let outcome = execute_selected_tactic(
            &mut timed_worker,
            &job.selected,
            catalog,
            &[],
            &job.source_snapshot,
            &job.source_route_tape,
            job.checkpoint_source.as_ref(),
            &job.paths,
        )
        .map_err(route_error);
        let native_elapsed = timed_worker.native_elapsed;
        let work = outcome.map(|outcome| {
            let elapsed = execution_started.elapsed();
            NativeTacticProposalWork {
                worker_slot,
                outcome,
                native_elapsed,
                preparation_elapsed: elapsed.saturating_sub(native_elapsed),
            }
        });
        let _ = job.response.send(work);
    }
    worker.shutdown().map_err(route_error)
}

fn elapsed_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn decision_trace_is_useful(decision: &NativeTacticDecisionTrace) -> bool {
    decision.terminal
        || decision.reward > 0.0
        || decision.goal_distance_after < decision.goal_distance_before
}

fn decision_evaluated_ticks(decision: &NativeTacticDecisionTrace) -> u64 {
    if decision.proposal_batch.is_empty() {
        u64::from(decision.reward_components.duration_ticks)
    } else {
        decision
            .proposal_batch
            .iter()
            .map(|proposal| u64::from(proposal.realized_ticks))
            .sum()
    }
}

fn per_second_millionths(count: u64, wall_micros: u64) -> u64 {
    if count == 0 || wall_micros == 0 {
        return 0;
    }
    let scaled = u128::from(count)
        .saturating_mul(1_000_000)
        .saturating_mul(1_000_000)
        / u128::from(wall_micros);
    u64::try_from(scaled).unwrap_or(u64::MAX)
}

fn aggregate_route_timing(seeds: &[NativeTacticSeedResult]) -> NativeTacticRouteTiming {
    let mut timing = NativeTacticRouteTiming::default();
    for seed in seeds {
        timing.wall_micros = timing.wall_micros.saturating_add(seed.timing.wall_micros);
        timing.tactic_selection_micros = timing
            .tactic_selection_micros
            .saturating_add(seed.timing.tactic_selection_micros);
        timing.checkpoint_branching_micros = timing
            .checkpoint_branching_micros
            .saturating_add(seed.timing.checkpoint_branching_micros);
        timing.tactic_execution_micros = timing
            .tactic_execution_micros
            .saturating_add(seed.timing.tactic_execution_micros);
        timing.native_simulation_micros = timing
            .native_simulation_micros
            .saturating_add(seed.timing.native_simulation_micros);
        timing.tactic_preparation_and_fact_extraction_micros = timing
            .tactic_preparation_and_fact_extraction_micros
            .saturating_add(seed.timing.tactic_preparation_and_fact_extraction_micros);
        timing.model_update_micros = timing
            .model_update_micros
            .saturating_add(seed.timing.model_update_micros);
        timing.evidence_projection_and_persistence_micros = timing
            .evidence_projection_and_persistence_micros
            .saturating_add(seed.timing.evidence_projection_and_persistence_micros);
        timing.retained_candidate_artifact_micros = timing
            .retained_candidate_artifact_micros
            .saturating_add(seed.timing.retained_candidate_artifact_micros);
    }
    refresh_route_throughput(&mut timing, seeds);
    timing
}

fn refresh_route_throughput(
    timing: &mut NativeTacticRouteTiming,
    seeds: &[NativeTacticSeedResult],
) {
    let useful_decisions = seeds.iter().map(|seed| seed.useful_decisions).sum();
    let native_ticks = seeds.iter().map(|seed| seed.native_ticks).sum();
    let episodes = seeds.iter().map(|seed| seed.episodes).sum();
    timing.useful_decisions_per_second_millionths =
        per_second_millionths(useful_decisions, timing.wall_micros);
    timing.native_ticks_per_second_millionths =
        per_second_millionths(native_ticks, timing.wall_micros);
    timing.episodes_per_second_millionths = per_second_millionths(episodes, timing.wall_micros);
}

#[allow(clippy::too_many_arguments)]
fn run_seed(
    config: &NativeTacticRouteRunConfig<'_>,
    pool: &NativeTacticProposalPool,
    catalog: &dusklight_learning::tactic_asset::TacticAssetCatalog,
    registry: &FactRegistry,
    encoder: &GoalConditionedTacticFeatureEncoder,
    reward_spec: &TacticRewardSpec,
    initial_facts: &FactSnapshot,
    route_prefix: &InputTape,
    root_checkpoint_sha256: Digest,
    seed_index: usize,
    seed: u64,
) -> Result<NativeTacticSeedResult, NativeTacticRouteRunError> {
    let seed_root = config
        .output_root
        .join(format!("seed-{seed_index:03}-{seed}"));
    let resuming_seed = seed_root.exists();
    let (mut campaign, mut trace, mut selection_counts, mut native_ticks, mut episode) =
        if resuming_seed {
            if !config.resume {
                return Err(route_message(
                    "unexpected pre-existing tactic seed evidence",
                ));
            }
            resume_seed(
                config,
                &seed_root,
                encoder.schema_sha256,
                root_checkpoint_sha256,
                seed_index,
                seed,
            )?
        } else {
            fs::create_dir_all(&seed_root).map_err(route_error)?;
            let current =
                LearnerState::build(initial_facts.clone(), registry, catalog, &[], |_| true)
                    .map_err(route_error)?;
            let campaign = TacticQCampaign::new(
                encoder.schema_sha256,
                config.optimization.terminal_predicate.definition_sha256,
                root_checkpoint_sha256,
                seed_group(seed_index, 0)?,
                current,
                route_prefix.clone(),
                route_option_value_config(seed),
                TacticExplorationConfig {
                    seed,
                    epsilon_per_million: config.epsilon_per_million,
                },
            )
            .map_err(route_error)?;
            (campaign, Vec::new(), BTreeMap::new(), 0, 0)
        };
    let has_performance =
        resuming_seed && seed_performance_exists(&seed_root, campaign.decision_index)?;
    let performance = if resuming_seed {
        load_seed_performance(&seed_root, campaign.decision_index)?
    } else {
        NativeTacticSeedPerformance {
            schema: TACTIC_ROUTE_PERFORMANCE_SCHEMA_V1.into(),
            decisions: 0,
            useful_decisions: 0,
            timing: NativeTacticRouteTiming::default(),
        }
    };
    let mut timing = performance.timing;
    let prior_wall_micros = timing.wall_micros;
    let invocation_started = Instant::now();
    let mut useful_decisions = if has_performance {
        performance.useful_decisions
    } else {
        trace
            .iter()
            .filter(|decision| decision_trace_is_useful(decision))
            .count() as u64
    };
    let source_frame = config.optimization.route.source_boundary_index;
    let horizon = config.optimization.budgets.exploration_horizon_ticks;
    let maximum_tactic_ticks = catalog
        .entries()
        .iter()
        .map(|entry| u64::from(entry.description().duration.maximum_ticks))
        .max()
        .ok_or_else(|| route_message("tactic catalog is empty"))?;
    let encode = |facts: &FactSnapshot| encoder.encode(facts);
    let checkpoint_root = seed_root.join("checkpoints");
    let checkpoint_content_root = tactic_content_store_path(&seed_root);
    let content_store =
        TacticQContentStore::initialize(&checkpoint_content_root).map_err(route_error)?;
    let mut rolling_checkpoint = None;
    // Process-local handles intentionally do not survive campaign resume.
    let mut cached_frontier: Option<CachedTacticFrontier> = None;

    while campaign.decision_index < config.decisions_per_seed
        && native_ticks < config.optimization.budgets.simulated_tick_budget
        && campaign.current.snapshot.terminal.reached != Some(true)
    {
        if cancellation_requested(config) {
            timing.wall_micros =
                prior_wall_micros.saturating_add(elapsed_micros(invocation_started.elapsed()));
            persist_seed_performance(
                &seed_root,
                campaign.decision_index,
                &timing,
                useful_decisions,
            )?;
            pause_tactic_campaign(&seed_root, &campaign)?;
            return Err(route_cancelled("native tactic route paused"));
        }
        let periodic_branch = campaign.decision_index > 0
            && campaign.decision_index % config.branch_every_decisions == 0;
        if !campaign.replay.is_empty() && periodic_branch {
            let branch_started = Instant::now();
            episode = episode
                .checked_add(1)
                .ok_or_else(|| route_message("episode counter overflowed"))?;
            let maximum_frontier_frames = usize::try_from(
                source_frame.saturating_add(horizon.saturating_sub(maximum_tactic_ticks)),
            )
            .map_err(route_error)?;
            let [root, frontier] = campaign
                .sample_root_and_frontier(
                    seed,
                    frontier_sampling_round(episode),
                    &[],
                    maximum_frontier_frames,
                )
                .map_err(route_error)?;
            let prefer_root = episode % 4 == 0;
            campaign
                .restore_branch(
                    if prefer_root { &root } else { &frontier },
                    seed_group(seed_index, episode)?,
                    registry,
                    catalog,
                    &[],
                    |_| true,
                )
                .map_err(route_error)?;
            timing.checkpoint_branching_micros = timing
                .checkpoint_branching_micros
                .saturating_add(elapsed_micros(branch_started.elapsed()));
        }

        // Reserve horizon for the tactic Q actually selected at this state,
        // not for the longest unrelated entry in the catalog. This lets short
        // tactics compose beyond `horizon - catalog_maximum` while still
        // branching before any selected tactic could exceed the bound.
        //
        // Restoring a branch can change the selected tactic. Recheck until the
        // preview fits; the periodic root sample guarantees convergence because
        // every catalog entry is itself bounded by the exploration horizon.
        let proposal_batch = loop {
            let suffix_ticks = campaign
                .route_tape
                .frames
                .len()
                .saturating_sub(source_frame as usize) as u64;
            let selection_started = Instant::now();
            let mut preview = campaign
                .decide_batch(catalog, &[], &encode, TACTIC_PROPOSALS_PER_DECISION)
                .map_err(route_error)?;
            timing.tactic_selection_micros = timing
                .tactic_selection_micros
                .saturating_add(elapsed_micros(selection_started.elapsed()));
            let primary = preview
                .proposals
                .first()
                .ok_or_else(|| route_message("tactic proposal batch is empty"))?;
            let selected_maximum_ticks = catalog
                .entry(&primary.descriptor.option_id)
                .ok_or_else(|| route_message("selected tactic is absent from its catalog"))?
                .description()
                .duration
                .maximum_ticks;
            if selected_tactic_fits_horizon(suffix_ticks, selected_maximum_ticks, horizon) {
                preview.proposals.retain(|proposal| {
                    catalog
                        .entry(&proposal.descriptor.option_id)
                        .is_some_and(|entry| {
                            selected_tactic_fits_horizon(
                                suffix_ticks,
                                entry.description().duration.maximum_ticks,
                                horizon,
                            )
                        })
                });
                break preview;
            }
            let branch_started = Instant::now();
            episode = episode
                .checked_add(1)
                .ok_or_else(|| route_message("episode counter overflowed"))?;
            let maximum_frontier_frames = usize::try_from(
                source_frame
                    .saturating_add(horizon.saturating_sub(u64::from(selected_maximum_ticks))),
            )
            .map_err(route_error)?;
            let [root, frontier] = campaign
                .sample_root_and_frontier(
                    seed,
                    frontier_sampling_round(episode),
                    &[],
                    maximum_frontier_frames,
                )
                .map_err(route_error)?;
            let prefer_root = episode % 4 == 0;
            campaign
                .restore_branch(
                    if prefer_root { &root } else { &frontier },
                    seed_group(seed_index, episode)?,
                    registry,
                    catalog,
                    &[],
                    |_| true,
                )
                .map_err(route_error)?;
            timing.checkpoint_branching_micros = timing
                .checkpoint_branching_micros
                .saturating_add(elapsed_micros(branch_started.elapsed()));
        };

        let decision_index = campaign.decision_index;
        let refit_model = tactic_model_refit_due(
            decision_index
                .checked_add(1)
                .ok_or_else(|| route_message("decision index overflowed"))?,
            config.refit_every_decisions,
        );
        let paths_root = seed_root
            .join("native")
            .join(format!("decision-{decision_index:06}"));
        fs::create_dir_all(&paths_root).map_err(route_error)?;
        let execution_started = Instant::now();
        let source_snapshot = campaign.current.snapshot.clone();
        let source_route_tape = campaign.route_tape.clone();
        let source_snapshot_sha256 = source_snapshot.content_sha256().map_err(route_error)?;
        let usable_cached_frontier = cached_frontier.as_ref().filter(|frontier| {
            pool.direct_restore_enabled
                && frontier.state_sha256 == source_snapshot_sha256
                && frontier.route_frames == source_route_tape.frames.len()
        });
        let directly_restored_frontier = usable_cached_frontier.is_some();
        let proposal_work = pool.execute_batch(
            &proposal_batch.proposals,
            &source_snapshot,
            &source_route_tape,
            usable_cached_frontier,
            &paths_root,
        )?;
        let native_elapsed = proposal_work.iter().fold(Duration::ZERO, |total, work| {
            total.saturating_add(work.native_elapsed)
        });
        let preparation_elapsed = proposal_work.iter().fold(Duration::ZERO, |total, work| {
            total.saturating_add(work.preparation_elapsed)
        });
        let proposal_worker_slots = proposal_work
            .iter()
            .map(|work| work.worker_slot)
            .collect::<Vec<_>>();
        let evaluated = proposal_work
            .into_iter()
            .map(|work| {
                campaign
                    .evaluate_rewarded_outcome(work.outcome, &encode, reward_spec)
                    .map_err(route_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let execution_elapsed = execution_started.elapsed();
        timing.tactic_execution_micros = timing
            .tactic_execution_micros
            .saturating_add(elapsed_micros(execution_elapsed));
        timing.native_simulation_micros = timing
            .native_simulation_micros
            .saturating_add(elapsed_micros(native_elapsed));
        timing.tactic_preparation_and_fact_extraction_micros = timing
            .tactic_preparation_and_fact_extraction_micros
            .saturating_add(elapsed_micros(preparation_elapsed));
        let winner_index = (1..evaluated.len()).fold(0, |winner, candidate| {
            if proposal_outcome_is_better(&evaluated[candidate], &evaluated[winner]) {
                candidate
            } else {
                winner
            }
        });
        let winning_outcome = evaluated[winner_index].outcome.clone();
        let expected_transition = evaluated[winner_index].transition.clone();
        let decision = TacticQDecision {
            ranking: proposal_batch.ranking,
            selected: winning_outcome.selected.clone(),
        };
        let evaluated_native_ticks = evaluated.iter().fold(0_u64, |total, proposal| {
            total.saturating_add(u64::from(
                proposal.outcome.execution.duration.realized_ticks,
            ))
        });
        let model_started = Instant::now();
        let step = campaign
            .retain_and_refit_rewarded(
                decision,
                winning_outcome.clone(),
                catalog,
                &[],
                registry,
                &encode,
                |_| true,
                reward_spec,
                refit_model,
            )
            .map_err(route_error)?;
        if step.step.transition != expected_transition
            || step.reward != evaluated[winner_index].reward
        {
            return Err(route_message(
                "retained tactic proposal differs from its pre-admission evaluation",
            ));
        }
        timing.model_update_micros = timing
            .model_update_micros
            .saturating_add(elapsed_micros(model_started.elapsed()));
        cached_frontier = match (
            winning_outcome.retained_native_checkpoint.as_ref(),
            winning_outcome
                .retained_native_boundary_fingerprint
                .as_ref(),
        ) {
            (Some(checkpoint), Some(boundary_fingerprint))
                if checkpoint.route_ticks
                    == winning_outcome
                        .route_tape
                        .frames
                        .len()
                        .saturating_sub(source_frame as usize) as u64 =>
            {
                Some(CachedTacticFrontier {
                    worker_slot: proposal_worker_slots[winner_index],
                    source: NativeTacticCheckpointSource {
                        restore_identity: checkpoint.restore_identity.clone(),
                        boundary_fingerprint: boundary_fingerprint.clone(),
                        route_ticks: checkpoint.route_ticks as usize,
                    },
                    state_sha256: step.step.transition.after_state_sha256,
                    route_frames: winning_outcome.route_tape.frames.len(),
                })
            }
            (None, None) => None,
            _ => {
                return Err(route_message(
                    "retained native checkpoint lacks its exact endpoint boundary",
                ));
            }
        };
        let evidence_started = Instant::now();
        let selected = &step.step.decision.selected;
        *selection_counts
            .entry(selected.descriptor.option_id.clone())
            .or_default() += 1;
        let selected_q = step
            .step
            .decision
            .ranking
            .values
            .ranked
            .iter()
            .find(|ranked| ranked.descriptor == selected.descriptor)
            .map(|ranked| ranked.mean_q);
        let best_q = step
            .step
            .decision
            .ranking
            .values
            .ranked
            .first()
            .map(|ranked| ranked.mean_q);
        native_ticks = native_ticks.saturating_add(evaluated_native_ticks);
        let frontier_cells = campaign
            .frontier_archive()
            .map_err(route_error)?
            .tactic_len();
        let before_features = encoder
            .encode(&step.step.transition.before)
            .map_err(route_error)?;
        let after_features = encoder
            .encode(&step.step.transition.after)
            .map_err(route_error)?;
        let proposal_traces = evaluated
            .iter()
            .enumerate()
            .map(|(index, proposal)| {
                let after_features = encoder
                    .encode(&proposal.transition.after)
                    .map_err(route_error)?;
                Ok(NativeTacticProposalTrace {
                    option_id: proposal.outcome.selected.descriptor.option_id.clone(),
                    selection_reason: proposal.outcome.selected.reason,
                    reward: proposal.reward.training_reward,
                    reward_components: proposal.reward.clone(),
                    realized_ticks: proposal.outcome.execution.duration.realized_ticks,
                    terminal: proposal.outcome.terminal,
                    goal_distance_after: after_features[encoder.goal_distance_feature()],
                    after_snapshot_sha256: proposal.transition.after_state_sha256,
                    retained: index == winner_index,
                })
            })
            .collect::<Result<Vec<_>, NativeTacticRouteRunError>>()?;
        let decision_trace = NativeTacticDecisionTrace {
            decision_index,
            episode,
            route_suffix_ticks: campaign
                .route_tape
                .frames
                .len()
                .saturating_sub(source_frame as usize) as u64,
            selected_option_id: selected.descriptor.option_id.clone(),
            selection_reason: selected.reason,
            selected_q,
            best_q,
            reward: step.reward.training_reward,
            reward_components: step.reward.clone(),
            goal_distance_before: before_features[encoder.goal_distance_feature()],
            goal_distance_after: after_features[encoder.goal_distance_feature()],
            terminal: step.step.transition.value_sample.terminal,
            frontier_cells,
            logical_frontier_records: frontier_cells.saturating_add(1),
            directly_restorable_native_frontiers: usize::from(directly_restored_frontier),
            replay_only_frontiers: frontier_cells
                .saturating_sub(usize::from(directly_restored_frontier)),
            visited_states: campaign.visited_state_count(),
            before: tactic_state_trace(&step.step.transition.before)?,
            after: tactic_state_trace(&step.step.transition.after)?,
            measurements: Vec::new(),
            applicable_tactics: Vec::new(),
            proposal_batch: proposal_traces,
        };
        if decision_trace_is_useful(&decision_trace) {
            useful_decisions = useful_decisions.saturating_add(1);
        }
        let proposal_records = evaluated
            .iter()
            .zip(&decision_trace.proposal_batch)
            .map(|(proposal, trace)| {
                Ok(NativeTacticProposalRecord {
                    trace: trace.clone(),
                    transition: content_store
                        .store_option_transition(&proposal.transition, &proposal.outcome.route_tape)
                        .map_err(route_error)?,
                })
            })
            .collect::<Result<Vec<_>, NativeTacticRouteRunError>>()?;
        let transition = proposal_records[winner_index].transition;
        let root_tape = content_store
            .store_tape(route_prefix)
            .map_err(route_error)?;
        append_tactic_decision_record(
            &seed_root,
            &decision_record(
                &decision_trace,
                campaign.episode_group,
                campaign.root_checkpoint_sha256,
                root_tape,
                transition,
                proposal_records,
            ),
        )?;
        trace.push(decision_trace);
        if cancellation_requested(config) {
            timing.evidence_projection_and_persistence_micros = timing
                .evidence_projection_and_persistence_micros
                .saturating_add(elapsed_micros(evidence_started.elapsed()));
            timing.wall_micros =
                prior_wall_micros.saturating_add(elapsed_micros(invocation_started.elapsed()));
            persist_seed_performance(
                &seed_root,
                campaign.decision_index,
                &timing,
                useful_decisions,
            )?;
            pause_tactic_campaign(&seed_root, &campaign)?;
            return Err(route_cancelled("native tactic route paused"));
        }
        let run_finished = step.step.transition.value_sample.terminal
            || campaign.decision_index >= config.decisions_per_seed
            || native_ticks >= config.optimization.budgets.simulated_tick_budget;
        if !run_finished
            && tactic_checkpoint_due(
                campaign.decision_index,
                config.optimization.resume.checkpoint_every_candidates,
                false,
            )
        {
            let checkpoint = campaign
                .write_checkpoint_with_store(&checkpoint_root, &checkpoint_content_root)
                .map_err(route_error)?;
            advance_rolling_checkpoint(&checkpoint_root, &mut rolling_checkpoint, checkpoint)?;
        }
        timing.evidence_projection_and_persistence_micros = timing
            .evidence_projection_and_persistence_micros
            .saturating_add(elapsed_micros(evidence_started.elapsed()));
        if step.step.transition.value_sample.terminal {
            break;
        }
    }

    let final_persistence_started = Instant::now();
    compact_tactic_decision_journal(&seed_root)?;
    let success = campaign.current.snapshot.terminal.reached == Some(true);
    let final_checkpoint = campaign
        .write_checkpoint_with_store(
            &seed_root.join("final-checkpoint"),
            &checkpoint_content_root,
        )
        .map_err(route_error)?;
    remove_rolling_checkpoint(&checkpoint_root, &mut rolling_checkpoint)?;
    let (successful_tape, final_result) = if success {
        let retained_candidate_started = Instant::now();
        let tape_path = seed_root.join("successful.tape");
        write_new(
            &tape_path,
            &campaign.route_tape.encode().map_err(route_error)?,
        )?;
        let result_path = seed_root.join("final-result.json");
        write_new(
            &result_path,
            &serde_json::to_vec_pretty(&campaign.final_result().map_err(route_error)?)
                .map_err(route_error)?,
        )?;
        timing.retained_candidate_artifact_micros = timing
            .retained_candidate_artifact_micros
            .saturating_add(elapsed_micros(retained_candidate_started.elapsed()));
        (Some(path_text(&tape_path)), Some(path_text(&result_path)))
    } else {
        (None, None)
    };
    timing.evidence_projection_and_persistence_micros = timing
        .evidence_projection_and_persistence_micros
        .saturating_add(elapsed_micros(final_persistence_started.elapsed()));
    timing.wall_micros =
        prior_wall_micros.saturating_add(elapsed_micros(invocation_started.elapsed()));
    timing.useful_decisions_per_second_millionths =
        per_second_millionths(useful_decisions, timing.wall_micros);
    timing.native_ticks_per_second_millionths =
        per_second_millionths(native_ticks, timing.wall_micros);
    timing.episodes_per_second_millionths =
        per_second_millionths(episode.saturating_add(1), timing.wall_micros);
    if trace.len() as u64 != campaign.decision_index {
        return Err(route_message(
            "in-memory tactic trace is detached from the completed campaign",
        ));
    }
    persist_seed_performance(
        &seed_root,
        campaign.decision_index,
        &timing,
        useful_decisions,
    )?;
    Ok(NativeTacticSeedResult {
        seed,
        success,
        decisions: campaign.decision_index,
        episodes: episode + 1,
        native_ticks,
        replay_rows: campaign.replay.len(),
        visited_states: campaign.visited_state_count(),
        useful_decisions,
        timing,
        selection_counts,
        diagnostics: None,
        final_checkpoint: Some(path_text(&final_checkpoint)),
        graph: None,
        successful_tape,
        final_result,
        trace,
    })
}

fn cancellation_requested(config: &NativeTacticRouteRunConfig<'_>) -> bool {
    config
        .cancellation
        .is_some_and(|cancellation| cancellation.load(Ordering::Acquire))
}

fn pause_tactic_campaign(
    seed_root: &Path,
    campaign: &TacticQCampaign,
) -> Result<PathBuf, NativeTacticRouteRunError> {
    campaign
        .write_checkpoint_with_store(
            &seed_root
                .join("pause-checkpoints")
                .join(format!("decision-{:06}", campaign.decision_index)),
            &tactic_content_store_path(seed_root),
        )
        .map_err(route_error)
}

fn seed_performance_root(seed_root: &Path) -> PathBuf {
    seed_root.join("performance")
}

fn seed_performance_prefix(decisions: u64) -> String {
    format!("decision-{decisions:06}-attempt-")
}

fn seed_performance_exists(
    seed_root: &Path,
    decisions: u64,
) -> Result<bool, NativeTacticRouteRunError> {
    let root = seed_performance_root(seed_root);
    if !root.exists() {
        return Ok(false);
    }
    let prefix = seed_performance_prefix(decisions);
    Ok(fs::read_dir(root).map_err(route_error)?.any(|entry| {
        entry.is_ok_and(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
                && entry.file_name().to_string_lossy().starts_with(&prefix)
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
        })
    }))
}

fn persist_seed_performance(
    seed_root: &Path,
    decisions: u64,
    timing: &NativeTacticRouteTiming,
    useful_decisions: u64,
) -> Result<(), NativeTacticRouteRunError> {
    let performance = NativeTacticSeedPerformance {
        schema: TACTIC_ROUTE_PERFORMANCE_SCHEMA_V1.into(),
        decisions,
        useful_decisions,
        timing: timing.clone(),
    };
    let root = seed_performance_root(seed_root);
    fs::create_dir_all(&root).map_err(route_error)?;
    for attempt in 0..MAX_ROUTE_ATTEMPTS {
        let path = root.join(format!(
            "{}{attempt:04}.json",
            seed_performance_prefix(decisions)
        ));
        if path.exists() {
            let existing: NativeTacticSeedPerformance = read_bounded_json(&path)?;
            if existing == performance {
                return Ok(());
            }
            continue;
        }
        return write_new(
            &path,
            &serde_json::to_vec_pretty(&performance).map_err(route_error)?,
        );
    }
    Err(route_message(
        "immutable tactic performance checkpoint capacity is exhausted",
    ))
}

fn load_seed_performance(
    seed_root: &Path,
    decisions: u64,
) -> Result<NativeTacticSeedPerformance, NativeTacticRouteRunError> {
    let root = seed_performance_root(seed_root);
    if !root.exists() {
        return Ok(NativeTacticSeedPerformance {
            schema: TACTIC_ROUTE_PERFORMANCE_SCHEMA_V1.into(),
            decisions,
            useful_decisions: 0,
            timing: NativeTacticRouteTiming::default(),
        });
    }
    let prefix = seed_performance_prefix(decisions);
    let mut paths = fs::read_dir(root)
        .map_err(route_error)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
                && entry.file_name().to_string_lossy().starts_with(&prefix)
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    let Some(path) = paths.last() else {
        return Ok(NativeTacticSeedPerformance {
            schema: TACTIC_ROUTE_PERFORMANCE_SCHEMA_V1.into(),
            decisions,
            useful_decisions: 0,
            timing: NativeTacticRouteTiming::default(),
        });
    };
    let performance: NativeTacticSeedPerformance = read_bounded_json(path)?;
    if performance.schema != TACTIC_ROUTE_PERFORMANCE_SCHEMA_V1
        || performance.decisions != decisions
        || performance.useful_decisions > decisions
    {
        return Err(route_message(
            "paused tactic performance checkpoint is invalid",
        ));
    }
    Ok(performance)
}

type ResumedSeedState = (
    TacticQCampaign,
    Vec<NativeTacticDecisionTrace>,
    BTreeMap<String, u64>,
    u64,
    u64,
);

fn resume_seed(
    config: &NativeTacticRouteRunConfig<'_>,
    seed_root: &Path,
    feature_schema_sha256: Digest,
    root_checkpoint_sha256: Digest,
    seed_index: usize,
    seed: u64,
) -> Result<ResumedSeedState, NativeTacticRouteRunError> {
    let (checkpoint_decision, checkpoint_path) = latest_pause_checkpoint(seed_root)?;
    let checkpoint =
        TacticQCampaign::read_checkpoint_payload(&checkpoint_path).map_err(route_error)?;
    let expected_exploration = TacticExplorationConfig {
        seed,
        epsilon_per_million: config.epsilon_per_million,
    };
    if checkpoint.decision_index != checkpoint_decision
        || checkpoint.decision_index > config.decisions_per_seed
        || checkpoint.feature_schema_sha256 != feature_schema_sha256
        || checkpoint.objective_sha256 != config.optimization.terminal_predicate.definition_sha256
        || checkpoint.root_checkpoint_sha256 != root_checkpoint_sha256
        || checkpoint.model_config != route_option_value_config(seed)
        || checkpoint.exploration != expected_exploration
    {
        return Err(route_message(
            "paused tactic checkpoint does not match this authenticated run",
        ));
    }
    let campaign = TacticQCampaign::resume(checkpoint).map_err(route_error)?;
    if campaign.replay.len() as u64 != campaign.decision_index {
        return Err(route_message(
            "paused tactic checkpoint has a detached decision history",
        ));
    }
    let trace = read_resumed_trace(seed_root, campaign.decision_index)?;
    let episode = trace.last().map_or(0, |decision| decision.episode);
    if campaign.episode_group != seed_group(seed_index, episode)?
        || trace
            .iter()
            .zip(&campaign.replay)
            .any(|(decision, replay)| decision.selected_option_id != replay.execution.option_id)
    {
        return Err(route_message(
            "paused tactic checkpoint and decision trace disagree",
        ));
    }
    let mut selection_counts = BTreeMap::new();
    let mut native_ticks = 0_u64;
    for decision in &trace {
        *selection_counts
            .entry(decision.selected_option_id.clone())
            .or_default() += 1;
        native_ticks = native_ticks
            .checked_add(decision_evaluated_ticks(decision))
            .ok_or_else(|| route_message("resumed native tick count overflowed"))?;
    }
    if native_ticks > config.optimization.budgets.simulated_tick_budget {
        return Err(route_message(
            "paused tactic checkpoint exceeds the simulated tick budget",
        ));
    }
    Ok((campaign, trace, selection_counts, native_ticks, episode))
}

fn latest_pause_checkpoint(seed_root: &Path) -> Result<(u64, PathBuf), NativeTacticRouteRunError> {
    let pause_root = seed_root.join("pause-checkpoints");
    let mut checkpoints = Vec::new();
    for entry in fs::read_dir(&pause_root).map_err(|error| {
        route_message(format!(
            "paused tactic checkpoint is unavailable at {}: {error}",
            pause_root.display()
        ))
    })? {
        let entry = entry.map_err(route_error)?;
        let metadata = entry.file_type().map_err(route_error)?;
        if !metadata.is_dir() || metadata.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(decision) = name
            .strip_prefix("decision-")
            .and_then(|value| value.parse::<u64>().ok())
        else {
            continue;
        };
        let mut files = fs::read_dir(entry.path())
            .map_err(route_error)?
            .filter_map(Result::ok)
            .filter(|candidate| {
                candidate
                    .file_type()
                    .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
                    && candidate
                        .file_name()
                        .to_string_lossy()
                        .starts_with("tactic-q-")
                    && candidate
                        .path()
                        .extension()
                        .is_some_and(|value| value == TACTIC_Q_CHECKPOINT_EXTENSION)
            })
            .map(|candidate| candidate.path())
            .collect::<Vec<_>>();
        if files.len() != 1 {
            return Err(route_message(
                "paused tactic checkpoint directory must contain exactly one checkpoint",
            ));
        }
        checkpoints.push((decision, files.remove(0)));
    }
    checkpoints
        .into_iter()
        .max_by_key(|(decision, _)| *decision)
        .ok_or_else(|| route_message("no resumable paused tactic checkpoint exists"))
}

fn read_resumed_trace(
    seed_root: &Path,
    decision_count: u64,
) -> Result<Vec<NativeTacticDecisionTrace>, NativeTacticRouteRunError> {
    let trace = read_tactic_decision_journal(seed_root)?;
    if trace.len() as u64 != decision_count {
        return Err(route_message(
            "paused tactic decision journal does not exactly match its checkpoint",
        ));
    }
    Ok(trace)
}

pub fn tactic_decision_journal_path(seed_root: &Path) -> PathBuf {
    seed_root.join(NATIVE_TACTIC_DECISION_JOURNAL_FILE)
}

pub fn has_tactic_decision_journal(seed_root: &Path) -> bool {
    tactic_decision_journal_path(seed_root).is_file()
        || tactic_decision_segments_root(seed_root).is_dir()
}

pub fn tactic_content_store_path(seed_root: &Path) -> PathBuf {
    seed_root.join(NATIVE_TACTIC_CONTENT_STORE_DIRECTORY)
}

pub fn read_tactic_decision_journal(
    seed_root: &Path,
) -> Result<Vec<NativeTacticDecisionTrace>, NativeTacticRouteRunError> {
    if !has_tactic_decision_journal(seed_root) {
        return Ok(Vec::new());
    }
    let store =
        TacticQContentStore::open(tactic_content_store_path(seed_root)).map_err(route_error)?;
    read_tactic_decision_records(seed_root)?
        .into_iter()
        .map(|record| project_tactic_decision_record(&store, record))
        .collect()
}

fn read_tactic_decision_records(
    seed_root: &Path,
) -> Result<Vec<NativeTacticDecisionRecord>, NativeTacticRouteRunError> {
    let mut records = read_compacted_tactic_decision_records(seed_root)?;
    let path = tactic_decision_journal_path(seed_root);
    if !path.exists() {
        return Ok(records);
    }
    let metadata = fs::symlink_metadata(&path).map_err(route_error)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(route_message(
            "tactic decision journal is not a physical file",
        ));
    }
    let bytes = fs::read(&path).map_err(route_error)?;
    let decoded = decode_tactic_decision_journal(&bytes)?;
    for record in decoded.records {
        let index = usize::try_from(record.decision_index).map_err(route_error)?;
        if index < records.len() {
            if records[index] != record {
                return Err(route_message(
                    "active tactic journal conflicts with its compacted segment",
                ));
            }
        } else if index == records.len() {
            records.push(record);
        } else {
            return Err(route_message(
                "active tactic journal is detached from its compacted segments",
            ));
        }
    }
    Ok(records)
}

pub fn project_tactic_decision_graph(
    seed_root: &Path,
) -> Result<Option<TacticCampaignGraphProjection>, NativeTacticRouteRunError> {
    if !has_tactic_decision_journal(seed_root) {
        return Ok(None);
    }
    let replay = load_tactic_journal_replay(seed_root)?;
    let first = replay
        .transitions
        .first()
        .ok_or_else(|| route_message("tactic graph has no root transition"))?;
    let last = replay
        .transitions
        .last()
        .ok_or_else(|| route_message("tactic graph has no current transition"))?;
    let root_checkpoint_sha256 = first.source_checkpoint_sha256;
    let root_state_sha256 = first.before_state_sha256;
    let current = (last.next_checkpoint_sha256, last.after_state_sha256);
    let mut archive = BehaviorArchive::default();
    for (index, (transition, route)) in replay.transitions.iter().zip(&replay.routes).enumerate() {
        archive
            .consider_tactic_endpoint(
                replay.root_checkpoint_sha256,
                transition.clone(),
                route.clone(),
                index as u64,
            )
            .map_err(route_error)?;
    }
    let retained = archive.tactic_route_checkpoints().collect::<BTreeSet<_>>();
    let mut nodes = BTreeMap::new();
    let mut edges = Vec::with_capacity(replay.records.len());
    for (record, transition) in replay.records.iter().zip(&replay.transitions) {
        for (checkpoint_sha256, state_sha256, state, terminal) in [
            (
                transition.source_checkpoint_sha256,
                transition.before_state_sha256,
                &transition.before,
                transition.before.terminal.reached == Some(true),
            ),
            (
                transition.next_checkpoint_sha256,
                transition.after_state_sha256,
                &transition.after,
                transition.value_sample.terminal,
            ),
        ] {
            let identity = (checkpoint_sha256, state_sha256);
            let node = TacticCampaignGraphProjectionNode {
                checkpoint_sha256,
                state_sha256,
                stage: state.world.stage.clone(),
                room: state.world.room,
                player_position: state.player.position_f32_bits.map(f32::from_bits),
                terminal,
                retained_frontier: retained.contains(&checkpoint_sha256),
                current: identity == current,
            };
            if nodes
                .insert(identity, node.clone())
                .is_some_and(|existing| existing != node)
            {
                return Err(route_message(
                    "tactic decision journal has conflicting graph nodes",
                ));
            }
        }
        edges.push(TacticCampaignGraphProjectionEdge {
            edge_index: record.decision_index,
            episode_group: record.episode_group,
            before_state_sha256: transition.before_state_sha256,
            after_state_sha256: transition.after_state_sha256,
            source_checkpoint_sha256: transition.source_checkpoint_sha256,
            next_checkpoint_sha256: transition.next_checkpoint_sha256,
            option_id: transition.value_sample.action.option_id.clone(),
            reward: record.reward,
            duration_ticks: transition.execution.duration.realized_ticks,
            terminal: transition.value_sample.terminal,
            start_frame: transition.execution.realized_tape_range.start_frame,
            end_frame_exclusive: transition.execution.realized_tape_range.end_frame_exclusive,
        });
    }
    let mut reachable = BTreeSet::from([(root_checkpoint_sha256, root_state_sha256)]);
    loop {
        let before = reachable.len();
        for edge in &edges {
            if reachable.contains(&(edge.source_checkpoint_sha256, edge.before_state_sha256)) {
                reachable.insert((edge.next_checkpoint_sha256, edge.after_state_sha256));
            }
        }
        if reachable.len() == before {
            break;
        }
    }
    Ok(Some(TacticCampaignGraphProjection {
        schema: "dusklight-tactic-campaign-graph-projection/v1".into(),
        root_checkpoint_sha256,
        root_state_sha256,
        root_connected: nodes.keys().all(|identity| reachable.contains(identity)),
        frontier_cells: retained.len(),
        nodes: nodes.into_values().collect(),
        edges,
    }))
}

pub fn project_tactic_decision_diagnostics(
    seed_root: &Path,
) -> Result<Option<TacticCampaignDiagnostics>, NativeTacticRouteRunError> {
    if !has_tactic_decision_journal(seed_root) {
        return Ok(None);
    }
    let replay = load_tactic_journal_replay(seed_root)?;
    let graph = project_tactic_decision_graph(seed_root)?
        .ok_or_else(|| route_message("tactic diagnostics have no graph"))?;
    let mut compositions = BTreeMap::<u64, Vec<Digest>>::new();
    let mut selected_actions = BTreeSet::new();
    for (record, transition) in replay.records.iter().zip(&replay.transitions) {
        let bytes = serde_cbor::to_vec(&transition.value_sample.action).map_err(route_error)?;
        let digest = Digest(Sha256::digest(&bytes).into());
        selected_actions.insert(digest);
        compositions
            .entry(record.episode_group)
            .or_default()
            .push(digest);
    }
    let mut composition_counts = BTreeMap::<Vec<Digest>, usize>::new();
    for composition in compositions.into_values().filter(|row| !row.is_empty()) {
        *composition_counts.entry(composition).or_default() += 1;
    }
    let episode_groups = replay
        .records
        .iter()
        .map(|record| record.episode_group)
        .collect::<Vec<_>>();
    let (logical_frontier_records, directly_restorable_native_frontiers, replay_only_frontiers) =
        replay
            .records
            .last()
            .map_or((graph.nodes.len(), 0, graph.frontier_cells), |record| {
                if record.logical_frontier_records == 0 {
                    (graph.nodes.len(), 0, graph.frontier_cells)
                } else {
                    (
                        record.logical_frontier_records,
                        record.directly_restorable_native_frontiers,
                        record.replay_only_frontiers,
                    )
                }
            });
    Ok(Some(TacticCampaignDiagnostics {
        replay_rows: replay.transitions.len(),
        frontier_cells: graph.frontier_cells,
        logical_frontier_records,
        directly_restorable_native_frontiers,
        replay_only_frontiers,
        unique_selected_actions: selected_actions.len(),
        zero_diversity_selection: replay.transitions.len() >= 2 && selected_actions.len() <= 1,
        repeated_identical_compositions: composition_counts.values().any(|count| *count > 1),
        no_progress_loop: has_no_progress_loop(&replay.transitions, &episode_groups)
            .map_err(route_error)?,
        frontier_lost_root_connectivity: !graph.root_connected,
    }))
}

pub fn materialize_tactic_decision_route(
    seed_root: &Path,
    decision_index: u64,
) -> Result<InputTape, NativeTacticRouteRunError> {
    let replay = load_tactic_journal_replay(seed_root)?;
    let target_index = usize::try_from(decision_index).map_err(route_error)?;
    let target_record = replay
        .records
        .get(target_index)
        .ok_or_else(|| route_message("tactic decision route is absent"))?;
    if target_record.decision_index != decision_index {
        return Err(route_message("tactic decision route index is detached"));
    }
    replay
        .routes
        .get(target_index)
        .cloned()
        .ok_or_else(|| route_message("tactic decision route is absent"))
}

struct TacticJournalReplay {
    root_checkpoint_sha256: Digest,
    records: Vec<NativeTacticDecisionRecord>,
    transitions: Vec<dusklight_learning::option_transition::OptionTransitionSample>,
    routes: Vec<InputTape>,
}

fn load_tactic_journal_replay(
    seed_root: &Path,
) -> Result<TacticJournalReplay, NativeTacticRouteRunError> {
    let records = read_tactic_decision_records(seed_root)?;
    let first_record = records
        .first()
        .ok_or_else(|| route_message("tactic decision journal is empty"))?;
    if first_record.root_checkpoint_sha256 == Digest::ZERO
        || records.iter().any(|record| {
            record.root_checkpoint_sha256 != first_record.root_checkpoint_sha256
                || record.root_tape != first_record.root_tape
        })
    {
        return Err(route_message(
            "tactic decision journal has conflicting root identities",
        ));
    }
    let store =
        TacticQContentStore::open(tactic_content_store_path(seed_root)).map_err(route_error)?;
    let root_tape = store
        .load_tape(first_record.root_tape)
        .map_err(route_error)?;
    let transitions = records
        .iter()
        .map(|record| {
            store
                .load_option_transition(record.transition)
                .map_err(route_error)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let first_transition = transitions
        .first()
        .ok_or_else(|| route_message("tactic decision journal has no root transition"))?;
    if route_checkpoint(first_record.root_checkpoint_sha256, &root_tape).map_err(route_error)?
        != first_transition.source_checkpoint_sha256
        || root_tape.frames.len() as u64
            != first_transition.execution.realized_tape_range.start_frame
    {
        return Err(route_message(
            "tactic decision journal root tape is detached",
        ));
    }
    let root_identity = (
        first_transition.source_checkpoint_sha256,
        first_transition.before_state_sha256,
    );
    let mut parents = BTreeMap::<(Digest, Digest), usize>::new();
    for (index, transition) in transitions.iter().enumerate() {
        parents
            .entry((
                transition.next_checkpoint_sha256,
                transition.after_state_sha256,
            ))
            .or_insert(index);
    }
    let routes = transitions
        .iter()
        .enumerate()
        .map(|(index, _)| {
            materialize_journal_route(index, &root_tape, root_identity, &parents, &transitions)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for ((record, transition), route) in records.iter().zip(&transitions).zip(&routes) {
        transition
            .execution
            .validate_against_tape(route)
            .map_err(route_error)?;
        if record.terminal != transition.value_sample.terminal
            || record.reward_components.duration_ticks
                != transition.execution.duration.realized_ticks
            || route_checkpoint(first_record.root_checkpoint_sha256, route).map_err(route_error)?
                != transition.next_checkpoint_sha256
            || route.frames.len() as u64
                != transition.execution.realized_tape_range.end_frame_exclusive
        {
            return Err(route_message(
                "tactic decision journal transition route is detached",
            ));
        }
    }
    Ok(TacticJournalReplay {
        root_checkpoint_sha256: first_record.root_checkpoint_sha256,
        records,
        transitions,
        routes,
    })
}

fn materialize_journal_route(
    target_index: usize,
    root_tape: &InputTape,
    root_identity: (Digest, Digest),
    parents: &BTreeMap<(Digest, Digest), usize>,
    transitions: &[dusklight_learning::option_transition::OptionTransitionSample],
) -> Result<InputTape, NativeTacticRouteRunError> {
    let target = transitions
        .get(target_index)
        .ok_or_else(|| route_message("tactic decision route is absent"))?;
    let mut cursor = (target.source_checkpoint_sha256, target.before_state_sha256);
    let mut fragments = vec![target.execution.emitted_raw_actions.clone()];
    while cursor != root_identity {
        let parent_index = *parents
            .get(&cursor)
            .ok_or_else(|| route_message("tactic decision route is detached from its root"))?;
        if parent_index >= target_index {
            return Err(route_message(
                "tactic decision route parent is not journaled before its child",
            ));
        }
        let parent = &transitions[parent_index];
        fragments.push(parent.execution.emitted_raw_actions.clone());
        cursor = (parent.source_checkpoint_sha256, parent.before_state_sha256);
        if fragments.len() > transitions.len() {
            return Err(route_message("tactic decision route contains a cycle"));
        }
    }
    let mut route = root_tape.clone();
    for fragment in fragments.into_iter().rev() {
        route.frames.extend(fragment);
    }
    route.validate().map_err(route_error)?;
    Ok(route)
}

fn decision_record(
    trace: &NativeTacticDecisionTrace,
    episode_group: u64,
    root_checkpoint_sha256: Digest,
    root_tape: StoredContentRef,
    transition: StoredContentRef,
    proposal_batch: Vec<NativeTacticProposalRecord>,
) -> NativeTacticDecisionRecord {
    NativeTacticDecisionRecord {
        decision_index: trace.decision_index,
        episode: trace.episode,
        episode_group,
        route_suffix_ticks: trace.route_suffix_ticks,
        selection_reason: trace.selection_reason,
        selected_q: trace.selected_q,
        best_q: trace.best_q,
        reward: trace.reward,
        reward_components: trace.reward_components.clone(),
        goal_distance_before: trace.goal_distance_before,
        goal_distance_after: trace.goal_distance_after,
        terminal: trace.terminal,
        frontier_cells: trace.frontier_cells,
        logical_frontier_records: trace.logical_frontier_records,
        directly_restorable_native_frontiers: trace.directly_restorable_native_frontiers,
        replay_only_frontiers: trace.replay_only_frontiers,
        visited_states: trace.visited_states,
        root_checkpoint_sha256,
        root_tape,
        transition,
        proposal_batch,
    }
}

fn project_tactic_decision_record(
    store: &TacticQContentStore,
    record: NativeTacticDecisionRecord,
) -> Result<NativeTacticDecisionTrace, NativeTacticRouteRunError> {
    let transition = store
        .load_option_transition(record.transition)
        .map_err(route_error)?;
    if transition.execution.duration.realized_ticks != record.reward_components.duration_ticks
        || transition.value_sample.action.option_id.is_empty()
        || transition.before.content_sha256().map_err(route_error)? == Digest::ZERO
        || transition.after.content_sha256().map_err(route_error)? == Digest::ZERO
    {
        return Err(route_message(
            "tactic decision journal references are detached",
        ));
    }
    if record.logical_frontier_records != 0
        && (record.logical_frontier_records != record.frontier_cells.saturating_add(1)
            || record
                .directly_restorable_native_frontiers
                .saturating_add(record.replay_only_frontiers)
                != record.frontier_cells)
    {
        return Err(route_message(
            "tactic frontier availability accounting is detached",
        ));
    }
    let proposal_batch = record
        .proposal_batch
        .iter()
        .map(|proposal| {
            let candidate = store
                .load_option_transition(proposal.transition)
                .map_err(route_error)?;
            if candidate.value_sample.action.option_id != proposal.trace.option_id
                || candidate.execution.duration.realized_ticks != proposal.trace.realized_ticks
                || candidate.value_sample.terminal != proposal.trace.terminal
                || candidate.after_state_sha256 != proposal.trace.after_snapshot_sha256
                || candidate.value_sample.reward.to_bits() != proposal.trace.reward.to_bits()
            {
                return Err(route_message(
                    "tactic proposal journal reference is detached",
                ));
            }
            Ok(proposal.trace.clone())
        })
        .collect::<Result<Vec<_>, NativeTacticRouteRunError>>()?;
    if !proposal_batch.is_empty()
        && (proposal_batch
            .iter()
            .filter(|proposal| proposal.retained)
            .count()
            != 1
            || !record.proposal_batch.iter().any(|proposal| {
                proposal.trace.retained && proposal.transition == record.transition
            }))
    {
        return Err(route_message(
            "tactic proposal journal has no unique retained transition",
        ));
    }
    Ok(NativeTacticDecisionTrace {
        decision_index: record.decision_index,
        episode: record.episode,
        route_suffix_ticks: record.route_suffix_ticks,
        selected_option_id: transition.value_sample.action.option_id.clone(),
        selection_reason: record.selection_reason,
        selected_q: record.selected_q,
        best_q: record.best_q,
        reward: record.reward,
        reward_components: record.reward_components,
        goal_distance_before: record.goal_distance_before,
        goal_distance_after: record.goal_distance_after,
        terminal: record.terminal,
        frontier_cells: record.frontier_cells,
        logical_frontier_records: record.logical_frontier_records,
        directly_restorable_native_frontiers: record.directly_restorable_native_frontiers,
        replay_only_frontiers: record.replay_only_frontiers,
        visited_states: record.visited_states,
        before: tactic_state_trace(&transition.before)?,
        after: tactic_state_trace(&transition.after)?,
        measurements: Vec::new(),
        applicable_tactics: Vec::new(),
        proposal_batch,
    })
}

fn append_tactic_decision_record(
    seed_root: &Path,
    decision: &NativeTacticDecisionRecord,
) -> Result<(), NativeTacticRouteRunError> {
    fs::create_dir_all(seed_root).map_err(route_error)?;
    let path = tactic_decision_journal_path(seed_root);
    ensure_tactic_decision_journal(&path)?;
    let compacted_count = compacted_tactic_decision_count(seed_root)?;
    let bytes = fs::read(&path).map_err(route_error)?;
    let decoded = decode_tactic_decision_journal(&bytes)?;
    let active_count = decoded
        .records
        .iter()
        .filter(|record| record.decision_index >= compacted_count)
        .count() as u64;
    if compacted_count.saturating_add(active_count) != decision.decision_index {
        return Err(route_message(
            "tactic decision journal append index is detached",
        ));
    }
    if decoded.valid_bytes != bytes.len() {
        OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|file| file.set_len(decoded.valid_bytes as u64))
            .map_err(route_error)?;
    }
    let record = encode_tactic_decision_record(decision)?;
    let mut file = OpenOptions::new()
        .append(true)
        .open(&path)
        .map_err(route_error)?;
    file.write_all(&record)
        .and_then(|_| file.sync_data())
        .map_err(route_error)?;
    drop(file);
    if active_count.saturating_add(1) >= NATIVE_TACTIC_DECISION_COMPACTION_RECORDS {
        compact_tactic_decision_journal(seed_root)?;
    }
    Ok(())
}

fn ensure_tactic_decision_journal(path: &Path) -> Result<(), NativeTacticRouteRunError> {
    if path.exists() {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| route_message("tactic decision journal has no parent"))?;
    let partial = parent.join(format!(
        ".{NATIVE_TACTIC_DECISION_JOURNAL_FILE}.{}.partial",
        std::process::id()
    ));
    if partial.exists() {
        fs::remove_file(&partial).map_err(route_error)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(route_error)?;
    file.write_all(NATIVE_TACTIC_DECISION_JOURNAL_MAGIC)
        .and_then(|_| file.write_all(&NATIVE_TACTIC_DECISION_JOURNAL_VERSION.to_le_bytes()))
        .and_then(|_| file.write_all(&0_u16.to_le_bytes()))
        .and_then(|_| file.sync_all())
        .map_err(route_error)?;
    drop(file);
    match fs::rename(&partial, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(&partial).map_err(route_error)
        }
        Err(error) => Err(route_error(error)),
    }
}

struct DecodedTacticDecisionJournal {
    records: Vec<NativeTacticDecisionRecord>,
    valid_bytes: usize,
}

#[derive(Clone, Copy)]
struct TacticDecisionSegmentHeader {
    start_index: u64,
    record_count: u64,
    raw_len: usize,
    raw_sha256: [u8; 32],
}

fn tactic_decision_segments_root(seed_root: &Path) -> PathBuf {
    seed_root.join(NATIVE_TACTIC_DECISION_SEGMENTS_DIRECTORY)
}

fn tactic_decision_segment_paths(
    seed_root: &Path,
) -> Result<Vec<PathBuf>, NativeTacticRouteRunError> {
    let root = tactic_decision_segments_root(seed_root);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let metadata = fs::symlink_metadata(&root).map_err(route_error)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(route_message(
            "tactic decision segment root is not a physical directory",
        ));
    }
    let mut paths = fs::read_dir(&root)
        .map_err(route_error)?
        .map(|entry| entry.map(|entry| entry.path()).map_err(route_error))
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| path.extension().and_then(|value| value.to_str()) == Some("dtqz"));
    if paths.len() > MAXIMUM_TACTIC_DECISION_SEGMENTS {
        return Err(route_message(
            "tactic decision journal exceeds its segment bound",
        ));
    }
    paths.sort();
    Ok(paths)
}

fn read_tactic_decision_segment_header(
    path: &Path,
) -> Result<TacticDecisionSegmentHeader, NativeTacticRouteRunError> {
    let metadata = fs::symlink_metadata(path).map_err(route_error)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() < NATIVE_TACTIC_DECISION_SEGMENT_HEADER_SIZE as u64
        || metadata.len()
            > (NATIVE_TACTIC_DECISION_SEGMENT_HEADER_SIZE
                + MAXIMUM_TACTIC_DECISION_SEGMENT_BYTES
                + 1024 * 1024) as u64
    {
        return Err(route_message(
            "tactic decision segment is not a physical file",
        ));
    }
    let mut bytes = [0_u8; NATIVE_TACTIC_DECISION_SEGMENT_HEADER_SIZE];
    OpenOptions::new()
        .read(true)
        .open(path)
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(route_error)?;
    decode_tactic_decision_segment_header(&bytes)
}

fn decode_tactic_decision_segment_header(
    bytes: &[u8],
) -> Result<TacticDecisionSegmentHeader, NativeTacticRouteRunError> {
    if bytes.len() < NATIVE_TACTIC_DECISION_SEGMENT_HEADER_SIZE
        || &bytes[..8] != NATIVE_TACTIC_DECISION_SEGMENT_MAGIC
        || u16::from_le_bytes(bytes[8..10].try_into().expect("fixed slice"))
            != NATIVE_TACTIC_DECISION_SEGMENT_VERSION
        || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) != 0
    {
        return Err(route_message("tactic decision segment header is invalid"));
    }
    let start_index = u64::from_le_bytes(bytes[12..20].try_into().expect("fixed slice"));
    let record_count = u64::from_le_bytes(bytes[20..28].try_into().expect("fixed slice"));
    let raw_len_u64 = u64::from_le_bytes(bytes[28..36].try_into().expect("fixed slice"));
    let raw_len = usize::try_from(raw_len_u64).map_err(route_error)?;
    let raw_sha256 = bytes[36..68].try_into().expect("fixed slice");
    if record_count == 0
        || record_count > NATIVE_TACTIC_DECISION_COMPACTION_RECORDS
        || raw_len < NATIVE_TACTIC_DECISION_JOURNAL_HEADER_SIZE
        || raw_len > MAXIMUM_TACTIC_DECISION_SEGMENT_BYTES
    {
        return Err(route_message("tactic decision segment bounds are invalid"));
    }
    Ok(TacticDecisionSegmentHeader {
        start_index,
        record_count,
        raw_len,
        raw_sha256,
    })
}

fn compacted_tactic_decision_count(seed_root: &Path) -> Result<u64, NativeTacticRouteRunError> {
    let mut next = 0_u64;
    for path in tactic_decision_segment_paths(seed_root)? {
        let header = read_tactic_decision_segment_header(&path)?;
        if header.start_index != next {
            return Err(route_message(
                "tactic decision segments are overlapping or detached",
            ));
        }
        next = next
            .checked_add(header.record_count)
            .ok_or_else(|| route_message("tactic decision segment count overflowed"))?;
    }
    Ok(next)
}

fn read_compacted_tactic_decision_records(
    seed_root: &Path,
) -> Result<Vec<NativeTacticDecisionRecord>, NativeTacticRouteRunError> {
    let mut records: Vec<NativeTacticDecisionRecord> = Vec::new();
    for path in tactic_decision_segment_paths(seed_root)? {
        let bytes = fs::read(&path).map_err(route_error)?;
        let segment = decode_tactic_decision_segment(&bytes)?;
        if segment
            .first()
            .is_none_or(|record| record.decision_index != records.len() as u64)
        {
            return Err(route_message(
                "tactic decision segments are overlapping or detached",
            ));
        }
        records.extend(segment);
    }
    Ok(records)
}

fn decode_tactic_decision_segment(
    bytes: &[u8],
) -> Result<Vec<NativeTacticDecisionRecord>, NativeTacticRouteRunError> {
    let header = decode_tactic_decision_segment_header(bytes)?;
    let raw = zstd::bulk::decompress(
        &bytes[NATIVE_TACTIC_DECISION_SEGMENT_HEADER_SIZE..],
        header.raw_len,
    )
    .map_err(route_error)?;
    if raw.len() != header.raw_len
        || <Sha256 as sha2::Digest>::digest(&raw)[..] != header.raw_sha256
    {
        return Err(route_message(
            "tactic decision segment payload digest is invalid",
        ));
    }
    let decoded = decode_tactic_decision_journal(&raw)?;
    if decoded.valid_bytes != raw.len()
        || decoded.records.len() as u64 != header.record_count
        || decoded
            .records
            .first()
            .is_none_or(|record| record.decision_index != header.start_index)
    {
        return Err(route_message("tactic decision segment payload is detached"));
    }
    Ok(decoded.records)
}

fn compact_tactic_decision_journal(seed_root: &Path) -> Result<(), NativeTacticRouteRunError> {
    let path = tactic_decision_journal_path(seed_root);
    ensure_tactic_decision_journal(&path)?;
    let compacted_count = compacted_tactic_decision_count(seed_root)?;
    let bytes = fs::read(&path).map_err(route_error)?;
    let decoded = decode_tactic_decision_journal(&bytes)?;
    if decoded.valid_bytes != bytes.len() {
        return Err(route_message(
            "cannot compact a tactic decision journal with a truncated tail",
        ));
    }
    let records = decoded
        .records
        .into_iter()
        .filter(|record| record.decision_index >= compacted_count)
        .collect::<Vec<_>>();
    if records.is_empty() {
        rewrite_tactic_decision_journal(&path, &[])?;
        return Ok(());
    }
    if records[0].decision_index != compacted_count {
        return Err(route_message(
            "tactic decision journal is detached from its compacted segments",
        ));
    }
    let raw = encode_tactic_decision_journal(&records)?;
    if raw.len() > MAXIMUM_TACTIC_DECISION_SEGMENT_BYTES {
        return Err(route_message(
            "tactic decision segment exceeds its size bound",
        ));
    }
    let compressed = zstd::bulk::compress(&raw, NATIVE_TACTIC_DECISION_SEGMENT_COMPRESSION_LEVEL)
        .map_err(route_error)?;
    let start_index = records[0].decision_index;
    let end_index = start_index
        .checked_add(records.len() as u64)
        .ok_or_else(|| route_message("tactic decision segment index overflowed"))?;
    let mut segment =
        Vec::with_capacity(NATIVE_TACTIC_DECISION_SEGMENT_HEADER_SIZE + compressed.len());
    segment.extend_from_slice(NATIVE_TACTIC_DECISION_SEGMENT_MAGIC);
    segment.extend_from_slice(&NATIVE_TACTIC_DECISION_SEGMENT_VERSION.to_le_bytes());
    segment.extend_from_slice(&0_u16.to_le_bytes());
    segment.extend_from_slice(&start_index.to_le_bytes());
    segment.extend_from_slice(&(records.len() as u64).to_le_bytes());
    segment.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    segment.extend_from_slice(&Sha256::digest(&raw));
    segment.extend_from_slice(&compressed);

    let root = tactic_decision_segments_root(seed_root);
    fs::create_dir_all(&root).map_err(route_error)?;
    let destination = root.join(format!("segment-{start_index:012}-{end_index:012}.dtqz"));
    if destination.exists() {
        if decode_tactic_decision_segment(&fs::read(&destination).map_err(route_error)?)? != records
        {
            return Err(route_message(
                "existing tactic decision segment conflicts with compaction",
            ));
        }
    } else {
        let partial = root.join(format!(
            ".segment-{start_index:012}-{end_index:012}.{}.partial",
            std::process::id()
        ));
        if partial.exists() {
            fs::remove_file(&partial).map_err(route_error)?;
        }
        write_new(&partial, &segment)?;
        fs::rename(&partial, &destination).map_err(route_error)?;
    }
    rewrite_tactic_decision_journal(&path, &[])
}

fn rewrite_tactic_decision_journal(
    path: &Path,
    records: &[NativeTacticDecisionRecord],
) -> Result<(), NativeTacticRouteRunError> {
    let bytes = encode_tactic_decision_journal(records)?;
    let parent = path
        .parent()
        .ok_or_else(|| route_message("tactic decision journal has no parent"))?;
    let partial = parent.join(format!(
        ".{NATIVE_TACTIC_DECISION_JOURNAL_FILE}.{}.rewrite",
        std::process::id()
    ));
    if partial.exists() {
        fs::remove_file(&partial).map_err(route_error)?;
    }
    write_new(&partial, &bytes)?;
    if path.exists() {
        fs::remove_file(path).map_err(route_error)?;
    }
    fs::rename(&partial, path).map_err(route_error)
}

fn encode_tactic_decision_journal(
    records: &[NativeTacticDecisionRecord],
) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(NATIVE_TACTIC_DECISION_JOURNAL_MAGIC);
    bytes.extend_from_slice(&NATIVE_TACTIC_DECISION_JOURNAL_VERSION.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    for record in records {
        bytes.extend_from_slice(&encode_tactic_decision_record(record)?);
    }
    Ok(bytes)
}

fn encode_tactic_decision_record(
    decision: &NativeTacticDecisionRecord,
) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    let payload = serde_cbor::to_vec(decision).map_err(route_error)?;
    if payload.len() > MAXIMUM_TACTIC_DECISION_RECORD_BYTES {
        return Err(route_message(
            "tactic decision journal record exceeds its size bound",
        ));
    }
    let payload_len = u32::try_from(payload.len()).map_err(route_error)?;
    let payload_sha256: [u8; 32] = Sha256::digest(&payload).into();
    let mut record = Vec::with_capacity(NATIVE_TACTIC_DECISION_RECORD_HEADER_SIZE + payload.len());
    record.extend_from_slice(&payload_len.to_le_bytes());
    record.extend_from_slice(&payload_sha256);
    record.extend_from_slice(&payload);
    Ok(record)
}

fn decode_tactic_decision_journal(
    bytes: &[u8],
) -> Result<DecodedTacticDecisionJournal, NativeTacticRouteRunError> {
    if bytes.len() < NATIVE_TACTIC_DECISION_JOURNAL_HEADER_SIZE
        || &bytes[..8] != NATIVE_TACTIC_DECISION_JOURNAL_MAGIC
        || u16::from_le_bytes(bytes[8..10].try_into().expect("fixed slice"))
            != NATIVE_TACTIC_DECISION_JOURNAL_VERSION
        || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) != 0
    {
        return Err(route_message("tactic decision journal header is invalid"));
    }
    let mut records: Vec<NativeTacticDecisionRecord> = Vec::new();
    let mut cursor = NATIVE_TACTIC_DECISION_JOURNAL_HEADER_SIZE;
    while cursor < bytes.len() {
        let remaining = bytes.len() - cursor;
        if remaining < NATIVE_TACTIC_DECISION_RECORD_HEADER_SIZE {
            break;
        }
        let payload_len =
            u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().expect("fixed slice")) as usize;
        if payload_len > MAXIMUM_TACTIC_DECISION_RECORD_BYTES {
            return Err(route_message(
                "tactic decision journal record length is invalid",
            ));
        }
        let record_len = NATIVE_TACTIC_DECISION_RECORD_HEADER_SIZE
            .checked_add(payload_len)
            .ok_or_else(|| route_message("tactic decision journal record length overflows"))?;
        if remaining < record_len {
            break;
        }
        let expected_sha256: [u8; 32] = bytes[cursor + 4..cursor + 36]
            .try_into()
            .expect("fixed slice");
        let payload =
            &bytes[cursor + NATIVE_TACTIC_DECISION_RECORD_HEADER_SIZE..cursor + record_len];
        let actual_sha256: [u8; 32] = Sha256::digest(payload).into();
        if actual_sha256 != expected_sha256 {
            return Err(route_message(
                "tactic decision journal record digest is invalid",
            ));
        }
        let mut deserializer = serde_cbor::Deserializer::from_slice(payload);
        let decision =
            NativeTacticDecisionRecord::deserialize(&mut deserializer).map_err(route_error)?;
        deserializer.end().map_err(route_error)?;
        let expected_index = records.first().map_or(decision.decision_index, |first| {
            first.decision_index + records.len() as u64
        });
        if decision.decision_index != expected_index {
            return Err(route_message(
                "tactic decision journal record index is detached",
            ));
        }
        records.push(decision);
        cursor += record_len;
    }
    Ok(DecodedTacticDecisionJournal {
        records,
        valid_bytes: cursor,
    })
}

fn read_completed_seed_result(
    path: &Path,
    seed: u64,
    decisions_per_seed: u64,
) -> Result<NativeTacticSeedResult, NativeTacticRouteRunError> {
    let result: NativeTacticSeedResult = read_bounded_json(path)?;
    if result.seed != seed
        || result.decisions > decisions_per_seed
        || result.useful_decisions > result.decisions
        || result.success != result.successful_tape.is_some()
        || result.success != result.final_result.is_some()
        || (!result.success && result.timing.retained_candidate_artifact_micros != 0)
        || result.trace.len() as u64 != result.decisions
        || result
            .trace
            .iter()
            .enumerate()
            .any(|(index, decision)| decision.decision_index != index as u64)
        || result.native_ticks
            != result
                .trace
                .iter()
                .map(decision_evaluated_ticks)
                .sum::<u64>()
    {
        return Err(route_message(
            "completed tactic seed result is invalid or belongs to another run",
        ));
    }
    Ok(result)
}

fn read_bounded_json<T: for<'de> Deserialize<'de>>(
    path: &Path,
) -> Result<T, NativeTacticRouteRunError> {
    let metadata = fs::symlink_metadata(path).map_err(route_error)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_RESUME_JSON_BYTES
    {
        return Err(route_message(format!(
            "resumable tactic JSON is invalid or oversized: {}",
            path.display()
        )));
    }
    serde_json::from_slice(&fs::read(path).map_err(route_error)?).map_err(route_error)
}

fn reserve_attempt_root(output_root: &Path) -> Result<PathBuf, NativeTacticRouteRunError> {
    let attempts = output_root.join("attempts");
    fs::create_dir_all(&attempts).map_err(route_error)?;
    for index in 0..MAX_ROUTE_ATTEMPTS {
        let attempt = attempts.join(format!("attempt-{index:04}"));
        match fs::create_dir(&attempt) {
            Ok(()) => return Ok(attempt),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(route_error(error)),
        }
    }
    Err(route_message("tactic route attempt capacity is exhausted"))
}

fn tactic_state_trace(
    facts: &FactSnapshot,
) -> Result<NativeTacticStateTrace, NativeTacticRouteRunError> {
    let room = facts.world.room;
    Ok(NativeTacticStateTrace {
        snapshot_sha256: facts.content_sha256().map_err(route_error)?,
        stage: facts.world.stage.clone(),
        room,
        layer: facts.world.layer,
        point: facts.world.point,
        simulation_tick: facts.simulation_tick,
        tape_frame: facts.tape_frame,
        player_position: facts.player.position_f32_bits.map(f32::from_bits),
        player_velocity: facts
            .player
            .velocity_f32_bits
            .map(|bits| bits.map(f32::from_bits)),
        player_procedure: facts.player.procedure,
        player_contacts: facts.player.contacts,
        event_running: facts.event.as_ref().map(|event| event.running),
        event_id: facts.event.as_ref().map(|event| event.event_id),
        terminal_reached: facts.terminal.reached,
        actor_count: facts.actors.len(),
        same_room_actor_count: facts
            .actors
            .iter()
            .filter(|actor| actor.current_room == room)
            .count(),
        recent_option_id: facts
            .recent_option
            .as_ref()
            .map(|option| option.option_id.clone()),
    })
}

fn frontier_sampling_round(episode: u64) -> u64 {
    episode.saturating_sub(1 + episode / 4)
}

fn tactic_checkpoint_due(decision_index: u64, interval: u64, terminal: bool) -> bool {
    terminal || decision_index % interval == 0
}

fn tactic_model_refit_due(decision_count: u64, interval: u64) -> bool {
    decision_count == 1 || decision_count % interval == 0
}

fn advance_rolling_checkpoint(
    directory: &Path,
    current: &mut Option<PathBuf>,
    next: PathBuf,
) -> Result<(), NativeTacticRouteRunError> {
    if next.parent() != Some(directory) || !next.is_file() {
        return Err(route_message(
            "rolling tactic checkpoint is outside its checkpoint directory",
        ));
    }
    if let Some(previous) = current.replace(next.clone()) {
        if previous != next {
            if previous.parent() != Some(directory) {
                return Err(route_message(
                    "previous rolling tactic checkpoint is outside its checkpoint directory",
                ));
            }
            fs::remove_file(previous).map_err(route_error)?;
        }
    }
    Ok(())
}

fn remove_rolling_checkpoint(
    directory: &Path,
    current: &mut Option<PathBuf>,
) -> Result<(), NativeTacticRouteRunError> {
    let Some(previous) = current.take() else {
        return Ok(());
    };
    if previous.parent() != Some(directory) {
        return Err(route_message(
            "rolling tactic checkpoint is outside its checkpoint directory",
        ));
    }
    fs::remove_file(previous).map_err(route_error)
}

fn initial_probe_batch(
    config: &NativeTacticRouteRunConfig<'_>,
) -> Result<NativeSuffixBatch, NativeTacticRouteRunError> {
    tactic_root_probe_batch(config.optimization, config.execution)
}

#[derive(Clone, Debug, PartialEq)]
struct GoalTransitionTarget {
    source_stage: String,
    source_room: i8,
    destination_stage: String,
    destination_room: i8,
    destination_point: i16,
    coordinate: [f32; 3],
    supporting_load_triggers: usize,
    source_inventory_sha256: Digest,
}

pub(crate) struct GoalConditionedTacticRuntime {
    pub catalog: dusklight_learning::tactic_asset::TacticAssetCatalog,
    pub encoder: GoalConditionedTacticFeatureEncoder,
    pub report: NativeTacticGoalTargetReport,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticGoalTargetReport {
    pub source_stage: String,
    pub source_room: i8,
    pub destination_stage: String,
    pub destination_room: i8,
    pub destination_point: i16,
    pub coordinate: [f32; 3],
    pub source_coordinate: [f32; 3],
    pub tactic_targets: Vec<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub route_sequences: Vec<Vec<[f32; 3]>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authored_route_ids: Vec<String>,
    pub supporting_load_triggers: usize,
    pub source_inventory_sha256: Digest,
    pub authored_route_coordinates_used: bool,
}

impl GoalTransitionTarget {
    fn report(
        &self,
        source_coordinate: [f32; 3],
        tactic_targets: Vec<[f32; 3]>,
        route_sequences: Vec<Vec<[f32; 3]>>,
        authored_route_ids: Vec<String>,
    ) -> NativeTacticGoalTargetReport {
        let authored_route_coordinates_used = !authored_route_ids.is_empty();
        NativeTacticGoalTargetReport {
            source_stage: self.source_stage.clone(),
            source_room: self.source_room,
            destination_stage: self.destination_stage.clone(),
            destination_room: self.destination_room,
            destination_point: self.destination_point,
            coordinate: self.coordinate,
            source_coordinate,
            tactic_targets,
            route_sequences,
            authored_route_ids,
            supporting_load_triggers: self.supporting_load_triggers,
            source_inventory_sha256: self.source_inventory_sha256,
            authored_route_coordinates_used,
        }
    }
}

pub(crate) fn goal_conditioned_tactic_runtime(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    initial_facts: &FactSnapshot,
) -> Result<GoalConditionedTacticRuntime, NativeTacticRouteRunError> {
    let (target, inventory) = resolve_goal_transition_target(root, optimization, execution)?;
    if initial_facts.world.stage != target.source_stage
        || initial_facts.world.room != target.source_room
    {
        return Err(route_message(
            "native source observation differs from the objective's source world",
        ));
    }
    let source_coordinate = initial_facts.player.position_f32_bits.map(f32::from_bits);
    let (tactic_targets, route_sequences, authored_route_ids) = goal_route_targets(
        source_coordinate,
        target.coordinate,
        target.source_room,
        &inventory,
    )?;
    let maximum_ticks = goal_tactic_maximum_ticks(optimization.budgets.exploration_horizon_ticks)?;
    let route_sequence_maximum_ticks =
        goal_route_sequence_maximum_ticks(optimization.budgets.exploration_horizon_ticks)?;
    let catalog = goal_conditioned_route_tactic_catalog(
        &tactic_targets,
        &route_sequences,
        maximum_ticks,
        route_sequence_maximum_ticks,
    )
    .map_err(route_error)?;
    let encoder =
        GoalConditionedTacticFeatureEncoder::new(target.coordinate).map_err(route_error)?;
    Ok(GoalConditionedTacticRuntime {
        catalog,
        encoder,
        report: target.report(
            source_coordinate,
            tactic_targets,
            route_sequences,
            authored_route_ids,
        ),
    })
}

fn goal_corridor_targets(
    source: [f32; 3],
    goal: [f32; 3],
) -> Result<(Vec<[f32; 3]>, Vec<Vec<[f32; 3]>>), NativeTacticRouteRunError> {
    if source
        .iter()
        .chain(goal.iter())
        .any(|value| !value.is_finite())
    {
        return Err(route_message(
            "goal corridor requires finite source and target coordinates",
        ));
    }
    let dx = goal[0] - source[0];
    let dz = goal[2] - source[2];
    let distance = dx.hypot(dz);
    if distance <= 0.0 || !distance.is_finite() {
        return Err(route_message(
            "goal corridor requires distinct source and target coordinates",
        ));
    }
    let perpendicular = [-dz / distance, dx / distance];
    let mut targets = vec![goal];
    let mut identities = BTreeSet::from([goal.map(f32::to_bits)]);
    let offsets = [-768.0_f32, -384.0, 0.0, 384.0, 768.0];
    let mut route_sequences = vec![Vec::new(); offsets.len()];
    for fraction in [0.25_f32, 0.5, 0.75, 1.0] {
        let center = [
            source[0] + dx * fraction,
            source[1] + (goal[1] - source[1]) * fraction,
            source[2] + dz * fraction,
        ];
        for (lane, offset) in offsets.iter().copied().enumerate() {
            let target = [
                center[0] + perpendicular[0] * offset,
                center[1],
                center[2] + perpendicular[1] * offset,
            ];
            if identities.insert(target.map(f32::to_bits)) {
                targets.push(target);
            }
            if fraction < 1.0 {
                route_sequences[lane].push(target);
            }
        }
    }
    for route in &mut route_sequences {
        route.push(goal);
    }
    Ok((targets, route_sequences))
}

#[derive(Clone)]
struct AuthoredRouteCandidate {
    identity: String,
    coordinates: Vec<[f32; 3]>,
    endpoint_cost: f32,
}

fn goal_route_targets(
    source: [f32; 3],
    goal: [f32; 3],
    room: i8,
    inventory: &WorldInventory,
) -> Result<(Vec<[f32; 3]>, Vec<Vec<[f32; 3]>>, Vec<String>), NativeTacticRouteRunError> {
    if source
        .iter()
        .chain(goal.iter())
        .any(|value| !value.is_finite())
    {
        return Err(route_message(
            "goal routes require finite source and target coordinates",
        ));
    }
    let paths = inventory
        .paths
        .iter()
        .filter(|path| path.scope.room == Some(room))
        .map(|path| ((path.source_sha256, path.record_index), path))
        .collect::<BTreeMap<_, _>>();
    if paths.is_empty() {
        let (targets, routes) = goal_corridor_targets(source, goal)?;
        return Ok((targets, routes, Vec::new()));
    }
    let points = inventory
        .path_points
        .iter()
        .filter(|point| point.scope.room == Some(room))
        .fold(BTreeMap::<_, Vec<_>>::new(), |mut by_source, point| {
            by_source
                .entry(point.source_sha256)
                .or_default()
                .push(point);
            by_source
        });
    let incoming = paths
        .values()
        .filter_map(|path| {
            path.next_path_index
                .map(|next| (path.source_sha256, usize::from(next)))
        })
        .collect::<BTreeSet<_>>();
    let roots = paths
        .keys()
        .filter(|key| !incoming.contains(key))
        .copied()
        .collect::<Vec<_>>();
    let direct_distance = planar_distance(source, goal);
    let attachment_limit = (direct_distance * 0.25).max(512.0);
    let mut candidates = Vec::new();
    for root in roots {
        let mut coordinates = Vec::new();
        let mut identities = Vec::new();
        let mut visited = BTreeSet::new();
        let mut current = Some(root);
        while let Some(key) = current {
            if !visited.insert(key) {
                return Err(route_message("authored path chain contains a cycle"));
            }
            let path = paths
                .get(&key)
                .ok_or_else(|| route_message("authored path chain references an absent path"))?;
            let source_points = points
                .get(&path.source_sha256)
                .ok_or_else(|| route_message("authored path has no point table"))?;
            let end = path
                .first_point_index
                .checked_add(usize::from(path.point_count))
                .ok_or_else(|| route_message("authored path point range overflowed"))?;
            let path_points = source_points
                .get(path.first_point_index..end)
                .ok_or_else(|| route_message("authored path point range is unavailable"))?;
            for point in path_points {
                let coordinate = [point.position.x, point.position.y, point.position.z];
                if coordinates.last() != Some(&coordinate) {
                    coordinates.push(coordinate);
                }
            }
            identities.push(path.stable_id.as_str());
            current = path
                .next_path_index
                .map(|next| (path.source_sha256, usize::from(next)));
        }
        if coordinates.is_empty() || coordinates.len() >= MAX_GOAL_SEEK_TARGETS {
            continue;
        }
        for (orientation, mut oriented) in [
            ("forward", coordinates.clone()),
            ("reverse", {
                let mut reverse = coordinates.clone();
                reverse.reverse();
                reverse
            }),
        ] {
            let first = *oriented.first().expect("nonempty authored path");
            let last = *oriented.last().expect("nonempty authored path");
            let source_cost = planar_distance(source, first);
            let goal_cost = planar_distance(last, goal);
            if source_cost > attachment_limit || goal_cost > attachment_limit {
                continue;
            }
            if last.map(f32::to_bits) != goal.map(f32::to_bits) {
                oriented.push(goal);
            }
            candidates.push(AuthoredRouteCandidate {
                identity: format!("{}:{orientation}", identities.join("+")),
                coordinates: oriented,
                endpoint_cost: source_cost + goal_cost,
            });
        }
    }
    candidates.sort_by(|left, right| {
        left.endpoint_cost
            .total_cmp(&right.endpoint_cost)
            .then_with(|| left.identity.cmp(&right.identity))
    });
    candidates.dedup_by(|left, right| {
        left.coordinates
            .iter()
            .map(|coordinate| coordinate.map(f32::to_bits))
            .eq(right
                .coordinates
                .iter()
                .map(|coordinate| coordinate.map(f32::to_bits)))
    });
    candidates.truncate(5);
    if candidates.is_empty() {
        let (targets, routes) = goal_corridor_targets(source, goal)?;
        return Ok((targets, routes, Vec::new()));
    }

    let route_sequences = candidates
        .iter()
        .map(|candidate| candidate.coordinates.clone())
        .collect::<Vec<_>>();
    let authored_route_ids = candidates
        .iter()
        .map(|candidate| candidate.identity.clone())
        .collect::<Vec<_>>();
    let mut targets = vec![goal];
    let mut target_identities = BTreeSet::from([goal.map(f32::to_bits)]);
    for coordinate in route_sequences.iter().flatten().copied() {
        if targets.len() == MAX_GOAL_SEEK_TARGETS {
            break;
        }
        if target_identities.insert(coordinate.map(f32::to_bits)) {
            targets.push(coordinate);
        }
    }
    Ok((targets, route_sequences, authored_route_ids))
}

fn planar_distance(left: [f32; 3], right: [f32; 3]) -> f32 {
    (right[0] - left[0]).hypot(right[2] - left[2])
}

fn resolve_goal_transition_target(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
) -> Result<(GoalTransitionTarget, WorldInventory), NativeTacticRouteRunError> {
    let program_bytes =
        fs::read(root.join(&execution.milestone_program.path)).map_err(route_error)?;
    let decoded =
        dusklight_objectives::milestone_dsl::decode(&program_bytes).map_err(route_error)?;
    let definition = decoded
        .program
        .definitions
        .iter()
        .find(|definition| definition.name == optimization.terminal_predicate.goal)
        .ok_or_else(|| route_message("goal definition is absent from milestone program"))?;
    let source_stage = exact_symbol_literal(&definition.when, Field::StageName)?;
    let source_room = exact_i8_literal(&definition.when, Field::StageRoom)?;
    let destination_stage = exact_symbol_literal(&definition.when, Field::NextStageName)?;
    let destination_room = exact_i8_literal(&definition.when, Field::NextStageRoom)?;
    let destination_point = exact_i16_literal(&definition.when, Field::NextStageSpawn)?;

    let context_path = root.join(&execution.world_context.path);
    let context_bytes = fs::read(&context_path).map_err(route_error)?;
    let context = WorldContext::decode_canonical(&context_bytes).map_err(route_error)?;
    if context.digest().map_err(route_error)? != execution.world_context.sha256 {
        return Err(route_message(
            "goal target world context differs from its execution binding",
        ));
    }
    let stage_binding = context
        .stages
        .iter()
        .find(|stage| stage.stage == source_stage)
        .ok_or_else(|| route_message("goal source stage is absent from world context"))?;
    let inventory_path = context_path
        .parent()
        .ok_or_else(|| route_message("world context has no artifact directory"))?
        .join(format!("{source_stage}.inventory.json"));
    let inventory =
        WorldInventory::decode_canonical(&fs::read(&inventory_path).map_err(route_error)?)
            .map_err(route_error)?;
    if inventory.stage != source_stage
        || inventory.digest().map_err(route_error)? != stage_binding.inventory_sha256
    {
        return Err(route_message(
            "goal source inventory differs from the pinned world context",
        ));
    }

    let collision_ids = inventory
        .load_triggers
        .iter()
        .filter(|trigger| {
            trigger.room == source_room
                && trigger.destination_stage == destination_stage
                && trigger.destination_room == destination_room
                && trigger.destination_point == destination_point
        })
        .map(|trigger| trigger.collision_id.as_str())
        .collect::<BTreeSet<_>>();
    if collision_ids.is_empty() {
        return Err(route_message(
            "goal transition has no matching load trigger in the pinned world",
        ));
    }

    let mut sum = [0.0_f64; 3];
    let mut points = 0_u64;
    for collision in &inventory.collisions {
        if !collision_ids.contains(collision.prism.authored.stable_id.as_str()) {
            continue;
        }
        let KclReconstruction::Reconstructed { triangle, .. } = &collision.prism.reconstruction
        else {
            continue;
        };
        for point in triangle {
            sum[0] += f64::from(point.x);
            sum[1] += f64::from(point.y);
            sum[2] += f64::from(point.z);
            points += 1;
        }
    }
    if points == 0 {
        return Err(route_message(
            "goal load triggers have no reconstructed target surface",
        ));
    }
    let coordinate = sum.map(|axis| (axis / points as f64) as f32);
    if coordinate.iter().any(|value| !value.is_finite()) {
        return Err(route_message("goal target centroid is non-finite"));
    }
    Ok((
        GoalTransitionTarget {
            source_stage,
            source_room,
            destination_stage,
            destination_room,
            destination_point,
            coordinate,
            supporting_load_triggers: collision_ids.len(),
            source_inventory_sha256: stage_binding.inventory_sha256,
        },
        inventory,
    ))
}

fn exact_symbol_literal(
    expression: &Expression,
    field: Field,
) -> Result<String, NativeTacticRouteRunError> {
    let values = exact_literals(expression, field);
    let mut symbols = values
        .into_iter()
        .map(|value| match value {
            Value::Symbol(symbol) => Ok(symbol),
            _ => Err(route_message("goal transition literal has the wrong type")),
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if symbols.len() != 1 {
        return Err(route_message(
            "goal transition requires one exact symbolic field literal",
        ));
    }
    Ok(symbols.pop_first().expect("one checked symbol"))
}

fn exact_i8_literal(
    expression: &Expression,
    field: Field,
) -> Result<i8, NativeTacticRouteRunError> {
    i8::try_from(exact_integer_literal(expression, field)?).map_err(route_error)
}

fn exact_i16_literal(
    expression: &Expression,
    field: Field,
) -> Result<i16, NativeTacticRouteRunError> {
    i16::try_from(exact_integer_literal(expression, field)?).map_err(route_error)
}

fn exact_integer_literal(
    expression: &Expression,
    field: Field,
) -> Result<i64, NativeTacticRouteRunError> {
    let values = exact_literals(expression, field);
    let integers = values
        .into_iter()
        .map(|value| match value {
            Value::I32(value) => Ok(i64::from(value)),
            Value::U32(value) => Ok(i64::from(value)),
            Value::U64(value) => i64::try_from(value).map_err(route_error),
            _ => Err(route_message("goal transition literal has the wrong type")),
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if integers.len() != 1 {
        return Err(route_message(
            "goal transition requires one exact integer field literal",
        ));
    }
    Ok(*integers.first().expect("one checked integer"))
}

fn exact_literals(expression: &Expression, field: Field) -> Vec<Value> {
    match expression {
        Expression::Compare {
            field: candidate,
            operator: Comparison::Equal,
            value,
        } if *candidate == field => vec![value.clone()],
        Expression::And(left, right) => {
            let mut values = exact_literals(left, field);
            values.extend(exact_literals(right, field));
            values
        }
        _ => Vec::new(),
    }
}

fn goal_tactic_maximum_ticks(horizon: u64) -> Result<u32, NativeTacticRouteRunError> {
    let horizon = u32::try_from(horizon).map_err(route_error)?;
    if horizon == 0 {
        return Err(route_message("goal tactic requires a nonzero horizon"));
    }
    // Route-relative seeks are navigation decisions, not whole-route
    // controllers. Reserve room for four reactive decisions so the learner can
    // redirect around contact geometry instead of spending half its horizon on
    // one stalled target.
    Ok((horizon / 4).clamp(1, 40))
}

fn goal_route_sequence_maximum_ticks(horizon: u64) -> Result<u32, NativeTacticRouteRunError> {
    let horizon = u32::try_from(horizon).map_err(route_error)?;
    if horizon == 0 {
        return Err(route_message(
            "goal route sequence requires a nonzero horizon",
        ));
    }
    Ok((horizon / 4).clamp(1, 160))
}

fn route_tactic_reward_spec(
    encoder: &GoalConditionedTacticFeatureEncoder,
    initial_facts: &FactSnapshot,
) -> Result<TacticRewardSpec, NativeTacticRouteRunError> {
    let initial_features = encoder.encode(initial_facts).map_err(route_error)?;
    let start_distance = initial_features[encoder.goal_distance_feature()];
    if !start_distance.is_finite() || start_distance <= 0.0 {
        return Err(route_message(
            "goal-conditioned source distance must be finite and positive",
        ));
    }
    let mut reward = route_tactic_base_reward_spec();
    reward.potential = Some(PotentialShapingSpec {
        schema: POTENTIAL_SHAPING_SCHEMA_V1.into(),
        feature_schema: encoder.schema_sha256,
        terms: vec![PotentialTerm::CorridorProgress {
            name: "goal_planar_distance".into(),
            feature: encoder.goal_distance_feature(),
            start: start_distance,
            end: 0.0,
            weight: 5.0,
            unavailable_value: None,
        }],
    });
    Ok(reward)
}

fn route_tactic_base_reward_spec() -> TacticRewardSpec {
    TacticRewardSpec {
        schema: TACTIC_REWARD_SPEC_SCHEMA_V1.into(),
        terminal_reward: 100.0,
        // The first route-learning proof is about competence, not speed. Keep
        // temporary detours value-neutral so the learner can discover paths
        // around collision geometry without paying an implicit route-time
        // objective that the product contract explicitly excludes.
        tick_cost: 0.0,
        novelty_reward: ROUTE_TACTIC_NOVELTY_REWARD,
        per_tick_discount: 1.0,
        potential: None,
    }
}

fn route_option_value_config(seed: u64) -> OptionValueConfig {
    OptionValueConfig {
        fitted_q: FqiConfig {
            iterations: 12,
            trees_per_action: 15,
            max_tree_depth: 8,
            // Keep a mild contraction so zero-reward waypoint holds lose
            // value, without erasing a terminal reached late in the declared
            // discovery horizon.
            discount: ROUTE_TACTIC_VALUE_DISCOUNT,
            seed: 0xd15c_a11d_5eed_f017 ^ seed,
            ..FqiConfig::default()
        },
    }
}

pub(crate) fn tactic_root_probe_batch(
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
) -> Result<NativeSuffixBatch, NativeTacticRouteRunError> {
    let maximum_ticks =
        usize::try_from(optimization.budgets.exploration_horizon_ticks).map_err(route_error)?;
    Ok(NativeSuffixBatch {
        schema: NATIVE_SUFFIX_BATCH_SCHEMA.into(),
        source_frame: usize::try_from(optimization.route.source_boundary_index)
            .map_err(route_error)?,
        source_boundary_fingerprint: optimization
            .route
            .native_source_boundary_fingerprint
            .clone(),
        checkpoint_validation: NativeCheckpointValidation {
            kind: "recorded_replay_window".into(),
            ticks: usize::try_from(execution.checkpoint_validation_ticks).map_err(route_error)?,
        },
        maximum_ticks,
        verify_state_hashes: execution.verify_state_hashes,
        checkpoint_cache: None,
        candidates: vec![NativeSuffixCandidate {
            id: "tactic-root-probe".into(),
            actions: vec![MacroAction::PadRun {
                pad: SearchPadState::from(RawPadState::default()),
                frames: u32::try_from(maximum_ticks).map_err(route_error)?,
            }],
            controller_program_hex: None,
        }],
    })
}

pub(crate) fn initial_facts(
    initial: &ValidatedNativeSuffixBatch,
) -> Result<FactSnapshot, NativeTacticRouteRunError> {
    let shard = NativeEpisodeShard::decode(
        &fs::read(Path::new(&initial.episode_shard_path)).map_err(route_error)?,
    )
    .map_err(route_error)?;
    let episode = shard
        .episodes
        .iter()
        .find(|episode| episode.id == "tactic-root-probe")
        .ok_or_else(|| route_message("initial native shard has no root probe"))?;
    let observation = &episode
        .steps
        .first()
        .ok_or_else(|| route_message("initial native root probe has no step"))?
        .pre_input;
    FactSnapshot::from_native_learning(observation, &[], None, Vec::new()).map_err(route_error)
}

fn validate_config(
    config: &NativeTacticRouteRunConfig<'_>,
) -> Result<(), NativeTacticRouteRunError> {
    if config.exploration_seeds.is_empty()
        || config.exploration_seeds.len() > MAX_ROUTE_SEEDS
        || config.workers == 0
        || config.workers > MAX_ROUTE_WORKERS
        || config.decisions_per_seed == 0
        || config.decisions_per_seed > MAX_ROUTE_DECISIONS
        || config.branch_every_decisions == 0
        || config.branch_every_decisions > config.decisions_per_seed
        || config.refit_every_decisions == 0
        || config.refit_every_decisions > config.decisions_per_seed
        || config.epsilon_per_million > 1_000_000
        || config
            .exploration_seeds
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || config.decisions_per_seed > config.optimization.budgets.candidate_budget
    {
        return Err(route_message(
            "native tactic route configuration is invalid",
        ));
    }
    Ok(())
}

fn seed_group(seed_index: usize, episode: u64) -> Result<u64, NativeTacticRouteRunError> {
    (seed_index as u64)
        .checked_mul(1_000_000)
        .and_then(|base| base.checked_add(episode))
        .ok_or_else(|| route_message("episode group overflowed"))
}

fn selected_tactic_fits_horizon(
    suffix_ticks: u64,
    selected_maximum_ticks: u32,
    horizon: u64,
) -> bool {
    suffix_ticks.saturating_add(u64::from(selected_maximum_ticks)) <= horizon
}

fn proposal_outcome_is_better(
    candidate: &EvaluatedRewardedTacticOutcome,
    incumbent: &EvaluatedRewardedTacticOutcome,
) -> bool {
    candidate
        .outcome
        .terminal
        .cmp(&incumbent.outcome.terminal)
        .then_with(|| {
            candidate
                .reward
                .training_reward
                .total_cmp(&incumbent.reward.training_reward)
        })
        .then_with(|| {
            incumbent
                .outcome
                .execution
                .duration
                .realized_ticks
                .cmp(&candidate.outcome.execution.duration.realized_ticks)
        })
        .is_gt()
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<(), NativeTacticRouteRunError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(route_error)?;
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(route_error)?;
    file.write_all(bytes).map_err(route_error)?;
    file.sync_all().map_err(route_error)
}

pub(crate) fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeTacticRouteRunError {
    message: String,
    cancelled: bool,
}

impl NativeTacticRouteRunError {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}

impl fmt::Display for NativeTacticRouteRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for NativeTacticRouteRunError {}

fn route_message(message: impl Into<String>) -> NativeTacticRouteRunError {
    NativeTacticRouteRunError {
        message: message.into(),
        cancelled: false,
    }
}

fn route_cancelled(message: impl Into<String>) -> NativeTacticRouteRunError {
    NativeTacticRouteRunError {
        message: message.into(),
        cancelled: true,
    }
}

fn route_error(error: impl fmt::Display) -> NativeTacticRouteRunError {
    route_message(error.to_string())
}

impl From<NativeSuffixWorkerError> for NativeTacticRouteRunError {
    fn from(error: NativeSuffixWorkerError) -> Self {
        route_error(error)
    }
}

impl From<TacticQCampaignError> for NativeTacticRouteRunError {
    fn from(error: TacticQCampaignError) -> Self {
        route_error(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal_trace(decision_index: u64) -> NativeTacticDecisionTrace {
        serde_json::from_value(serde_json::json!({
            "decision_index": decision_index,
            "episode": 2,
            "route_suffix_ticks": decision_index + 4,
            "selected_option_id": format!("move.{decision_index}"),
            "selection_reason": "epsilon",
            "selected_q": 1.5,
            "best_q": 2.0,
            "reward": 0.25,
            "reward_components": {
                "terminal_observed": false,
                "endpoint_novel": true,
                "duration_ticks": 4,
                "terminal_component": 0.0,
                "tick_cost_component": 0.0,
                "novelty_component": 0.25,
                "base_reward": 0.25,
                "potential": null,
                "training_reward": 0.25,
                "terminal_objective_unchanged": true,
                "promotion_authority": false
            },
            "goal_distance_before": 8.0,
            "goal_distance_after": 7.0,
            "terminal": false,
            "frontier_cells": 3,
            "visited_states": 4,
            "before": {
                "snapshot_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "stage": "F_SP103",
                "room": 1,
                "layer": 0,
                "point": 0,
                "simulation_tick": 10,
                "tape_frame": 20,
                "player_position": [0.0, 1.0, 2.0],
                "player_velocity": [0.0, 0.0, 1.0],
                "player_procedure": 3,
                "player_contacts": 1,
                "event_running": false,
                "event_id": -1,
                "terminal_reached": false,
                "actor_count": 4,
                "same_room_actor_count": 3,
                "recent_option_id": null
            },
            "after": {
                "snapshot_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "stage": "F_SP103",
                "room": 1,
                "layer": 0,
                "point": 0,
                "simulation_tick": 14,
                "tape_frame": 24,
                "player_position": [1.0, 1.0, 2.0],
                "player_velocity": [1.0, 0.0, 0.0],
                "player_procedure": 3,
                "player_contacts": 1,
                "event_running": false,
                "event_id": -1,
                "terminal_reached": false,
                "actor_count": 4,
                "same_room_actor_count": 3,
                "recent_option_id": format!("move.{decision_index}")
            },
            "measurements": [{"name": "goal_distance", "before": 8.0, "after": 7.0}],
            "applicable_tactics": [{
                "option_id": format!("move.{decision_index}"),
                "mean_q": 1.5,
                "ensemble_variance": 0.25,
                "selected": true
            }]
        }))
        .unwrap()
    }

    fn journal_record(decision_index: u64) -> NativeTacticDecisionRecord {
        let root_tape = StoredContentRef {
            kind: dusklight_evidence::content_store::ContentKind::InputTape,
            sha256: Digest([2; 32]),
        };
        let transition = StoredContentRef {
            kind: dusklight_evidence::content_store::ContentKind::TacticTransition,
            sha256: Digest([1; 32]),
        };
        decision_record(
            &journal_trace(decision_index),
            2,
            Digest([3; 32]),
            root_tape,
            transition,
            Vec::new(),
        )
    }

    #[test]
    fn tactic_decision_journal_round_trips_and_recovers_a_truncated_tail() {
        let root = std::env::temp_dir().join(format!(
            "dusklight-tactic-decision-journal-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        append_tactic_decision_record(&root, &journal_record(0)).unwrap();
        append_tactic_decision_record(&root, &journal_record(1)).unwrap();
        let records = read_tactic_decision_records(&root).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].decision_index, 1);

        let path = tactic_decision_journal_path(&root);
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&[7, 8, 9])
            .unwrap();
        assert_eq!(read_tactic_decision_records(&root).unwrap().len(), 2);
        append_tactic_decision_record(&root, &journal_record(2)).unwrap();
        assert_eq!(read_tactic_decision_records(&root).unwrap().len(), 3);

        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(read_tactic_decision_records(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tactic_decision_journal_compacts_reference_records_without_rewriting_segments() {
        let root = std::env::temp_dir().join(format!(
            "dusklight-tactic-journal-compaction-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        for decision_index in 0..NATIVE_TACTIC_DECISION_COMPACTION_RECORDS {
            append_tactic_decision_record(&root, &journal_record(decision_index)).unwrap();
        }
        assert_eq!(
            fs::metadata(tactic_decision_journal_path(&root))
                .unwrap()
                .len() as usize,
            NATIVE_TACTIC_DECISION_JOURNAL_HEADER_SIZE
        );
        let first_segment = tactic_decision_segment_paths(&root).unwrap();
        assert_eq!(first_segment.len(), 1);
        let first_segment_bytes = fs::read(&first_segment[0]).unwrap();
        let stale_active = (0..NATIVE_TACTIC_DECISION_COMPACTION_RECORDS)
            .map(journal_record)
            .collect::<Vec<_>>();
        fs::write(
            tactic_decision_journal_path(&root),
            encode_tactic_decision_journal(&stale_active).unwrap(),
        )
        .unwrap();
        assert_eq!(
            read_tactic_decision_records(&root).unwrap().len() as u64,
            NATIVE_TACTIC_DECISION_COMPACTION_RECORDS
        );
        fs::remove_file(tactic_decision_journal_path(&root)).unwrap();
        assert!(has_tactic_decision_journal(&root));
        assert_eq!(
            read_tactic_decision_records(&root).unwrap().len() as u64,
            NATIVE_TACTIC_DECISION_COMPACTION_RECORDS
        );

        append_tactic_decision_record(
            &root,
            &journal_record(NATIVE_TACTIC_DECISION_COMPACTION_RECORDS),
        )
        .unwrap();
        compact_tactic_decision_journal(&root).unwrap();
        let segments = tactic_decision_segment_paths(&root).unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(fs::read(&first_segment[0]).unwrap(), first_segment_bytes);
        assert_eq!(
            read_tactic_decision_records(&root).unwrap().len() as u64,
            NATIVE_TACTIC_DECISION_COMPACTION_RECORDS + 1
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn journal_projects_graph_and_materializes_routes_from_content_objects() {
        use dusklight_control::option_execution::{
            OptionCondition, OptionEndReason, OptionExecution, OptionType, TapeRange,
        };
        use dusklight_evidence::native_episode_shard::NativeObservationPhase;
        use dusklight_learning::fact_snapshot::{FactSnapshot, FactTerminalReason};
        use dusklight_learning::option_transition::OptionTransitionSample;

        let root = std::env::temp_dir().join(format!(
            "dusklight-tactic-journal-route-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let native = &shard.episodes[0].steps[0];
        let mut before =
            FactSnapshot::from_native_learning(&native.pre_input, &[], None, Vec::new()).unwrap();
        let mut next_boundary = native.post_simulation.clone();
        next_boundary.phase = NativeObservationPhase::PreInput;
        next_boundary.simulation_tick += 1;
        next_boundary.tape_frame += 1;
        let mut after = FactSnapshot::from_native_learning(
            &next_boundary,
            &[native.pre_input.clone()],
            None,
            Vec::new(),
        )
        .unwrap();
        before.terminal.configured = Some(true);
        before.terminal.reached = Some(false);
        before.terminal.reason = FactTerminalReason::None;
        after.terminal.configured = Some(true);
        after.terminal.reached = Some(false);
        after.terminal.reason = FactTerminalReason::None;
        let mut route = InputTape {
            frames: vec![
                dusklight_automation_contracts::tape::InputFrame::default();
                after.tape_frame as usize
            ],
            ..InputTape::default()
        };
        after.tape_frame = route.frames.len() as u64;
        route.frames[before.tape_frame as usize] =
            dusklight_automation_contracts::tape::InputFrame::default();
        let execution = OptionExecution::capture(
            "wait".into(),
            OptionType::Neutral,
            BTreeMap::new(),
            1,
            1,
            OptionCondition::DurationElapsed,
            Vec::new(),
            OptionEndReason::Completed,
            &route,
            TapeRange {
                start_frame: before.tape_frame,
                end_frame_exclusive: after.tape_frame,
            },
        )
        .unwrap();
        let root_tape = InputTape {
            boot: route.boot.clone(),
            tick_rate_numerator: route.tick_rate_numerator,
            tick_rate_denominator: route.tick_rate_denominator,
            frames: route.frames[..before.tape_frame as usize].to_vec(),
        };
        let root_checkpoint_sha256 = Digest([9; 32]);
        let source_checkpoint_sha256 =
            route_checkpoint(root_checkpoint_sha256, &root_tape).unwrap();
        let next_checkpoint_sha256 = route_checkpoint(root_checkpoint_sha256, &route).unwrap();
        let transition = OptionTransitionSample::capture(
            Digest([1; 32]),
            source_checkpoint_sha256,
            next_checkpoint_sha256,
            before,
            after,
            execution,
            &route,
            0.25,
            false,
            |facts| Ok::<_, &'static str>(vec![facts.player.position_f32_bits[0] as f32]),
        )
        .unwrap();
        let store = TacticQContentStore::initialize(tactic_content_store_path(&root)).unwrap();
        let root_ref = store.store_tape(&root_tape).unwrap();
        let transition_ref = store.store_option_transition(&transition, &route).unwrap();
        let mut trace = journal_trace(0);
        trace.reward_components.duration_ticks = 1;
        let proposal_trace = NativeTacticProposalTrace {
            option_id: transition.value_sample.action.option_id.clone(),
            selection_reason: trace.selection_reason,
            reward: transition.value_sample.reward,
            reward_components: trace.reward_components.clone(),
            realized_ticks: transition.execution.duration.realized_ticks,
            terminal: transition.value_sample.terminal,
            goal_distance_after: trace.goal_distance_after,
            after_snapshot_sha256: transition.after_state_sha256,
            retained: true,
        };
        trace.proposal_batch = vec![proposal_trace.clone()];
        append_tactic_decision_record(
            &root,
            &decision_record(
                &trace,
                2,
                root_checkpoint_sha256,
                root_ref,
                transition_ref,
                vec![NativeTacticProposalRecord {
                    trace: proposal_trace,
                    transition: transition_ref,
                }],
            ),
        )
        .unwrap();

        let projected_trace = read_tactic_decision_journal(&root).unwrap();
        assert_eq!(projected_trace[0].proposal_batch, trace.proposal_batch);
        let graph = project_tactic_decision_graph(&root).unwrap().unwrap();
        assert!(graph.root_connected);
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].option_id, "wait");
        let diagnostics = project_tactic_decision_diagnostics(&root).unwrap().unwrap();
        assert_eq!(diagnostics.replay_rows, 1);
        assert_eq!(diagnostics.frontier_cells, 1);
        assert_eq!(diagnostics.logical_frontier_records, 2);
        assert_eq!(diagnostics.directly_restorable_native_frontiers, 0);
        assert_eq!(diagnostics.replay_only_frontiers, 1);
        assert_eq!(diagnostics.unique_selected_actions, 1);
        assert!(!diagnostics.frontier_lost_root_connectivity);
        let materialized = materialize_tactic_decision_route(&root, 0).unwrap();
        assert_eq!(materialized, route);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn first_route_proof_does_not_optimize_speed() {
        let reward = route_tactic_base_reward_spec();
        let values = route_option_value_config(42);

        assert_eq!(reward.tick_cost, 0.0);
        assert_eq!(reward.per_tick_discount, 1.0);
        assert_eq!(values.fitted_q.discount, ROUTE_TACTIC_VALUE_DISCOUNT);
        assert!(values.fitted_q.discount > 0.995);
        assert!(values.fitted_q.discount < 1.0);
        assert!(reward.novelty_reward > 0.0);
        assert!(reward.terminal_reward > reward.novelty_reward);
    }

    #[test]
    fn root_probe_uses_the_full_declared_horizon() {
        assert!(MAX_ROUTE_DECISIONS > 0);
    }

    #[test]
    fn horizon_fit_uses_the_selected_tactic_duration() {
        assert!(selected_tactic_fits_horizon(88, 8, 160));
        assert!(selected_tactic_fits_horizon(152, 8, 160));
        assert!(!selected_tactic_fits_horizon(88, 80, 160));
        assert!(!selected_tactic_fits_horizon(u64::MAX, 1, 160));
    }

    #[test]
    fn throughput_rates_use_measured_wall_time_and_sum_seed_phases() {
        let seed = NativeTacticSeedResult {
            seed: 7,
            success: false,
            decisions: 4,
            episodes: 2,
            native_ticks: 30,
            replay_rows: 4,
            visited_states: 3,
            useful_decisions: 2,
            timing: NativeTacticRouteTiming {
                wall_micros: 2_000_000,
                tactic_selection_micros: 10,
                checkpoint_branching_micros: 20,
                tactic_execution_micros: 1_000_000,
                native_simulation_micros: 900_000,
                tactic_preparation_and_fact_extraction_micros: 100_000,
                model_update_micros: 200_000,
                evidence_projection_and_persistence_micros: 300_000,
                ..NativeTacticRouteTiming::default()
            },
            selection_counts: BTreeMap::new(),
            diagnostics: None,
            final_checkpoint: None,
            graph: None,
            successful_tape: None,
            final_result: None,
            trace: Vec::new(),
        };
        let timing = aggregate_route_timing(&[seed]);

        assert_eq!(timing.useful_decisions_per_second_millionths, 1_000_000);
        assert_eq!(timing.native_ticks_per_second_millionths, 15_000_000);
        assert_eq!(timing.episodes_per_second_millionths, 1_000_000);
        assert_eq!(timing.native_simulation_micros, 900_000);
    }

    #[test]
    fn coordinator_results_are_projected_in_seed_order() {
        let seed_count = 11;
        let mut report_order = vec![8, 0, 10, 3, 5, 1, 9, 2, 7, 6, 4];
        report_order.sort_unstable();
        assert_eq!(report_order, (0..seed_count).collect::<Vec<_>>());
    }

    #[test]
    fn evaluated_tick_accounting_includes_every_proposal() {
        let mut trace = journal_trace(0);
        assert_eq!(decision_evaluated_ticks(&trace), 4);
        trace.proposal_batch = [6, 14, 40]
            .into_iter()
            .enumerate()
            .map(|(index, realized_ticks)| NativeTacticProposalTrace {
                option_id: format!("proposal-{index}"),
                selection_reason: TacticSelectionReason::BatchDiversity,
                reward: index as f32,
                reward_components: trace.reward_components.clone(),
                realized_ticks,
                terminal: index == 1,
                goal_distance_after: 7.0,
                after_snapshot_sha256: Digest([index as u8 + 1; 32]),
                retained: index == 1,
            })
            .collect();
        assert_eq!(decision_evaluated_ticks(&trace), 60);
    }

    #[test]
    fn pause_performance_is_immutable_and_resume_bound() {
        let directory = std::env::temp_dir().join(format!(
            "dusklight-tactic-route-performance-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        let timing = NativeTacticRouteTiming {
            wall_micros: 100,
            tactic_execution_micros: 80,
            native_simulation_micros: 70,
            ..NativeTacticRouteTiming::default()
        };

        persist_seed_performance(&directory, 3, &timing, 2).unwrap();
        let loaded = load_seed_performance(&directory, 3).unwrap();
        assert_eq!(loaded.decisions, 3);
        assert_eq!(loaded.useful_decisions, 2);
        assert_eq!(loaded.timing, timing);
        persist_seed_performance(&directory, 3, &timing, 2).unwrap();
        persist_seed_performance(&directory, 3, &timing, 1).unwrap();
        assert_eq!(
            load_seed_performance(&directory, 3)
                .unwrap()
                .useful_decisions,
            1
        );
        assert_eq!(
            load_seed_performance(&directory, 2)
                .unwrap()
                .timing
                .wall_micros,
            0
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn root_episode_slots_do_not_skip_frontier_rotation_rounds() {
        let frontier_rounds = (1..=11)
            .filter(|episode| episode % 4 != 0)
            .map(frontier_sampling_round)
            .collect::<Vec<_>>();
        assert_eq!(frontier_rounds, (0..9).collect::<Vec<_>>());
    }

    #[test]
    fn tactic_checkpoints_follow_the_sealed_resume_interval_and_terminal() {
        assert!(!tactic_checkpoint_due(1, 64, false));
        assert!(!tactic_checkpoint_due(63, 64, false));
        assert!(tactic_checkpoint_due(64, 64, false));
        assert!(tactic_checkpoint_due(65, 64, true));
    }

    #[test]
    fn tactic_model_refits_once_initially_and_then_in_batches() {
        assert!(tactic_model_refit_due(1, 4));
        assert!(!tactic_model_refit_due(2, 4));
        assert!(!tactic_model_refit_due(3, 4));
        assert!(tactic_model_refit_due(4, 4));
        assert!(tactic_model_refit_due(8, 4));
    }

    #[test]
    fn goal_seek_reserves_room_for_reactive_redirection() {
        assert_eq!(goal_tactic_maximum_ticks(160).unwrap(), 40);
        assert_eq!(goal_tactic_maximum_ticks(3).unwrap(), 1);
        assert_eq!(goal_tactic_maximum_ticks(1_000).unwrap(), 40);
        assert!(goal_tactic_maximum_ticks(0).is_err());
        assert_eq!(goal_route_sequence_maximum_ticks(160).unwrap(), 40);
        assert_eq!(goal_route_sequence_maximum_ticks(1_000).unwrap(), 160);
        assert!(goal_route_sequence_maximum_ticks(0).is_err());
    }

    #[test]
    fn goal_corridor_is_a_symmetric_start_and_goal_derived_action_basis() {
        let source = [0.0, 10.0, 0.0];
        let goal = [1000.0, 20.0, 0.0];
        let (targets, route_sequences) = goal_corridor_targets(source, goal).unwrap();

        assert_eq!(targets.len(), 20);
        assert_eq!(targets[0], goal);
        assert!(targets.contains(&[250.0, 12.5, -768.0]));
        assert!(targets.contains(&[250.0, 12.5, 768.0]));
        assert!(targets.contains(&[500.0, 15.0, 0.0]));
        assert_eq!(
            targets
                .iter()
                .map(|target| target.map(f32::to_bits))
                .collect::<BTreeSet<_>>()
                .len(),
            targets.len()
        );
        assert_eq!(route_sequences.len(), 5);
        assert!(route_sequences.iter().all(|route| route.len() == 4));
        assert_eq!(route_sequences[0][0], [250.0, 12.5, -768.0]);
        assert_eq!(route_sequences[2][1], [500.0, 15.0, 0.0]);
        assert!(
            route_sequences
                .iter()
                .all(|route| route.last() == Some(&goal))
        );
        assert!(goal_corridor_targets(source, source).is_err());
    }

    #[test]
    fn authored_room_paths_replace_the_synthetic_corridor_with_attached_routes() {
        use dusklight_world::world_inventory::{
            AuthoredPathPointRecord, AuthoredPathRecord, SourceKind, SourceScope,
            WORLD_INVENTORY_SCHEMA,
        };

        let source_digest = Digest([7; 32]);
        let scope = SourceScope {
            kind: SourceKind::Room,
            room: Some(1),
        };
        let path = |record_index, first_point_index, point_count| AuthoredPathRecord {
            stable_id: format!("path/{record_index}"),
            source_sha256: source_digest,
            scope,
            record_index,
            point_count,
            next_path_index: None,
            path_argument: u8::MAX,
            closed: false,
            closed_raw: 0,
            switch_no: None,
            unknown_07: 0,
            point_offset: u32::try_from(first_point_index * 16).unwrap(),
            first_point_index,
            raw_hex: "00".repeat(12),
        };
        let point = |record_index, position: [f32; 3]| AuthoredPathPointRecord {
            stable_id: format!("point/{record_index}"),
            source_sha256: source_digest,
            scope,
            record_index,
            arguments: [u8::MAX; 4],
            position: dusklight_world::world_geometry::Vec3 {
                x: position[0],
                y: position[1],
                z: position[2],
            },
            raw_hex: "00".repeat(16),
        };
        let inventory = WorldInventory {
            schema: WORLD_INVENTORY_SCHEMA.into(),
            stage: "TEST".into(),
            sources: Vec::new(),
            chunks: Vec::new(),
            placements: Vec::new(),
            player_spawns: Vec::new(),
            exits: Vec::new(),
            paths: vec![path(0, 0, 2), path(1, 2, 2), path(2, 4, 2)],
            path_points: vec![
                point(0, [100.0, 0.0, 100.0]),
                point(1, [100.0, 0.0, 900.0]),
                point(2, [300.0, 0.0, 200.0]),
                point(3, [300.0, 0.0, 700.0]),
                point(4, [450.0, 0.0, 300.0]),
                point(5, [450.0, 0.0, 600.0]),
            ],
            collisions: Vec::new(),
            load_triggers: Vec::new(),
        };
        let goal = [0.0, 0.0, 1_000.0];
        let (targets, routes, route_ids) =
            goal_route_targets([0.0, 0.0, 0.0], goal, 1, &inventory).unwrap();

        assert_eq!(routes.len(), 2);
        assert_eq!(route_ids.len(), routes.len());
        assert_eq!(routes[0][0], [100.0, 0.0, 100.0]);
        assert_eq!(routes[0][1], [100.0, 0.0, 900.0]);
        assert_eq!(routes[0].last(), Some(&goal));
        assert!(route_ids[0].contains("path/0:forward"));
        assert_eq!(targets[0], goal);
        assert!(targets.contains(&[100.0, 0.0, 100.0]));
    }

    #[test]
    fn real_f_sp104_authored_main_path_is_the_bootstrap_route_when_disc_is_present() {
        let stage_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .join("orig/GZ2E01/files/res/Stage/F_SP104");
        if !stage_dir.is_dir() {
            eprintln!("skipping F_SP104 route-basis golden: original disc data is absent");
            return;
        }
        let inventory = WorldInventory::build(&stage_dir, "F_SP104").unwrap();
        let source = [150.21315, 306.54245, -2785.0728];
        let goal = [-430.95392, 241.77234, -21165.0];
        let (_, routes, route_ids) = goal_route_targets(source, goal, 1, &inventory).unwrap();

        assert_eq!(routes.len(), 1);
        assert!(route_ids[0].contains("/chunk/RPAT/record/14:forward"));
        assert_eq!(routes[0][0], [300.0, 270.81253, -3950.0]);
        assert_eq!(routes[0][7], [-441.90887, 314.0304, -19270.963]);
        assert_eq!(routes[0].last(), Some(&goal));
    }

    #[test]
    fn rolling_checkpoint_keeps_only_the_latest_complete_file() {
        let directory = std::env::temp_dir().join(format!(
            "dusklight-tactic-route-rolling-checkpoint-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let first = directory.join("first.json");
        let second = directory.join("second.json");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();
        let mut current = None;

        advance_rolling_checkpoint(&directory, &mut current, first.clone()).unwrap();
        assert!(first.is_file());
        advance_rolling_checkpoint(&directory, &mut current, second.clone()).unwrap();
        assert!(!first.exists());
        assert!(second.is_file());
        remove_rolling_checkpoint(&directory, &mut current).unwrap();
        assert!(!second.exists());
        assert!(current.is_none());

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn route_attempts_are_append_only_across_resume_launches() {
        let directory = std::env::temp_dir().join(format!(
            "dusklight-tactic-route-attempts-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();

        let first = reserve_attempt_root(&directory).unwrap();
        let second = reserve_attempt_root(&directory).unwrap();

        assert_eq!(first.file_name().unwrap(), "attempt-0000");
        assert_eq!(second.file_name().unwrap(), "attempt-0001");
        assert!(first.is_dir());
        assert!(second.is_dir());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn resume_selects_the_latest_single_immutable_pause_checkpoint() {
        let directory = std::env::temp_dir().join(format!(
            "dusklight-tactic-route-pause-checkpoint-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        for decision in [2_u64, 7] {
            let checkpoint = directory
                .join("pause-checkpoints")
                .join(format!("decision-{decision:06}"));
            fs::create_dir_all(&checkpoint).unwrap();
            fs::write(
                checkpoint.join(format!(
                    "tactic-q-{decision}.{TACTIC_Q_CHECKPOINT_EXTENSION}"
                )),
                b"checkpoint",
            )
            .unwrap();
        }

        let (decision, path) = latest_pause_checkpoint(&directory).unwrap();
        assert_eq!(decision, 7);
        assert_eq!(
            path.file_name().unwrap(),
            format!("tactic-q-7.{TACTIC_Q_CHECKPOINT_EXTENSION}").as_str()
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
