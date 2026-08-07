//! Online option-Q campaign over authenticated learner states and native tactic
//! boundaries.

use self::graph_projection::{
    graph_frontier_entries_validated, graph_root_branch, validate_training_projection_and_keys,
};
pub(crate) use self::graph_projection::{
    graph_training_projection, graph_training_projection_rows, graph_training_projection_validated,
    merge_graph_training_projection, validate_graph_training_projection_merge,
};
use crate::native_tactic_worker::{
    NativeTacticWorkerError, NativeTacticWorkerOutcome, NativeTacticWorkerPaths,
    PersistentTacticBatchWorker, execute_selected_tactic,
};
use crate::state_graph::{
    StateGraph, StateGraphError, StateGraphIdentity, StateGraphValidationToken, ValidatedStateGraph,
};
use crate::tactic_q_checkpoint_store;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_control::option_execution::OptionExecution;
use dusklight_learning::fact_registry::FactRegistry;
use dusklight_learning::fact_snapshot::{FACT_SNAPSHOT_SCHEMA_V2, FactSnapshot};
use dusklight_learning::generalized_tactic_value::authenticated_terminal_conditional_returns;
use dusklight_learning::generalized_tactic_value::{
    GeneralizedTacticContext, GeneralizedTacticValueError, GeneralizedTacticValueModel,
};
use dusklight_learning::goal_reachability_calibration::GoalReachabilityCalibration;
use dusklight_learning::hindsight::{
    HindsightError, HindsightOptionReplay, RelabeledHindsightOption,
};
use dusklight_learning::learner_state::{
    LearnerActionMaskEntry, LearnerState, LearnerStateError, tactic_intrinsically_applicable,
};
use dusklight_learning::live_tactic_catalog::{
    LiveTacticCatalog, LiveTacticCatalogError, LiveTacticRanking,
};
use dusklight_learning::option_transition::{
    AuthenticatedOptionTransition, OptionTransitionError, OptionTransitionSample,
};
use dusklight_learning::option_values::{
    AvailableOptionRanking, MAX_OPTION_ACTIONS, OptionActionDescriptor, OptionValueBatch,
    OptionValueConfig, OptionValueError, OptionValueModel,
};
use dusklight_learning::reward_shaping::{ShapingError, TacticRewardBreakdown, TacticRewardSpec};
use dusklight_learning::tactic_asset::{TacticAssetCatalog, TacticAssetDescription};
use dusklight_learning::tactic_blueprint::{
    ApplicableTacticChoices, ConcreteTacticChoiceKind, TacticBlueprint,
};
use dusklight_learning::tactic_exploration::{
    SelectedTactic, TACTIC_EXPLORATION_SCHEMA_V1, TacticExplorationConfig, TacticExplorationError,
    TacticProposalPolicy, TacticSelectionReason, choose_tactic_batch_for_policy,
    choose_tactic_batch_with_state_untried, ensure_action_factor_coverage,
    ensure_generalized_value_acquisition, ensure_goal_reachability_acquisition,
    ensure_terminal_support_factor_acquisitions, retain_generalized_value_acquisition,
    retain_goal_reachability_acquisition,
};
use dusklight_learning::tactic_frozen_policy::{TacticFrozenPolicy, TacticFrozenPolicyError};
use dusklight_learning::tactic_value_treatment::{
    ContinuousTacticDoubleQModel, ContinuousTacticValueModel, TacticValueTreatment,
};
use dusklight_learning::terminal_action_calibration::TerminalActionCalibration;
use dusklight_proposals::behavior_archive::{
    BehaviorArchive, TacticEndpointDescriptor, TacticFrontierEntry, TacticStateDescriptor,
    tactic_endpoint_descriptor_for_state, tactic_state_descriptor,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const TACTIC_Q_CAMPAIGN_SCHEMA_V1: &str = "dusklight-tactic-q-campaign/v1";
pub const TACTIC_Q_CHECKPOINT_SCHEMA_V5: &str = "dusklight-tactic-q-checkpoint/v5";
pub const TACTIC_Q_CHECKPOINT_SCHEMA_V6: &str = "dusklight-tactic-q-checkpoint/v6";
pub const TACTIC_Q_CHECKPOINT_EXTENSION: &str = "dtqz";
pub const TACTIC_Q_CHECKPOINT_SERIALIZATION_BENCHMARK_SCHEMA_V1: &str =
    "dusklight-tactic-q-checkpoint-serialization-benchmark/v1";
pub const TACTIC_Q_FINAL_RESULT_SCHEMA_V2: &str = "dusklight-tactic-q-final-result/v2";
pub const TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V1: &str = "dusklight-tactic-q-learner-snapshot/v1";
pub const TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V2: &str = "dusklight-tactic-q-learner-snapshot/v2";
pub const TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V3: &str = "dusklight-tactic-q-learner-snapshot/v3";
pub const TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V4: &str = "dusklight-tactic-q-learner-snapshot/v4";
pub const TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V5: &str = "dusklight-tactic-q-learner-snapshot/v5";
/// Episode group reserved for critic evidence that must never become an
/// executable frontier.
pub const TACTIC_Q_MODEL_ONLY_EPISODE_GROUP: u64 = u64::MAX;
/// Episode group reserved for an authenticated demonstration trajectory.
///
/// Demonstration actions remain ordinary off-policy evidence, but their
/// nonterminal endpoints are valid curriculum frontiers: the native worker can
/// replay the exact prefix, restore that state, and evaluate different
/// executable tactics from it.
pub const TACTIC_Q_DEMONSTRATION_EPISODE_GROUP: u64 = u64::MAX - 1;
const MAX_RANKED_FRONTIER_CANDIDATES: usize = 16;
fn digest_is_zero(value: &Digest) -> bool {
    *value == Digest::ZERO
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticQDecision {
    pub ranking: LiveTacticRanking,
    pub selected: SelectedTactic,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticQProposalBatch {
    pub ranking: LiveTacticRanking,
    pub proposals: Vec<SelectedTactic>,
    pub goal_reachability_estimates: Vec<TacticQGoalReachabilityEstimate>,
    pub goal_reachability_calibration: Option<GoalReachabilityCalibration>,
    pub terminal_action_calibration: Option<TerminalActionCalibration>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticQGoalReachabilityEstimate {
    pub descriptor: OptionActionDescriptor,
    pub predicted_goal_progress_per_tick: f32,
    pub nearest_distance: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatedRewardedTacticOutcome {
    pub outcome: NativeTacticWorkerOutcome,
    pub transition: AuthenticatedOptionTransition,
    pub reward: TacticRewardBreakdown,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticQCampaignStep {
    pub decision: TacticQDecision,
    pub reward: f32,
    pub replay_rows: usize,
    pub transition: OptionTransitionSample,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RewardedTacticQCampaignStep {
    pub step: TacticQCampaignStep,
    pub reward: TacticRewardBreakdown,
}

/// Crash-safe resume state. The fitted Q model is intentionally absent and is
/// reconstructed from `replay` after every load.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticQCampaignCheckpoint {
    pub schema: String,
    pub content_sha256: Digest,
    #[serde(default, skip_serializing_if = "digest_is_zero")]
    pub execution_authority_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub objective_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
    pub episode_group: u64,
    pub decision_index: u64,
    pub current: LearnerState,
    pub route_tape: InputTape,
    pub state_graph: StateGraph,
    pub replay: Vec<OptionTransitionSample>,
    pub replay_routes: Vec<InputTape>,
    pub episode_groups: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub training_replay: Vec<OptionTransitionSample>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub training_replay_routes: Vec<InputTape>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub training_episode_groups: Vec<u64>,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub model_revision: u64,
    pub model_config: OptionValueConfig,
    pub exploration: TacticExplorationConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistence: Option<TacticQCheckpointPersistence>,
    #[serde(skip)]
    pub(crate) persistence_validated: bool,
}

pub const TACTIC_Q_CHECKPOINT_PERSISTENCE_SCHEMA_V1: &str =
    "dusklight-tactic-q-checkpoint-persistence/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticQCheckpointPersistence {
    pub schema: String,
    pub state_graph_head_sha256: Digest,
    pub state_graph_depth: u64,
    pub replay_index_sha256: Digest,
    pub replay_rows: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticBranchKind {
    Root,
    RetainedFrontier,
}

/// Portable identity of a frontier that can always be reconstructed by
/// restoring the authenticated native root and replaying `replayed_prefix_ticks`.
/// This is not a native emulator checkpoint handle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogicalTacticFrontierRecord {
    pub identity_sha256: Digest,
    pub state_sha256: Digest,
    pub route_frames: u64,
    pub replayed_prefix_ticks: u64,
}

/// Process-local native checkpoint authority. Unlike a logical frontier
/// digest, this record names a concrete worker restore handle and accounts for
/// the native memory it owns.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RestorableNativeTacticCheckpoint {
    pub worker_slot: usize,
    pub native_source_sha256: Digest,
    pub logical_frontier_sha256: Digest,
    pub state_sha256: Digest,
    pub restore_identity: String,
    pub checkpoint_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticCampaignBranch {
    pub kind: TacticBranchKind,
    pub logical_frontier: LogicalTacticFrontierRecord,
    pub restorable_native_checkpoint: Option<RestorableNativeTacticCheckpoint>,
    pub acquisition: Option<TacticFrontierAcquisition>,
    pub state: FactSnapshot,
    pub route_tape: InputTape,
    pub descriptor: Option<TacticEndpointDescriptor>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticFrontierAcquisition {
    pub expansion_count: u64,
    pub terminal: bool,
    /// True once authenticated replay contains terminal supervision. Before
    /// then, sparse return only measures action cost and must not masquerade
    /// as evidence that a frontier leads to the objective.
    #[serde(default)]
    pub terminal_value_supported: bool,
    /// Legacy checkpoint field for the former achieved-goal return ordering.
    /// New campaigns keep this false because arbitrary replay endpoints do not
    /// confer objective-value authority.
    #[serde(default)]
    pub achieved_goal_value_supported: bool,
    /// True when learned target-relative physical progress can prioritize
    /// exploration. This is not authored-objective value support.
    #[serde(default)]
    pub goal_reachability_supported: bool,
    /// True when the model produced a frontier prediction, even if held-out
    /// calibration denied that prediction policy authority.
    #[serde(default)]
    pub goal_reachability_evidence_available: bool,
    pub reward: f32,
    pub best_mean_q: Option<f64>,
    /// Best learned target-relative distance reduction per native tick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_goal_progress_per_tick: Option<f64>,
    /// Learned terminal-supported cost from the frontier through first hit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicted_terminal_ticks_to_go: Option<f64>,
    /// Root-relative route cost: replayed prefix plus learned ticks-to-go.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicted_total_terminal_ticks: Option<f64>,
    /// Exact authenticated ticks from this retained state through a replay
    /// path whose final transition reaches the native terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_terminal_ticks_to_go: Option<u64>,
    /// Root-relative cost of the retained prefix followed by the shortest
    /// exact authenticated terminal suffix currently in replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_total_terminal_ticks: Option<u64>,
    pub maximum_ensemble_variance: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generalized_nearest_distance: Option<f32>,
    /// Squared quantized distance from this node to the nearest graph node
    /// that already owned expansion work during preterminal discovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_spatial_novelty: Option<u128>,
    pub novelty_rank: u64,
    pub replayed_prefix_ticks: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticCampaignGraph {
    pub schema: String,
    pub root_checkpoint_sha256: Digest,
    pub root_state_sha256: Digest,
    pub root_connected: bool,
    pub nodes: Vec<TacticCampaignGraphNode>,
    pub edges: Vec<TacticCampaignGraphEdge>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticCampaignGraphNode {
    pub checkpoint_sha256: Digest,
    pub state_sha256: Digest,
    pub state: FactSnapshot,
    pub route_tape: InputTape,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticCampaignGraphEdge {
    pub episode_group: u64,
    pub before_state_sha256: Digest,
    pub after_state_sha256: Digest,
    pub source_checkpoint_sha256: Digest,
    pub next_checkpoint_sha256: Digest,
    pub action: OptionActionDescriptor,
    pub execution: OptionExecution,
    pub reward: f32,
    pub terminal: bool,
    pub route_tape: InputTape,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticCampaignGraphProjection {
    pub schema: String,
    pub root_checkpoint_sha256: Digest,
    pub root_state_sha256: Digest,
    pub root_connected: bool,
    pub frontier_cells: usize,
    pub nodes: Vec<TacticCampaignGraphProjectionNode>,
    pub edges: Vec<TacticCampaignGraphProjectionEdge>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticCampaignGraphProjectionNode {
    pub checkpoint_sha256: Digest,
    pub state_sha256: Digest,
    pub stage: String,
    pub room: i8,
    pub player_position: [f32; 3],
    pub terminal: bool,
    pub retained_frontier: bool,
    pub current: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticCampaignGraphProjectionEdge {
    pub edge_index: u64,
    pub episode_group: u64,
    pub before_state_sha256: Digest,
    pub after_state_sha256: Digest,
    pub source_checkpoint_sha256: Digest,
    pub next_checkpoint_sha256: Digest,
    pub option_id: String,
    pub reward: f32,
    pub duration_ticks: u32,
    pub terminal: bool,
    pub start_frame: u64,
    pub end_frame_exclusive: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticCampaignDiagnostics {
    pub replay_rows: usize,
    pub frontier_cells: usize,
    #[serde(default)]
    pub logical_frontier_records: usize,
    #[serde(default)]
    pub directly_restorable_native_frontiers: usize,
    #[serde(default)]
    pub replay_only_frontiers: usize,
    pub unique_selected_actions: usize,
    pub zero_diversity_selection: bool,
    pub repeated_identical_compositions: bool,
    pub no_progress_loop: bool,
    pub frontier_lost_root_connectivity: bool,
}

/// Mutable state for one connected tactic episode.
///
/// The model is deliberately transient: authenticated fact snapshots, exact
/// option executions, and their route tapes are the source of truth from which
/// every refit is rebuilt.
#[derive(Debug)]
pub struct TacticQCampaign {
    pub schema: String,
    pub execution_authority_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub objective_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
    pub episode_group: u64,
    pub decision_index: u64,
    pub current: LearnerState,
    pub route_tape: InputTape,
    state_graph: Option<StateGraph>,
    // Process-local only. Checkpoint load earns a fresh token through complete
    // graph validation; checked transactional mutations preserve it.
    state_graph_validation: Option<crate::state_graph::StateGraphValidationToken>,
    // The currently retained cursor lineage, kept for final-route export.
    // It is not consulted to decide which exact states or terminal paths exist.
    replay: Vec<OptionTransitionSample>,
    replay_routes: Vec<InputTape>,
    episode_groups: Vec<u64>,
    // Read-only learner publication caches. Every production mutation replaces
    // all three from `state_graph`; checkpoint validation rejects drift.
    training_replay: Vec<OptionTransitionSample>,
    training_replay_routes: Vec<InputTape>,
    training_episode_groups: Vec<u64>,
    training_projection_keys: Vec<(Digest, Digest)>,
    // Scheduler indexes only. Neither collection grants exact-state or
    // terminal authority; that belongs to `state_graph`.
    frontier_archive: BehaviorArchive,
    model_config: OptionValueConfig,
    exploration: TacticExplorationConfig,
    model: Option<Arc<OptionValueModel>>,
    model_revision: u64,
    campaign_learner_authority_managed: bool,
    value_treatment: TacticValueTreatment,
    generalized_model: RefCell<Option<CachedGeneralizedTacticValueModel>>,
    native_terminal_model: RefCell<Option<CachedGeneralizedTacticValueModel>>,
    native_terminal_action_model: RefCell<Option<CachedContinuousTacticDoubleQModel>>,
    continuous_model: RefCell<Option<CachedContinuousTacticValueModel>>,
    goal_reachability_calibration: Option<GoalReachabilityCalibration>,
    terminal_action_calibration: Option<TerminalActionCalibration>,
    visited_states: BTreeSet<TacticStateDescriptor>,
    hindsight: HindsightOptionReplay,
    checkpoint_persistence: Option<TacticQCheckpointPersistence>,
}

/// In-memory training evidence shared across independent tactic episodes.
///
/// Executable episode lineage remains local to each campaign. This corpus
/// carries only authenticated transition rows and their exact controller
/// routes so a later episode can fit from earlier native trials without
/// pretending those trials belong to its retained path.
#[derive(Clone, Debug, PartialEq)]
pub struct TacticQTrainingCorpus {
    pub execution_authority_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub objective_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
    pub transitions: Vec<OptionTransitionSample>,
    pub routes: Vec<InputTape>,
    pub episode_groups: Vec<u64>,
}

impl TacticQTrainingCorpus {
    /// Writes only this corpus's authenticated rows and route references. This
    /// is the durable completed-episode handoff; it intentionally excludes the
    /// campaign's inherited history and executable checkpoint state.
    pub fn write(&self, path: &Path, content_root: &Path) -> Result<(), TacticQCampaignError> {
        tactic_q_checkpoint_store::write_training_corpus(self, path, content_root)
    }

    pub fn read(path: &Path) -> Result<Self, TacticQCampaignError> {
        tactic_q_checkpoint_store::read_training_corpus(path)
    }
}

fn consider_frontier_transition(
    archive: &mut BehaviorArchive,
    root_checkpoint_sha256: Digest,
    transition: &OptionTransitionSample,
    route: &InputTape,
    episode_group: u64,
    generation: usize,
) -> Result<(), TacticQCampaignError> {
    // Model-only evidence teaches return without granting the policy a route
    // to replay. Terminal endpoints are leaves; refinement must branch from
    // their source state, never execute beyond the goal.
    if episode_group != TACTIC_Q_MODEL_ONLY_EPISODE_GROUP && !transition.value_sample.terminal {
        archive
            .consider_tactic_endpoint(
                root_checkpoint_sha256,
                transition.clone(),
                route.clone(),
                generation as u64,
            )
            .map_err(|error| TacticQCampaignError::Frontier(error.to_string()))?;
    }
    Ok(())
}

fn build_frontier_archive(
    root_checkpoint_sha256: Digest,
    transitions: &[OptionTransitionSample],
    routes: &[InputTape],
    episode_groups: &[u64],
) -> Result<BehaviorArchive, TacticQCampaignError> {
    let mut archive = BehaviorArchive::default();
    for (generation, ((transition, route), episode_group)) in transitions
        .iter()
        .zip(routes)
        .zip(episode_groups)
        .enumerate()
    {
        consider_frontier_transition(
            &mut archive,
            root_checkpoint_sha256,
            transition,
            route,
            *episode_group,
            generation,
        )?;
    }
    Ok(archive)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticQCheckpointSerializationBenchmark {
    pub schema: String,
    pub iterations: u64,
    pub decision_index: u64,
    pub replay_transitions: u64,
    pub legacy_json_bytes_per_iteration: u64,
    pub current_manifest_envelope_bytes_per_iteration: u64,
    pub legacy_json_serialization_total_nanos: u64,
    pub current_manifest_serialization_total_nanos: u64,
}

impl TacticQCampaign {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        feature_schema_sha256: Digest,
        objective_sha256: Digest,
        root_checkpoint_sha256: Digest,
        episode_group: u64,
        current: LearnerState,
        route_tape: InputTape,
        model_config: OptionValueConfig,
        exploration: TacticExplorationConfig,
    ) -> Result<Self, TacticQCampaignError> {
        current.validate()?;
        route_tape
            .validate()
            .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
        if feature_schema_sha256 == Digest::ZERO
            || objective_sha256 == Digest::ZERO
            || root_checkpoint_sha256 == Digest::ZERO
            || current.snapshot.tape_frame != route_tape.frames.len() as u64
        {
            return Err(TacticQCampaignError::InvalidState(
                "campaign identity or initial route is invalid",
            ));
        }
        let visited_states = BTreeSet::from([tactic_state_descriptor(
            &current.snapshot,
            current.snapshot.terminal.reached == Some(true),
        )]);
        let hindsight = HindsightOptionReplay::new(feature_schema_sha256)
            .map_err(TacticQCampaignError::Hindsight)?;
        Ok(Self {
            schema: TACTIC_Q_CAMPAIGN_SCHEMA_V1.into(),
            execution_authority_sha256: Digest::ZERO,
            feature_schema_sha256,
            objective_sha256,
            root_checkpoint_sha256,
            episode_group,
            decision_index: 0,
            current,
            route_tape,
            state_graph: None,
            state_graph_validation: None,
            replay: Vec::new(),
            replay_routes: Vec::new(),
            episode_groups: Vec::new(),
            training_replay: Vec::new(),
            training_replay_routes: Vec::new(),
            training_episode_groups: Vec::new(),
            training_projection_keys: Vec::new(),
            frontier_archive: BehaviorArchive::default(),
            model_config,
            exploration,
            model: None,
            model_revision: 0,
            campaign_learner_authority_managed: false,
            value_treatment: TacticValueTreatment::LocalGeneralizedFittedQKnnV1,
            generalized_model: RefCell::new(None),
            native_terminal_model: RefCell::new(None),
            native_terminal_action_model: RefCell::new(None),
            continuous_model: RefCell::new(None),
            goal_reachability_calibration: None,
            terminal_action_calibration: None,
            visited_states,
            hindsight,
            checkpoint_persistence: None,
        })
    }

    pub fn model(&self) -> Option<&OptionValueModel> {
        self.model.as_deref()
    }

    pub fn model_revision(&self) -> u64 {
        self.model_revision
    }

    pub fn learner_snapshot(&self) -> Result<TacticQLearnerSnapshot, TacticQCampaignError> {
        let model_sha256 = self
            .model
            .as_ref()
            .map(|model| {
                serde_cbor::to_vec(model.as_ref())
                    .map(|raw| sha256(&raw))
                    .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))
            })
            .transpose()?;
        let snapshot = TacticQLearnerSnapshot {
            schema: TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V5.into(),
            kind: TacticQLearnerSnapshotKind::Learned,
            value_treatment: self.value_treatment,
            execution_authority_sha256: self.execution_authority_sha256,
            feature_schema_sha256: self.feature_schema_sha256,
            objective_sha256: self.objective_sha256,
            root_checkpoint_sha256: self.root_checkpoint_sha256,
            training_replay_rows: self.training_replay.len() as u64,
            training_replay_sha256: training_replay_sha256(
                &self.training_replay,
                &self.training_episode_groups,
            )?,
            model_revision: self.model_revision,
            model_config: self.model_config.clone(),
            model_sha256,
            goal_reachability_calibration: self.goal_reachability_calibration.clone(),
            terminal_action_calibration: self.terminal_action_calibration.clone(),
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Consume a campaign-published immutable policy and its authenticated
    /// evidence without fitting either model in this lane.
    pub fn consume_learner_snapshot(
        &mut self,
        snapshot: &TacticQImmutableLearnerSnapshot,
    ) -> Result<usize, TacticQCampaignError> {
        self.consume_learner_snapshot_with_exploration_filter(snapshot, |_| true)
    }

    /// Consume globally shared value evidence while retaining only selected
    /// routes as this campaign's exploration frontier.
    ///
    /// Parallel lanes must learn from every authenticated transition without
    /// treating peer visitation as their own coverage. Otherwise every lane
    /// imports the same frontier archive and independent exploration collapses
    /// into one correlated search.
    pub(crate) fn consume_learner_snapshot_with_exploration_filter<F>(
        &mut self,
        snapshot: &TacticQImmutableLearnerSnapshot,
        exploration_episode: F,
    ) -> Result<usize, TacticQCampaignError>
    where
        F: Fn(u64) -> bool,
    {
        snapshot.manifest.validate()?;
        if snapshot.sha256 != snapshot.manifest.content_sha256()?
            || snapshot.manifest.execution_authority_sha256 != self.execution_authority_sha256
            || snapshot.manifest.feature_schema_sha256 != self.feature_schema_sha256
            || snapshot.manifest.objective_sha256 != self.objective_sha256
            || snapshot.manifest.root_checkpoint_sha256 != self.root_checkpoint_sha256
            || snapshot.manifest.model_config != self.model_config
            || (self.campaign_learner_authority_managed
                && snapshot.manifest.value_treatment != self.value_treatment)
            || (self.campaign_learner_authority_managed
                && snapshot.manifest.model_revision < self.model_revision)
            || snapshot.manifest.training_replay_rows
                != snapshot.training_corpus.transitions.len() as u64
            || snapshot.replay_revision != snapshot.manifest.training_replay_rows
        {
            return Err(TacticQCampaignError::InvalidState(
                "immutable learner snapshot belongs to another campaign",
            ));
        }
        let admitted = self.import_training_corpora_with_refit_and_exploration_filter(
            std::slice::from_ref(snapshot.training_corpus()),
            false,
            &exploration_episode,
        )?;
        self.model = snapshot.model.clone();
        self.model_revision = snapshot.manifest.model_revision;
        self.campaign_learner_authority_managed = true;
        self.value_treatment = snapshot.manifest.value_treatment;
        *self.generalized_model.borrow_mut() =
            snapshot
                .generalized_model
                .as_ref()
                .map(|model| CachedGeneralizedTacticValueModel {
                    goal_distance_feature: snapshot.goal_distance_feature,
                    model_revision: snapshot.manifest.model_revision,
                    model: Arc::clone(model),
                });
        *self.native_terminal_model.borrow_mut() =
            snapshot.native_terminal_model.as_ref().map(|model| {
                CachedGeneralizedTacticValueModel {
                    goal_distance_feature: snapshot.goal_distance_feature,
                    model_revision: snapshot.manifest.model_revision,
                    model: Arc::clone(model),
                }
            });
        *self.native_terminal_action_model.borrow_mut() = snapshot
            .native_terminal_action_model
            .as_ref()
            .map(|model| CachedContinuousTacticDoubleQModel {
                goal_distance_feature: snapshot.goal_distance_feature,
                model_revision: snapshot.manifest.model_revision,
                model: Arc::clone(model),
            });
        *self.continuous_model.borrow_mut() =
            snapshot
                .continuous_model
                .as_ref()
                .map(|model| CachedContinuousTacticValueModel {
                    goal_distance_feature: snapshot.goal_distance_feature,
                    model_revision: snapshot.manifest.model_revision,
                    model: Arc::clone(model),
                });
        self.goal_reachability_calibration =
            snapshot.manifest.goal_reachability_calibration.clone();
        self.terminal_action_calibration = snapshot.manifest.terminal_action_calibration.clone();
        Ok(admitted)
    }

    pub fn bind_execution_authority(
        &mut self,
        execution_authority_sha256: Digest,
    ) -> Result<(), TacticQCampaignError> {
        if execution_authority_sha256 == Digest::ZERO
            || self.decision_index != 0
            || !self.replay.is_empty()
            || !self.training_replay.is_empty()
            || self.execution_authority_sha256 != Digest::ZERO
        {
            return Err(TacticQCampaignError::InvalidState(
                "execution authority can only bind a fresh campaign".into(),
            ));
        }
        self.execution_authority_sha256 = execution_authority_sha256;
        let state_graph = StateGraph::new(
            StateGraphIdentity {
                execution_authority_sha256,
                future_equivalence_validator_sha256: execution_authority_sha256,
                feature_schema_sha256: self.feature_schema_sha256,
                objective_sha256: self.objective_sha256,
                root_checkpoint_sha256: self.root_checkpoint_sha256,
            },
            self.current.snapshot.clone(),
            self.route_tape.clone(),
        )?;
        self.state_graph_validation = Some(state_graph.validation_token()?);
        self.state_graph = Some(state_graph);
        Ok(())
    }

    pub fn training_replay_len(&self) -> usize {
        self.training_replay.len()
    }

    /// Whether authenticated replay contains native terminal supervision.
    ///
    /// Terminal learning authority is durable even when the current graph has
    /// no process-local terminal restoration handle. Policy partitioning and
    /// frontier scheduling must therefore consult replay, not transient graph
    /// restorability.
    pub fn native_terminal_supported(&self) -> bool {
        self.training_replay
            .iter()
            .any(|transition| transition.value_sample.terminal)
    }

    pub(crate) fn replay(&self) -> &[OptionTransitionSample] {
        &self.replay
    }

    pub fn training_corpus(&self) -> TacticQTrainingCorpus {
        self.training_corpus_from(0)
            .expect("zero is always a valid training corpus offset")
    }

    pub fn training_corpus_from(
        &self,
        first_row: usize,
    ) -> Result<TacticQTrainingCorpus, TacticQCampaignError> {
        if first_row > self.training_replay.len() {
            return Err(TacticQCampaignError::InvalidState(
                "shared tactic training corpus offset is invalid",
            ));
        }
        Ok(TacticQTrainingCorpus {
            execution_authority_sha256: self.execution_authority_sha256,
            feature_schema_sha256: self.feature_schema_sha256,
            objective_sha256: self.objective_sha256,
            root_checkpoint_sha256: self.root_checkpoint_sha256,
            transitions: self.training_replay[first_row..].to_vec(),
            routes: self.training_replay_routes[first_row..].to_vec(),
            episode_groups: self.training_episode_groups[first_row..].to_vec(),
        })
    }

    /// Merge evidence from independent episodes and immediately refit the
    /// critic. Duplicate transitions are ignored by their authenticated replay
    /// identity; conflicting or detached routes reject the entire import.
    pub fn import_training_corpora(
        &mut self,
        corpora: &[TacticQTrainingCorpus],
    ) -> Result<usize, TacticQCampaignError> {
        self.import_training_corpora_with_refit_and_exploration_filter(corpora, true, &|_| true)
    }

    pub fn import_training_corpora_without_refit(
        &mut self,
        corpora: &[TacticQTrainingCorpus],
    ) -> Result<usize, TacticQCampaignError> {
        self.import_training_corpora_with_refit_and_exploration_filter(corpora, false, &|_| true)
    }

    fn import_training_corpora_with_refit_and_exploration_filter<F>(
        &mut self,
        corpora: &[TacticQTrainingCorpus],
        refit: bool,
        exploration_episode: &F,
    ) -> Result<usize, TacticQCampaignError>
    where
        F: Fn(u64) -> bool,
    {
        let mut state_graph =
            self.state_graph
                .clone()
                .ok_or(TacticQCampaignError::InvalidState(
                    "training evidence requires a bound state graph",
                ))?;
        let mut frontier_archive = self.frontier_archive.clone();
        let mut visited_states = self.visited_states.clone();
        let mut admitted = 0_usize;

        for corpus in corpora {
            if corpus.execution_authority_sha256 != self.execution_authority_sha256
                || corpus.feature_schema_sha256 != self.feature_schema_sha256
                || corpus.objective_sha256 != self.objective_sha256
                || corpus.root_checkpoint_sha256 != self.root_checkpoint_sha256
                || corpus.transitions.len() != corpus.routes.len()
                || corpus.transitions.len() != corpus.episode_groups.len()
            {
                return Err(TacticQCampaignError::InvalidState(
                    "shared tactic training corpus identity or shape is invalid",
                ));
            }
            for ((transition, route), episode_group) in corpus
                .transitions
                .iter()
                .zip(&corpus.routes)
                .zip(&corpus.episode_groups)
            {
                validate_training_transition(
                    self.execution_authority_sha256,
                    self.feature_schema_sha256,
                    self.root_checkpoint_sha256,
                    transition,
                    route,
                )?;
                let admission = state_graph.admit_completed_expansion(
                    transition.clone(),
                    route.clone(),
                    *episode_group,
                    if *episode_group == TACTIC_Q_MODEL_ONLY_EPISODE_GROUP {
                        crate::state_graph::ExpansionEvidenceAuthority::LearnerEvidenceOnly
                    } else {
                        crate::state_graph::ExpansionEvidenceAuthority::Executable
                    },
                )?;
                if (!admission.duplicate || admission.authority_promoted)
                    && exploration_episode(*episode_group)
                {
                    consider_frontier_transition(
                        &mut frontier_archive,
                        self.root_checkpoint_sha256,
                        transition,
                        route,
                        *episode_group,
                        state_graph.expansion_count().saturating_sub(1),
                    )?;
                    visited_states.insert(tactic_state_descriptor(
                        &transition.before,
                        transition.before.terminal.reached == Some(true),
                    ));
                    visited_states.insert(tactic_state_descriptor(
                        &transition.after,
                        transition.value_sample.terminal,
                    ));
                }
                if !admission.duplicate {
                    admitted = admitted.saturating_add(1);
                }
            }
        }

        if admitted == 0 {
            return Ok(0);
        }
        let projection = graph_training_projection(&state_graph)?;
        let model = refit
            .then(|| {
                replay_model(
                    self.feature_schema_sha256,
                    self.objective_sha256,
                    &projection.transitions,
                    &projection.episode_groups,
                    &self.model_config,
                )
            })
            .transpose()?;
        self.state_graph = Some(state_graph);
        self.training_replay = projection.transitions;
        self.training_replay_routes = projection.routes;
        self.training_episode_groups = projection.episode_groups;
        self.training_projection_keys = projection.keys;
        self.frontier_archive = frontier_archive;
        self.visited_states = visited_states;
        if let Some(model) = model {
            self.model = model.map(Arc::new);
            self.model_revision = self.model_revision.saturating_add(1);
            self.campaign_learner_authority_managed = false;
        }
        Ok(admitted)
    }
}

mod learner_snapshot;
pub use learner_snapshot::{
    TacticQImmutableLearnerSnapshot, TacticQLearnerSnapshot, TacticQLearnerSnapshotKind,
};
mod decision;
mod final_result;
pub use final_result::TacticQFinalResult;
mod frontier;
mod graph_projection;
mod graph_scheduling;
pub use graph_scheduling::TacticRestorationContract;
pub use graph_scheduling::{
    EvaluatedTacticQProposalBatch, LeasedTacticQProposalBatch,
    TACTIC_POLICY_EVALUATION_DECISION_SCHEMA_V1, TACTIC_SCHEDULER_DECISION_SCHEMA_V1,
    TacticExpansionLease, TacticExpansionLeaseKind, TacticGraphSchedulingTiming,
    TacticPolicyEvaluationDecisionTrace, TacticScheduledExpansionEvidence,
    TacticSchedulerDecisionTrace,
};
mod persistence;
mod value_treatment;
use value_treatment::{
    CachedContinuousTacticDoubleQModel, CachedContinuousTacticValueModel,
    CachedGeneralizedTacticValueModel,
};

mod frontier_policy;
pub(crate) use frontier_policy::has_no_progress_loop;
#[cfg(test)]
use frontier_policy::semantic_state_digest;
use frontier_policy::{
    action_digest, applicable_untried_descriptors, compare_frontier_acquisition,
    ensure_blueprint_proposal, insert_graph_node, seeded_frontier_index,
};

mod validation;
pub(crate) use validation::*;

mod error;
pub use error::TacticQCampaignError;

#[cfg(test)]
#[path = "tactic_q_campaign/tests.rs"]
mod tests;
