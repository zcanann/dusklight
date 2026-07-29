//! Fresh-model tactic-Q route learning on an authenticated native checkpoint.

use crate::discovery_horizon::minimum_discovery_horizon_ticks;
use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::native_suffix_result::{NativeTerminalBinding, ValidatedNativeSuffixBatch};
use crate::native_suffix_worker::{
    NativeSuffixPrevalidatedFileIdentities, NativeSuffixWorkerError, NativeSuffixWorkerLaunch,
    NativeSuffixWorkerSession,
};
use crate::native_tactic_worker::{
    NativeGenericExecutionStrategy, NativeTacticCheckpointRetention, NativeTacticCheckpointSource,
    NativeTacticCheckpointStorage, NativeTacticWorkerError, NativeTacticWorkerOutcome,
    NativeTacticWorkerPaths, PersistentTacticBatchWorker,
    execute_selected_tactic_with_checkpoint_retention_and_strategy, materialize_tactic_frontier,
    tactic_root_checkpoint_sha256,
};
use crate::optimization_request::{CampaignClass, OptimizationRequest};
use crate::tactic_macro_store::{
    TACTIC_MACRO_REGISTRY_EXTENSION, read_tactic_macro_registry, write_tactic_macro_registry,
};
use crate::tactic_q_campaign::{
    EvaluatedRewardedTacticOutcome, TACTIC_Q_CHECKPOINT_EXTENSION,
    TACTIC_Q_DEMONSTRATION_EPISODE_GROUP, TacticCampaignDiagnostics, TacticCampaignGraphProjection,
    TacticCampaignGraphProjectionEdge, TacticCampaignGraphProjectionNode,
    TacticFrontierAcquisition, TacticQCampaign, TacticQCampaignError, TacticQDecision,
    TacticQFinalResult, TacticQImmutableLearnerSnapshot, TacticQLearnerSnapshot,
    TacticQTrainingCorpus, TacticRestorationContract, has_no_progress_loop, route_checkpoint,
    validate_training_corpus,
};
use crate::tactic_q_checkpoint_store::{StoredContentRef, TacticQContentStore};
use crate::tactic_replay_control_plane::{
    TacticReplayAdmissionMetrics, TacticReplayAdmissionOutcome, TacticReplayControlPlane,
    TacticReplayControlPlaneIdentity, TacticReplaySnapshot,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::{InputFrame, InputTape, RawPadState};
use dusklight_control::option_execution::{OptionParameter, OptionType};
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use dusklight_learning::default_tactic_catalog::MAX_GOAL_SEEK_TARGETS;
use dusklight_learning::fact_registry::FactRegistry;
use dusklight_learning::fact_snapshot::FactSnapshot;
use dusklight_learning::fqi::FqiConfig;
use dusklight_learning::learner_state::LearnerState;
use dusklight_learning::option_transition::OptionTransitionSample;
use dusklight_learning::option_values::{OptionActionDescriptor, OptionValueConfig};
use dusklight_learning::parameterized_tactic_proposals::{
    ParameterizedTacticFeedback, ParameterizedTacticProposalCatalog,
    ParameterizedTacticProposalContext, parameterized_tactic_family_schema_sha256,
    propose_parameterized_tactics,
};
use dusklight_learning::reward_shaping::{
    TACTIC_REWARD_SPEC_SCHEMA_V2, TacticRewardBreakdown, TacticRewardSpec,
};
use dusklight_learning::tactic_asset::{TacticAssetCatalog, TacticAssetSource, TacticCatalogEntry};
use dusklight_learning::tactic_blueprint::TacticBlueprint;
use dusklight_learning::tactic_exploration::{
    SelectedTactic, TACTIC_EXPLORATION_SCHEMA_V1, TacticExplorationConfig, TacticProposalPolicy,
    TacticSelectionReason,
};
use dusklight_learning::tactic_features::GoalConditionedTacticFeatureEncoder;
use dusklight_learning::tactic_macro_promotion::{
    DiscoveredMacroCandidate, MAX_DISCOVERED_MACRO_TICKS, MAX_DISCOVERED_MACROS,
    MAX_DISCOVERY_OBSERVATIONS, MIN_PROMOTION_COMPARISONS, MacroComparisonEvidence,
    MacroDiscoveryObservation, MacroEntryObservation, MacroPromotionStatus, MacroSourceProvenance,
    TacticMacroEntryCondition, TacticMacroPromotionRegistry, discover_replay_macros,
    replay_macro_candidate,
};
use dusklight_learning::tactic_value_treatment::TacticValueTreatment;
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
use dusklight_world::world_surface_graph::WorldSurfaceGraph;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V32: &str = "dusklight-native-tactic-route-report/v32";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V33: &str = "dusklight-native-tactic-route-report/v33";
pub const NATIVE_TACTIC_DECISION_SUMMARY_SCHEMA_V1: &str =
    "dusklight-native-tactic-decision-summary/v1";
pub const NATIVE_TACTIC_DECISION_JOURNAL_FILE: &str = "decisions.dtqj";
pub const NATIVE_TACTIC_REPLAY_CONTROL_PLANE_FILE: &str = "campaign-replay.dtrp";
pub const NATIVE_TACTIC_CONTENT_STORE_DIRECTORY: &str = "objects";
const NATIVE_TACTIC_DEMONSTRATION_CORPUS_FILE: &str = "demonstration-training.dtqc";
const NATIVE_TACTIC_DEMONSTRATION_REPORT_FILE: &str = "demonstration-report.json";
const NATIVE_TACTIC_DEMONSTRATION_REPORT_SCHEMA_V1: &str =
    "dusklight-native-tactic-demonstration-report/v1";
const NATIVE_TACTIC_DECISION_SEGMENTS_DIRECTORY: &str = "decision-journal";
const NATIVE_TACTIC_DECISION_SEGMENT_MAGIC: &[u8; 8] = b"DSKTQJC1";
const NATIVE_TACTIC_DECISION_SEGMENT_VERSION: u16 = 1;
const NATIVE_TACTIC_DECISION_SEGMENT_HEADER_SIZE: usize = 8 + 2 + 2 + 8 + 8 + 8 + 32;
const NATIVE_TACTIC_DECISION_SEGMENT_COMPRESSION_LEVEL: i32 = 3;
const NATIVE_TACTIC_DECISION_COMPACTION_RECORDS: u64 = 256;
const NATIVE_TACTIC_DECISION_JOURNAL_MAGIC: &[u8; 8] = b"DSKTQJ01";
const NATIVE_TACTIC_DECISION_JOURNAL_VERSION: u16 = 2;
const LOAD_TRIGGER_TARGET_INTERIOR_CLEARANCE: f32 = 64.0;
const NATIVE_TACTIC_DECISION_JOURNAL_HEADER_SIZE: usize = 8 + 2 + 2;
const NATIVE_TACTIC_DECISION_RECORD_HEADER_SIZE: usize = 4 + 32;
const MAXIMUM_TACTIC_DECISION_RECORD_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_TACTIC_DECISION_SEGMENT_BYTES: usize = 256 * 1024 * 1024;
const MAXIMUM_TACTIC_DECISION_SEGMENTS: usize =
    (MAX_ROUTE_DECISIONS as usize).div_ceil(NATIVE_TACTIC_DECISION_COMPACTION_RECORDS as usize) + 1;
// A seed retains terminal routes and resumes from an authenticated root so a
// tie or slow success cannot silently cap the optimization horizon. Keep
// enough bounded restarts for discoveries near the end of a parallel dispatch
// window to feed back into later proposals.
const MAX_ROUTE_SEEDS: usize = 256;
const MAX_ROUTE_WORKERS: usize = 32;
const MAX_ROUTE_DECISIONS: u64 = 100_000;
const ROUTE_TACTIC_VALUE_DISCOUNT: f32 = 0.999;
const ROUTE_TACTIC_TICK_COST: f32 = 0.01;
const MAX_TACTIC_PROPOSALS_PER_DECISION: usize = 16;
const NAVIGABLE_SURFACE_MINIMUM_UP_NORMAL: f32 = 0.5;
const NAVIGABLE_SURFACE_MAXIMUM_ATTACHMENT_DISTANCE: f32 = 512.0;
const NAVIGABLE_SURFACE_ROUTE_TARGETS: usize = 8;
const NAVIGABLE_SURFACE_ROUTE_RESOLUTIONS: [usize; 3] = [4, 6, 8];
const NAVIGABLE_SURFACE_PORTAL_CLEARANCE: f32 = 24.0;
const MAX_RESUME_JSON_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ROUTE_ATTEMPTS: usize = 10_000;
const TACTIC_ROUTE_PERFORMANCE_SCHEMA_V2: &str = "dusklight-native-tactic-route-performance/v2";
const TACTIC_MACRO_ENTRY_GOAL_DISTANCE_PADDING: f32 = 128.0;

mod report;
use report::{
    CompletedNativeTacticSeed, NativeTacticDecisionRecord, NativeTacticDemonstration,
    NativeTacticProposalRecord, NativeTacticSeedPerformance,
};
pub use report::{
    NativeTacticDecisionTrace, NativeTacticDemonstrationReport, NativeTacticFrontierAvailability,
    NativeTacticImportedMacroReport, NativeTacticLearnerAuthorityReport,
    NativeTacticMacroDiscoveryReport, NativeTacticMacroReuseReport, NativeTacticMeasurementTrace,
    NativeTacticProposalTrace, NativeTacticReplaySharingTelemetry, NativeTacticRestoreAccounting,
    NativeTacticRestoreSource, NativeTacticRouteReport, NativeTacticRouteRunConfig,
    NativeTacticRouteTiming, NativeTacticSeedResult, NativeTacticStateTrace,
    NativeTacticValueTrace,
};

mod execution_plan;
pub use execution_plan::{
    NATIVE_TACTIC_EXECUTION_PLAN_FILE, NATIVE_TACTIC_EXECUTION_PLAN_SCHEMA_V1,
    NATIVE_TACTIC_EXECUTION_PLAN_SCHEMA_V2, NATIVE_TACTIC_EXECUTION_PLAN_SCHEMA_V3,
    NATIVE_TACTIC_EXECUTION_PLAN_SCHEMA_V4, NativeTacticAcquisitionPlan,
    NativeTacticCheckpointFallback, NativeTacticCheckpointOwnership, NativeTacticCheckpointPlan,
    NativeTacticExecutionPlan, NativeTacticExecutionPlanRequest, NativeTacticGenerationPlan,
    NativeTacticInterventionPlan, NativeTacticLanePlan, NativeTacticLaneRole,
    NativeTacticPlanBudgets, NativeTacticReplaySharingPlan, NativeTacticResourceLimit,
};

mod throughput_curve;
pub use throughput_curve::{
    NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V1, NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V2,
    NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS, NativeTacticThroughputCurveCell,
    NativeTacticThroughputCurveConfig, NativeTacticThroughputCurveReport,
    NativeTacticThroughputCurveSample, run_native_tactic_throughput_curve,
};

mod restore_locality;
pub use restore_locality::{
    NATIVE_TACTIC_RESTORE_LOCALITY_SCHEMA_V1, NativeTacticRestoreLocalityConfig,
    NativeTacticRestoreLocalityPair, NativeTacticRestoreLocalityReport,
    NativeTacticRestoreLocalitySample, NativeTacticRestoreLocalityTreatment,
    run_native_tactic_restore_locality,
};

mod learner_authority;
use learner_authority::{
    CampaignLearnerPublishResult, CampaignTacticLearnerAuthority, SharedTacticLearnerAuthority,
    lock_learner_authority,
};

fn launch_native_tactic_worker_fleet(
    config: &NativeTacticRouteRunConfig<'_>,
    fleet_root: &Path,
    worker_count: usize,
) -> Result<NativeTacticWorkerFleet, NativeTacticRouteRunError> {
    validate_config(config)?;
    let root = config.repository_root.canonicalize().map_err(route_error)?;
    config
        .execution
        .validate_files(&root, config.optimization)
        .map_err(route_error)?;
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
    NativeTacticWorkerFleet::launch(
        config,
        &root,
        fleet_root,
        &initial_batch,
        &terminal,
        &card_fixture,
        worker_count,
    )
}

pub fn run_native_tactic_route(
    config: &NativeTacticRouteRunConfig<'_>,
) -> Result<NativeTacticRouteReport, NativeTacticRouteRunError> {
    run_native_tactic_route_with_optional_fleet(config, None)
}

fn run_native_tactic_route_with_fleet(
    config: &NativeTacticRouteRunConfig<'_>,
    fleet: &NativeTacticWorkerFleet,
) -> Result<NativeTacticRouteReport, NativeTacticRouteRunError> {
    run_native_tactic_route_with_optional_fleet(config, Some(fleet))
}

fn run_native_tactic_route_with_optional_fleet(
    config: &NativeTacticRouteRunConfig<'_>,
    external_fleet: Option<&NativeTacticWorkerFleet>,
) -> Result<NativeTacticRouteReport, NativeTacticRouteRunError> {
    let campaign_started = Instant::now();
    validate_config(config)?;
    let imported_promoted_tactics = load_imported_promoted_tactics(config)?;
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
    let execution_plan_path = config.output_root.join(NATIVE_TACTIC_EXECUTION_PLAN_FILE);
    let execution_plan_sha256 = if config.resume {
        let persisted = NativeTacticExecutionPlan::read(&execution_plan_path)?;
        if persisted != *config.execution_plan {
            return Err(route_message(
                "resumed tactic route execution plan does not match the sealed plan",
            ));
        }
        persisted.identity()?
    } else {
        config.execution_plan.write(&execution_plan_path)?
    };

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
    let campaign_content_store = TacticQContentStore::initialize(
        config
            .output_root
            .join(NATIVE_TACTIC_CONTENT_STORE_DIRECTORY),
    )
    .map_err(route_error)?;
    let root_tape_ref = campaign_content_store
        .store_tape(&route_prefix)
        .map_err(route_error)?;

    let worker_count = config.workers;
    let owned_fleet = external_fleet
        .is_none()
        .then(|| launch_native_tactic_worker_fleet(config, config.output_root, worker_count))
        .transpose()?;
    let fleet = external_fleet
        .or(owned_fleet.as_ref())
        .ok_or_else(|| route_message("native tactic worker fleet is absent"))?;
    fleet.validate_for(config)?;
    let process_launch_micros = if external_fleet.is_some() {
        0
    } else {
        fleet.launch_micros()
    };
    let initial_facts = fleet.initial_facts().clone();
    let GoalConditionedTacticContext {
        encoder,
        report: goal_target,
    } = atomic_goal_conditioned_tactic_context(
        &root,
        config.optimization,
        config.execution,
        &initial_facts,
    )?;
    let action_schema_sha256 = parameterized_policy_action_schema_sha256(
        imported_promoted_tactics
            .as_ref()
            .map(|imported| imported.report.registry_sha256),
    );
    let root_checkpoint_sha256 = fleet.root_checkpoint_sha256();
    let reward_spec = route_tactic_reward_spec();
    let root_source_frame = usize::try_from(initial_facts.tape_frame)
        .map_err(|_| route_message("native tactic source frame exceeds platform limits"))?;
    let replay_control_plane_path = config
        .output_root
        .join(NATIVE_TACTIC_REPLAY_CONTROL_PLANE_FILE);
    let replay_control_plane_identity = TacticReplayControlPlaneIdentity::new(
        execution_plan_sha256,
        encoder.schema_sha256,
        config.optimization.terminal_predicate.definition_sha256,
        root_checkpoint_sha256,
    )
    .map_err(route_error)?;
    let replay_content_root = config
        .output_root
        .join(NATIVE_TACTIC_CONTENT_STORE_DIRECTORY);
    let replay_control_plane = if replay_control_plane_path.exists() {
        TacticReplayControlPlane::open(
            &replay_control_plane_path,
            &replay_content_root,
            &replay_control_plane_identity,
        )
        .map_err(route_error)?
    } else {
        TacticReplayControlPlane::create(
            &replay_control_plane_path,
            &replay_content_root,
            replay_control_plane_identity,
        )
        .map_err(route_error)?
    };
    let learner_authority: SharedTacticLearnerAuthority =
        Arc::new(Mutex::new(CampaignTacticLearnerAuthority::new(
            replay_control_plane,
            route_option_value_config(execution_plan_sha256),
            encoder.goal_distance_feature(),
            config.execution_plan.value_treatment,
            config.execution_plan.refit_every_decisions,
        )?));

    let pool = fleet.pool(config, execution_plan_sha256, root_source_frame)?;
    let (mut indexed_results, tactic_macro_discovery, shared_training_replay_rows, demonstration) =
        (|| {
            let mut results = Vec::with_capacity(config.execution_plan.lanes.len());
            let demonstration = load_or_capture_demonstration(
                config,
                &pool,
                &encoder,
                &reward_spec,
                &process_tape,
                &initial_facts,
                &route_prefix,
                root_checkpoint_sha256,
            )?;
            let demonstration_report = demonstration.as_ref().map(|value| value.report.clone());
            if let Some(demonstration) = &demonstration {
                let mut learner = lock_learner_authority(&learner_authority)?;
                publish_demonstration_replay(&mut learner, demonstration)?;
            }
            for generation in &config.execution_plan.generations {
                let inherited_learner_snapshot = {
                    let mut learner = lock_learner_authority(&learner_authority)?;
                    match config.execution_plan.replay_sharing {
                        NativeTacticReplaySharingPlan::GenerationBarrier => {
                            let barrier_revision = deterministic_generation_barrier_revision(
                                learner.replay(),
                                generation,
                            )?;
                            learner.snapshot_through(barrier_revision)?
                        }
                        NativeTacticReplaySharingPlan::BoundedStaleness { .. } => {
                            learner.snapshot()
                        }
                    }
                };
                let live_learner = matches!(
                    config.execution_plan.replay_sharing,
                    NativeTacticReplaySharingPlan::BoundedStaleness { .. }
                )
                .then(|| Arc::clone(&learner_authority));
                let mut generation_results = std::thread::scope(|generation_scope| {
                    let coordinator_handles = generation
                        .lane_indices
                        .iter()
                        .map(|lane_index| {
                            let lane = &config.execution_plan.lanes[*lane_index];
                            let seed_index = lane.lane_index;
                            let seed = lane.seed;
                            let pool = pool.clone();
                            let registry = &registry;
                            let encoder = &encoder;
                            let reward_spec = &reward_spec;
                            let promoted_tactics = imported_promoted_tactics
                                .as_ref()
                                .map_or(&[][..], |imported| imported.entries.as_slice());
                            let initial_facts = &initial_facts;
                            let route_prefix = &route_prefix;
                            let live_learner = live_learner.clone();
                            let inherited_learner_snapshot =
                                Arc::clone(&inherited_learner_snapshot);
                            generation_scope.spawn(move || {
                                run_seed_coordinator(
                                    config,
                                    &pool,
                                    registry,
                                    encoder,
                                    reward_spec,
                                    initial_facts,
                                    route_prefix,
                                    action_schema_sha256,
                                    promoted_tactics,
                                    root_checkpoint_sha256,
                                    root_tape_ref,
                                    inherited_learner_snapshot,
                                    live_learner,
                                    seed_index,
                                    seed,
                                )
                                .map(|completion| (seed_index, completion))
                            })
                        })
                        .collect::<Vec<_>>();
                    coordinator_handles
                        .into_iter()
                        .map(|handle| {
                            handle.join().map_err(|_| {
                                route_message("native tactic route coordinator panicked")
                            })?
                        })
                        .collect::<Result<Vec<_>, NativeTacticRouteRunError>>()
                })?;
                generation_results.sort_by_key(|(seed_index, _)| *seed_index);
                // Live lanes already publish on each decision. Replaying
                // the completed corpus here is an idempotent resume repair:
                // if an older or interrupted campaign lost its shared
                // journal, completed lane artifacts reconstruct authority.
                let mut learner = lock_learner_authority(&learner_authority)?;
                for (_, completion) in &generation_results {
                    publish_completed_seed_replay(&mut learner, completion)?;
                }
                learner.force_update()?;
                results.extend(
                    generation_results
                        .into_iter()
                        .map(|(seed_index, completion)| (seed_index, completion.result)),
                );
            }
            let completion = (|| {
                let mined = mine_and_store_tactic_macros(
                    config.output_root,
                    &config.execution_plan.seeds,
                    &encoder,
                )?;
                validate_and_store_tactic_macros(
                    config,
                    &pool,
                    &encoder,
                    root_checkpoint_sha256,
                    mined,
                )
            })()?;
            let shared_training_replay_rows =
                lock_learner_authority(&learner_authority)?.replay().len() as u64;
            Ok::<_, NativeTacticRouteRunError>((
                results,
                completion,
                shared_training_replay_rows,
                demonstration_report,
            ))
        })()?;
    drop(pool);
    indexed_results.sort_by_key(|(seed_index, _)| *seed_index);
    if indexed_results.len() != config.execution_plan.seeds.len()
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
    let unique_useful_graph_expansions = seed_results
        .iter()
        .map(|seed| seed.unique_useful_graph_expansions)
        .sum();
    let mut native_restore_accounting = NativeTacticRestoreAccounting::default();
    for seed in &seed_results {
        native_restore_accounting.merge(&seed.native_restore_accounting);
    }
    if let Some(demonstration) = &demonstration {
        native_restore_accounting.merge(&demonstration.restore_accounting);
    }
    native_restore_accounting.merge(&tactic_macro_discovery.validation_restore_accounting);
    let mut time_to_first_terminal_micros = seed_results
        .iter()
        .filter_map(|seed| seed.time_to_first_terminal_micros)
        .map(|seed_wall| seed_wall.saturating_add(process_launch_micros))
        .collect::<Vec<_>>();
    time_to_first_terminal_micros.sort_unstable();
    let median_time_to_first_terminal_micros =
        median_sorted_wall_micros(&time_to_first_terminal_micros);
    let worst_time_to_first_terminal_micros = time_to_first_terminal_micros.last().copied();
    let mut timing = aggregate_route_timing(&seed_results);
    timing.process_launch_micros = process_launch_micros;
    if let Some(demonstration) = &demonstration {
        timing.tactic_execution_micros = timing
            .tactic_execution_micros
            .saturating_add(demonstration.wall_micros);
        timing.native_simulation_micros = timing
            .native_simulation_micros
            .saturating_add(demonstration.native_simulation_micros);
        timing.ipc_and_result_transport_micros = timing
            .ipc_and_result_transport_micros
            .saturating_add(demonstration.ipc_and_result_transport_micros);
        timing.native_observation_capture_micros = timing
            .native_observation_capture_micros
            .saturating_add(demonstration.native_observation_capture_micros);
        timing.native_corpus_encoding_micros = timing
            .native_corpus_encoding_micros
            .saturating_add(demonstration.native_corpus_encoding_micros);
        timing.rust_state_extraction_micros = timing
            .rust_state_extraction_micros
            .saturating_add(demonstration.rust_state_extraction_micros);
        timing.tactic_preparation_and_fact_extraction_micros = timing
            .tactic_preparation_and_fact_extraction_micros
            .saturating_add(demonstration.preparation_micros);
    }
    timing.tactic_execution_micros = timing
        .tactic_execution_micros
        .saturating_add(tactic_macro_discovery.validation_wall_micros);
    timing.native_simulation_micros = timing
        .native_simulation_micros
        .saturating_add(tactic_macro_discovery.validation_native_simulation_micros);
    timing.ipc_and_result_transport_micros = timing
        .ipc_and_result_transport_micros
        .saturating_add(tactic_macro_discovery.validation_ipc_and_result_transport_micros);
    timing.native_observation_capture_micros = timing
        .native_observation_capture_micros
        .saturating_add(tactic_macro_discovery.validation_native_observation_capture_micros);
    timing.native_corpus_encoding_micros = timing
        .native_corpus_encoding_micros
        .saturating_add(tactic_macro_discovery.validation_native_corpus_encoding_micros);
    timing.rust_state_extraction_micros = timing
        .rust_state_extraction_micros
        .saturating_add(tactic_macro_discovery.validation_rust_state_extraction_micros);
    timing.tactic_preparation_and_fact_extraction_micros = timing
        .tactic_preparation_and_fact_extraction_micros
        .saturating_add(tactic_macro_discovery.validation_preparation_micros);
    timing.wall_micros = elapsed_micros(campaign_started.elapsed());
    refresh_route_throughput(&mut timing, &seed_results);
    let reporting_started = Instant::now();
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
    let learner_authority = lock_learner_authority(&learner_authority)?;
    let final_replay = learner_authority.replay().snapshot().map_err(route_error)?;
    let final_replay_snapshot = final_replay.version;
    let replay_admission = learner_authority.replay().invocation_metrics();
    let learner_metrics = learner_authority.invocation_metrics();
    let learner_updates = learner_metrics.updates;
    timing.model_update_micros = learner_metrics.update_micros;
    let latest_learner_snapshot = learner_authority.snapshot();
    let declared_model_snapshots_consumed = seed_results
        .iter()
        .flat_map(|seed| &seed.trace)
        .map(|decision| decision.learner_snapshot_sha256)
        .filter(|sha256| *sha256 != Digest::ZERO)
        .collect::<BTreeSet<_>>()
        .len() as u64;
    let lane_local_model_updates = seed_results.iter().map(|seed| seed.learner_updates).sum();
    let learner_authority_report = NativeTacticLearnerAuthorityReport {
        model_snapshots_published: learner_metrics.snapshots_published,
        latest_model_snapshot_sha256: latest_learner_snapshot.sha256,
        latest_model_revision: latest_learner_snapshot.manifest.model_revision,
        latest_training_replay_rows: latest_learner_snapshot.manifest.training_replay_rows,
        declared_model_snapshots_consumed,
        lane_local_model_updates,
    };
    let replay_sharing = seed_results.iter().fold(
        NativeTacticReplaySharingTelemetry::default(),
        |mut total, seed| {
            total.merge(seed.replay_sharing);
            total
        },
    );
    let useful_training_transitions =
        useful_training_transitions(&final_replay.corpus, encoder.goal_distance_feature());
    let censored_training_transitions = censored_training_transitions(&final_replay.corpus);
    let mut report = NativeTacticRouteReport {
        schema: NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V33.into(),
        optimization_request_sha256: config.optimization.content_sha256,
        execution_binding_sha256: config.execution.content_sha256,
        execution_plan_sha256,
        execution_plan_path: path_text(&execution_plan_path),
        replay_control_plane_path: path_text(&replay_control_plane_path),
        replay_revision: final_replay_snapshot.revision,
        replay_snapshot_sha256: final_replay_snapshot.sha256,
        replay_admission,
        objective_sha256: config.optimization.terminal_predicate.definition_sha256,
        feature_schema_sha256: encoder.schema_sha256,
        action_schema_sha256,
        imported_promoted_tactics: imported_promoted_tactics
            .as_ref()
            .map(|imported| imported.report.clone()),
        goal_target,
        reward_spec,
        demonstration_transitions: demonstration
            .as_ref()
            .map_or(0, |report| report.transition_count),
        demonstration: demonstration.clone(),
        exploration_seeds: config.execution_plan.seeds.to_vec(),
        proposal_policy: config.execution_plan.proposal_policy,
        value_treatment: config.execution_plan.value_treatment,
        execution_strategy: config.execution_plan.execution_strategy,
        workers: worker_count,
        decisions_per_seed: config.execution_plan.budgets.decisions_per_lane,
        resource_budgets: config.execution_plan.budgets,
        refit_every_decisions: config.execution_plan.refit_every_decisions,
        terminal_seeds: seed_results
            .iter()
            .filter(|seed| seed.terminal_discovered)
            .count() as u64,
        best_authenticated_tick: seed_results
            .iter()
            .filter_map(|seed| seed.best_authenticated_tick)
            .min(),
        promotion_successful_seeds: seed_results.iter().filter(|seed| seed.success).count() as u64,
        successful_seeds: seed_results.iter().filter(|seed| seed.success).count() as u64,
        median_time_to_first_terminal_micros,
        worst_time_to_first_terminal_micros,
        total_native_ticks: seed_results
            .iter()
            .map(|seed| seed.native_ticks)
            .sum::<u64>()
            .saturating_add(
                demonstration
                    .as_ref()
                    .map_or(0, |report| report.native_ticks),
            )
            .saturating_add(tactic_macro_discovery.validation_native_ticks),
        total_decisions: seed_results.iter().map(|seed| seed.decisions).sum(),
        useful_decisions,
        unique_useful_graph_expansions,
        learner_authority: learner_authority_report,
        learner_updates,
        learner_updates_per_second_millionths: per_second_millionths(
            learner_updates,
            timing.wall_micros,
        ),
        useful_training_transitions,
        useful_transitions_per_learner_update_millionths: ratio_per_million(
            useful_training_transitions,
            learner_updates,
        ),
        learned_episodes_per_generation: config
            .execution_plan
            .generations
            .iter()
            .map(|generation| generation.lane_indices.len())
            .max()
            .unwrap_or(0),
        training_replay_rows: seed_results
            .iter()
            .map(|seed| {
                seed.training_replay_rows
                    .saturating_sub(seed.imported_training_replay_rows) as u64
            })
            .sum(),
        shared_training_replay_rows,
        duplicate_training_transitions: seed_results
            .iter()
            .map(|seed| seed.duplicate_training_transitions)
            .sum(),
        censored_training_transitions,
        replay_sharing,
        frontier_availability,
        native_restore_accounting,
        tactic_macro_discovery,
        timing,
        seeds: seed_results,
    };
    serde_json::to_writer(std::io::sink(), &report).map_err(route_error)?;
    report.timing.reporting_micros = elapsed_micros(reporting_started.elapsed());
    write_new(
        &config.output_root.join("report.json"),
        &serde_json::to_vec_pretty(&report).map_err(route_error)?,
    )?;
    if let Some(fleet) = owned_fleet {
        fleet.shutdown()?;
    }
    Ok(report)
}

fn median_sorted_wall_micros(sorted: &[u64]) -> Option<u64> {
    let midpoint = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted.get(midpoint).copied()
    } else {
        let upper = sorted.get(midpoint).copied()?;
        let lower = sorted.get(midpoint.checked_sub(1)?).copied()?;
        Some(lower / 2 + upper / 2 + (lower % 2 + upper % 2) / 2)
    }
}

mod macro_discovery;
use macro_discovery::{mine_and_store_tactic_macros, validate_and_store_tactic_macros};
mod macro_import;
pub use macro_import::tactic_macro_registry_identity;
use macro_import::{ImportedPromotedTactic, load_imported_promoted_tactics};

mod replay_sharing;
use replay_sharing::{
    BoundedStalenessReplaySession, build_replay_session, deterministic_generation_barrier_revision,
    lane_generated_training_corpus, publish_completed_seed_replay, publish_demonstration_replay,
};

mod worker_pool;
use worker_pool::{
    CachedTacticFrontier, NativeTacticProposalPool, applicable_parameterized_descriptors_for_state,
    load_or_capture_demonstration, parameterized_catalog_for_state_with_promoted,
    parameterized_feedback_for_state, run_seed_coordinator,
};
mod worker_fleet;
use worker_fleet::NativeTacticWorkerFleet;
mod campaign;
use campaign::{NATIVE_TACTIC_RESULT_ADMISSION_SCHEMA_V1, run_seed};
mod timing_metrics;
use timing_metrics::{
    aggregate_route_timing, censored_training_transitions, decision_evaluated_ticks,
    decision_trace_is_useful, elapsed_micros, per_second_millionths, ratio_per_million,
    refresh_route_throughput, useful_training_transitions,
};
mod candidate_retention;
use candidate_retention::{
    authenticated_first_hit_tick, load_best_retained_success, retain_successful_result,
};
mod campaign_persistence;
use campaign_persistence::{
    cancellation_requested, load_seed_performance, pause_tactic_campaign, persist_seed_performance,
    read_completed_seed_result, resume_seed, seed_performance_exists,
};
mod journal;
use journal::{
    append_tactic_decision_record, compact_tactic_decision_journal, decision_record,
    journal_transition, journal_transition_sha256, load_tactic_journal_replay,
    read_tactic_decision_records,
};
pub use journal::{
    has_tactic_decision_journal, materialize_tactic_decision_route,
    project_tactic_decision_diagnostics, project_tactic_decision_graph,
    read_tactic_decision_journal, tactic_content_store_path, tactic_decision_journal_path,
};

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
    // The root observation is the first pre-input row. Running the entire
    // exploration horizon here produces no additional authority: the
    // persistent worker has already captured the authenticated source
    // checkpoint before it evaluates this candidate, and subsequent batches
    // declare their own bounded horizons.
    tactic_root_probe_batch_with_ticks(config.optimization, config.execution, 1)
}

mod goal_target;
pub use goal_target::NativeTacticGoalTargetReport;
pub(crate) use goal_target::{GoalConditionedTacticContext, goal_conditioned_tactic_runtime};
use goal_target::{
    atomic_goal_conditioned_tactic_context, parameterized_policy_action_schema_sha256,
    planar_distance,
};

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
    goal_tactic_maximum_ticks(horizon)
}

fn route_tactic_reward_spec() -> TacticRewardSpec {
    route_tactic_base_reward_spec()
}

fn route_tactic_base_reward_spec() -> TacticRewardSpec {
    TacticRewardSpec {
        schema: TACTIC_REWARD_SPEC_SCHEMA_V2.into(),
        terminal_reward: 100.0,
        // Terminal evidence remains overwhelmingly dominant, while every
        // simulated controller tick has a small explicit cost. This makes the
        // learned value function prefer a shorter terminal route without
        // making necessary collision-avoidance detours worse than failure.
        tick_cost: ROUTE_TACTIC_TICK_COST,
        novelty_reward: 0.0,
        per_tick_discount: 1.0,
        potential: None,
        motion_cost: None,
    }
}

fn route_option_value_config(execution_authority_sha256: Digest) -> OptionValueConfig {
    let learner_seed = u64::from_le_bytes(
        execution_authority_sha256.0[..8]
            .try_into()
            .expect("fixed slice"),
    );
    OptionValueConfig {
        fitted_q: FqiConfig {
            iterations: 12,
            trees_per_action: 15,
            max_tree_depth: 8,
            // Keep a mild contraction so zero-reward waypoint holds lose
            // value, without erasing a terminal reached late in the declared
            // discovery horizon.
            discount: ROUTE_TACTIC_VALUE_DISCOUNT,
            seed: 0xd15c_a11d_5eed_f017 ^ learner_seed,
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
    tactic_root_probe_batch_with_ticks(optimization, execution, maximum_ticks)
}

fn tactic_root_probe_batch_with_ticks(
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    maximum_ticks: usize,
) -> Result<NativeSuffixBatch, NativeTacticRouteRunError> {
    if maximum_ticks == 0
        || maximum_ticks
            > usize::try_from(optimization.budgets.exploration_horizon_ticks)
                .map_err(route_error)?
    {
        return Err(route_message(
            "tactic root probe exceeds the exploration horizon",
        ));
    }
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
    config.execution_plan.validate()?;
    validate_unassisted_discovery_horizon(config)?;
    let maximum_demonstration_chunk_ticks =
        goal_tactic_maximum_ticks(config.optimization.budgets.exploration_horizon_ticks)?;
    if config.workers == 0
        || config.workers > MAX_ROUTE_WORKERS
        || config.execution_plan.budgets.decisions_per_lane > MAX_ROUTE_DECISIONS
        || config
            .execution_plan
            .demonstration_chunk_ticks
            .is_some_and(|ticks| {
                ticks == 0
                    || ticks > maximum_demonstration_chunk_ticks
                    || config.execution_plan.proposal_policy != TacticProposalPolicy::Learned
            })
        || !planned_decisions_fit_candidate_budget(
            config.execution_plan.budgets.decisions_per_lane,
            config.execution_plan.seeds.len(),
            config.optimization.budgets.candidate_budget,
        )
        || config
            .execution_plan
            .promoted_tactic_registry_sha256
            .is_some()
            != config.promoted_tactic_registry.is_some()
    {
        return Err(route_message(
            "native tactic route configuration is invalid",
        ));
    }
    Ok(())
}

fn planned_decisions_fit_candidate_budget(
    decisions_per_lane: u64,
    lane_count: usize,
    candidate_budget: u64,
) -> bool {
    u64::try_from(lane_count)
        .ok()
        .and_then(|lanes| decisions_per_lane.checked_mul(lanes))
        .is_some_and(|total| total <= candidate_budget)
}

fn validate_unassisted_discovery_horizon(
    config: &NativeTacticRouteRunConfig<'_>,
) -> Result<(), NativeTacticRouteRunError> {
    let plan = config.execution_plan;
    let required = unassisted_discovery_horizon_requirement(
        config.optimization.campaign_class,
        plan.proposal_policy,
        plan.demonstration_chunk_ticks.is_some(),
        plan.promoted_tactic_registry_sha256.is_some(),
        config.optimization.budgets.promotion_before_tick,
    )
    .map_err(route_message)?;
    let Some(minimum) = required else {
        return Ok(());
    };
    if config.optimization.budgets.exploration_horizon_ticks < minimum {
        return Err(route_message(format!(
            "unassisted learned tactic routing requires at least {minimum} discovery ticks; \
             promotion and terminal discovery horizons are separate authority"
        )));
    }
    Ok(())
}

fn unassisted_discovery_horizon_requirement(
    campaign_class: CampaignClass,
    proposal_policy: TacticProposalPolicy,
    has_demonstration: bool,
    has_promoted_tactics: bool,
    promotion_before_tick: u64,
) -> Result<Option<u64>, &'static str> {
    let unassisted_learning = proposal_policy == TacticProposalPolicy::Learned
        && !has_demonstration
        && !has_promoted_tactics;
    if !unassisted_learning {
        return Ok(None);
    }
    if campaign_class != CampaignClass::FromScratchDiscovery {
        return Err("unassisted learned tactic routing requires a from_scratch_discovery request");
    }
    minimum_discovery_horizon_ticks(promotion_before_tick)
        .map(Some)
        .ok_or("minimum discovery horizon overflowed")
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
#[path = "native_tactic_route_runner/tests.rs"]
mod tests;
