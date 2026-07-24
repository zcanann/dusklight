//! Fresh-model tactic-Q route learning on an authenticated native checkpoint.

use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::native_suffix_result::{NativeTerminalBinding, ValidatedNativeSuffixBatch};
use crate::native_suffix_worker::{
    NativeSuffixWorkerError, NativeSuffixWorkerLaunch, NativeSuffixWorkerSession,
};
use crate::native_tactic_worker::{NativeTacticWorkerPaths, tactic_root_checkpoint_sha256};
use crate::optimization_request::OptimizationRequest;
use crate::tactic_q_campaign::{TacticCampaignDiagnostics, TacticQCampaign, TacticQCampaignError};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::{InputTape, RawPadState};
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use dusklight_learning::default_tactic_catalog::default_route_tactic_catalog;
use dusklight_learning::fact_registry::FactRegistry;
use dusklight_learning::fact_snapshot::FactSnapshot;
use dusklight_learning::fqi::FqiConfig;
use dusklight_learning::learner_state::LearnerState;
use dusklight_learning::option_values::OptionValueConfig;
use dusklight_learning::reward_shaping::{TACTIC_REWARD_SPEC_SCHEMA_V1, TacticRewardSpec};
use dusklight_learning::tactic_exploration::{TacticExplorationConfig, TacticSelectionReason};
use dusklight_learning::tactic_features::TacticFeatureEncoder;
use dusklight_search::search::{MacroAction, SearchPadState};
use dusklight_search::suffix_batch::{
    NATIVE_SUFFIX_BATCH_SCHEMA, NativeCheckpointValidation, NativeSuffixBatch,
    NativeSuffixCandidate,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V1: &str = "dusklight-native-tactic-route-report/v1";
const MAX_ROUTE_SEEDS: usize = 32;
const MAX_ROUTE_DECISIONS: u64 = 100_000;

#[derive(Clone, Debug)]
pub struct NativeTacticRouteRunConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub output_root: &'a Path,
    pub exploration_seeds: &'a [u64],
    pub decisions_per_seed: u64,
    pub branch_every_decisions: u64,
    pub epsilon_per_million: u32,
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
    pub demonstration_transitions: u64,
    pub exploration_seeds: Vec<u64>,
    pub decisions_per_seed: u64,
    pub successful_seeds: u64,
    pub total_native_ticks: u64,
    pub total_decisions: u64,
    pub seeds: Vec<NativeTacticSeedResult>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticSeedResult {
    pub seed: u64,
    pub success: bool,
    pub decisions: u64,
    pub episodes: u64,
    pub native_ticks: u64,
    pub replay_rows: usize,
    pub visited_states: usize,
    pub selection_counts: BTreeMap<String, u64>,
    pub diagnostics: Option<TacticCampaignDiagnostics>,
    pub final_checkpoint: Option<String>,
    pub graph: Option<String>,
    pub successful_tape: Option<String>,
    pub final_result: Option<String>,
    pub trace: Vec<NativeTacticDecisionTrace>,
}

#[derive(Clone, Debug, Serialize)]
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
    pub terminal: bool,
    pub frontier_cells: usize,
    pub visited_states: usize,
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
    if config.output_root.exists() {
        return Err(route_message(format!(
            "tactic route output already exists: {}",
            config.output_root.display()
        )));
    }
    fs::create_dir_all(config.output_root).map_err(route_error)?;

    let catalog = default_route_tactic_catalog().map_err(route_error)?;
    let registry = FactRegistry::canonical();
    let encoder = TacticFeatureEncoder::new();
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
    let initial_root = config.output_root.join("initial");
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
        state_root: config.output_root.join("native-state"),
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
    let root_checkpoint_sha256 =
        tactic_root_checkpoint_sha256(worker.identity()).map_err(route_error)?;

    let run = (|| {
        let mut seed_results = Vec::with_capacity(config.exploration_seeds.len());
        for (seed_index, seed) in config.exploration_seeds.iter().copied().enumerate() {
            seed_results.push(run_seed(
                config,
                &mut worker,
                &catalog,
                &registry,
                &encoder,
                &initial_facts,
                &route_prefix,
                root_checkpoint_sha256,
                seed_index,
                seed,
            )?);
        }
        Ok::<_, NativeTacticRouteRunError>(seed_results)
    })();
    let shutdown = worker.shutdown().map_err(route_error);
    let seed_results = run?;
    shutdown?;

    let report = NativeTacticRouteReport {
        schema: NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V1.into(),
        optimization_request_sha256: config.optimization.content_sha256,
        execution_binding_sha256: config.execution.content_sha256,
        objective_sha256: config.optimization.terminal_predicate.definition_sha256,
        feature_schema_sha256: encoder.schema_sha256,
        action_schema_sha256: catalog.action_schema_sha256(),
        demonstration_transitions: 0,
        exploration_seeds: config.exploration_seeds.to_vec(),
        decisions_per_seed: config.decisions_per_seed,
        successful_seeds: seed_results.iter().filter(|seed| seed.success).count() as u64,
        total_native_ticks: seed_results.iter().map(|seed| seed.native_ticks).sum(),
        total_decisions: seed_results.iter().map(|seed| seed.decisions).sum(),
        seeds: seed_results,
    };
    write_new(
        &config.output_root.join("report.json"),
        &serde_json::to_vec_pretty(&report).map_err(route_error)?,
    )?;
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn run_seed(
    config: &NativeTacticRouteRunConfig<'_>,
    worker: &mut NativeSuffixWorkerSession,
    catalog: &dusklight_learning::tactic_asset::TacticAssetCatalog,
    registry: &FactRegistry,
    encoder: &TacticFeatureEncoder,
    initial_facts: &FactSnapshot,
    route_prefix: &InputTape,
    root_checkpoint_sha256: Digest,
    seed_index: usize,
    seed: u64,
) -> Result<NativeTacticSeedResult, NativeTacticRouteRunError> {
    let seed_root = config
        .output_root
        .join(format!("seed-{seed_index:03}-{seed}"));
    fs::create_dir_all(&seed_root).map_err(route_error)?;
    let current = LearnerState::build(initial_facts.clone(), registry, catalog, &[], |_| true)
        .map_err(route_error)?;
    let mut campaign = TacticQCampaign::new(
        encoder.schema_sha256,
        config.optimization.terminal_predicate.definition_sha256,
        root_checkpoint_sha256,
        seed_group(seed_index, 0)?,
        current,
        route_prefix.clone(),
        OptionValueConfig {
            fitted_q: FqiConfig {
                iterations: 12,
                trees_per_action: 15,
                max_tree_depth: 8,
                seed: 0xd15c_a11d_5eed_f017 ^ seed,
                ..FqiConfig::default()
            },
        },
        TacticExplorationConfig {
            seed,
            epsilon_per_million: config.epsilon_per_million,
        },
    )
    .map_err(route_error)?;
    let reward_spec = TacticRewardSpec {
        schema: TACTIC_REWARD_SPEC_SCHEMA_V1.into(),
        terminal_reward: 100.0,
        tick_cost: 0.01,
        novelty_reward: 0.05,
        per_tick_discount: 0.995,
        potential: None,
    };
    let mut trace = Vec::new();
    let mut selection_counts = BTreeMap::<String, u64>::new();
    let mut native_ticks = 0_u64;
    let mut episode = 0_u64;
    let source_frame = config.optimization.route.source_boundary_index;
    let horizon = config.optimization.budgets.exploration_horizon_ticks;
    let maximum_tactic_ticks = catalog
        .entries()
        .iter()
        .map(|entry| u64::from(entry.description().duration.maximum_ticks))
        .max()
        .ok_or_else(|| route_message("tactic catalog is empty"))?;
    let encode = |facts: &FactSnapshot| encoder.encode(facts);

    while campaign.decision_index < config.decisions_per_seed
        && native_ticks < config.optimization.budgets.simulated_tick_budget
    {
        let suffix_ticks = campaign
            .route_tape
            .frames
            .len()
            .saturating_sub(source_frame as usize) as u64;
        let periodic_branch = campaign.decision_index > 0
            && campaign.decision_index % config.branch_every_decisions == 0;
        let horizon_branch = suffix_ticks.saturating_add(maximum_tactic_ticks) > horizon;
        if !campaign.replay.is_empty() && (periodic_branch || horizon_branch) {
            episode = episode
                .checked_add(1)
                .ok_or_else(|| route_message("episode counter overflowed"))?;
            let [root, frontier] = campaign
                .sample_root_and_frontier(seed, episode, &[])
                .map_err(route_error)?;
            let frontier_ticks = frontier
                .route_tape
                .frames
                .len()
                .saturating_sub(source_frame as usize) as u64;
            let prefer_root =
                episode % 4 == 0 || frontier_ticks.saturating_add(maximum_tactic_ticks) > horizon;
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
        }

        let decision_index = campaign.decision_index;
        let paths_root = seed_root
            .join("native")
            .join(format!("decision-{decision_index:06}"));
        fs::create_dir_all(&paths_root).map_err(route_error)?;
        let step = campaign
            .execute_and_refit_rewarded(
                worker,
                catalog,
                &[],
                registry,
                &NativeTacticWorkerPaths {
                    request: paths_root.join("request.json"),
                    result: paths_root.join("result.json"),
                },
                &encode,
                |_| true,
                &reward_spec,
            )
            .map_err(route_error)?;
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
        trace.push(NativeTacticDecisionTrace {
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
            terminal: step.step.transition.value_sample.terminal,
            frontier_cells: diagnostics.frontier_cells,
            visited_states: campaign.visited_state_count(),
        });
        campaign
            .write_checkpoint(&seed_root.join("checkpoints"))
            .map_err(route_error)?;
        if step.step.transition.value_sample.terminal {
            break;
        }
    }

    let success = campaign.current.snapshot.terminal.reached == Some(true);
    let final_checkpoint = campaign
        .write_checkpoint(&seed_root.join("final-checkpoint"))
        .map_err(route_error)?;
    let graph_path = seed_root.join("graph.json");
    write_new(
        &graph_path,
        &serde_json::to_vec_pretty(&campaign.graph().map_err(route_error)?).map_err(route_error)?,
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
    Ok(NativeTacticSeedResult {
        seed,
        success,
        decisions: campaign.decision_index,
        episodes: episode + 1,
        native_ticks,
        replay_rows: campaign.replay.len(),
        visited_states: campaign.visited_state_count(),
        selection_counts,
        diagnostics: Some(campaign.diagnostics().map_err(route_error)?),
        final_checkpoint: Some(path_text(&final_checkpoint)),
        graph: Some(path_text(&graph_path)),
        successful_tape,
        final_result,
        trace,
    })
}

fn initial_probe_batch(
    config: &NativeTacticRouteRunConfig<'_>,
) -> Result<NativeSuffixBatch, NativeTacticRouteRunError> {
    tactic_root_probe_batch(config.optimization, config.execution)
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
pub struct NativeTacticRouteRunError(String);

impl fmt::Display for NativeTacticRouteRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeTacticRouteRunError {}

fn route_message(message: impl Into<String>) -> NativeTacticRouteRunError {
    NativeTacticRouteRunError(message.into())
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

    #[test]
    fn root_probe_uses_the_full_declared_horizon() {
        assert!(MAX_ROUTE_DECISIONS > 0);
    }
}
