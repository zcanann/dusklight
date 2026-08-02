//! Cold-root native episodes driven by the minimal route-agnostic scratch Q loop.

use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::native_suffix_result::NativeTerminalBinding;
use crate::native_suffix_worker::{
    NativeSuffixPrevalidatedFileIdentities, NativeSuffixWorkerLaunch, NativeSuffixWorkerSession,
};
use crate::native_tactic_route_runner::{
    NativeTacticColdReplayAttempt, NativeTapeColdReplayConfig, exact_cold_replay_attempts,
    initial_facts, run_native_tape_cold_replay_after_execution_validation,
    tactic_root_probe_batch_with_ticks,
};
use crate::native_tactic_worker::{
    NativeGenericExecutionStrategy, NativeTacticCheckpointRetention, NativeTacticCheckpointSource,
    NativeTacticCheckpointStorage, NativeTacticWorkerPaths,
    execute_selected_tactic_with_checkpoint_retention_and_strategy,
};
use crate::optimization_request::OptimizationRequest;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_learning::fact_registry::FactRegistry;
use dusklight_learning::fact_snapshot::FRONT_ROLL_DO_STATUS;
use dusklight_learning::learner_state::LearnerState;
use dusklight_learning::scratch_action_catalog::scratch_action_catalog;
use dusklight_learning::scratch_q::{
    EPSILON_SCALE, MAX_SCRATCH_EPISODE_TICKS, ScratchQTable, ScratchSelectionReason,
    ScratchStateKey, ScratchTransition, transition_sha256,
};
use dusklight_learning::tactic_exploration::{
    SelectedTactic, TACTIC_EXPLORATION_SCHEMA_V1, TacticSelectionReason,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
use std::path::Path;
use std::time::{Duration, Instant};

pub const NATIVE_SCRATCH_REPORT_SCHEMA_V1: &str = "dusklight-native-scratch-report/v1";
const NATIVE_SCRATCH_CHECKPOINT_SCHEMA_V1: &str = "dusklight-native-scratch-checkpoint/v1";
const CHECKPOINT_MAGIC: &[u8; 8] = b"DSSCRQ01";
const CHECKPOINT_VERSION: u16 = 1;
const CHECKPOINT_HEADER_BYTES: usize = 8 + 2 + 2 + 8 + 32;
const CHECKPOINT_COMPRESSION_LEVEL: i32 = 1;
const MAX_EPISODES: u64 = 1_000_000;
const LIVE_CHECKPOINT_CACHE_BYTES: usize = 640 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct NativeScratchRunConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub output_root: &'a Path,
    pub seed: u64,
    pub episodes: u64,
    pub maximum_episode_ticks: u32,
    pub epsilon_per_million: u32,
    pub cold_replay_timeout: Duration,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeScratchReport {
    pub schema: String,
    pub report_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub objective_sha256: Digest,
    pub action_universe_sha256: Digest,
    pub seed: u64,
    pub maximum_episode_ticks: u32,
    pub epsilon_per_million: u32,
    pub completed_episodes: u64,
    pub distinct_episode_tapes: u64,
    pub duplicate_episode_tapes: u64,
    pub unique_transitions: u64,
    pub terminal_episodes: u64,
    pub fastest_selected_ticks: Option<u64>,
    pub learner_updates: u64,
    pub changed_choices: u64,
    pub q_state_actions: u64,
    pub native_ticks: u64,
    pub native_wall_micros: u64,
    pub wall_micros: u64,
    pub first_terminal_wall_micros: Option<u64>,
    pub fastest_tape: Option<String>,
    pub fastest_tape_sha256: Option<Digest>,
    pub strict_winners_cold_replayed: u64,
    pub episodes: Vec<NativeScratchEpisodeReport>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeScratchEpisodeReport {
    pub episode_index: u64,
    pub action_sequence_sha256: Digest,
    pub decisions: u64,
    pub native_ticks: u64,
    pub terminal: bool,
    pub selected_ticks: Option<u64>,
    pub unique_transitions_added: u64,
    pub learner_updates: u64,
    pub changed_choices: u64,
    pub native_wall_micros: u64,
    pub tape_sha256: Digest,
    pub strict_winner: bool,
    pub cold_replay_attempts: Vec<NativeTacticColdReplayAttempt>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeScratchCheckpoint {
    schema: String,
    optimization_request_sha256: Digest,
    execution_binding_sha256: Digest,
    objective_sha256: Digest,
    action_universe_sha256: Digest,
    seed: u64,
    maximum_episode_ticks: u32,
    epsilon_per_million: u32,
    q: ScratchQTable,
    unique_transitions: BTreeMap<Digest, ScratchTransition>,
    completed_action_sequences: Vec<Vec<usize>>,
    report: NativeScratchReport,
}

struct EpisodeOutcome {
    transitions: Vec<ScratchTransition>,
    action_sequence: Vec<usize>,
    native_ticks: u64,
    terminal: bool,
    tape: InputTape,
    native_wall_micros: u64,
}

pub fn run_native_scratch_learner(
    config: &NativeScratchRunConfig<'_>,
) -> Result<NativeScratchReport, NativeScratchRunError> {
    validate_config(config)?;
    let started = Instant::now();
    let root = config.repository_root.canonicalize().map_err(run_error)?;
    config
        .execution
        .validate_files(&root, config.optimization)
        .map_err(run_error)?;
    let authority =
        crate::native_residual_campaign::ValidatedNativeResidualExecution::authenticate(
            &root,
            config.optimization,
            config.execution,
        )
        .map_err(run_error)?;
    let process_tape = InputTape::decode(
        &fs::read(root.join(&config.execution.process_boot_tape.path)).map_err(run_error)?,
    )
    .map_err(run_error)?
    .tape;
    let source_frame =
        usize::try_from(config.optimization.route.source_boundary_index).map_err(run_error)?;
    let prefix_frames = process_tape
        .frames
        .get(..source_frame)
        .ok_or_else(|| run_message("scratch source frame is beyond the process tape"))?
        .to_vec();
    let catalog = scratch_action_catalog().map_err(run_error)?;
    let action_universe_sha256 = catalog.action_schema_sha256();
    let checkpoint_path = config.output_root.join("checkpoint.dssq");
    let mut checkpoint = if checkpoint_path.exists() {
        read_checkpoint(&checkpoint_path)?
    } else {
        if config.output_root.exists() {
            return Err(run_message(format!(
                "scratch output exists without a checkpoint: {}",
                config.output_root.display()
            )));
        }
        fs::create_dir_all(config.output_root).map_err(run_error)?;
        let mut checkpoint =
            fresh_checkpoint(config, action_universe_sha256, catalog.entries().len())?;
        seal_report(&mut checkpoint.report)?;
        checkpoint
    };
    validate_checkpoint(&checkpoint, config, action_universe_sha256)?;
    let prior_wall_micros = checkpoint.report.wall_micros;

    while checkpoint.report.completed_episodes < config.episodes {
        let episode_index = checkpoint.report.completed_episodes;
        let episode_root = config
            .output_root
            .join("episodes")
            .join(format!("episode-{episode_index:06}"));
        let outcome = run_episode(
            &root,
            config,
            &process_tape,
            &prefix_frames,
            &catalog,
            &checkpoint.q,
            &checkpoint.completed_action_sequences,
            episode_index,
            &episode_root,
        )?;
        let action_sequence_sha256 = sequence_sha256(&outcome.action_sequence)?;
        let duplicate = checkpoint
            .completed_action_sequences
            .iter()
            .any(|sequence| sequence == &outcome.action_sequence);
        let before_unique = checkpoint.unique_transitions.len();
        for transition in &outcome.transitions {
            checkpoint
                .unique_transitions
                .entry(transition_sha256(transition).map_err(run_error)?)
                .or_insert_with(|| transition.clone());
        }
        let unique_transitions_added = checkpoint.unique_transitions.len() - before_unique;
        let update = if duplicate {
            dusklight_learning::scratch_q::ScratchUpdateSummary {
                updates: 0,
                changed_choices: 0,
            }
        } else {
            checkpoint
                .q
                .update_episode(
                    &outcome.transitions,
                    outcome.terminal,
                    config.maximum_episode_ticks,
                )
                .map_err(run_error)?
        };
        let tape_bytes = outcome.tape.encode().map_err(run_error)?;
        let tape_sha256 = sha256(&tape_bytes);
        let selected_ticks = outcome.terminal.then(|| {
            u64::try_from(outcome.tape.frames.len())
                .unwrap_or(u64::MAX)
                .saturating_sub(config.optimization.route.source_boundary_index)
                .saturating_sub(1)
        });
        let strict_winner = selected_ticks.is_some_and(|ticks| {
            checkpoint
                .report
                .fastest_selected_ticks
                .is_none_or(|fastest| ticks < fastest)
        });
        let mut cold_replay_attempts = Vec::new();
        if strict_winner {
            let winner_index = checkpoint.report.strict_winners_cold_replayed;
            let winner_root = config
                .output_root
                .join("winners")
                .join(format!("winner-{winner_index:04}"));
            let replay_root = winner_root.join("cold-replay");
            let (controller_tape, attempts) =
                run_native_tape_cold_replay_after_execution_validation(
                    &NativeTapeColdReplayConfig {
                        repository_root: &root,
                        optimization: config.optimization,
                        execution: config.execution,
                        tape: &outcome.tape,
                        tape_bytes: &tape_bytes,
                        first_hit_tick: selected_ticks.unwrap(),
                        repetitions: 2,
                        timeout: config.cold_replay_timeout,
                        output_root: &replay_root,
                    },
                    &authority,
                )
                .map_err(run_error)?;
            if !exact_cold_replay_attempts(
                &attempts,
                &controller_tape,
                config.optimization.route.source_boundary_index,
                selected_ticks.unwrap(),
                outcome.tape.frames.len() as u64,
            ) {
                return Err(run_message(
                    "scratch strict winner did not cold-replay with identical evidence",
                ));
            }
            cold_replay_attempts = attempts;
            fs::create_dir_all(&winner_root).map_err(run_error)?;
            write_new(&winner_root.join("selected.tape"), &tape_bytes)?;
            checkpoint.report.fastest_selected_ticks = selected_ticks;
            checkpoint.report.fastest_tape = Some(path_text(&winner_root.join("selected.tape")));
            checkpoint.report.fastest_tape_sha256 = Some(tape_sha256);
            checkpoint.report.strict_winners_cold_replayed += 1;
        }
        if checkpoint.report.first_terminal_wall_micros.is_none() && outcome.terminal {
            checkpoint.report.first_terminal_wall_micros =
                Some(prior_wall_micros.saturating_add(elapsed_micros(started.elapsed())));
        }
        checkpoint.report.completed_episodes += 1;
        checkpoint.report.distinct_episode_tapes += u64::from(!duplicate);
        checkpoint.report.duplicate_episode_tapes += u64::from(duplicate);
        checkpoint.report.unique_transitions = checkpoint.unique_transitions.len() as u64;
        checkpoint.report.terminal_episodes += u64::from(outcome.terminal);
        checkpoint.report.learner_updates += update.updates;
        checkpoint.report.changed_choices += update.changed_choices;
        checkpoint.report.q_state_actions = checkpoint.q.unique_state_actions() as u64;
        checkpoint.report.native_ticks += outcome.native_ticks;
        checkpoint.report.native_wall_micros += outcome.native_wall_micros;
        checkpoint
            .completed_action_sequences
            .push(outcome.action_sequence);
        checkpoint.report.episodes.push(NativeScratchEpisodeReport {
            episode_index,
            action_sequence_sha256,
            decisions: outcome.transitions.len() as u64,
            native_ticks: outcome.native_ticks,
            terminal: outcome.terminal,
            selected_ticks,
            unique_transitions_added: unique_transitions_added as u64,
            learner_updates: update.updates,
            changed_choices: update.changed_choices,
            native_wall_micros: outcome.native_wall_micros,
            tape_sha256,
            strict_winner,
            cold_replay_attempts,
        });
        checkpoint.report.wall_micros =
            prior_wall_micros.saturating_add(elapsed_micros(started.elapsed()));
        seal_report(&mut checkpoint.report)?;
        write_checkpoint_atomic(&checkpoint_path, &checkpoint)?;
        write_report_atomic(&config.output_root.join("report.json"), &checkpoint.report)?;
    }
    Ok(checkpoint.report)
}

#[allow(clippy::too_many_arguments)]
fn run_episode(
    root: &Path,
    config: &NativeScratchRunConfig<'_>,
    process_tape: &InputTape,
    prefix_frames: &[dusklight_automation_contracts::tape::InputFrame],
    catalog: &dusklight_learning::tactic_asset::TacticAssetCatalog,
    q: &ScratchQTable,
    completed_sequences: &[Vec<usize>],
    episode_index: u64,
    episode_root: &Path,
) -> Result<EpisodeOutcome, NativeScratchRunError> {
    fs::create_dir_all(episode_root).map_err(run_error)?;
    let initial_batch =
        tactic_root_probe_batch_with_ticks(config.optimization, config.execution, 1)
            .map_err(run_error)?;
    let initial_root = episode_root.join("initial");
    fs::create_dir_all(&initial_root).map_err(run_error)?;
    let request_path = initial_root.join("request.json");
    write_new(
        &request_path,
        &serde_json::to_vec(&initial_batch).map_err(run_error)?,
    )?;
    let launch = NativeSuffixWorkerLaunch {
        executable: root.join(&config.execution.executable.path),
        game_data: root.join(&config.execution.game_data.path),
        input_tape: root.join(&config.execution.process_boot_tape.path),
        milestone_program: root.join(&config.execution.milestone_program.path),
        card_fixture: config
            .execution
            .card_fixture_root(root, config.optimization)
            .map_err(run_error)?,
        card_fixture_sha256: config.execution.card_fixture_manifest.sha256,
        working_directory: root.to_path_buf(),
        state_root: episode_root.join("native-state"),
        world_context_sha256: config.execution.world_context.sha256,
        terminal: NativeTerminalBinding {
            goal: config.optimization.terminal_predicate.goal.clone(),
            program_sha256: config.optimization.terminal_predicate.program_sha256,
            definition_sha256: config.optimization.terminal_predicate.definition_sha256,
        },
        initial_batch: request_path,
        initial_result: initial_root.join("result.json"),
        initial_winner_tape: None,
    };
    let episode_started = Instant::now();
    let (mut worker, initial) = NativeSuffixWorkerSession::launch_compact_with_prevalidated_files(
        &launch,
        NativeSuffixPrevalidatedFileIdentities {
            executable_sha256: config.execution.executable.sha256,
            game_data_sha256: config.execution.game_data.sha256,
        },
    )
    .map_err(run_error)?;
    let run = (|| {
        let registry = FactRegistry::canonical();
        let initial = initial_facts(&initial).map_err(run_error)?;
        if initial.tape_frame != config.optimization.route.source_boundary_index
            || initial.terminal.reached != Some(false)
        {
            return Err(run_message(
                "scratch episode did not start at the authenticated root",
            ));
        }
        let mut facts = initial;
        let mut tape = InputTape {
            boot: process_tape.boot.clone(),
            tick_rate_numerator: process_tape.tick_rate_numerator,
            tick_rate_denominator: process_tape.tick_rate_denominator,
            frames: prefix_frames.to_vec(),
        };
        tape.validate().map_err(run_error)?;
        let mut transitions = Vec::new();
        let mut sequence = Vec::new();
        let mut native_ticks = 0_u64;
        let mut checkpoint_source = None;
        while facts.terminal.reached != Some(true)
            && native_ticks < u64::from(config.maximum_episode_ticks)
        {
            let learner_state =
                LearnerState::build(facts.clone(), &registry, catalog, &[], |_| true)
                    .map_err(run_error)?;
            let state = ScratchStateKey::from_snapshot(&facts).map_err(run_error)?;
            let remaining = u64::from(config.maximum_episode_ticks) - native_ticks;
            let front_roll_available = facts
                .player
                .action_state
                .is_some_and(|action| action.do_status == FRONT_ROLL_DO_STATUS);
            let mut eligible = learner_state
                .action_mask
                .iter()
                .enumerate()
                .filter(|(_, action)| {
                    action.applicable
                        && action_prompt_available(
                            &action.descriptor.option_id,
                            front_roll_available,
                        )
                        && u64::from(action.duration.maximum_ticks) <= remaining
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            exclude_completed_duplicate(&sequence, completed_sequences, &mut eligible);
            if eligible.is_empty() {
                break;
            }
            let selection = q
                .select(
                    &state,
                    &eligible,
                    config.seed,
                    episode_index,
                    sequence.len() as u64,
                    config.epsilon_per_million,
                )
                .map_err(run_error)?;
            let action = learner_state
                .action_mask
                .get(selection.action_index)
                .ok_or_else(|| run_message("scratch selected action is absent"))?;
            let selected = SelectedTactic {
                schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
                learner_snapshot_sha256: learner_state.snapshot_sha256,
                decision_index: sequence.len() as u64,
                descriptor: action.descriptor.clone(),
                reason: match selection.reason {
                    ScratchSelectionReason::Greedy => TacticSelectionReason::Greedy,
                    ScratchSelectionReason::Epsilon => TacticSelectionReason::Epsilon,
                },
                exploration_draw: selection.draw,
            };
            let decision_root = episode_root
                .join("native")
                .join(format!("decision-{:06}", sequence.len()));
            let outcome = execute_selected_tactic_with_checkpoint_retention_and_strategy(
                &mut worker,
                &selected,
                catalog,
                &[],
                &facts,
                &tape,
                checkpoint_source.as_ref(),
                &NativeTacticWorkerPaths {
                    request: decision_root.join("request.dsbx"),
                    result: decision_root.join("result.json"),
                },
                NativeTacticCheckpointRetention::LiveEndpoint,
                NativeGenericExecutionStrategy::NativeController,
                LIVE_CHECKPOINT_CACHE_BYTES,
            )
            .map_err(run_error)?;
            let realized_ticks = outcome.execution.duration.realized_ticks;
            native_ticks = native_ticks
                .checked_add(u64::from(realized_ticks))
                .ok_or_else(|| run_message("scratch native tick count overflowed"))?;
            let next_state =
                ScratchStateKey::from_snapshot(&outcome.next_facts).map_err(run_error)?;
            transitions.push(ScratchTransition {
                state,
                action_index: selection.action_index,
                realized_ticks,
                next_state,
                terminal: outcome.terminal,
            });
            sequence.push(selection.action_index);
            let next_checkpoint_source = if outcome.terminal {
                None
            } else {
                Some(retained_source(
                    &outcome,
                    config.optimization.route.source_boundary_index,
                )?)
            };
            tape = outcome.route_tape;
            facts = outcome.next_facts;
            checkpoint_source = next_checkpoint_source;
        }
        if transitions.is_empty() {
            return Err(run_message("scratch episode executed no action"));
        }
        Ok(EpisodeOutcome {
            transitions,
            action_sequence: sequence,
            native_ticks,
            terminal: facts.terminal.reached == Some(true),
            tape,
            native_wall_micros: elapsed_micros(episode_started.elapsed()),
        })
    })();
    let shutdown = worker.shutdown().map_err(run_error);
    let outcome = run?;
    shutdown?;
    Ok(outcome)
}

fn retained_source(
    outcome: &crate::native_tactic_worker::NativeTacticWorkerOutcome,
    source_frame: u64,
) -> Result<NativeTacticCheckpointSource, NativeScratchRunError> {
    let checkpoint = outcome
        .retained_native_checkpoint
        .as_ref()
        .ok_or_else(|| run_message("scratch live endpoint was not retained"))?;
    let boundary = outcome
        .retained_native_boundary_fingerprint
        .clone()
        .ok_or_else(|| run_message("scratch live endpoint has no boundary fingerprint"))?;
    let expected_ticks = outcome.route_tape.frames.len() as u64 - source_frame;
    if checkpoint.route_ticks != expected_ticks || checkpoint.storage_kind != "live_endpoint" {
        return Err(run_message(
            "scratch live endpoint identity is inconsistent",
        ));
    }
    Ok(NativeTacticCheckpointSource {
        restore_identity: checkpoint.restore_identity.clone(),
        boundary_fingerprint: boundary,
        route_ticks: checkpoint.route_ticks as usize,
        storage: NativeTacticCheckpointStorage::LiveEndpoint,
    })
}

fn exclude_completed_duplicate(
    prefix: &[usize],
    completed: &[Vec<usize>],
    eligible: &mut Vec<usize>,
) {
    let forbidden = completed
        .iter()
        .filter(|sequence| sequence.len() == prefix.len() + 1 && sequence.starts_with(prefix))
        .map(|sequence| sequence[prefix.len()])
        .collect::<BTreeSet<_>>();
    if forbidden.len() < eligible.len() {
        eligible.retain(|action| !forbidden.contains(action));
    }
}

fn action_prompt_available(option_id: &str, front_roll_available: bool) -> bool {
    front_roll_available
        || !(option_id.starts_with("scratch.roll.")
            || option_id.starts_with("scratch.camera_roll."))
}

fn fresh_checkpoint(
    config: &NativeScratchRunConfig<'_>,
    action_universe_sha256: Digest,
    action_count: usize,
) -> Result<NativeScratchCheckpoint, NativeScratchRunError> {
    Ok(NativeScratchCheckpoint {
        schema: NATIVE_SCRATCH_CHECKPOINT_SCHEMA_V1.into(),
        optimization_request_sha256: config.optimization.content_sha256,
        execution_binding_sha256: config.execution.content_sha256,
        objective_sha256: config.optimization.terminal_predicate.definition_sha256,
        action_universe_sha256,
        seed: config.seed,
        maximum_episode_ticks: config.maximum_episode_ticks,
        epsilon_per_million: config.epsilon_per_million,
        q: ScratchQTable::new(action_count).map_err(run_error)?,
        unique_transitions: BTreeMap::new(),
        completed_action_sequences: Vec::new(),
        report: NativeScratchReport {
            schema: NATIVE_SCRATCH_REPORT_SCHEMA_V1.into(),
            report_sha256: Digest::ZERO,
            optimization_request_sha256: config.optimization.content_sha256,
            execution_binding_sha256: config.execution.content_sha256,
            objective_sha256: config.optimization.terminal_predicate.definition_sha256,
            action_universe_sha256,
            seed: config.seed,
            maximum_episode_ticks: config.maximum_episode_ticks,
            epsilon_per_million: config.epsilon_per_million,
            completed_episodes: 0,
            distinct_episode_tapes: 0,
            duplicate_episode_tapes: 0,
            unique_transitions: 0,
            terminal_episodes: 0,
            fastest_selected_ticks: None,
            learner_updates: 0,
            changed_choices: 0,
            q_state_actions: 0,
            native_ticks: 0,
            native_wall_micros: 0,
            wall_micros: 0,
            first_terminal_wall_micros: None,
            fastest_tape: None,
            fastest_tape_sha256: None,
            strict_winners_cold_replayed: 0,
            episodes: Vec::new(),
        },
    })
}

fn validate_config(config: &NativeScratchRunConfig<'_>) -> Result<(), NativeScratchRunError> {
    if config.episodes == 0
        || config.episodes > MAX_EPISODES
        || config.maximum_episode_ticks < 900
        || config.maximum_episode_ticks > MAX_SCRATCH_EPISODE_TICKS
        || u64::from(config.maximum_episode_ticks)
            > config.optimization.budgets.exploration_horizon_ticks
        || config.epsilon_per_million > EPSILON_SCALE
        || config.cold_replay_timeout.is_zero()
    {
        return Err(run_message("native scratch configuration is invalid"));
    }
    Ok(())
}

fn validate_checkpoint(
    checkpoint: &NativeScratchCheckpoint,
    config: &NativeScratchRunConfig<'_>,
    action_universe_sha256: Digest,
) -> Result<(), NativeScratchRunError> {
    checkpoint.q.validate().map_err(run_error)?;
    if checkpoint.schema != NATIVE_SCRATCH_CHECKPOINT_SCHEMA_V1
        || checkpoint.optimization_request_sha256 != config.optimization.content_sha256
        || checkpoint.execution_binding_sha256 != config.execution.content_sha256
        || checkpoint.objective_sha256 != config.optimization.terminal_predicate.definition_sha256
        || checkpoint.action_universe_sha256 != action_universe_sha256
        || checkpoint.seed != config.seed
        || checkpoint.maximum_episode_ticks != config.maximum_episode_ticks
        || checkpoint.epsilon_per_million != config.epsilon_per_million
        || checkpoint.report.completed_episodes as usize
            != checkpoint.completed_action_sequences.len()
        || checkpoint.report.episodes.len() != checkpoint.completed_action_sequences.len()
        || checkpoint.report.unique_transitions != checkpoint.unique_transitions.len() as u64
        || checkpoint.report.report_sha256 != report_identity(&checkpoint.report)?
        || checkpoint.report.completed_episodes > config.episodes
    {
        return Err(run_message(
            "native scratch checkpoint is detached or inconsistent",
        ));
    }
    Ok(())
}

fn write_checkpoint_atomic(
    path: &Path,
    checkpoint: &NativeScratchCheckpoint,
) -> Result<(), NativeScratchRunError> {
    write_atomic(path, &encode_checkpoint(checkpoint)?)
}

fn encode_checkpoint(
    checkpoint: &NativeScratchCheckpoint,
) -> Result<Vec<u8>, NativeScratchRunError> {
    let raw = serde_cbor::to_vec(checkpoint).map_err(run_error)?;
    let compressed = zstd::stream::encode_all(Cursor::new(&raw), CHECKPOINT_COMPRESSION_LEVEL)
        .map_err(run_error)?;
    let mut bytes = Vec::with_capacity(CHECKPOINT_HEADER_BYTES + compressed.len());
    bytes.extend_from_slice(CHECKPOINT_MAGIC);
    bytes.extend_from_slice(&CHECKPOINT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&Sha256::digest(&raw));
    bytes.extend_from_slice(&compressed);
    Ok(bytes)
}

fn read_checkpoint(path: &Path) -> Result<NativeScratchCheckpoint, NativeScratchRunError> {
    let bytes = fs::read(path).map_err(run_error)?;
    decode_checkpoint(&bytes)
}

fn decode_checkpoint(bytes: &[u8]) -> Result<NativeScratchCheckpoint, NativeScratchRunError> {
    if bytes.len() <= CHECKPOINT_HEADER_BYTES
        || &bytes[..8] != CHECKPOINT_MAGIC
        || u16::from_le_bytes(bytes[8..10].try_into().unwrap()) != CHECKPOINT_VERSION
        || bytes[10..12] != [0, 0]
    {
        return Err(run_message("native scratch checkpoint header is invalid"));
    }
    let expected_len = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
    let raw = zstd::stream::decode_all(Cursor::new(&bytes[CHECKPOINT_HEADER_BYTES..]))
        .map_err(run_error)?;
    if raw.len() as u64 != expected_len || Sha256::digest(&raw)[..] != bytes[20..52] {
        return Err(run_message("native scratch checkpoint checksum is invalid"));
    }
    serde_cbor::from_slice(&raw).map_err(run_error)
}

fn write_report_atomic(
    path: &Path,
    report: &NativeScratchReport,
) -> Result<(), NativeScratchRunError> {
    let mut bytes = serde_json::to_vec_pretty(report).map_err(run_error)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), NativeScratchRunError> {
    let parent = path
        .parent()
        .ok_or_else(|| run_message("output has no parent"))?;
    fs::create_dir_all(parent).map_err(run_error)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(run_error)?;
    file.write_all(bytes).map_err(run_error)?;
    file.sync_all().map_err(run_error)?;
    drop(file);
    if path.exists() {
        fs::remove_file(path).map_err(run_error)?;
    }
    fs::rename(&temporary, path).map_err(run_error)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), NativeScratchRunError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(run_error)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(run_error)?;
    file.write_all(bytes).map_err(run_error)?;
    file.sync_all().map_err(run_error)
}

fn seal_report(report: &mut NativeScratchReport) -> Result<(), NativeScratchRunError> {
    report.report_sha256 = Digest::ZERO;
    report.report_sha256 = report_identity(report)?;
    Ok(())
}

fn report_identity(report: &NativeScratchReport) -> Result<Digest, NativeScratchRunError> {
    let mut canonical = report.clone();
    canonical.report_sha256 = Digest::ZERO;
    Ok(sha256(&serde_json::to_vec(&canonical).map_err(run_error)?))
}

fn sequence_sha256(sequence: &[usize]) -> Result<Digest, NativeScratchRunError> {
    Ok(sha256(
        &serde_cbor::to_vec(&sequence.to_vec()).map_err(run_error)?,
    ))
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn elapsed_micros(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeScratchRunError(String);

impl fmt::Display for NativeScratchRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeScratchRunError {}

fn run_message(message: impl Into<String>) -> NativeScratchRunError {
    NativeScratchRunError(message.into())
}

fn run_error(error: impl fmt::Display) -> NativeScratchRunError {
    run_message(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_episode_prefix_prevents_an_exact_duplicate() {
        let completed = vec![vec![2, 4, 6]];
        let mut eligible = vec![3, 4, 5];
        exclude_completed_duplicate(&[2], &completed, &mut eligible);
        assert_eq!(eligible, vec![3, 4, 5]);
        exclude_completed_duplicate(&[2, 4], &completed, &mut eligible);
        assert_eq!(eligible, vec![3, 4, 5]);
        let mut eligible = vec![5, 6, 7];
        exclude_completed_duplicate(&[2, 4], &completed, &mut eligible);
        assert_eq!(eligible, vec![5, 7]);
    }

    #[test]
    fn roll_actions_require_the_native_front_roll_prompt() {
        assert!(!action_prompt_available("scratch.roll.h00.r03", false));
        assert!(!action_prompt_available(
            "scratch.camera_roll.h00.t08.s0",
            false
        ));
        assert!(action_prompt_available("scratch.move.h00.t04", false));
        assert!(action_prompt_available("scratch.roll.h00.r03", true));
    }

    #[test]
    fn binary_checkpoint_round_trips_and_rejects_corruption() {
        let mut report = NativeScratchReport {
            schema: NATIVE_SCRATCH_REPORT_SCHEMA_V1.into(),
            report_sha256: Digest::ZERO,
            optimization_request_sha256: Digest([1; 32]),
            execution_binding_sha256: Digest([2; 32]),
            objective_sha256: Digest([3; 32]),
            action_universe_sha256: Digest([4; 32]),
            seed: 5,
            maximum_episode_ticks: 900,
            epsilon_per_million: 200_000,
            completed_episodes: 0,
            distinct_episode_tapes: 0,
            duplicate_episode_tapes: 0,
            unique_transitions: 0,
            terminal_episodes: 0,
            fastest_selected_ticks: None,
            learner_updates: 0,
            changed_choices: 0,
            q_state_actions: 0,
            native_ticks: 0,
            native_wall_micros: 0,
            wall_micros: 0,
            first_terminal_wall_micros: None,
            fastest_tape: None,
            fastest_tape_sha256: None,
            strict_winners_cold_replayed: 0,
            episodes: Vec::new(),
        };
        seal_report(&mut report).unwrap();
        let checkpoint = NativeScratchCheckpoint {
            schema: NATIVE_SCRATCH_CHECKPOINT_SCHEMA_V1.into(),
            optimization_request_sha256: report.optimization_request_sha256,
            execution_binding_sha256: report.execution_binding_sha256,
            objective_sha256: report.objective_sha256,
            action_universe_sha256: report.action_universe_sha256,
            seed: report.seed,
            maximum_episode_ticks: report.maximum_episode_ticks,
            epsilon_per_million: report.epsilon_per_million,
            q: ScratchQTable::new(3).unwrap(),
            unique_transitions: BTreeMap::new(),
            completed_action_sequences: Vec::new(),
            report,
        };
        let encoded = encode_checkpoint(&checkpoint).unwrap();
        assert_eq!(decode_checkpoint(&encoded).unwrap(), checkpoint);
        let mut corrupted = encoded;
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0x80;
        assert!(decode_checkpoint(&corrupted).is_err());
    }
}
