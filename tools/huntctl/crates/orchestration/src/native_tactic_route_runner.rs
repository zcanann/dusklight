//! Fresh-model tactic-Q route learning on an authenticated native checkpoint.

use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::native_suffix_result::{NativeTerminalBinding, ValidatedNativeSuffixBatch};
use crate::native_suffix_worker::{
    NativeSuffixWorkerError, NativeSuffixWorkerLaunch, NativeSuffixWorkerSession,
};
use crate::native_tactic_worker::{
    NativeTacticWorkerError, NativeTacticWorkerPaths, PersistentTacticBatchWorker,
    execute_selected_tactic, tactic_root_checkpoint_sha256,
};
use crate::optimization_request::OptimizationRequest;
use crate::tactic_q_campaign::{
    TacticCampaignDiagnostics, TacticQCampaign, TacticQCampaignCheckpoint, TacticQCampaignError,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::{InputTape, RawPadState};
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use dusklight_learning::default_tactic_catalog::goal_conditioned_route_tactic_catalog;
use dusklight_learning::fact_registry::FactRegistry;
use dusklight_learning::fact_snapshot::FactSnapshot;
use dusklight_learning::fqi::FqiConfig;
use dusklight_learning::learner_state::LearnerState;
use dusklight_learning::option_values::OptionValueConfig;
use dusklight_learning::reward_shaping::{
    POTENTIAL_SHAPING_SCHEMA_V1, PotentialShapingSpec, PotentialTerm, TACTIC_REWARD_SPEC_SCHEMA_V1,
    TacticRewardBreakdown, TacticRewardSpec,
};
use dusklight_learning::tactic_blueprint::TacticBlueprint;
use dusklight_learning::tactic_exploration::{TacticExplorationConfig, TacticSelectionReason};
use dusklight_learning::tactic_features::GoalConditionedTacticFeatureEncoder;
use dusklight_objectives::milestone_dsl::{Comparison, Expression, Field, Value};
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
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V4: &str = "dusklight-native-tactic-route-report/v4";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V5: &str = "dusklight-native-tactic-route-report/v5";
pub const NATIVE_TACTIC_DECISION_SUMMARY_SCHEMA_V1: &str =
    "dusklight-native-tactic-decision-summary/v1";
const MAX_ROUTE_SEEDS: usize = 32;
const MAX_ROUTE_DECISIONS: u64 = 100_000;
const ROUTE_TACTIC_VALUE_DISCOUNT: f32 = 0.999;
const ROUTE_TACTIC_NOVELTY_REWARD: f32 = 0.05;
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
    pub cancellation: Option<&'a AtomicBool>,
    pub resume: bool,
    pub blueprints: &'a [TacticBlueprint],
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
    pub decisions_per_seed: u64,
    pub refit_every_decisions: u64,
    pub successful_seeds: u64,
    pub total_native_ticks: u64,
    pub total_decisions: u64,
    pub useful_decisions: u64,
    pub timing: NativeTacticRouteTiming,
    pub seeds: Vec<NativeTacticSeedResult>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticRouteTiming {
    pub wall_micros: u64,
    pub tactic_selection_micros: u64,
    pub checkpoint_branching_micros: u64,
    pub tactic_execution_micros: u64,
    pub native_simulation_micros: u64,
    pub tactic_preparation_and_fact_extraction_micros: u64,
    pub model_update_micros: u64,
    pub evidence_projection_and_persistence_micros: u64,
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
    pub visited_states: usize,
    pub before: NativeTacticStateTrace,
    pub after: NativeTacticStateTrace,
    pub measurements: Vec<NativeTacticMeasurementTrace>,
    pub applicable_tactics: Vec<NativeTacticValueTrace>,
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

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeTacticDecisionSummary<'a> {
    schema: &'static str,
    decision_index: u64,
    episode: u64,
    selected_option_id: &'a str,
    selection_reason: TacticSelectionReason,
    reward: f32,
    duration_ticks: u32,
    goal_distance_before: f32,
    goal_distance_after: f32,
    terminal: bool,
}

pub fn run_native_tactic_route(
    config: &NativeTacticRouteRunConfig<'_>,
) -> Result<NativeTacticRouteReport, NativeTacticRouteRunError> {
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
    let attempt_root = reserve_attempt_root(config.output_root)?;

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
    let initial_root = attempt_root.join("initial");
    fs::create_dir_all(&initial_root).map_err(route_error)?;
    let initial_batch_path = initial_root.join("request.json");
    write_new(
        &initial_batch_path,
        &serde_json::to_vec_pretty(&initial_batch).map_err(route_error)?,
    )?;
    let terminal = NativeTerminalBinding {
        goal: config.optimization.terminal_predicate.goal.clone(),
        program_sha256: config.optimization.terminal_predicate.program_sha256,
        definition_sha256: config.optimization.terminal_predicate.definition_sha256,
    };
    let card_fixture = config
        .execution
        .card_fixture_root(&root, config.optimization)
        .map_err(route_error)?;
    let launch = NativeSuffixWorkerLaunch {
        executable: root.join(&config.execution.executable.path),
        game_data: root.join(&config.execution.game_data.path),
        input_tape: root.join(&config.execution.process_boot_tape.path),
        milestone_program: root.join(&config.execution.milestone_program.path),
        card_fixture,
        card_fixture_sha256: config.execution.card_fixture_manifest.sha256,
        working_directory: root.clone(),
        state_root: attempt_root.join("native-state"),
        world_context_sha256: config.execution.world_context.sha256,
        terminal,
        initial_batch: initial_batch_path,
        initial_result: initial_root.join("result.json"),
        initial_winner_tape: None,
    };
    let (mut worker, initial) = NativeSuffixWorkerSession::launch(&launch).map_err(route_error)?;
    let initial_facts = initial_facts(&initial)?;
    if initial_facts.tape_frame != config.optimization.route.source_boundary_index
        || initial_facts.terminal.reached != Some(false)
    {
        return Err(route_message(
            "native source observation is not the requested nonterminal tactic boundary",
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
    let root_checkpoint_sha256 =
        tactic_root_checkpoint_sha256(worker.identity()).map_err(route_error)?;
    let reward_spec = route_tactic_reward_spec(&encoder, &initial_facts)?;
    let action_schema_sha256 = route_action_schema_sha256(&catalog, config.blueprints)?;

    let run = (|| {
        let mut seed_results = Vec::with_capacity(config.exploration_seeds.len());
        for (seed_index, seed) in config.exploration_seeds.iter().copied().enumerate() {
            let seed_root = config
                .output_root
                .join(format!("seed-{seed_index:03}-{seed}"));
            let seed_result_path = seed_root.join("seed-result.json");
            if seed_result_path.exists() {
                if !config.resume {
                    return Err(route_message("unexpected pre-existing tactic seed result"));
                }
                seed_results.push(read_completed_seed_result(
                    &seed_result_path,
                    seed,
                    config.decisions_per_seed,
                )?);
                continue;
            }
            let result = run_seed(
                config,
                &mut worker,
                &catalog,
                &registry,
                &encoder,
                &reward_spec,
                &initial_facts,
                &route_prefix,
                root_checkpoint_sha256,
                seed_index,
                seed,
            )?;
            write_new(
                &seed_result_path,
                &serde_json::to_vec_pretty(&result).map_err(route_error)?,
            )?;
            seed_results.push(result);
        }
        Ok::<_, NativeTacticRouteRunError>(seed_results)
    })();
    let shutdown = worker.shutdown().map_err(route_error);
    let seed_results = match (run, shutdown) {
        (Ok(seed_results), Ok(())) => seed_results,
        (Err(error), _) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
    };

    let useful_decisions = seed_results.iter().map(|seed| seed.useful_decisions).sum();
    let timing = aggregate_route_timing(&seed_results);
    let report = NativeTacticRouteReport {
        schema: NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V5.into(),
        optimization_request_sha256: config.optimization.content_sha256,
        execution_binding_sha256: config.execution.content_sha256,
        objective_sha256: config.optimization.terminal_predicate.definition_sha256,
        feature_schema_sha256: encoder.schema_sha256,
        action_schema_sha256,
        goal_target,
        reward_spec,
        demonstration_transitions: 0,
        exploration_seeds: config.exploration_seeds.to_vec(),
        decisions_per_seed: config.decisions_per_seed,
        refit_every_decisions: config.refit_every_decisions,
        successful_seeds: seed_results.iter().filter(|seed| seed.success).count() as u64,
        total_native_ticks: seed_results.iter().map(|seed| seed.native_ticks).sum(),
        total_decisions: seed_results.iter().map(|seed| seed.decisions).sum(),
        useful_decisions,
        timing,
        seeds: seed_results,
    };
    write_new(
        &config.output_root.join("report.json"),
        &serde_json::to_vec_pretty(&report).map_err(route_error)?,
    )?;
    Ok(report)
}

fn route_action_schema_sha256(
    catalog: &dusklight_learning::tactic_asset::TacticAssetCatalog,
    blueprints: &[TacticBlueprint],
) -> Result<Digest, NativeTacticRouteRunError> {
    if blueprints.is_empty() {
        return Ok(catalog.action_schema_sha256());
    }
    let mut authored = blueprints
        .iter()
        .map(|blueprint| {
            Ok((
                blueprint.asset_id.clone(),
                blueprint.content_sha256().map_err(route_error)?,
            ))
        })
        .collect::<Result<Vec<_>, NativeTacticRouteRunError>>()?;
    authored.sort();
    let bytes =
        serde_json::to_vec(&(catalog.action_schema_sha256(), authored)).map_err(route_error)?;
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.route-tactic-action-schema/v1\0");
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
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

fn elapsed_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn decision_trace_is_useful(decision: &NativeTacticDecisionTrace) -> bool {
    decision.terminal
        || decision.reward > 0.0
        || decision.goal_distance_after < decision.goal_distance_before
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
    let mut useful_decisions = 0_u64;
    let mut native_ticks = 0_u64;
    let mut episodes = 0_u64;
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
        useful_decisions = useful_decisions.saturating_add(seed.useful_decisions);
        native_ticks = native_ticks.saturating_add(seed.native_ticks);
        episodes = episodes.saturating_add(seed.episodes);
    }
    timing.useful_decisions_per_second_millionths =
        per_second_millionths(useful_decisions, timing.wall_micros);
    timing.native_ticks_per_second_millionths =
        per_second_millionths(native_ticks, timing.wall_micros);
    timing.episodes_per_second_millionths = per_second_millionths(episodes, timing.wall_micros);
    timing
}

#[allow(clippy::too_many_arguments)]
fn run_seed(
    config: &NativeTacticRouteRunConfig<'_>,
    worker: &mut NativeSuffixWorkerSession,
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
            let current = LearnerState::build(
                initial_facts.clone(),
                registry,
                catalog,
                config.blueprints,
                |_| true,
            )
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
    let maximum_catalog_tactic_ticks = catalog
        .entries()
        .iter()
        .map(|entry| u64::from(entry.description().duration.maximum_ticks))
        .max()
        .ok_or_else(|| route_message("tactic catalog is empty"))?;
    let maximum_blueprint_ticks = config
        .blueprints
        .iter()
        .map(|blueprint| {
            blueprint
                .duration_against_catalog(catalog)
                .map(|duration| u64::from(duration.maximum_ticks))
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(route_error)?
        .into_iter()
        .max()
        .unwrap_or(0);
    let maximum_tactic_ticks = maximum_catalog_tactic_ticks.max(maximum_blueprint_ticks);
    let encode = |facts: &FactSnapshot| encoder.encode(facts);
    let checkpoint_root = seed_root.join("checkpoints");
    let mut rolling_checkpoint = None;

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
                    config.blueprints,
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
        let decision = loop {
            let suffix_ticks = campaign
                .route_tape
                .frames
                .len()
                .saturating_sub(source_frame as usize) as u64;
            let selection_started = Instant::now();
            let preview = campaign
                .decide(catalog, config.blueprints, &encode)
                .map_err(route_error)?;
            timing.tactic_selection_micros = timing
                .tactic_selection_micros
                .saturating_add(elapsed_micros(selection_started.elapsed()));
            let selected_maximum_ticks = preview
                .ranking
                .choices
                .iter()
                .find(|choice| choice.descriptor == preview.selected.descriptor)
                .ok_or_else(|| route_message("selected tactic is absent from its live choices"))?
                .duration
                .maximum_ticks;
            if selected_tactic_fits_horizon(suffix_ticks, selected_maximum_ticks, horizon) {
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
                    config.blueprints,
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
        let paths = NativeTacticWorkerPaths {
            request: paths_root.join("request.json"),
            result: paths_root.join("result.json"),
        };
        let execution_started = Instant::now();
        let (outcome, native_elapsed) = {
            let mut timed_worker = TimedTacticWorker::new(worker);
            let outcome = execute_selected_tactic(
                &mut timed_worker,
                &decision.selected,
                catalog,
                config.blueprints,
                &campaign.current.snapshot,
                &campaign.route_tape,
                &paths,
            )
            .map_err(route_error)?;
            (outcome, timed_worker.native_elapsed)
        };
        let execution_elapsed = execution_started.elapsed();
        timing.tactic_execution_micros = timing
            .tactic_execution_micros
            .saturating_add(elapsed_micros(execution_elapsed));
        timing.native_simulation_micros = timing
            .native_simulation_micros
            .saturating_add(elapsed_micros(native_elapsed));
        timing.tactic_preparation_and_fact_extraction_micros = timing
            .tactic_preparation_and_fact_extraction_micros
            .saturating_add(elapsed_micros(
                execution_elapsed.saturating_sub(native_elapsed),
            ));
        let model_started = Instant::now();
        let step = campaign
            .retain_and_refit_rewarded(
                decision,
                outcome,
                catalog,
                config.blueprints,
                registry,
                &encode,
                |_| true,
                reward_spec,
                refit_model,
            )
            .map_err(route_error)?;
        timing.model_update_micros = timing
            .model_update_micros
            .saturating_add(elapsed_micros(model_started.elapsed()));
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
        native_ticks = native_ticks.saturating_add(u64::from(
            step.step.transition.execution.duration.realized_ticks,
        ));
        let diagnostics = campaign.diagnostics().map_err(route_error)?;
        let before_features = encoder
            .encode(&step.step.transition.before)
            .map_err(route_error)?;
        let after_features = encoder
            .encode(&step.step.transition.after)
            .map_err(route_error)?;
        let measurements = encoder
            .feature_names
            .iter()
            .cloned()
            .zip(before_features.iter().copied())
            .zip(after_features.iter().copied())
            .map(|((name, before), after)| NativeTacticMeasurementTrace {
                name,
                before,
                after,
            })
            .collect();
        let applicable_tactics = step
            .step
            .decision
            .ranking
            .choices
            .iter()
            .map(|choice| {
                let value = step
                    .step
                    .decision
                    .ranking
                    .values
                    .ranked
                    .iter()
                    .find(|value| value.descriptor == choice.descriptor);
                NativeTacticValueTrace {
                    option_id: choice.choice_id.clone(),
                    mean_q: value.map(|value| value.mean_q),
                    ensemble_variance: value.map(|value| value.ensemble_variance),
                    selected: choice.descriptor == selected.descriptor,
                }
            })
            .collect();
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
            frontier_cells: diagnostics.frontier_cells,
            visited_states: campaign.visited_state_count(),
            before: tactic_state_trace(&step.step.transition.before)?,
            after: tactic_state_trace(&step.step.transition.after)?,
            measurements,
            applicable_tactics,
        };
        if decision_trace_is_useful(&decision_trace) {
            useful_decisions = useful_decisions.saturating_add(1);
        }
        let decision_trace_root = seed_root.join("decision-trace");
        fs::create_dir_all(&decision_trace_root).map_err(route_error)?;
        write_new(
            &decision_trace_root.join(format!("decision-{decision_index:06}.json")),
            &serde_json::to_vec_pretty(&decision_trace).map_err(route_error)?,
        )?;
        write_new(
            &seed_root
                .join("decision-summary")
                .join(format!("decision-{decision_index:06}.json")),
            &serde_json::to_vec_pretty(&NativeTacticDecisionSummary {
                schema: NATIVE_TACTIC_DECISION_SUMMARY_SCHEMA_V1,
                decision_index,
                episode,
                selected_option_id: &decision_trace.selected_option_id,
                selection_reason: decision_trace.selection_reason,
                reward: decision_trace.reward,
                duration_ticks: decision_trace.reward_components.duration_ticks,
                goal_distance_before: decision_trace.goal_distance_before,
                goal_distance_after: decision_trace.goal_distance_after,
                terminal: decision_trace.terminal,
            })
            .map_err(route_error)?,
        )?;
        write_new(
            &seed_root
                .join("edge-tapes")
                .join(format!("edge-{decision_index:06}.tape")),
            &campaign.route_tape.encode().map_err(route_error)?,
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
        if step.step.transition.value_sample.terminal
            || campaign.decision_index % config.branch_every_decisions == 0
        {
            let graph_root = seed_root.join("knowledge-graph");
            fs::create_dir_all(&graph_root).map_err(route_error)?;
            write_new(
                &graph_root.join(format!("graph-{:06}.json", campaign.decision_index)),
                &serde_json::to_vec_pretty(&campaign.graph_projection().map_err(route_error)?)
                    .map_err(route_error)?,
            )?;
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
                .write_checkpoint(&checkpoint_root)
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
    let success = campaign.current.snapshot.terminal.reached == Some(true);
    let final_checkpoint = campaign
        .write_checkpoint(&seed_root.join("final-checkpoint"))
        .map_err(route_error)?;
    remove_rolling_checkpoint(&checkpoint_root, &mut rolling_checkpoint)?;
    let graph_path = seed_root.join("graph.json");
    write_new(
        &graph_path,
        &serde_json::to_vec_pretty(&campaign.graph_projection().map_err(route_error)?)
            .map_err(route_error)?,
    )?;
    let (successful_tape, final_result) = if success {
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
        diagnostics: Some(campaign.diagnostics().map_err(route_error)?),
        final_checkpoint: Some(path_text(&final_checkpoint)),
        graph: Some(path_text(&graph_path)),
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
        .write_checkpoint(
            &seed_root
                .join("pause-checkpoints")
                .join(format!("decision-{:06}", campaign.decision_index)),
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
        || performance.timing.native_simulation_micros > performance.timing.tactic_execution_micros
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
    let checkpoint: TacticQCampaignCheckpoint = read_bounded_json(&checkpoint_path)?;
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
            .checked_add(u64::from(decision.reward_components.duration_ticks))
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
                        .is_some_and(|value| value == "json")
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
    let trace_root = seed_root.join("decision-trace");
    if decision_count == 0 && !trace_root.exists() {
        return Ok(Vec::new());
    }
    let actual_count = fs::read_dir(&trace_root)
        .map_err(route_error)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
                && entry
                    .path()
                    .extension()
                    .is_some_and(|value| value == "json")
        })
        .count() as u64;
    if actual_count != decision_count {
        return Err(route_message(
            "paused tactic decision trace does not exactly match its checkpoint",
        ));
    }
    let mut trace = Vec::with_capacity(usize::try_from(decision_count).map_err(route_error)?);
    for decision_index in 0..decision_count {
        let path = trace_root.join(format!("decision-{decision_index:06}.json"));
        let decision: NativeTacticDecisionTrace = read_bounded_json(&path)?;
        if decision.decision_index != decision_index {
            return Err(route_message(
                "paused tactic decision trace has a detached index",
            ));
        }
        trace.push(decision);
    }
    Ok(trace)
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
        || result.timing.native_simulation_micros > result.timing.tactic_execution_micros
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
                .map(|decision| u64::from(decision.reward_components.duration_ticks))
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
    pub supporting_load_triggers: usize,
    pub source_inventory_sha256: Digest,
    pub authored_route_coordinates_used: bool,
}

impl GoalTransitionTarget {
    fn report(
        &self,
        source_coordinate: [f32; 3],
        tactic_targets: Vec<[f32; 3]>,
    ) -> NativeTacticGoalTargetReport {
        NativeTacticGoalTargetReport {
            source_stage: self.source_stage.clone(),
            source_room: self.source_room,
            destination_stage: self.destination_stage.clone(),
            destination_room: self.destination_room,
            destination_point: self.destination_point,
            coordinate: self.coordinate,
            source_coordinate,
            tactic_targets,
            supporting_load_triggers: self.supporting_load_triggers,
            source_inventory_sha256: self.source_inventory_sha256,
            authored_route_coordinates_used: false,
        }
    }
}

pub(crate) fn goal_conditioned_tactic_runtime(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    initial_facts: &FactSnapshot,
) -> Result<GoalConditionedTacticRuntime, NativeTacticRouteRunError> {
    let target = resolve_goal_transition_target(root, optimization, execution)?;
    if initial_facts.world.stage != target.source_stage
        || initial_facts.world.room != target.source_room
    {
        return Err(route_message(
            "native source observation differs from the objective's source world",
        ));
    }
    let source_coordinate = initial_facts.player.position_f32_bits.map(f32::from_bits);
    let tactic_targets = goal_corridor_targets(source_coordinate, target.coordinate)?;
    let maximum_ticks = goal_tactic_maximum_ticks(optimization.budgets.exploration_horizon_ticks)?;
    let catalog = goal_conditioned_route_tactic_catalog(&tactic_targets, maximum_ticks)
        .map_err(route_error)?;
    let encoder =
        GoalConditionedTacticFeatureEncoder::new(target.coordinate).map_err(route_error)?;
    Ok(GoalConditionedTacticRuntime {
        catalog,
        encoder,
        report: target.report(source_coordinate, tactic_targets),
    })
}

fn goal_corridor_targets(
    source: [f32; 3],
    goal: [f32; 3],
) -> Result<Vec<[f32; 3]>, NativeTacticRouteRunError> {
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
    for fraction in [0.25_f32, 0.5, 0.75, 1.0] {
        let center = [
            source[0] + dx * fraction,
            source[1] + (goal[1] - source[1]) * fraction,
            source[2] + dz * fraction,
        ];
        for offset in [-768.0_f32, -384.0, 0.0, 384.0, 768.0] {
            let target = [
                center[0] + perpendicular[0] * offset,
                center[1],
                center[2] + perpendicular[1] * offset,
            ];
            if identities.insert(target.map(f32::to_bits)) {
                targets.push(target);
            }
        }
    }
    Ok(targets)
}

fn resolve_goal_transition_target(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
) -> Result<GoalTransitionTarget, NativeTacticRouteRunError> {
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
    Ok(GoalTransitionTarget {
        source_stage,
        source_room,
        destination_stage,
        destination_room,
        destination_point,
        coordinate,
        supporting_load_triggers: collision_ids.len(),
        source_inventory_sha256: stage_binding.inventory_sha256,
    })
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
    use dusklight_learning::tactic_blueprint::TacticBlueprintNode;

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
    fn route_action_schema_authenticates_authored_blueprints() {
        let catalog = goal_conditioned_route_tactic_catalog(&[[1.0, 0.0, 1.0]], 120).unwrap();
        let first = TacticBlueprint::new(
            "patient_wait",
            TacticBlueprintNode::Invoke {
                option_id: "wait.neutral.04".into(),
            },
        )
        .unwrap();
        let second = TacticBlueprint::new(
            "patient_wait",
            TacticBlueprintNode::Sequence {
                steps: vec![
                    TacticBlueprintNode::Invoke {
                        option_id: "wait.neutral.04".into(),
                    },
                    TacticBlueprintNode::Invoke {
                        option_id: "wait.neutral.04".into(),
                    },
                ],
            },
        )
        .unwrap();

        assert_eq!(
            route_action_schema_sha256(&catalog, &[]).unwrap(),
            catalog.action_schema_sha256()
        );
        assert_ne!(
            route_action_schema_sha256(&catalog, &[first]).unwrap(),
            route_action_schema_sha256(&catalog, &[second]).unwrap()
        );
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
    }

    #[test]
    fn goal_corridor_is_a_symmetric_start_and_goal_derived_action_basis() {
        let source = [0.0, 10.0, 0.0];
        let goal = [1000.0, 20.0, 0.0];
        let targets = goal_corridor_targets(source, goal).unwrap();

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
        assert!(goal_corridor_targets(source, source).is_err());
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
                checkpoint.join(format!("tactic-q-{decision}.json")),
                b"checkpoint",
            )
            .unwrap();
        }

        let (decision, path) = latest_pause_checkpoint(&directory).unwrap();
        assert_eq!(decision, 7);
        assert_eq!(path.file_name().unwrap(), "tactic-q-7.json");
        fs::remove_dir_all(directory).unwrap();
    }
}
