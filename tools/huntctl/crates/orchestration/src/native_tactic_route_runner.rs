//! Fresh-model tactic-Q route learning on an authenticated native checkpoint.

use crate::discovery_horizon::minimum_discovery_horizon_ticks;
use crate::native_residual_campaign::{
    NativeResidualExecutionBinding, ValidatedNativeResidualExecution,
};
use crate::native_suffix_result::{NativeTerminalBinding, ValidatedNativeSuffixBatch};
use crate::native_suffix_worker::{
    NativeSuffixPrevalidatedFileIdentities, NativeSuffixWorkerError, NativeSuffixWorkerLaunch,
    NativeSuffixWorkerSession,
};
use crate::native_tactic_worker::{
    NativeGenericExecutionStrategy, NativeTacticCheckpointRetention, NativeTacticCheckpointSource,
    NativeTacticCheckpointStorage, NativeTacticWorkerError, NativeTacticWorkerOutcome,
    NativeTacticWorkerPaths, PersistentTacticBatchWorker, TACTIC_CHECKPOINT_CACHE_BYTES,
    TACTIC_CHECKPOINT_CACHE_ENTRIES, TACTIC_INTERMEDIATE_BOUNDARY_STRIDE,
    execute_selected_tactic_batch_if_compatible,
    execute_selected_tactic_with_checkpoint_retention_and_strategy,
    materialize_tactic_frontier_with_cache_capacity, selected_tactic_batch_is_compatible,
    tactic_checkpoint_cache_request, tactic_root_checkpoint_sha256,
};
use crate::optimization_request::{CampaignClass, OptimizationRequest};
use crate::reporting::GraphSearchReport;
use crate::tactic_macro_store::{
    TACTIC_MACRO_REGISTRY_EXTENSION, read_tactic_macro_registry, write_tactic_macro_registry,
};
#[cfg(test)]
use crate::tactic_q_campaign::TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V4;
use crate::tactic_q_campaign::{
    EvaluatedRewardedTacticOutcome, TACTIC_Q_CHECKPOINT_EXTENSION,
    TACTIC_Q_DEMONSTRATION_EPISODE_GROUP, TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V7,
    TacticCampaignDiagnostics, TacticCampaignGraphProjection, TacticCampaignGraphProjectionEdge,
    TacticCampaignGraphProjectionNode, TacticExpansionLease, TacticFrontierAcquisition,
    TacticGraphSchedulingTiming, TacticPolicyEvaluationDecisionTrace, TacticQCampaign,
    TacticQCampaignCheckpoint, TacticQCampaignError, TacticQFinalResult,
    TacticQImmutableLearnerSnapshot, TacticQLearnerSnapshot, TacticQLearnerSnapshotKind,
    TacticQOnlineActionSelectionRequest, TacticQOnlineActionSurface, TacticQOnlineDecisionPlan,
    TacticQOnlineDecisionRequest, TacticQOnlineHorizonPlan, TacticQOnlineLearningController,
    TacticQOnlineLeaseMode, TacticQOnlinePolicyUpdate, TacticQOnlineRolloutRequest,
    TacticQProposalBatch, TacticQTrainingCorpus, TacticRestorationContract,
    TacticSchedulerDecisionTrace, has_no_progress_loop, online_tactic_fits_horizon,
    plan_online_horizon, route_checkpoint, validate_training_corpus,
};
use crate::tactic_q_checkpoint_store::{StoredContentRef, TacticQContentStore};
use crate::tactic_replay_control_plane::{
    TacticReplayAdmissionMetrics, TacticReplayAdmissionOutcome, TacticReplayControlPlane,
    TacticReplayControlPlaneIdentity, TacticReplaySnapshot, TacticReplaySnapshotVersion,
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
    MAX_DISCOVERY_OBSERVATIONS, MIN_DISCOVERY_OCCURRENCES, MacroComparisonEvidence,
    MacroDiscoveryObservation, MacroEntryObservation, MacroPromotionStatus, MacroSourceProvenance,
    TacticMacroComponent, TacticMacroEntryCondition, TacticMacroPromotionRegistry,
    discover_replay_macros, replay_macro_candidate, terminal_lineage_macro_candidate,
};
use dusklight_learning::tactic_value_treatment::TacticValueTreatment;
use dusklight_learning::terminal_action_calibration::TerminalActionCalibration;
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
use std::sync::{Arc, Condvar, Mutex, mpsc};
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
pub const NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V45: &str = "dusklight-native-tactic-route-report/v45";
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
// Completed v45 campaigns wrote pretty-printed seed and route reports. Accept
// those existing artifacts for replay/migration while new authoritative seed
// results remain capped by the smaller compact-writer bound below.
const MAX_RESUME_JSON_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SEED_RESULT_JSON_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ROUTE_ATTEMPTS: usize = 10_000;
const TACTIC_ROUTE_PERFORMANCE_SCHEMA_V2: &str = "dusklight-native-tactic-route-performance/v2";
const TACTIC_MACRO_ENTRY_GOAL_DISTANCE_PADDING: f32 = 128.0;

mod report;
use report::{
    CompletedNativeTacticSeed, NativeTacticDecisionRecord, NativeTacticDemonstration,
    NativeTacticProposalRecord, NativeTacticSeedPerformance,
};
pub use report::{
    NATIVE_TACTIC_PAIRED_TERMINAL_RETURN_SCHEMA_V1, NativeTacticCampaignAdmissionTiming,
    NativeTacticDecisionTrace, NativeTacticDemonstrationReport, NativeTacticFrontierAvailability,
    NativeTacticGraphMetrics, NativeTacticImportedMacroReport, NativeTacticLearnerAuthorityReport,
    NativeTacticMacroDiscoveryReport, NativeTacticMacroReuseReport, NativeTacticMeasurementTrace,
    NativeTacticOrchestrationTiming, NativeTacticPairedTerminalReturnRole,
    NativeTacticPairedTerminalReturnStatus, NativeTacticPairedTerminalReturnTrace,
    NativeTacticPersistenceTiming, NativeTacticProposalTrace, NativeTacticReplaySharingTelemetry,
    NativeTacticRestoreAccounting, NativeTacticRestoreSource, NativeTacticRouteReport,
    NativeTacticRouteRunConfig, NativeTacticRouteTiming, NativeTacticSeedResult,
    NativeTacticSeedStopReason, NativeTacticStateTrace, NativeTacticValueTrace,
    NativeTacticWorkerUtilization,
};
mod causal_policy_probe;
pub use causal_policy_probe::NativeTacticPolicyUpdateProbe;
#[cfg(test)]
use causal_policy_probe::build_policy_update_probe;
use causal_policy_probe::{PolicyUpdateProbeContext, consume_policy_update_with_probe};
mod completion_marker;
use completion_marker::publish_completion;
mod seed_completion;
use seed_completion::{
    NATIVE_TACTIC_SEED_COMPLETION_FILE, NativeTacticSeedCompletion,
    NativeTacticSeedCompletionProjection, publish_seed_completion,
};
mod learner_completion;
use learner_completion::{
    NATIVE_TACTIC_LEARNER_COMPLETION_FILE, NativeTacticLearnerCompletion,
    publish_learner_completion,
};
mod exclusive_timing;
pub use completion_marker::{
    NATIVE_TACTIC_CAMPAIGN_COMPLETION_FILE, NATIVE_TACTIC_CAMPAIGN_COMPLETION_SCHEMA_V1,
    NativeTacticCampaignCompletion,
};
use exclusive_timing::{
    CampaignExclusiveTimingInput, CampaignPhaseWallTiming, ExclusiveTopTimingSnapshot,
    SeedOrchestrationPhase as OrchestrationPhase, attribute_campaign_timing,
    cumulative_route_wall_micros, orchestration_detail_total, reconcile_recovered_seed_timing,
    record_orchestration_detail, record_orchestration_total,
};
mod campaign_summary;
pub use campaign_summary::{
    NATIVE_TACTIC_CAMPAIGN_SUMMARY_FILE, NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V1,
    NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V2, NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V3,
    NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V4, NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V5,
    NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V6, NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V7,
    NATIVE_TACTIC_CAMPAIGN_SUMMARY_SCHEMA_V8, NativeTacticCampaignCausalSummary,
    NativeTacticCampaignEfficiencySummary, NativeTacticCampaignGoalReachabilitySummary,
    NativeTacticCampaignIdentities, NativeTacticCampaignOutcomeSummary,
    NativeTacticCampaignResourceSummary, NativeTacticCampaignSummary,
    NativeTacticCampaignTimingSummary, NativeTacticCampaignTreatmentSummary,
    NativeTacticCampaignWorkSummary, NativeTacticCausalLink,
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
pub(crate) use cold_replay::{
    NativeTapeColdReplayConfig, exact_cold_replay_attempts,
    run_native_tape_cold_replay_after_execution_validation,
    validate_native_tape_cold_replay_artifacts,
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
    NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V6, NATIVE_TACTIC_SCRATCH_CAMPAIGN_AUDIT_SCHEMA_V7,
    NativeTacticCampaignResourceAudit, NativeTacticScratchCampaignAudit,
    NativeTacticScratchDecisionAudit, NativeTacticScratchSeedAudit, NativeTacticScratchStopReason,
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
    CampaignLearnerFinalizationSnapshot, CampaignLearnerPublishResult,
    CampaignLearnerUpdateMetrics, CampaignTacticLearnerAuthority, CompletedCampaignLearnerView,
    SharedTacticLearnerAuthority, lock_learner_authority,
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
    // A completed campaign is an immutable artifact chain. Returning it must
    // not depend on re-authenticating native runtime files that cannot change
    // the already sealed report and are not part of its recovery decision.
    if let Some(report) = recover_completed_campaign(config, imported_promoted_tactics.as_ref())? {
        return Ok(report);
    }
    let orchestrator_executable_sha256 = current_executable_sha256()?;
    let root = config.repository_root.canonicalize().map_err(route_error)?;
    config
        .execution
        .validate_control_files(&root, config.optimization)
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
    let mut completed_preflight =
        load_completed_seed_preflight(config, execution_plan_sha256, &process_tape)?;
    let native_fleet_required =
        completed_seed_preflight_requires_native_fleet(completed_preflight.as_ref());
    let owned_fleet = (external_fleet.is_none() && native_fleet_required)
        .then(|| launch_native_tactic_worker_fleet(config, config.output_root, worker_count))
        .transpose()?;
    let fleet = external_fleet.or(owned_fleet.as_ref());
    if native_fleet_required {
        fleet
            .ok_or_else(|| route_message("native tactic worker fleet is absent"))?
            .validate_for(config)?;
    }
    let process_launch_micros = if completed_preflight.is_some() || external_fleet.is_some() {
        0
    } else {
        fleet
            .ok_or_else(|| route_message("native tactic worker fleet is absent"))?
            .launch_micros()
    };
    let initial_facts = completed_preflight
        .as_ref()
        .map(|preflight| preflight.initial_facts.clone())
        .or_else(|| fleet.map(|fleet| fleet.initial_facts().clone()))
        .ok_or_else(|| route_message("native tactic root facts are absent"))?;
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
    let mut active_promoted_tactics = imported_promoted_tactics
        .as_ref()
        .map_or_else(Vec::new, |imported| imported.entries.clone());
    let root_checkpoint_sha256 = completed_preflight
        .as_ref()
        .map(|preflight| preflight.root_checkpoint_sha256)
        .or_else(|| fleet.map(NativeTacticWorkerFleet::root_checkpoint_sha256))
        .ok_or_else(|| route_message("native tactic root checkpoint is absent"))?;
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
    let (learner_authority, completed_learner_view) = if completed_preflight.is_some() {
        (
            None,
            Some(CompletedCampaignLearnerView::open(
                config.output_root,
                campaign_content_store.clone(),
                &replay_control_plane_identity,
            )?),
        )
    } else {
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
                replay_control_plane_identity.clone(),
            )
            .map_err(route_error)?
        };
        (
            Some(Arc::new(Mutex::new(CampaignTacticLearnerAuthority::new(
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
            )?))),
            None,
        )
    };
    // A frozen control snapshots the policy after the common setup corpus is
    // fitted below. Capturing here would freeze an empty learner while the
    // adaptive treatment receives the incumbent/demonstration prior.
    let mut frozen_policy_snapshot = None;

    let pool = if native_fleet_required {
        Some(
            fleet
                .ok_or_else(|| route_message("native tactic worker fleet is absent"))?
                .pool(config, execution_plan_sha256, root_source_frame)?,
        )
    } else {
        None
    };
    let checkpoint_cache_capacity_per_worker_bytes = u64::try_from(pool.as_ref().map_or_else(
        || {
            tactic_checkpoint_cache_capacity_per_worker(
                config.execution_plan.budgets.memory_bytes,
                config.checkpoint_capacity_workers,
            )
        },
        |pool| Ok(pool.checkpoint_cache_capacity_bytes),
    )?)
    .map_err(route_error)?;
    let mut campaign_phase_wall = CampaignPhaseWallTiming::default();
    let (
        mut indexed_results,
        tactic_macro_discovery,
        shared_training_replay_rows,
        demonstration,
        prevalidated_useful_graph_expansions,
    ) = (|| {
        if let Some(preflight) = completed_preflight.take() {
            // Reused demonstration evidence retains its historical execution
            // phase in aggregate timing even though this invocation does not
            // execute it. Keep the setup wall large enough to own that durable
            // phase rather than charging fast validation as demonstration work.
            let durable_demonstration_wall_micros = preflight
                .demonstration
                .as_ref()
                .map_or(0, |demonstration| demonstration.wall_micros);
            campaign_phase_wall.campaign_setup_micros =
                elapsed_micros(campaign_started.elapsed()).max(durable_demonstration_wall_micros);
            campaign_phase_wall.campaign_finalization_started_micros =
                campaign_phase_wall.campaign_setup_micros;
            let tactic_macro_discovery = match preflight.tactic_macro_discovery {
                Some(report) => report,
                None => finalize_tactic_macro_discovery(
                    config,
                    pool.as_ref().ok_or_else(|| {
                        route_message("native tactic macro finalization pool is absent")
                    })?,
                    &encoder,
                    execution_plan_sha256,
                    root_checkpoint_sha256,
                    &ActiveTacticMacroLifecycle::default(),
                )?,
            };
            let shared_training_replay_rows = completed_learner_view
                .as_ref()
                .ok_or_else(|| route_message("completed learner authority is absent"))?
                .replay_len() as u64;
            return Ok((
                preflight.indexed_results,
                tactic_macro_discovery,
                shared_training_replay_rows,
                preflight.demonstration,
                Some(preflight.useful_graph_expansions),
            ));
        }
        let pool = pool
            .clone()
            .ok_or_else(|| route_message("native tactic worker pool is absent"))?;
        let learner_authority = learner_authority
            .as_ref()
            .cloned()
            .ok_or_else(|| route_message("live learner authority is absent"))?;
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
        let learner = lock_learner_authority(&learner_authority)?;
        if config.execution_plan.proposal_policy == TacticProposalPolicy::FrozenPolicy {
            frozen_policy_snapshot = Some(learner.snapshot());
        }
        campaign_phase_wall.campaign_setup_model_update_micros =
            learner.invocation_metrics().update_micros;
        drop(learner);
        campaign_phase_wall.campaign_setup_micros = elapsed_micros(campaign_started.elapsed());
        let mut active_macro_lifecycle = ActiveTacticMacroLifecycle::default();
        for (generation_position, generation) in
            config.execution_plan.generations.iter().enumerate()
        {
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
                    NativeTacticReplaySharingPlan::BoundedStaleness { .. } => learner.snapshot(),
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
            let cross_lane_replay = if let Some(learner) = live_learner.as_ref()
                && generation.lane_indices.len() > 1
            {
                Some(CrossLaneReplayCoordinator::new(
                    Arc::clone(learner),
                    &generation.lane_indices,
                )?)
            } else {
                None
            };
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
                        let promoted_tactics = active_promoted_tactics.as_slice();
                        let initial_facts = &initial_facts;
                        let route_prefix = &route_prefix;
                        let live_learner = live_learner.clone();
                        let cross_lane_replay = cross_lane_replay.clone();
                        let content_store = campaign_content_store.clone();
                        let inherited_learner_snapshot = Arc::clone(&inherited_learner_snapshot);
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
                                cross_lane_replay,
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
            let generation_update = learner.force_update()?;
            campaign_phase_wall.generation_model_update_micros = campaign_phase_wall
                .generation_model_update_micros
                .checked_add(generation_update.update_micros)
                .ok_or_else(|| route_message("native tactic generation model timing overflowed"))?;
            let generation_wall_micros = elapsed_micros(generation_started.elapsed());
            let critical_lane_wall_micros = generation_results
                .iter()
                .map(|(_, completion)| completion.invocation_wall_micros)
                .max()
                .unwrap_or(0);
            campaign_phase_wall.seed_invocation_critical_lane_wall_micros = campaign_phase_wall
                .seed_invocation_critical_lane_wall_micros
                .checked_add(critical_lane_wall_micros)
                .ok_or_else(|| route_message("native tactic invocation lane timing overflowed"))?;
            let seed_invocation_model_update_micros = generation_results
                .iter()
                .try_fold(0_u64, |total, (_, completion)| {
                    total.checked_add(completion.invocation_model_update_micros)
                })
                .ok_or_else(|| route_message("native tactic invocation model timing overflowed"))?;
            campaign_phase_wall.seed_invocation_model_update_micros = campaign_phase_wall
                .seed_invocation_model_update_micros
                .checked_add(seed_invocation_model_update_micros)
                .ok_or_else(|| route_message("native tactic invocation model timing overflowed"))?;
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
            if should_refresh_active_tactic_macros(
                config.execution_plan.proposal_policy,
                generation_position,
                config.execution_plan.generations.len(),
            ) {
                let active_refresh_started = Instant::now();
                let source_lanes = results
                    .iter()
                    .map(|(seed_index, _)| TacticMacroSourceLane {
                        seed_index: *seed_index,
                        seed: config.execution_plan.seeds[*seed_index],
                    })
                    .collect::<Vec<_>>();
                if let Some(refresh) = refresh_active_tactic_macros(
                    config,
                    &pool,
                    &encoder,
                    root_checkpoint_sha256,
                    &source_lanes,
                    generation.generation_index,
                )? {
                    if refresh.promoted_tactics.is_empty() != refresh.policy_evidence.is_empty() {
                        return Err(route_message(
                            "active tactic promotion is detached from its policy evidence",
                        ));
                    }
                    let policy_evidence = publish_tactic_macro_policy_evidence(
                        &mut learner,
                        &refresh.policy_evidence,
                    )?;
                    campaign_phase_wall.generation_model_update_micros = campaign_phase_wall
                        .generation_model_update_micros
                        .checked_add(policy_evidence.update.update_micros)
                        .ok_or_else(|| {
                            route_message("native tactic macro policy update timing overflowed")
                        })?;
                    active_macro_lifecycle.policy_evidence_rows = active_macro_lifecycle
                        .policy_evidence_rows
                        .saturating_add(refresh.policy_evidence.len() as u64);
                    active_macro_lifecycle.policy_evidence_admitted_rows = active_macro_lifecycle
                        .policy_evidence_admitted_rows
                        .saturating_add(policy_evidence.admitted_rows);
                    campaign_phase_wall.active_macro_validation_micros = campaign_phase_wall
                        .active_macro_validation_micros
                        .checked_add(refresh.report.validation_wall_micros)
                        .ok_or_else(|| {
                            route_message("active tactic macro validation timing overflowed")
                        })?;
                    active_macro_lifecycle.promoted_option_ids.extend(
                        refresh.promoted_tactics.iter().filter_map(|tactic| {
                            (!active_promoted_tactics.iter().any(|existing| {
                                existing.entry.option_id() == tactic.entry.option_id()
                            }))
                            .then(|| tactic.entry.option_id().to_owned())
                        }),
                    );
                    merge_promoted_tactic_entries(
                        &mut active_promoted_tactics,
                        refresh.promoted_tactics,
                    )?;
                    active_macro_lifecycle
                        .validation_reports
                        .push(refresh.report);
                }
                // `generation_wall_micros` ends before active refresh. Charge
                // the complete refresh wall here: mining, native validation,
                // evidence publication, learner update, and bookkeeping. The
                // exclusive projection later subtracts the measured native
                // validation and model-update phases from this enclosing wall.
                campaign_phase_wall.generation_coordination_micros = campaign_phase_wall
                    .generation_coordination_micros
                    .checked_add(elapsed_micros(active_refresh_started.elapsed()))
                    .ok_or_else(|| {
                        route_message("active tactic macro refresh timing overflowed")
                    })?;
            }
        }
        campaign_phase_wall.campaign_finalization_started_micros =
            elapsed_micros(campaign_started.elapsed());
        let source_lanes = results
            .iter()
            .map(|(seed_index, _)| TacticMacroSourceLane {
                seed_index: *seed_index,
                seed: config.execution_plan.seeds[*seed_index],
            })
            .collect::<Vec<_>>();
        active_macro_lifecycle.selected_decisions = count_active_tactic_selections(
            config.output_root,
            &source_lanes,
            &active_macro_lifecycle.promoted_option_ids,
        )?;
        let completion = finalize_tactic_macro_discovery(
            config,
            &pool,
            &encoder,
            execution_plan_sha256,
            root_checkpoint_sha256,
            &active_macro_lifecycle,
        )?;
        let shared_training_replay_rows =
            lock_learner_authority(&learner_authority)?.replay().len() as u64;
        Ok::<_, NativeTacticRouteRunError>((
            results,
            completion,
            shared_training_replay_rows,
            demonstration_report,
            None,
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
    let campaign_useful_graph_expansions = match prevalidated_useful_graph_expansions {
        Some(expansions) => expansions,
        None => campaign_useful_graph_expansion_set(config.repository_root, &seed_results)?,
    };
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
    // Seed timing is durable across resume, while the observed campaign wall
    // covers only this invocation. Replace this invocation's seed critical
    // lane with the complete durable lane so neither portion is lost or
    // counted twice.
    // A completed-seed preflight reuses durable macro validation instead of
    // waiting through it again. Keep that authenticated historical phase in
    // the campaign wall floor so exclusive timing remains additive.
    let final_macro_validation_wall_micros = tactic_macro_discovery
        .validation_wall_micros
        .checked_sub(campaign_phase_wall.active_macro_validation_micros)
        .ok_or_else(|| route_message("active macro validation exceeds total validation"))?;
    let observed_route_cutoff_wall_micros = elapsed_micros(campaign_started.elapsed()).max(
        campaign_phase_wall
            .campaign_finalization_started_micros
            .saturating_add(final_macro_validation_wall_micros),
    );
    let campaign_finalization_wall_micros = observed_route_cutoff_wall_micros
        .checked_sub(campaign_phase_wall.campaign_finalization_started_micros)
        .ok_or_else(|| route_message("native tactic campaign finalization clock regressed"))?;
    timing.wall_micros = cumulative_route_wall_micros(
        observed_route_cutoff_wall_micros,
        accumulated_coordinator_wall_micros(config.execution_plan, &seed_results),
        campaign_phase_wall.seed_invocation_critical_lane_wall_micros,
    )?;
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
    let learner_finalization = if let Some(learner_authority) = &learner_authority {
        lock_learner_authority(learner_authority)?
            .finalization_snapshot(encoder.goal_distance_feature())?
    } else {
        completed_learner_view
            .as_ref()
            .ok_or_else(|| route_message("completed learner authority is absent"))?
            .finalization_snapshot(encoder.goal_distance_feature())?
    };
    let final_replay_snapshot = learner_finalization.replay_snapshot;
    let replay_admission = learner_finalization.replay_admission;
    let learner_metrics = learner_finalization.learner_metrics;
    let learner_updates = learner_finalization.learner_updates;
    publish_learner_completion(
        config.output_root,
        &replay_control_plane_identity,
        &learner_finalization,
    )?;
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
            active_macro_validation_execution_micros: tactic_macro_discovery
                .validation_wall_micros
                .checked_sub(final_macro_validation_wall_micros)
                .ok_or_else(|| route_message("active macro validation timing regressed"))?,
            learner_update_micros: learner_metrics.update_micros,
            learner_reconstruction_micros: learner_metrics.reconstruction_micros,
            campaign_setup_model_update_micros: campaign_phase_wall
                .campaign_setup_model_update_micros,
            generation_model_update_micros: campaign_phase_wall.generation_model_update_micros,
            seed_invocation_model_update_micros: campaign_phase_wall
                .seed_invocation_model_update_micros,
            campaign_setup_wall_micros: campaign_phase_wall.campaign_setup_micros,
            generation_coordination_wall_micros: campaign_phase_wall.generation_coordination_micros,
            campaign_finalization_wall_micros,
        },
    )?;
    let declared_model_snapshots_consumed = seed_results
        .iter()
        .flat_map(|seed| &seed.trace)
        .map(|decision| decision.learner_snapshot_sha256)
        .filter(|sha256| *sha256 != Digest::ZERO)
        .collect::<BTreeSet<_>>()
        .len() as u64;
    let lane_local_model_updates = seed_results.iter().map(|seed| seed.learner_updates).sum();
    let learner_authority_report = NativeTacticLearnerAuthorityReport {
        model_snapshots_published: learner_finalization.model_snapshots_published,
        latest_model_snapshot_sha256: learner_finalization.latest_snapshot_sha256,
        latest_model_revision: learner_finalization.latest_manifest.model_revision,
        latest_training_replay_rows: learner_finalization.latest_manifest.training_replay_rows,
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
    let useful_training_transitions = learner_finalization.useful_training_transitions;
    let censored_training_transitions = learner_finalization.censored_training_transitions;
    let mut report = NativeTacticRouteReport {
        schema: NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V45.into(),
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
    let report_build_probe = route_report_manifest(&report);
    serde_json::to_writer(std::io::sink(), &report_build_probe).map_err(route_error)?;
    report.timing.reporting_micros = report_build_before_shutdown_micros
        .checked_add(elapsed_micros(final_report_build_started.elapsed()))
        .ok_or_else(|| route_message("native tactic report build timing overflowed"))?;
    let report_build_micros = report.timing.reporting_micros;
    let final_artifact_persistence_started = Instant::now();
    let summary = NativeTacticCampaignSummary::build(&report, config.execution_plan)?;
    let summary_bytes = summary.to_pretty_json()?;
    let report_manifest = route_report_manifest(&report);
    let report_bytes = serde_json::to_vec_pretty(&report_manifest).map_err(route_error)?;
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

fn route_report_manifest(report: &NativeTacticRouteReport) -> NativeTacticRouteReport {
    let mut manifest = report.clone();
    for seed in &mut manifest.seeds {
        seed.trace.clear();
    }
    manifest
}

pub fn read_native_tactic_route_report(
    path: &Path,
) -> Result<NativeTacticRouteReport, NativeTacticRouteRunError> {
    let mut report: NativeTacticRouteReport = read_bounded_json(path)?;
    let campaign_root = path
        .parent()
        .ok_or_else(|| route_message("native tactic route report has no campaign directory"))?;
    for (seed_index, seed) in report.seeds.iter_mut().enumerate() {
        if seed.trace.is_empty() && seed.decisions != 0 {
            let seed_root = campaign_root.join(format!("seed-{seed_index:03}-{}", seed.seed));
            hydrate_seed_result_trace(&seed_root.join("seed-result.json"), seed)?;
        }
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

pub(super) fn supports_current_route_report_schema(schema: &str) -> bool {
    matches!(
        schema,
        NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V39
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V40
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V41
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V42
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V43
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V44
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V45
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
use macro_discovery::{
    ActiveTacticMacroLifecycle, TacticMacroSourceLane, count_active_tactic_selections,
    finalize_tactic_macro_discovery, refresh_active_tactic_macros,
    should_refresh_active_tactic_macros,
};
mod macro_policy_evidence;
use macro_policy_evidence::{
    TacticMacroPolicyEvidence, capture_tactic_macro_policy_evidence,
    publish_tactic_macro_policy_evidence,
};
mod macro_lineage_mining;
#[cfg(test)]
use macro_lineage_mining::connected_macro_candidates;
use macro_lineage_mining::mine_terminal_lineage_tactic_macro_compositions;
mod macro_discovery_report_store;
use macro_discovery_report_store::{
    NATIVE_TACTIC_MACRO_DISCOVERY_FILE, read_macro_discovery_report, write_macro_discovery_report,
};
mod macro_import;
pub use macro_import::tactic_macro_registry_identity;
use macro_import::{
    ImportedPromotedTactic, ImportedPromotedTactics, load_imported_promoted_tactics,
    merge_promoted_tactic_entries, promoted_tactic_entries,
};

mod replay_sharing;
use replay_sharing::{
    BoundedStalenessReplaySession, CrossLaneReplayCoordinator, build_replay_session,
    deterministic_generation_barrier_revision, lane_generated_training_corpus,
    publish_completed_seed_replay, publish_demonstration_replay,
};
mod replay_content;
use replay_content::{persist_evaluated_replay_content, retained_replay_components};

mod worker_pool;
use worker_pool::{
    CachedTacticFrontier, NativeTacticProposalPool, NativeTacticProposalWork,
    RetainedNativeTacticFrontiers, load_or_capture_demonstration,
    parameterized_action_surface_for_state, parameterized_catalog_for_state,
    parameterized_catalog_for_state_with_promoted, parameterized_feedback_for_state,
    run_seed_coordinator,
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
mod paired_terminal_returns;
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
    cancellation_requested, encode_seed_result_manifest, hydrate_seed_result_trace,
    load_seed_performance, read_completed_seed, read_completed_seed_preflight,
    read_completed_seed_result, resume_seed,
};
mod campaign_completion_recovery;
use campaign_completion_recovery::recover_completed_campaign;
mod completed_seed_preflight;
use completed_seed_preflight::{
    completed_seed_preflight_requires_native_fleet, load_completed_seed_preflight,
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

mod route_setup;
use route_setup::*;
pub(crate) use route_setup::{
    initial_facts, tactic_root_probe_batch, tactic_root_probe_batch_with_ticks,
};

mod goal_target;
pub use goal_target::NativeTacticGoalTargetReport;
pub(crate) use goal_target::parameterized_policy_action_schema_sha256;
pub(crate) use goal_target::{GoalConditionedTacticContext, goal_conditioned_tactic_runtime};
use goal_target::{atomic_goal_conditioned_tactic_context, planar_distance};

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
