use super::*;

#[derive(Clone, Debug)]
pub struct NativeTacticRouteRunConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub execution_plan: &'a NativeTacticExecutionPlan,
    /// Optional previously validated macro registry. Its content identity is
    /// sealed into `execution_plan`; the path is only a local artifact locator.
    pub promoted_tactic_registry: Option<&'a Path>,
    pub output_root: &'a Path,
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
    pub execution_plan_sha256: Digest,
    pub execution_plan_path: String,
    pub replay_control_plane_path: String,
    pub replay_revision: u64,
    pub replay_snapshot_sha256: Digest,
    pub replay_admission: TacticReplayAdmissionMetrics,
    pub objective_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_promoted_tactics: Option<NativeTacticImportedMacroReport>,
    pub goal_target: NativeTacticGoalTargetReport,
    pub reward_spec: TacticRewardSpec,
    pub demonstration_transitions: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub demonstration: Option<NativeTacticDemonstrationReport>,
    pub exploration_seeds: Vec<u64>,
    pub proposal_policy: TacticProposalPolicy,
    pub value_treatment: TacticValueTreatment,
    pub execution_strategy: NativeGenericExecutionStrategy,
    pub workers: usize,
    pub decisions_per_seed: u64,
    pub refit_every_decisions: u64,
    /// Seeds that retained at least one authenticated terminal candidate,
    /// regardless of whether it cleared the promotion threshold.
    pub terminal_seeds: u64,
    /// Best zero-based first-hit tick relative to the source boundary.
    pub best_authenticated_tick: Option<u64>,
    /// Seeds whose best terminal candidate cleared `promotion_before_tick`.
    pub promotion_successful_seeds: u64,
    /// Legacy alias for `promotion_successful_seeds`.
    pub successful_seeds: u64,
    pub total_native_ticks: u64,
    pub total_decisions: u64,
    pub useful_decisions: u64,
    pub learner_authority: NativeTacticLearnerAuthorityReport,
    pub learner_updates: u64,
    pub learner_updates_per_second_millionths: u64,
    pub useful_training_transitions: u64,
    pub useful_transitions_per_learner_update_millionths: u64,
    pub learned_episodes_per_generation: usize,
    pub training_replay_rows: u64,
    pub shared_training_replay_rows: u64,
    pub duplicate_training_transitions: u64,
    pub censored_training_transitions: u64,
    pub replay_sharing: NativeTacticReplaySharingTelemetry,
    pub frontier_availability: NativeTacticFrontierAvailability,
    pub native_restore_accounting: NativeTacticRestoreAccounting,
    pub tactic_macro_discovery: NativeTacticMacroDiscoveryReport,
    pub timing: NativeTacticRouteTiming,
    pub seeds: Vec<NativeTacticSeedResult>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticImportedMacroReport {
    pub registry_path: String,
    pub registry_sha256: Digest,
    pub promoted_count: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticLearnerAuthorityReport {
    pub model_snapshots_published: u64,
    pub latest_model_snapshot_sha256: Digest,
    pub latest_model_revision: u64,
    pub latest_training_replay_rows: u64,
    pub declared_model_snapshots_consumed: u64,
    pub lane_local_model_updates: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticDemonstrationReport {
    pub schema: String,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub objective_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
    pub source_boundary_index: u64,
    pub chunk_ticks: u32,
    pub transition_count: u64,
    /// Zero-based native tick of the first authenticated terminal hit,
    /// relative to the source boundary.
    pub first_hit_tick: u64,
    /// Controller inputs consumed through and including `first_hit_tick`.
    pub native_ticks: u64,
    pub wall_micros: u64,
    pub native_simulation_micros: u64,
    pub preparation_micros: u64,
    pub restore_accounting: NativeTacticRestoreAccounting,
    pub corpus_path: String,
    pub corpus_sha256: Digest,
    pub demonstrated_route_tape_sha256: Digest,
}

pub(super) struct NativeTacticDemonstration {
    pub(super) corpus: TacticQTrainingCorpus,
    pub(super) report: NativeTacticDemonstrationReport,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticMacroDiscoveryReport {
    pub observation_count: u64,
    pub high_value_observation_count: u64,
    pub mined_observation_count: u64,
    pub candidate_count: u64,
    pub entry_condition_count: u64,
    pub held_out_compatible_candidate_count: u64,
    pub source_state_exclusion_count: u64,
    pub entry_incompatible_frontier_count: u64,
    pub proposed_count: u64,
    pub promoted_count: u64,
    pub demoted_count: u64,
    pub validation_state_count: u64,
    pub comparison_count: u64,
    /// Primitive outcomes reused from the proposal batch already evaluated at
    /// the exact authenticated decision frontier.
    #[serde(default)]
    pub reused_primitive_baseline_count: u64,
    pub validation_native_ticks: u64,
    pub validation_wall_micros: u64,
    pub validation_native_simulation_micros: u64,
    pub validation_preparation_micros: u64,
    pub validation_restore_accounting: NativeTacticRestoreAccounting,
    pub reuse: Option<NativeTacticMacroReuseReport>,
    pub registry_path: String,
    pub registry_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticMacroReuseReport {
    pub candidate_sha256: Digest,
    pub option_id: String,
    pub promotion_registry_sha256: Digest,
    pub seed: u64,
    pub source_state_sha256: Digest,
    pub held_out_from_promotion_states: bool,
    pub realized_ticks: u32,
    pub goal_progress: f32,
    pub terminal: bool,
    pub after_state_sha256: Digest,
    pub emitted_tape_sha256: Digest,
    pub complete_route_tape_sha256: Digest,
    pub complete_route_tape_path: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticFrontierAvailability {
    pub logical_frontier_records: usize,
    pub directly_restorable_native_frontiers: usize,
    pub replay_only_frontiers: usize,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticReplaySharingTelemetry {
    pub refreshes: u64,
    pub imported_rows: u64,
    pub maximum_observed_stale_revisions: u64,
}

impl NativeTacticReplaySharingTelemetry {
    pub(super) fn merge(&mut self, other: Self) {
        self.refreshes = self.refreshes.saturating_add(other.refreshes);
        self.imported_rows = self.imported_rows.saturating_add(other.imported_rows);
        self.maximum_observed_stale_revisions = self
            .maximum_observed_stale_revisions
            .max(other.maximum_observed_stale_revisions);
    }
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
    /// Legacy aggregate retained for report and resume compatibility.
    pub evidence_projection_and_persistence_micros: u64,
    #[serde(default)]
    pub evidence_projection_micros: u64,
    #[serde(default)]
    pub persistence_micros: u64,
    /// Active coordinator work outside learner updates, evidence projection,
    /// persistence, and time blocked on native workers.
    #[serde(default)]
    pub orchestration_micros: u64,
    #[serde(default)]
    pub result_validation_and_fact_extraction_micros: u64,
    #[serde(default)]
    pub campaign_admission_micros: u64,
    /// Candidate-only artifact generation after native terminal admission.
    /// Exploration-only seeds keep this at zero; cold replay is a separate
    /// explicit command and is never charged here.
    #[serde(default)]
    pub retained_candidate_artifact_micros: u64,
    pub useful_decisions_per_second_millionths: u64,
    pub native_ticks_per_second_millionths: u64,
    pub episodes_per_second_millionths: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticRestoreAccounting {
    /// Validated native suffix requests, including reactive tactic steps.
    pub native_requests: u64,
    pub authenticated_root_restore_requests: u64,
    pub direct_process_local_restore_requests: u64,
    #[serde(default)]
    pub direct_process_local_continuation_requests: u64,
    /// Direct requests whose owner-local handle had been evicted and were
    /// reconstructed by exact authenticated-root replay.
    #[serde(default)]
    pub direct_restore_fallback_replays: u64,
    /// Root replays performed solely to retain a logical non-root frontier.
    pub prefix_materializations: u64,
    pub replayed_prefix_ticks: u64,
    /// Native restore timing samples. A request may report more than one
    /// internal restore, so this is intentionally distinct from request count.
    pub restore_samples: u64,
    pub restore_micros: u64,
    pub mean_restore_micros: u64,
    pub direct_restore_request_rate_per_million: u64,
    /// Deltas from the persistent native cache's own counters. A direct
    /// request may perform multiple internal lookups.
    pub cache_hits: u64,
    pub cache_misses: u64,
    #[serde(default)]
    pub cache_evictions: u64,
    pub cache_hit_rate_per_million: u64,
    pub checkpoint_capture_attempts: u64,
    pub checkpoint_capture_successes: u64,
    pub checkpoint_capture_micros: u64,
    #[serde(default)]
    pub live_endpoint_retention_attempts: u64,
    #[serde(default)]
    pub live_endpoint_retention_successes: u64,
    #[serde(default)]
    pub live_endpoint_retention_nanos: u64,
    pub peak_resident_entries: u64,
    pub peak_resident_bytes: u64,
    pub peak_resident_checkpoint_bytes: u64,
    pub peak_resident_host_snapshot_bytes: u64,
    #[serde(default)]
    pub peak_live_endpoint_entries: u64,
    #[serde(default)]
    pub peak_live_endpoint_host_snapshot_bytes: u64,
    /// Logical proposal outcomes admitted to batch evaluation. A transition is
    /// useful when it reaches terminal, has positive shaped reward, or reduces
    /// goal distance from its shared source.
    pub proposal_transitions: u64,
    pub useful_transitions: u64,
    pub useful_transitions_per_restore_millionths: u64,
}

impl NativeTacticRestoreAccounting {
    pub(super) fn merge(&mut self, other: &Self) {
        self.native_requests = self.native_requests.saturating_add(other.native_requests);
        self.authenticated_root_restore_requests = self
            .authenticated_root_restore_requests
            .saturating_add(other.authenticated_root_restore_requests);
        self.direct_process_local_restore_requests = self
            .direct_process_local_restore_requests
            .saturating_add(other.direct_process_local_restore_requests);
        self.direct_process_local_continuation_requests = self
            .direct_process_local_continuation_requests
            .saturating_add(other.direct_process_local_continuation_requests);
        self.direct_restore_fallback_replays = self
            .direct_restore_fallback_replays
            .saturating_add(other.direct_restore_fallback_replays);
        self.prefix_materializations = self
            .prefix_materializations
            .saturating_add(other.prefix_materializations);
        self.replayed_prefix_ticks = self
            .replayed_prefix_ticks
            .saturating_add(other.replayed_prefix_ticks);
        self.restore_samples = self.restore_samples.saturating_add(other.restore_samples);
        self.restore_micros = self.restore_micros.saturating_add(other.restore_micros);
        self.cache_hits = self.cache_hits.saturating_add(other.cache_hits);
        self.cache_misses = self.cache_misses.saturating_add(other.cache_misses);
        self.cache_evictions = self.cache_evictions.saturating_add(other.cache_evictions);
        self.checkpoint_capture_attempts = self
            .checkpoint_capture_attempts
            .saturating_add(other.checkpoint_capture_attempts);
        self.checkpoint_capture_successes = self
            .checkpoint_capture_successes
            .saturating_add(other.checkpoint_capture_successes);
        self.checkpoint_capture_micros = self
            .checkpoint_capture_micros
            .saturating_add(other.checkpoint_capture_micros);
        self.live_endpoint_retention_attempts = self
            .live_endpoint_retention_attempts
            .saturating_add(other.live_endpoint_retention_attempts);
        self.live_endpoint_retention_successes = self
            .live_endpoint_retention_successes
            .saturating_add(other.live_endpoint_retention_successes);
        self.live_endpoint_retention_nanos = self
            .live_endpoint_retention_nanos
            .saturating_add(other.live_endpoint_retention_nanos);
        self.peak_resident_entries = self.peak_resident_entries.max(other.peak_resident_entries);
        self.peak_resident_bytes = self.peak_resident_bytes.max(other.peak_resident_bytes);
        self.peak_resident_checkpoint_bytes = self
            .peak_resident_checkpoint_bytes
            .max(other.peak_resident_checkpoint_bytes);
        self.peak_resident_host_snapshot_bytes = self
            .peak_resident_host_snapshot_bytes
            .max(other.peak_resident_host_snapshot_bytes);
        self.peak_live_endpoint_entries = self
            .peak_live_endpoint_entries
            .max(other.peak_live_endpoint_entries);
        self.peak_live_endpoint_host_snapshot_bytes = self
            .peak_live_endpoint_host_snapshot_bytes
            .max(other.peak_live_endpoint_host_snapshot_bytes);
        self.proposal_transitions = self
            .proposal_transitions
            .saturating_add(other.proposal_transitions);
        self.useful_transitions = self
            .useful_transitions
            .saturating_add(other.useful_transitions);
        self.refresh_rates();
    }

    pub(super) fn refresh_rates(&mut self) {
        self.mean_restore_micros = self
            .restore_micros
            .checked_div(self.restore_samples)
            .unwrap_or(0);
        self.direct_restore_request_rate_per_million = ratio_per_million(
            self.direct_process_local_restore_requests
                .saturating_add(self.direct_process_local_continuation_requests),
            self.native_requests,
        );
        self.cache_hit_rate_per_million = ratio_per_million(
            self.cache_hits,
            self.cache_hits.saturating_add(self.cache_misses),
        );
        self.useful_transitions_per_restore_millionths =
            ratio_per_million(self.useful_transitions, self.restore_samples);
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeTacticSeedPerformance {
    pub(super) schema: String,
    pub(super) decisions: u64,
    pub(super) useful_decisions: u64,
    pub(super) native_restore_accounting: NativeTacticRestoreAccounting,
    pub(super) timing: NativeTacticRouteTiming,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticSeedResult {
    #[serde(default)]
    pub execution_plan_sha256: Digest,
    pub seed: u64,
    /// Whether any evaluated proposal reached the authenticated terminal.
    #[serde(default)]
    pub terminal_discovered: bool,
    /// Best zero-based terminal first-hit tick relative to the source.
    #[serde(default)]
    pub best_authenticated_tick: Option<u64>,
    pub success: bool,
    pub decisions: u64,
    pub episodes: u64,
    pub native_ticks: u64,
    pub replay_rows: usize,
    #[serde(default)]
    pub training_replay_rows: usize,
    #[serde(default)]
    pub imported_training_replay_rows: usize,
    #[serde(default)]
    pub duplicate_training_transitions: u64,
    #[serde(default)]
    pub censored_training_transitions: u64,
    #[serde(default)]
    pub learner_updates: u64,
    #[serde(default)]
    pub replay_sharing: NativeTacticReplaySharingTelemetry,
    pub visited_states: usize,
    #[serde(default)]
    pub useful_decisions: u64,
    pub native_restore_accounting: NativeTacticRestoreAccounting,
    #[serde(default)]
    pub timing: NativeTacticRouteTiming,
    pub selection_counts: BTreeMap<String, u64>,
    pub diagnostics: Option<TacticCampaignDiagnostics>,
    /// Durable binary campaign checkpoint containing the authoritative graph.
    pub final_checkpoint: String,
    /// Content identity of the graph embedded in `final_checkpoint`.
    pub state_graph_sha256: Digest,
    /// Exact best terminal node selected by the graph, when one exists.
    pub best_terminal_state_sha256: Option<Digest>,
    /// Exact route-checkpoint identity for the graph-selected terminal.
    pub best_terminal_route_checkpoint_sha256: Option<Digest>,
    #[serde(default)]
    pub best_terminal_tape: Option<String>,
    #[serde(default)]
    pub best_terminal_result: Option<String>,
    pub successful_tape: Option<String>,
    pub final_result: Option<String>,
    pub trace: Vec<NativeTacticDecisionTrace>,
}

pub(super) struct CompletedNativeTacticSeed {
    pub(super) result: NativeTacticSeedResult,
    pub(super) generated_training: TacticQTrainingCorpus,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticDecisionTrace {
    #[serde(default)]
    pub execution_plan_sha256: Digest,
    pub decision_index: u64,
    #[serde(default)]
    pub learner_snapshot_sha256: Digest,
    #[serde(default)]
    pub replay_rows_at_decision: u64,
    #[serde(default)]
    pub replay_generation: u64,
    #[serde(default)]
    pub lane_index: usize,
    #[serde(default)]
    pub lane_role: Option<NativeTacticLaneRole>,
    #[serde(default)]
    pub acquisition_rank: u64,
    #[serde(default)]
    pub frontier_identity: Digest,
    #[serde(default)]
    pub restore_source: Option<NativeTacticRestoreSource>,
    #[serde(default)]
    pub result_admission_schema: String,
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
    #[serde(default)]
    pub newly_admitted_training_rows: u64,
    #[serde(default)]
    pub duplicate_training_transitions: u64,
    #[serde(default)]
    pub training_replay_rows: u64,
    #[serde(default)]
    pub branch_acquisition: Option<TacticFrontierAcquisition>,
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
    pub proposal_feedback: Option<ParameterizedTacticFeedback>,
    #[serde(default)]
    pub proposal_batch: Vec<NativeTacticProposalTrace>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTacticRestoreSource {
    AuthenticatedRoot,
    AuthenticatedRootReplay,
    ProcessLocalCheckpoint,
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
    #[serde(default)]
    pub execution_plan_sha256: Digest,
    pub option_id: String,
    pub selection_reason: TacticSelectionReason,
    pub reward: f32,
    pub reward_components: TacticRewardBreakdown,
    pub realized_ticks: u32,
    #[serde(default)]
    pub emitted_tape_sha256: Digest,
    pub terminal: bool,
    pub goal_distance_after: f32,
    pub after_snapshot_sha256: Digest,
    pub retained: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeTacticProposalRecord {
    pub(super) trace: NativeTacticProposalTrace,
    /// Legacy content-store reference retained for exact replay of existing
    /// journals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) transition: Option<StoredContentRef>,
    /// New journals keep the compact learning row inside their already
    /// authenticated, compressed decision segment. This avoids five durable
    /// tiny-file installs per proposal on the hot path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) inline_transition: Option<OptionTransitionSample>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeTacticDecisionRecord {
    #[serde(default)]
    pub(super) execution_plan_sha256: Digest,
    pub(super) decision_index: u64,
    #[serde(default)]
    pub(super) learner_snapshot_sha256: Digest,
    #[serde(default)]
    pub(super) replay_rows_at_decision: u64,
    #[serde(default)]
    pub(super) replay_generation: u64,
    #[serde(default)]
    pub(super) lane_index: usize,
    #[serde(default)]
    pub(super) lane_role: Option<NativeTacticLaneRole>,
    #[serde(default)]
    pub(super) acquisition_rank: u64,
    #[serde(default)]
    pub(super) frontier_identity: Digest,
    #[serde(default)]
    pub(super) restore_source: Option<NativeTacticRestoreSource>,
    #[serde(default)]
    pub(super) result_admission_schema: String,
    pub(super) episode: u64,
    pub(super) episode_group: u64,
    pub(super) route_suffix_ticks: u64,
    pub(super) selection_reason: TacticSelectionReason,
    pub(super) selected_q: Option<f64>,
    pub(super) best_q: Option<f64>,
    pub(super) reward: f32,
    pub(super) reward_components: TacticRewardBreakdown,
    pub(super) goal_distance_before: f32,
    pub(super) goal_distance_after: f32,
    pub(super) terminal: bool,
    #[serde(default)]
    pub(super) newly_admitted_training_rows: u64,
    #[serde(default)]
    pub(super) duplicate_training_transitions: u64,
    #[serde(default)]
    pub(super) training_replay_rows: u64,
    #[serde(default)]
    pub(super) branch_acquisition: Option<TacticFrontierAcquisition>,
    pub(super) frontier_cells: usize,
    #[serde(default)]
    pub(super) logical_frontier_records: usize,
    #[serde(default)]
    pub(super) directly_restorable_native_frontiers: usize,
    #[serde(default)]
    pub(super) replay_only_frontiers: usize,
    pub(super) visited_states: usize,
    pub(super) root_checkpoint_sha256: Digest,
    pub(super) root_tape: StoredContentRef,
    /// Exact route at this decision's source boundary. Cross-seed replay can
    /// restore a root-derived frontier whose parent edge is not in this seed's
    /// local journal; this content-addressed anchor keeps that valid branch
    /// independently materializable. Legacy journals reconstruct local chains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) source_route_tape: Option<StoredContentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) transition: Option<StoredContentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) inline_transition: Option<OptionTransitionSample>,
    #[serde(default)]
    pub(super) proposal_feedback: Option<ParameterizedTacticFeedback>,
    #[serde(default)]
    pub(super) proposal_batch: Vec<NativeTacticProposalRecord>,
}
