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
    NativeTacticWorkerPaths, PersistentTacticBatchWorker, TACTIC_CHECKPOINT_CACHE_BYTES,
    TACTIC_INTERMEDIATE_BOUNDARY_STRIDE,
    execute_selected_tactic_with_checkpoint_retention_and_strategy,
    materialize_tactic_frontier_with_cache_capacity, tactic_checkpoint_cache_request,
    tactic_root_checkpoint_sha256,
};
use crate::optimization_request::{CampaignClass, OptimizationRequest};
use crate::reporting::GraphSearchReport;
use crate::tactic_macro_store::{
    TACTIC_MACRO_REGISTRY_EXTENSION, read_tactic_macro_registry, write_tactic_macro_registry,
};
use crate::tactic_q_campaign::{
    EvaluatedRewardedTacticOutcome, TACTIC_Q_CHECKPOINT_EXTENSION,
    TACTIC_Q_DEMONSTRATION_EPISODE_GROUP, TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V3,
    TacticCampaignDiagnostics, TacticCampaignGraphProjection, TacticCampaignGraphProjectionEdge,
    TacticCampaignGraphProjectionNode, TacticExpansionLease, TacticFrontierAcquisition,
    TacticGraphSchedulingTiming, TacticQCampaign, TacticQCampaignError, TacticQDecision,
    TacticQFinalResult, TacticQImmutableLearnerSnapshot, TacticQLearnerSnapshot,
    TacticQLearnerSnapshotKind, TacticQProposalBatch, TacticQTrainingCorpus,
    TacticRestorationContract, TacticSchedulerDecisionTrace, has_no_progress_loop,
    route_checkpoint, validate_training_corpus,
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
use dusklight_learning::goal_reachability_calibration::GoalReachabilityCalibration;
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
    MAX_DISCOVERY_OBSERVATIONS, MIN_DISCOVERY_OCCURRENCES, MIN_PROMOTION_COMPARISONS,
    MacroComparisonEvidence, MacroDiscoveryObservation, MacroEntryObservation,
    MacroPromotionStatus, MacroSourceProvenance, TacticMacroComponent, TacticMacroEntryCondition,
    TacticMacroPromotionRegistry, discover_replay_macros, replay_macro_candidate,
};
use dusklight_learning::tactic_value_treatment::TacticValueTreatment;
use dusklight_objectives::milestone_dsl::{Comparison, Expression, Field, Value};
use dusklight_proposals::behavior_archive::BehaviorArchive;
use dusklight_search::search::{MacroAction, SearchPadState};
use dusklight_search::suffix_batch::{
    NATIVE_CACHED_SUFFIX_BATCH_SCHEMA, NATIVE_SUFFIX_BATCH_SCHEMA, NativeCheckpointValidation,
    NativeSuffixBatch, NativeSuffixCandidate,
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
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V34: &str = "dusklight-native-tactic-route-report/v34";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V35: &str = "dusklight-native-tactic-route-report/v35";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V36: &str = "dusklight-native-tactic-route-report/v36";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V37: &str = "dusklight-native-tactic-route-report/v37";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V38: &str = "dusklight-native-tactic-route-report/v38";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V39: &str = "dusklight-native-tactic-route-report/v39";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V40: &str = "dusklight-native-tactic-route-report/v40";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V41: &str = "dusklight-native-tactic-route-report/v41";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V42: &str = "dusklight-native-tactic-route-report/v42";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V43: &str = "dusklight-native-tactic-route-report/v43";
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V44: &str = "dusklight-native-tactic-route-report/v44";
pub const NATIVE_TACTIC_DECISION_SUMMARY_SCHEMA_V1: &str =
    "dusklight-native-tactic-decision-summary/v1";
pub const NATIVE_TACTIC_DECISION_JOURNAL_FILE: &str = "decisions.dtqj";
const NATIVE_TACTIC_LEASE_JOURNAL_FILE: &str = "leases.dtql";
pub const NATIVE_TACTIC_REPLAY_CONTROL_PLANE_FILE: &str = "campaign-replay.dtrp";
pub const NATIVE_TACTIC_CONTENT_STORE_DIRECTORY: &str = "objects";
pub const NATIVE_TACTIC_WORKER_HELLO_FILE: &str = "worker-hello.json";
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
    NativeTacticCampaignAdmissionTiming, NativeTacticDecisionTrace,
    NativeTacticDemonstrationReport, NativeTacticFrontierAvailability, NativeTacticGraphMetrics,
    NativeTacticImportedMacroReport, NativeTacticLearnerAuthorityReport,
    NativeTacticMacroDiscoveryReport, NativeTacticMacroReuseReport, NativeTacticMeasurementTrace,
    NativeTacticOrchestrationTiming, NativeTacticPersistenceTiming, NativeTacticProposalTrace,
    NativeTacticReplaySharingTelemetry, NativeTacticRestoreAccounting, NativeTacticRestoreSource,
    NativeTacticRouteReport, NativeTacticRouteRunConfig, NativeTacticRouteTiming,
    NativeTacticSeedResult, NativeTacticSeedStopReason, NativeTacticStateTrace,
    NativeTacticValueTrace, NativeTacticWorkerUtilization,
};
mod causal_policy_probe;
pub use causal_policy_probe::NativeTacticPolicyUpdateProbe;
#[cfg(test)]
use causal_policy_probe::build_policy_update_probe;
use causal_policy_probe::{PolicyUpdateProbeContext, consume_policy_update_with_probe};
mod completion_marker;
use completion_marker::publish_completion;
mod exclusive_timing;
pub use completion_marker::{
    NATIVE_TACTIC_CAMPAIGN_COMPLETION_FILE, NATIVE_TACTIC_CAMPAIGN_COMPLETION_SCHEMA_V1,
    NativeTacticCampaignCompletion,
};
use exclusive_timing::{
    CampaignExclusiveTimingInput, CampaignPhaseWallTiming, ExclusiveTopTimingSnapshot,
    SeedOrchestrationPhase as OrchestrationPhase, attribute_campaign_timing,
    orchestration_detail_total, reconcile_recovered_seed_timing, record_orchestration_detail,
    record_orchestration_total,
};
mod campaign_summary;
pub use campaign_summary::{
    NATIVE_TACTIC_CAMPAIGN_SUMMARY_FILE, NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V1,
    NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V2, NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V3,
    NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V4, NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V5,
    NativeTacticCampaignCausalSummary, NativeTacticCampaignEfficiencySummary,
    NativeTacticCampaignGoalReachabilitySummary, NativeTacticCampaignIdentities,
    NativeTacticCampaignOutcomeSummary, NativeTacticCampaignResourceSummary,
    NativeTacticCampaignSummary, NativeTacticCampaignTimingSummary,
    NativeTacticCampaignTreatmentSummary, NativeTacticCampaignWorkSummary, NativeTacticCausalLink,
};
mod lease_journal;
use lease_journal::NativeTacticLeaseLedger;
pub use lease_journal::{NativeTacticLeaseAccounting, NativeTacticLeaseOutcome};
mod recovery;
use recovery::{
    has_tactic_recovery_point, load_tactic_recovery_point, persist_tactic_recovery_point,
    prune_tactic_native_attempts, prune_tactic_partial_finalization, prune_tactic_recovery_points,
};
mod fault_injection;
use fault_injection::inject_tactic_fault;
pub use fault_injection::{
    NATIVE_TACTIC_FAULT_EXIT_CODE, NATIVE_TACTIC_FAULT_INJECTION_FILE,
    NATIVE_TACTIC_FAULT_INJECTION_SCHEMA_V1, NativeTacticFaultInjectionMarker,
    NativeTacticFaultInjector, NativeTacticFaultPoint,
};
mod fault_recovery_audit;
pub use fault_recovery_audit::{
    NATIVE_TACTIC_FAULT_RECOVERY_AUDIT_SCHEMA_V1, NATIVE_TACTIC_FAULT_RECOVERY_AUDIT_SCHEMA_V2,
    NativeTacticFaultRecoveryAudit, NativeTacticFaultRecoverySeedAudit,
    audit_native_tactic_fault_recovery,
};
mod fault_recovery_bundle;
pub use fault_recovery_bundle::{
    NATIVE_TACTIC_FAULT_RECOVERY_EVIDENCE_BUNDLE_SCHEMA_V1,
    NATIVE_TACTIC_FAULT_RECOVERY_EVIDENCE_MANIFEST, NativeTacticFaultRecoveryEvidenceBundle,
};
mod launch_smoke;
pub use launch_smoke::{
    NATIVE_TACTIC_LAUNCH_SMOKE_BUNDLE_SCHEMA_V1, NATIVE_TACTIC_LAUNCH_SMOKE_MANIFEST,
    NativeTacticLaunchSmokeBundle, NativeTacticLaunchSmokeSummary,
};
mod cold_replay;
pub use cold_replay::{
    NATIVE_TACTIC_COLD_REPLAY_PROOF_FILE, NATIVE_TACTIC_COLD_REPLAY_PROOF_SCHEMA_V1,
    NativeTacticColdReplayArtifact, NativeTacticColdReplayAttempt, NativeTacticColdReplayConfig,
    NativeTacticColdReplayFidelity, NativeTacticColdReplayProof,
    read_and_validate_native_tactic_cold_replay, run_native_tactic_cold_replay,
};
mod cold_replay_bundle;
pub use cold_replay_bundle::{
    NATIVE_TACTIC_COLD_REPLAY_EVIDENCE_BUNDLE_SCHEMA_V1,
    NATIVE_TACTIC_COLD_REPLAY_EVIDENCE_BUNDLE_SCHEMA_V2,
    NATIVE_TACTIC_COLD_REPLAY_EVIDENCE_MANIFEST, NativeTacticColdReplayAuthority,
    NativeTacticColdReplayEvidenceBundle,
};
mod optimization_handoff;
pub use optimization_handoff::{
    NATIVE_TACTIC_OPTIMIZATION_HANDOFF_MANIFEST, NATIVE_TACTIC_OPTIMIZATION_HANDOFF_SCHEMA_V1,
    NativeTacticOptimizationHandoff, NativeTacticOptimizationHandoffConfig,
    build_native_tactic_optimization_handoff,
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

mod scratch_discovery;
pub use scratch_discovery::{
    NATIVE_TACTIC_SCRATCH_DISCOVERY_SCHEMA_V1, NativeTacticScratchCondition,
    NativeTacticScratchDiscoveryReport, NativeTacticScratchTotals,
    ORDON_MEDIAN_TERMINAL_WALL_LIMIT_MICROS, ORDON_SCRATCH_DISCOVERY_GOAL,
    ORDON_SCRATCH_DISCOVERY_SEEDS, ORDON_WORST_TERMINAL_WALL_LIMIT_MICROS,
};
mod scratch_evidence_bundle;
pub use scratch_evidence_bundle::{
    NATIVE_TACTIC_SCRATCH_EVIDENCE_BUNDLE_SCHEMA_V2, NATIVE_TACTIC_SCRATCH_EVIDENCE_MANIFEST,
    NativeTacticScratchAuthorityArtifact, NativeTacticScratchBundleArtifact,
    NativeTacticScratchEvidenceBundle, NativeTacticScratchExecutionIdentity,
    NativeTacticScratchSeedEvidence,
};
mod terminal_evidence_bundle;
pub use terminal_evidence_bundle::{
    NATIVE_TACTIC_TERMINAL_EVIDENCE_BUNDLE_SCHEMA_V1, NATIVE_TACTIC_TERMINAL_EVIDENCE_MANIFEST,
    NativeTacticTerminalEvidenceBundle,
};
mod scratch_campaign_audit;
pub use scratch_campaign_audit::{
    NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V2, NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V3,
    NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V4, NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V5,
    NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V6, NativeTacticCampaignResourceAudit,
    NativeTacticScratchCampaignAudit, NativeTacticScratchDecisionAudit,
    NativeTacticScratchSeedAudit, NativeTacticScratchStopReason,
    NativeTacticScratchTerminalImprovementAudit,
};
mod scratch_comparison;
pub use scratch_comparison::{
    NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V2, NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V3,
    NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V4, NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V5,
    NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V6, NATIVE_TACTIC_SCRATCH_COMPARISON_SCHEMA_V7,
    NativeTacticScratchComparisonCell, NativeTacticScratchComparisonReport,
    NativeTacticScratchCriticalPathTiming, NativeTacticScratchEfficiencyMetrics,
    NativeTacticScratchRouteProgress, NativeTacticScratchTreatment,
};
mod route_diagnosis;
pub use route_diagnosis::{
    NATIVE_TACTIC_ROUTE_DIAGNOSIS_SCHEMA_V1, NativeTacticRouteActionSurfaceProfile,
    NativeTacticRouteDiagnosisReport, NativeTacticRouteProfile, NativeTacticScratchRouteDiagnosis,
};
mod observation_audit;
pub use observation_audit::{
    NATIVE_TACTIC_OBSERVATION_AUDIT_SCHEMA_V2, NativeTacticActionSurfaceCoverage,
    NativeTacticObservationAudit, NativeTacticPolicySignalContract, NativeTacticSignalCoverage,
};
mod post_terminal_controls;
pub use post_terminal_controls::{
    NATIVE_TACTIC_POST_TERMINAL_CONTROL_SCHEMA_V1, NATIVE_TACTIC_POST_TERMINAL_CONTROL_SCHEMA_V2,
    NativeTacticPostTerminalControl, NativeTacticPostTerminalControlReport,
    NativeTacticPostTerminalDecisionControl, NativeTacticPostTerminalRanking,
    NativeTacticPostTerminalSeedControl,
};

mod throughput_curve;
pub use throughput_curve::{
    NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V1, NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V2,
    NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V3, NATIVE_TACTIC_THROUGHPUT_CURVE_SCHEMA_V4,
    NATIVE_TACTIC_THROUGHPUT_WORKER_COUNTS, NativeTacticThroughputCurveCell,
    NativeTacticThroughputCurveConfig, NativeTacticThroughputCurveReport,
    NativeTacticThroughputCurveRun, NativeTacticThroughputCurveSample,
    run_native_tactic_throughput_curve, run_native_tactic_throughput_curve_controlled,
};
mod throughput_evidence_bundle;
pub use throughput_evidence_bundle::{
    NATIVE_TACTIC_THROUGHPUT_EVIDENCE_BUNDLE_SCHEMA_V1, NATIVE_TACTIC_THROUGHPUT_EVIDENCE_MANIFEST,
    NativeTacticThroughputCompressedArtifact, NativeTacticThroughputEvidenceBundle,
    NativeTacticThroughputSampleEvidence,
};
mod throughput_treatment_audit;
pub use throughput_treatment_audit::{
    NATIVE_TACTIC_THROUGHPUT_TREATMENT_AUDIT_SCHEMA_V1, NativeTacticThroughputTreatmentAudit,
    NativeTacticThroughputTreatmentMetrics,
};
mod throughput_treatment_bundle;
pub use throughput_treatment_bundle::{
    NATIVE_TACTIC_THROUGHPUT_TREATMENT_BUNDLE_SCHEMA_V1,
    NATIVE_TACTIC_THROUGHPUT_TREATMENT_MANIFEST, NativeTacticThroughputTreatmentBundle,
    NativeTacticThroughputTreatmentSampleEvidence,
};

mod restore_locality;
pub use restore_locality::{
    NATIVE_TACTIC_RESTORE_LOCALITY_SCHEMA_V1, NativeTacticRestoreLocalityConfig,
    NativeTacticRestoreLocalityPair, NativeTacticRestoreLocalityReport,
    NativeTacticRestoreLocalitySample, NativeTacticRestoreLocalityTreatment,
    run_native_tactic_restore_locality,
};

mod learner_head;
use learner_head::{CampaignLearnerHead, CampaignLearnerHeadJournal};
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
    let orchestrator_executable_sha256 = current_executable_sha256()?;
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
    let (report_path, summary_path, completion_path) =
        prepare_campaign_completion(config.output_root, config.resume)?;
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
    if campaign_content_store.root() != replay_content_root {
        return Err(route_message(
            "campaign and replay content authorities use different roots",
        ));
    }
    let replay_control_plane = if replay_control_plane_path.exists() {
        TacticReplayControlPlane::open_with_content_store(
            &replay_control_plane_path,
            campaign_content_store.clone(),
            &replay_control_plane_identity,
        )
        .map_err(route_error)?
    } else {
        TacticReplayControlPlane::create_with_content_store(
            &replay_control_plane_path,
            campaign_content_store.clone(),
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
            match config.execution_plan.replay_sharing {
                NativeTacticReplaySharingPlan::BoundedStaleness {
                    maximum_stale_replay_revisions,
                } => Some(maximum_stale_replay_revisions),
                NativeTacticReplaySharingPlan::GenerationBarrier => None,
            },
        )?));
    let frozen_policy_snapshot =
        if config.execution_plan.proposal_policy == TacticProposalPolicy::FrozenPolicy {
            Some(lock_learner_authority(&learner_authority)?.snapshot())
        } else {
            None
        };

    let pool = fleet.pool(config, execution_plan_sha256, root_source_frame)?;
    let checkpoint_cache_capacity_per_worker_bytes =
        u64::try_from(pool.checkpoint_cache_capacity_bytes).map_err(route_error)?;
    let mut campaign_phase_wall = CampaignPhaseWallTiming::default();
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
            campaign_phase_wall.campaign_setup_micros = elapsed_micros(campaign_started.elapsed());
            for generation in &config.execution_plan.generations {
                let generation_started = Instant::now();
                let inherited_learner_snapshot = if let Some(snapshot) = &frozen_policy_snapshot {
                    Arc::clone(snapshot)
                } else {
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
                let live_learner = (config
                    .execution_plan
                    .proposal_policy
                    .deploys_policy_updates()
                    && matches!(
                        config.execution_plan.replay_sharing,
                        NativeTacticReplaySharingPlan::BoundedStaleness { .. }
                    ))
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
                            let content_store = campaign_content_store.clone();
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
                                    content_store,
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
                let generation_wall_micros = elapsed_micros(generation_started.elapsed());
                let critical_lane_wall_micros = generation_results
                    .iter()
                    .map(|(_, completion)| completion.invocation_wall_micros)
                    .max()
                    .unwrap_or(0);
                let seed_invocation_model_update_micros = generation_results
                    .iter()
                    .try_fold(0_u64, |total, (_, completion)| {
                        total.checked_add(completion.invocation_model_update_micros)
                    })
                    .ok_or_else(|| {
                        route_message("native tactic invocation model timing overflowed")
                    })?;
                campaign_phase_wall.seed_invocation_model_update_micros = campaign_phase_wall
                    .seed_invocation_model_update_micros
                    .checked_add(seed_invocation_model_update_micros)
                    .ok_or_else(|| {
                        route_message("native tactic invocation model timing overflowed")
                    })?;
                campaign_phase_wall.generation_coordination_micros = campaign_phase_wall
                    .generation_coordination_micros
                    .checked_add(
                        generation_wall_micros
                            .checked_sub(critical_lane_wall_micros)
                            .ok_or_else(|| {
                                route_message(
                                    "native tactic generation lane wall exceeds generation wall",
                                )
                            })?,
                    )
                    .ok_or_else(|| {
                        route_message("native tactic generation coordination timing overflowed")
                    })?;
                results.extend(
                    generation_results
                        .into_iter()
                        .map(|(seed_index, completion)| (seed_index, completion.result)),
                );
            }
            campaign_phase_wall.campaign_finalization_started_micros =
                elapsed_micros(campaign_started.elapsed());
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
    let campaign_useful_graph_expansions =
        campaign_useful_graph_expansion_set(config.repository_root, &seed_results)?;
    let unique_useful_graph_expansions = campaign_useful_graph_expansions.count()?;
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
    let mut timing = aggregate_route_timing(&seed_results, unique_useful_graph_expansions)?;
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
    // Seed timing is durable across resume. Keep report wall time at least the
    // accumulated generation critical path so a short final invocation cannot
    // make cumulative work appear artificially fast.
    let observed_route_cutoff_wall_micros = elapsed_micros(campaign_started.elapsed());
    let campaign_finalization_wall_micros = observed_route_cutoff_wall_micros
        .checked_sub(campaign_phase_wall.campaign_finalization_started_micros)
        .ok_or_else(|| route_message("native tactic campaign finalization clock regressed"))?;
    timing.wall_micros = observed_route_cutoff_wall_micros.max(
        accumulated_coordinator_wall_micros(config.execution_plan, &seed_results),
    );
    refresh_route_throughput(&mut timing, &seed_results, unique_useful_graph_expansions);
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
    let learner_updates = learner_authority.total_updates();
    let demonstration_execution_micros =
        demonstration.as_ref().map_or(0, |value| value.wall_micros);
    attribute_campaign_timing(
        &mut timing,
        config.execution_plan,
        &seed_results,
        CampaignExclusiveTimingInput {
            process_launch_micros,
            demonstration_execution_micros,
            macro_validation_execution_micros: tactic_macro_discovery.validation_wall_micros,
            learner_update_micros: learner_metrics.update_micros,
            learner_reconstruction_micros: learner_metrics.reconstruction_micros,
            seed_invocation_model_update_micros: campaign_phase_wall
                .seed_invocation_model_update_micros,
            campaign_setup_wall_micros: campaign_phase_wall.campaign_setup_micros,
            generation_coordination_wall_micros: campaign_phase_wall.generation_coordination_micros,
            campaign_finalization_wall_micros,
        },
    )?;
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
        model_snapshots_published: learner_authority.published_snapshot_count(),
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
        schema: NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V44.into(),
        orchestrator_executable_sha256: Some(orchestrator_executable_sha256),
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
        worker_utilization: None,
        checkpoint_cache_capacity_per_worker_bytes,
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
    let report_build_before_shutdown_micros = elapsed_micros(reporting_started.elapsed());
    let fleet_shutdown_started = Instant::now();
    report.worker_utilization = owned_fleet
        .map(NativeTacticWorkerFleet::shutdown)
        .transpose()?;
    let fleet_shutdown_micros = elapsed_micros(fleet_shutdown_started.elapsed());
    let final_report_build_started = Instant::now();
    serde_json::to_writer(std::io::sink(), &report).map_err(route_error)?;
    report.timing.reporting_micros = report_build_before_shutdown_micros
        .checked_add(elapsed_micros(final_report_build_started.elapsed()))
        .ok_or_else(|| route_message("native tactic report build timing overflowed"))?;
    let report_build_micros = report.timing.reporting_micros;
    let final_artifact_persistence_started = Instant::now();
    let summary = NativeTacticCampaignSummary::build(&report, config.execution_plan)?;
    let summary_bytes = summary.to_pretty_json()?;
    let report_bytes = serde_json::to_vec_pretty(&report).map_err(route_error)?;
    publish_new_atomic(&summary_path, &summary_bytes)?;
    publish_new_atomic(&report_path, &report_bytes)?;
    let final_artifact_persistence_micros =
        elapsed_micros(final_artifact_persistence_started.elapsed());
    let completion = NativeTacticCampaignCompletion::build(
        execution_plan_sha256,
        &report_bytes,
        &summary_bytes,
        report.timing.wall_micros,
        report_build_micros,
        fleet_shutdown_micros,
        final_artifact_persistence_micros,
        elapsed_micros(campaign_started.elapsed()),
    )?;
    publish_completion(&completion_path, &completion)?;
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

pub(super) fn supports_current_route_report_schema(schema: &str) -> bool {
    matches!(
        schema,
        NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V39
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V40
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V41
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V42
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V43
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V44
    )
}

fn current_executable_sha256() -> Result<Digest, NativeTacticRouteRunError> {
    let path = std::env::current_exe().map_err(route_error)?;
    let mut file = fs::File::open(path).map_err(route_error)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(route_error)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(Digest(hasher.finalize().into()))
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
mod replay_content;
use replay_content::{persist_evaluated_replay_content, retained_replay_components};

mod worker_pool;
use worker_pool::{
    CachedTacticFrontier, NativeTacticProposalPool, NativeTacticProposalWork,
    applicable_parameterized_descriptors_for_state, load_or_capture_demonstration,
    parameterized_catalog_for_state, parameterized_catalog_for_state_with_promoted,
    parameterized_feedback_for_state, run_seed_coordinator,
};
mod action_surface_audit;
pub use action_surface_audit::{
    NATIVE_TACTIC_ACTION_SURFACE_AUDIT_CONTEXT_SCHEMA_V1, NativeTacticActionSurfaceAuditContext,
};
pub(crate) use action_surface_audit::{
    native_tactic_action_surface_audit_context, native_tactic_applicable_action_surface_identity,
};
mod worker_fleet;
use worker_fleet::NativeTacticWorkerFleet;
mod campaign;
mod campaign_schedule;
use campaign::{NATIVE_TACTIC_RESULT_ADMISSION_SCHEMA_V1, run_seed};
mod graph_metrics;
use graph_metrics::{
    CampaignUsefulGraphExpansionSet, campaign_useful_graph_expansion_set, tactic_graph_metrics,
    validate_seed_useful_graph_accounting,
};
mod timing_metrics;
use timing_metrics::{
    accumulated_coordinator_wall_micros, aggregate_route_timing, censored_training_transitions,
    decision_evaluated_ticks, decision_trace_is_useful, elapsed_micros, per_second_millionths,
    ratio_per_million, record_persistence_timing, refresh_route_throughput,
    useful_training_transitions,
};
mod candidate_retention;
use candidate_retention::{
    authenticated_first_hit_tick, load_best_retained_success, retain_successful_result,
    synchronize_graph_terminal_result,
};
mod campaign_persistence;
use campaign_persistence::{
    cancellation_requested, load_seed_performance, read_completed_seed_result, resume_seed,
};
mod journal;
use journal::{
    TacticDecisionJournalAppender, compact_tactic_decision_journal, decision_record,
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

fn initial_probe_batch(
    config: &NativeTacticRouteRunConfig<'_>,
) -> Result<NativeSuffixBatch, NativeTacticRouteRunError> {
    // The root observation is the first pre-input row. Running the entire
    // exploration horizon here produces no additional authority: the
    // persistent worker has already captured the authenticated source
    // checkpoint before it evaluates this candidate, and subsequent batches
    // declare their own bounded horizons.
    let mut batch = tactic_root_probe_batch_with_ticks(config.optimization, config.execution, 1)?;
    let capacity = tactic_checkpoint_cache_capacity_per_worker(
        config.execution_plan.budgets.memory_bytes,
        config.checkpoint_capacity_workers,
    )?;
    attach_root_probe_checkpoint_cache(&mut batch, capacity);
    Ok(batch)
}

fn attach_root_probe_checkpoint_cache(batch: &mut NativeSuffixBatch, capacity: usize) {
    batch.schema = NATIVE_CACHED_SUFFIX_BATCH_SCHEMA.into();
    batch.checkpoint_cache = Some(tactic_checkpoint_cache_request(
        None,
        NativeTacticCheckpointRetention::None,
        capacity,
    ));
}

mod goal_target;
pub use goal_target::NativeTacticGoalTargetReport;
pub(crate) use goal_target::parameterized_policy_action_schema_sha256;
pub(crate) use goal_target::{GoalConditionedTacticContext, goal_conditioned_tactic_runtime};
use goal_target::{atomic_goal_conditioned_tactic_context, planar_distance};

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

fn maximum_demonstration_chunk_ticks(horizon: u64) -> Result<u32, NativeTacticRouteRunError> {
    Ok(goal_tactic_maximum_ticks(horizon)?
        .min(u32::try_from(TACTIC_INTERMEDIATE_BOUNDARY_STRIDE).map_err(route_error)?))
}

fn validate_config(
    config: &NativeTacticRouteRunConfig<'_>,
) -> Result<(), NativeTacticRouteRunError> {
    config.execution_plan.validate()?;
    validate_unassisted_discovery_horizon(config)?;
    tactic_checkpoint_cache_capacity_per_worker(
        config.execution_plan.budgets.memory_bytes,
        config.checkpoint_capacity_workers,
    )?;
    let maximum_demonstration_chunk_ticks =
        maximum_demonstration_chunk_ticks(config.optimization.budgets.exploration_horizon_ticks)?;
    if !valid_worker_capacity_counts(config.workers, config.checkpoint_capacity_workers)
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
        || config.fault_injection.is_some_and(|fault| {
            fault.decision_index() >= config.execution_plan.budgets.decisions_per_lane
        })
    {
        return Err(route_message(
            "native tactic route configuration is invalid",
        ));
    }
    Ok(())
}

fn valid_worker_capacity_counts(workers: usize, checkpoint_capacity_workers: usize) -> bool {
    workers > 0
        && workers <= MAX_ROUTE_WORKERS
        && checkpoint_capacity_workers >= workers
        && checkpoint_capacity_workers <= MAX_ROUTE_WORKERS
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

fn tactic_checkpoint_cache_capacity_per_worker(
    memory_bytes: NativeTacticResourceLimit,
    workers: usize,
) -> Result<usize, NativeTacticRouteRunError> {
    let workers = u64::try_from(workers).map_err(route_error)?;
    if workers == 0 {
        return Err(route_message(
            "native tactic checkpoint cache requires at least one worker",
        ));
    }
    let capacity = match memory_bytes {
        NativeTacticResourceLimit::Bounded(total) => total / workers,
        NativeTacticResourceLimit::Unbounded => TACTIC_CHECKPOINT_CACHE_BYTES as u64,
    }
    .min(TACTIC_CHECKPOINT_CACHE_BYTES as u64);
    if capacity == 0 {
        return Err(route_message(
            "native tactic memory budget cannot provide every worker a checkpoint cache",
        ));
    }
    usize::try_from(capacity).map_err(route_error)
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

fn publish_new_atomic(path: &Path, bytes: &[u8]) -> Result<(), NativeTacticRouteRunError> {
    let parent = path
        .parent()
        .ok_or_else(|| route_message("published artifact has no parent directory"))?;
    fs::create_dir_all(parent).map_err(route_error)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| route_message("published artifact filename is invalid"))?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(route_error)?
        .as_nanos();
    let partial = parent.join(format!(
        ".{file_name}.{}.{}.partial",
        std::process::id(),
        nonce
    ));
    if let Err(error) = write_new(&partial, bytes) {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    match fs::rename(&partial, path) {
        Ok(()) => sync_published_artifact_parent(parent),
        Err(error) => {
            let _ = fs::remove_file(&partial);
            Err(route_error(error))
        }
    }
}

fn prepare_campaign_completion(
    output_root: &Path,
    resume: bool,
) -> Result<(PathBuf, PathBuf, PathBuf), NativeTacticRouteRunError> {
    let report_path = output_root.join("report.json");
    let summary_path = output_root.join(NATIVE_TACTIC_CAMPAIGN_SUMMARY_FILE);
    let completion_path = output_root.join(NATIVE_TACTIC_CAMPAIGN_COMPLETION_FILE);
    if completion_path.exists() {
        let completion = NativeTacticCampaignCompletion::read(&completion_path)?;
        completion.validate_files(&report_path, &summary_path)?;
        return Err(route_message("completed tactic route cannot be resumed"));
    }
    // The binary marker is published only after fleet shutdown and both final
    // artifacts. Without it, either JSON file is an interrupted derived tail
    // and resume rebuilds it from durable campaign authority.
    if resume {
        for derived in [&summary_path, &report_path] {
            if derived.exists() {
                fs::remove_file(derived).map_err(route_error)?;
            }
        }
    }
    Ok((report_path, summary_path, completion_path))
}

#[cfg(not(windows))]
fn sync_published_artifact_parent(parent: &Path) -> Result<(), NativeTacticRouteRunError> {
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(route_error)
}

#[cfg(windows)]
fn sync_published_artifact_parent(parent: &Path) -> Result<(), NativeTacticRouteRunError> {
    if parent.is_dir() {
        Ok(())
    } else {
        Err(route_message(
            "published artifact parent is not a directory",
        ))
    }
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
