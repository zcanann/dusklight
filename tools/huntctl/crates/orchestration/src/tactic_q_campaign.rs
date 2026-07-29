//! Online option-Q campaign over authenticated learner states and native tactic
//! boundaries.

use crate::native_tactic_worker::{
    NativeTacticWorkerError, NativeTacticWorkerOutcome, NativeTacticWorkerPaths,
    PersistentTacticBatchWorker, execute_selected_tactic,
};
use crate::tactic_q_checkpoint_store;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_control::option_execution::OptionExecution;
use dusklight_learning::fact_registry::FactRegistry;
use dusklight_learning::fact_snapshot::{FACT_SNAPSHOT_SCHEMA_V2, FactSnapshot};
use dusklight_learning::generalized_tactic_value::{
    GeneralizedTacticContext, GeneralizedTacticValueError, GeneralizedTacticValueModel,
};
use dusklight_learning::hindsight::{
    HindsightError, HindsightOptionReplay, RelabeledHindsightOption,
};
use dusklight_learning::learner_state::{
    LearnerActionMaskEntry, LearnerState, LearnerStateError, tactic_intrinsically_applicable,
};
use dusklight_learning::live_tactic_catalog::{
    LiveTacticCatalog, LiveTacticCatalogError, LiveTacticRanking,
};
use dusklight_learning::option_transition::{OptionTransitionError, OptionTransitionSample};
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
    SelectedTactic, TacticExplorationConfig, TacticExplorationError, TacticProposalPolicy,
    TacticSelectionReason, choose_tactic_batch_for_policy, choose_tactic_batch_with_state_untried,
    ensure_action_factor_coverage, ensure_generalized_value_acquisition,
    ensure_terminal_support_factor_acquisitions, retain_generalized_value_acquisition,
};
use dusklight_learning::tactic_frozen_policy::{TacticFrozenPolicy, TacticFrozenPolicyError};
use dusklight_learning::tactic_value_treatment::{
    ContinuousTacticDoubleQModel, ContinuousTacticValueModel, TacticValueTreatment,
};
use dusklight_proposals::behavior_archive::{
    BehaviorArchive, MAX_BEHAVIOR_ARCHIVE_ENTRIES, TacticEndpointDescriptor, TacticFrontierEntry,
    TacticStateDescriptor, tactic_endpoint_descriptor, tactic_endpoint_descriptor_for_state,
    tactic_state_descriptor,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const TACTIC_Q_CAMPAIGN_SCHEMA_V1: &str = "dusklight-tactic-q-campaign/v1";
pub const TACTIC_Q_CHECKPOINT_SCHEMA_V2: &str = "dusklight-tactic-q-checkpoint/v2";
pub const TACTIC_Q_CHECKPOINT_SCHEMA_V3: &str = "dusklight-tactic-q-checkpoint/v3";
pub const TACTIC_Q_CHECKPOINT_SCHEMA_V4: &str = "dusklight-tactic-q-checkpoint/v4";
pub const TACTIC_Q_CHECKPOINT_EXTENSION: &str = "dtqz";
pub const TACTIC_Q_CHECKPOINT_SERIALIZATION_BENCHMARK_SCHEMA_V1: &str =
    "dusklight-tactic-q-checkpoint-serialization-benchmark/v1";
pub const TACTIC_Q_FINAL_RESULT_SCHEMA_V1: &str = "dusklight-tactic-q-final-result/v1";
pub const TACTIC_Q_FINAL_RESULT_SCHEMA_V2: &str = "dusklight-tactic-q-final-result/v2";
pub const TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V1: &str = "dusklight-tactic-q-learner-snapshot/v1";
pub const TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V2: &str = "dusklight-tactic-q-learner-snapshot/v2";
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
const ROUTE_CHECKPOINT_SCHEMA_V1: &[u8] = b"dusklight-route-checkpoint/v1";

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
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatedRewardedTacticOutcome {
    pub outcome: NativeTacticWorkerOutcome,
    pub transition: OptionTransitionSample,
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
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticQFinalResult {
    pub schema: String,
    pub content_sha256: Digest,
    #[serde(default, skip_serializing_if = "digest_is_zero")]
    pub execution_authority_sha256: Digest,
    pub objective_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
    pub route_tape_sha256: Digest,
    pub replay_sha256: Digest,
    pub terminal_state_sha256: Digest,
    pub route_tape: InputTape,
    pub replay: Vec<OptionTransitionSample>,
    pub replay_routes: Vec<InputTape>,
    pub terminal: FactSnapshot,
}

impl TacticQFinalResult {
    pub fn write(&self, path: &Path) -> Result<(), TacticQCampaignError> {
        tactic_q_checkpoint_store::write_final_result(self, path)
    }

    pub fn read(path: &Path) -> Result<Self, TacticQCampaignError> {
        tactic_q_checkpoint_store::read_final_result(path)
    }
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
    /// True when a universal goal-conditioned treatment has authenticated
    /// achieved-goal return support. This may rank cold-start continuation,
    /// but never claims that the authored native terminal was reached.
    #[serde(default)]
    pub achieved_goal_value_supported: bool,
    pub reward: f32,
    pub best_mean_q: Option<f64>,
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
    pub replay: Vec<OptionTransitionSample>,
    pub replay_routes: Vec<InputTape>,
    pub episode_groups: Vec<u64>,
    training_replay: Vec<OptionTransitionSample>,
    training_replay_routes: Vec<InputTape>,
    training_episode_groups: Vec<u64>,
    training_identities: BTreeSet<Digest>,
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
    visited_states: BTreeSet<TacticStateDescriptor>,
    hindsight: HindsightOptionReplay,
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
            replay: Vec::new(),
            replay_routes: Vec::new(),
            episode_groups: Vec::new(),
            training_replay: Vec::new(),
            training_replay_routes: Vec::new(),
            training_episode_groups: Vec::new(),
            training_identities: BTreeSet::new(),
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
            visited_states,
            hindsight,
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
            schema: TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V2.into(),
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
        Ok(())
    }

    pub fn training_replay_len(&self) -> usize {
        self.training_replay.len()
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
        let mut training_replay = self.training_replay.clone();
        let mut training_replay_routes = self.training_replay_routes.clone();
        let mut training_episode_groups = self.training_episode_groups.clone();
        let mut identities = self.training_identities.clone();
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
                let identity = transition.replay_identity_sha256()?;
                if identities.insert(identity) {
                    if exploration_episode(*episode_group) {
                        consider_frontier_transition(
                            &mut frontier_archive,
                            self.root_checkpoint_sha256,
                            transition,
                            route,
                            *episode_group,
                            training_replay.len(),
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
                    training_replay.push(transition.clone());
                    training_replay_routes.push(route.clone());
                    training_episode_groups.push(*episode_group);
                    admitted = admitted.saturating_add(1);
                }
            }
        }

        if admitted == 0 {
            return Ok(0);
        }
        let model = refit
            .then(|| {
                replay_model(
                    self.feature_schema_sha256,
                    self.objective_sha256,
                    &training_replay,
                    &training_episode_groups,
                    &self.model_config,
                )
            })
            .transpose()?;
        self.training_replay = training_replay;
        self.training_replay_routes = training_replay_routes;
        self.training_episode_groups = training_episode_groups;
        self.training_identities = identities;
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
mod frontier;
mod persistence;
mod value_treatment;
use value_treatment::{
    CachedContinuousTacticDoubleQModel, CachedContinuousTacticValueModel,
    CachedGeneralizedTacticValueModel,
};

fn applicable_untried_descriptors(
    choices: &[LearnerActionMaskEntry],
    tried_here: &BTreeSet<&str>,
) -> Vec<OptionActionDescriptor> {
    choices
        .iter()
        .filter(|choice| {
            choice.applicable && !tried_here.contains(choice.descriptor.option_id.as_str())
        })
        .map(|choice| choice.descriptor.clone())
        .collect()
}

fn seeded_frontier_index(seed: u64, round: u64, choice_count: usize) -> usize {
    debug_assert!(choice_count > 0);
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-tactic-frontier-sample/v2");
    hasher.update(seed.to_le_bytes());
    let digest = hasher.finalize();
    let count = choice_count as u64;
    let offset = u64::from_le_bytes(digest[..8].try_into().unwrap()) % count;
    ((offset + round % count) % count) as usize
}

fn option_f64(value: Option<f64>) -> f64 {
    value.unwrap_or(f64::NEG_INFINITY)
}

fn compare_frontier_acquisition(
    left: &TacticFrontierAcquisition,
    right: &TacticFrontierAcquisition,
) -> std::cmp::Ordering {
    let terminal = right.terminal.cmp(&left.terminal);
    if terminal != std::cmp::Ordering::Equal {
        return terminal;
    }
    if !left.terminal_value_supported && !right.terminal_value_supported {
        if left.achieved_goal_value_supported != right.achieved_goal_value_supported {
            return right
                .achieved_goal_value_supported
                .cmp(&left.achieved_goal_value_supported);
        }
        if left.achieved_goal_value_supported && right.achieved_goal_value_supported {
            // Preserve one expansion of every fresh state cell, then let the
            // learned achieved-goal return decide which equally fresh
            // continuation is most promising. This is learned first-hit cost,
            // not sparse action cost or native terminal evidence.
            return left
                .expansion_count
                .cmp(&right.expansion_count)
                .then_with(|| {
                    option_f64(right.best_mean_q).total_cmp(&option_f64(left.best_mean_q))
                })
                .then_with(|| left.novelty_rank.cmp(&right.novelty_rank))
                .then_with(|| {
                    option_f64(right.generalized_nearest_distance.map(f64::from)).total_cmp(
                        &option_f64(left.generalized_nearest_distance.map(f64::from)),
                    )
                });
        }
        // With no authenticated terminal sample, sparse return is only the
        // negative duration already spent. Treating it as goal evidence traps
        // acquisition near cheap, shallow actions. Cover the farthest-first
        // semantic archive until real terminal supervision exists.
        return left
            .expansion_count
            .cmp(&right.expansion_count)
            .then_with(|| left.novelty_rank.cmp(&right.novelty_rank))
            .then_with(|| {
                option_f64(right.generalized_nearest_distance.map(f64::from)).total_cmp(
                    &option_f64(left.generalized_nearest_distance.map(f64::from)),
                )
            })
            .then_with(|| {
                option_f64(right.maximum_ensemble_variance)
                    .total_cmp(&option_f64(left.maximum_ensemble_variance))
            });
    }
    if left.terminal_value_supported != right.terminal_value_supported {
        return right
            .terminal_value_supported
            .cmp(&left.terminal_value_supported);
    }
    match (
        left.exact_total_terminal_ticks,
        right.exact_total_terminal_ticks,
    ) {
        (Some(left_ticks), Some(right_ticks)) => {
            return left_ticks
                .cmp(&right_ticks)
                .then_with(|| left.expansion_count.cmp(&right.expansion_count));
        }
        (Some(_), None) => return std::cmp::Ordering::Less,
        (None, Some(_)) => return std::cmp::Ordering::Greater,
        (None, None) => {}
    }
    match (
        left.predicted_total_terminal_ticks,
        right.predicted_total_terminal_ticks,
    ) {
        (Some(left_ticks), Some(right_ticks)) => {
            // Q-to-go alone systematically favors the latest checkpoint on
            // one successful route. Compare the learned first-hit cost from
            // the authenticated root, then spread trials across equal-cost
            // curriculum frontiers.
            left_ticks
                .total_cmp(&right_ticks)
                .then_with(|| left.expansion_count.cmp(&right.expansion_count))
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => option_f64(right.best_mean_q)
            .total_cmp(&option_f64(left.best_mean_q))
            .then_with(|| left.expansion_count.cmp(&right.expansion_count))
            .then_with(|| {
                option_f64(right.generalized_nearest_distance.map(f64::from)).total_cmp(
                    &option_f64(left.generalized_nearest_distance.map(f64::from)),
                )
            })
            .then_with(|| {
                option_f64(right.maximum_ensemble_variance)
                    .total_cmp(&option_f64(left.maximum_ensemble_variance))
            })
            .then_with(|| left.replayed_prefix_ticks.cmp(&right.replayed_prefix_ticks)),
    }
}

fn ensure_blueprint_proposal(
    ranking: &LiveTacticRanking,
    maximum_proposals: usize,
    proposals: &mut Vec<SelectedTactic>,
) -> Result<(), TacticQCampaignError> {
    if maximum_proposals <= 1
        || proposals
            .iter()
            .any(|proposal| proposal.descriptor.option_id.starts_with("blueprint/"))
    {
        return Ok(());
    }
    let Some(composition) = ranking
        .choices
        .iter()
        .find(|choice| choice.applicable && choice.kind == ConcreteTacticChoiceKind::Blueprint)
        .map(|choice| choice.descriptor.clone())
    else {
        return Ok(());
    };
    let mut selected = proposals
        .last()
        .cloned()
        .ok_or(TacticQCampaignError::InvalidState(
            "tactic proposal batch is empty",
        ))?;
    selected.descriptor = composition;
    selected.reason = TacticSelectionReason::BatchDiversity;
    if proposals.len() < maximum_proposals {
        proposals.push(selected);
    } else if let Some(last) = proposals.last_mut() {
        *last = selected;
    }
    Ok(())
}

fn action_digest(action: &OptionActionDescriptor) -> Result<Digest, TacticQCampaignError> {
    let bytes = serde_json::to_vec(action)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    Ok(sha256(&bytes))
}

fn insert_graph_node(
    nodes: &mut BTreeMap<(Digest, Digest), TacticCampaignGraphNode>,
    node: TacticCampaignGraphNode,
) -> Result<(), TacticQCampaignError> {
    let identity = (node.checkpoint_sha256, node.state_sha256);
    if let Some(existing) = nodes.get(&identity) {
        if existing != &node {
            return Err(TacticQCampaignError::InvalidState(
                "one checkpoint-state identity has conflicting campaign graph nodes",
            ));
        }
    } else {
        nodes.insert(identity, node);
    }
    Ok(())
}

pub(crate) fn has_no_progress_loop(
    replay: &[OptionTransitionSample],
    episode_groups: &[u64],
) -> Result<bool, TacticQCampaignError> {
    let mut visited = BTreeMap::<u64, BTreeSet<Digest>>::new();
    for (transition, episode_group) in replay.iter().zip(episode_groups) {
        let states = visited.entry(*episode_group).or_default();
        states.insert(semantic_state_digest(&transition.before)?);
        if !transition.value_sample.terminal
            && !states.insert(semantic_state_digest(&transition.after)?)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn semantic_state_digest(snapshot: &FactSnapshot) -> Result<Digest, TacticQCampaignError> {
    // Clocks, replay history, and the previously emitted pad identify when and
    // how a state was observed, not whether gameplay made semantic progress.
    // Everything else remains visible so actor, flag, event, kinematic, and
    // derived-condition progress all break a cycle.
    let mut normalized = snapshot.clone();
    normalized.schema = FACT_SNAPSHOT_SCHEMA_V2.into();
    normalized.boundary_index = 0;
    normalized.simulation_tick = 0;
    normalized.tape_frame = 0;
    normalized.state_identity = [0; 16];
    normalized.recent_history.clear();
    normalized.recent_option = None;
    normalized.player.previous_pad = None;
    let bytes = serde_json::to_vec(&normalized)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    Ok(sha256(&bytes))
}

pub(crate) fn validate_checkpoint(
    checkpoint: &TacticQCampaignCheckpoint,
) -> Result<(), TacticQCampaignError> {
    checkpoint.current.validate()?;
    checkpoint
        .route_tape
        .validate()
        .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
    let legacy = checkpoint.schema == TACTIC_Q_CHECKPOINT_SCHEMA_V2;
    let shared_replay = checkpoint.schema == TACTIC_Q_CHECKPOINT_SCHEMA_V3;
    let current = checkpoint.schema == TACTIC_Q_CHECKPOINT_SCHEMA_V4;
    if (!legacy && !shared_replay && !current)
        || checkpoint.content_sha256 == Digest::ZERO
        || checkpoint.content_sha256 != checkpoint_digest(checkpoint)?
        || checkpoint.feature_schema_sha256 == Digest::ZERO
        || checkpoint.objective_sha256 == Digest::ZERO
        || checkpoint.root_checkpoint_sha256 == Digest::ZERO
        || checkpoint.exploration.epsilon_per_million > 1_000_000
        || checkpoint.replay.len() != checkpoint.episode_groups.len()
        || checkpoint.replay.len() != checkpoint.replay_routes.len()
        || checkpoint.decision_index != checkpoint.replay.len() as u64
        || (legacy
            && (!checkpoint.training_replay.is_empty()
                || !checkpoint.training_replay_routes.is_empty()
                || !checkpoint.training_episode_groups.is_empty()))
        || ((shared_replay || current)
            && (checkpoint.training_replay.len() != checkpoint.training_replay_routes.len()
                || checkpoint.training_replay.len() != checkpoint.training_episode_groups.len()
                || checkpoint.training_replay.len() < checkpoint.replay.len()))
        || (shared_replay && checkpoint.model_revision != 0)
        || checkpoint.current.snapshot.tape_frame != checkpoint.route_tape.frames.len() as u64
    {
        return Err(TacticQCampaignError::InvalidState(
            "campaign checkpoint identity or shape is invalid",
        ));
    }
    let mut endpoints = BTreeMap::<u64, (Digest, Digest)>::new();
    for ((transition, route), episode_group) in checkpoint
        .replay
        .iter()
        .zip(&checkpoint.replay_routes)
        .zip(&checkpoint.episode_groups)
    {
        transition.validate()?;
        if transition.execution_authority_sha256 != checkpoint.execution_authority_sha256
            || transition.feature_schema_sha256 != checkpoint.feature_schema_sha256
            || endpoints.get(episode_group).is_some_and(|(state, route)| {
                *state != transition.before_state_sha256
                    || *route != transition.source_checkpoint_sha256
            })
        {
            return Err(TacticQCampaignError::InvalidState(
                "campaign checkpoint replay chain is detached",
            ));
        }
        transition
            .execution
            .validate_against_tape(route)
            .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
        let start = usize::try_from(transition.execution.realized_tape_range.start_frame)
            .map_err(|_| TacticQCampaignError::InvalidState("replay tape range overflows"))?;
        let end = usize::try_from(transition.execution.realized_tape_range.end_frame_exclusive)
            .map_err(|_| TacticQCampaignError::InvalidState("replay tape range overflows"))?;
        if end > route.frames.len()
            || transition.source_checkpoint_sha256
                != route_checkpoint(
                    checkpoint.root_checkpoint_sha256,
                    &tape_prefix(route, start),
                )?
            || transition.next_checkpoint_sha256
                != route_checkpoint(checkpoint.root_checkpoint_sha256, &tape_prefix(route, end))?
        {
            return Err(TacticQCampaignError::InvalidState(
                "campaign checkpoint replay route is detached",
            ));
        }
        endpoints.insert(
            *episode_group,
            (
                transition.after_state_sha256,
                transition.next_checkpoint_sha256,
            ),
        );
    }
    if let Some((after, route)) = endpoints.get(&checkpoint.episode_group)
        && (*after != checkpoint.current.snapshot_sha256
            || *route
                != route_checkpoint(checkpoint.root_checkpoint_sha256, &checkpoint.route_tape)?)
    {
        return Err(TacticQCampaignError::InvalidState(
            "campaign checkpoint current state is not the replay endpoint",
        ));
    }
    let (training_replay, training_routes, training_groups) = if legacy {
        (
            &checkpoint.replay,
            &checkpoint.replay_routes,
            &checkpoint.episode_groups,
        )
    } else {
        (
            &checkpoint.training_replay,
            &checkpoint.training_replay_routes,
            &checkpoint.training_episode_groups,
        )
    };
    let mut training_identities = BTreeSet::new();
    for ((transition, route), _) in training_replay
        .iter()
        .zip(training_routes)
        .zip(training_groups)
    {
        validate_training_transition(
            checkpoint.execution_authority_sha256,
            checkpoint.feature_schema_sha256,
            checkpoint.root_checkpoint_sha256,
            transition,
            route,
        )?;
        if !training_identities.insert(transition.replay_identity_sha256()?) {
            return Err(TacticQCampaignError::InvalidState(
                "campaign training replay is detached or duplicated",
            ));
        }
    }
    if checkpoint.replay.iter().any(|transition| {
        transition
            .replay_identity_sha256()
            .map_or(true, |identity| !training_identities.contains(&identity))
    }) {
        return Err(TacticQCampaignError::InvalidState(
            "retained replay is absent from training replay",
        ));
    }
    Ok(())
}

pub(crate) fn validate_training_corpus(
    corpus: &TacticQTrainingCorpus,
) -> Result<(), TacticQCampaignError> {
    if corpus.feature_schema_sha256 == Digest::ZERO
        || corpus.objective_sha256 == Digest::ZERO
        || corpus.root_checkpoint_sha256 == Digest::ZERO
        || corpus.transitions.len() != corpus.routes.len()
        || corpus.transitions.len() != corpus.episode_groups.len()
    {
        return Err(TacticQCampaignError::InvalidState(
            "shared tactic training corpus identity or shape is invalid",
        ));
    }
    let mut identities = BTreeSet::new();
    for (transition, route) in corpus.transitions.iter().zip(&corpus.routes) {
        validate_training_transition(
            corpus.execution_authority_sha256,
            corpus.feature_schema_sha256,
            corpus.root_checkpoint_sha256,
            transition,
            route,
        )?;
        if !identities.insert(transition.replay_identity_sha256()?) {
            return Err(TacticQCampaignError::InvalidState(
                "shared tactic training corpus contains duplicate transitions",
            ));
        }
    }
    Ok(())
}

fn validate_training_transition(
    execution_authority_sha256: Digest,
    feature_schema_sha256: Digest,
    root_checkpoint_sha256: Digest,
    transition: &OptionTransitionSample,
    route: &InputTape,
) -> Result<(), TacticQCampaignError> {
    transition.validate()?;
    transition
        .execution
        .validate_against_tape(route)
        .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
    let start = usize::try_from(transition.execution.realized_tape_range.start_frame)
        .map_err(|_| TacticQCampaignError::InvalidState("training tape range overflows"))?;
    let end = usize::try_from(transition.execution.realized_tape_range.end_frame_exclusive)
        .map_err(|_| TacticQCampaignError::InvalidState("training tape range overflows"))?;
    if transition.execution_authority_sha256 != execution_authority_sha256
        || transition.feature_schema_sha256 != feature_schema_sha256
        || end > route.frames.len()
        || transition.source_checkpoint_sha256
            != route_checkpoint(root_checkpoint_sha256, &tape_prefix(route, start))?
        || transition.next_checkpoint_sha256
            != route_checkpoint(root_checkpoint_sha256, &tape_prefix(route, end))?
    {
        return Err(TacticQCampaignError::InvalidState(
            "campaign training replay is detached",
        ));
    }
    Ok(())
}

fn replay_model(
    feature_schema_sha256: Digest,
    objective_sha256: Digest,
    replay: &[OptionTransitionSample],
    episode_groups: &[u64],
    config: &OptionValueConfig,
) -> Result<Option<OptionValueModel>, TacticQCampaignError> {
    let Some(first) = replay.first() else {
        return Ok(None);
    };
    let exact_actions = replay
        .iter()
        .map(|transition| transition.value_sample.action.content_sha256())
        .collect::<Result<BTreeSet<_>, OptionValueError>>()?;
    if exact_actions.len() > MAX_OPTION_ACTIONS {
        return Ok(None);
    }
    let batch = OptionValueBatch::new(
        feature_schema_sha256,
        objective_sha256,
        first.value_sample.state.len(),
        replay
            .iter()
            .map(|transition| transition.value_sample.clone())
            .collect(),
        episode_groups.to_vec(),
    )?;
    Ok(Some(OptionValueModel::fit_batch(&batch, config)?))
}

fn training_replay_sha256(
    transitions: &[OptionTransitionSample],
    episode_groups: &[u64],
) -> Result<Digest, TacticQCampaignError> {
    if transitions.len() != episode_groups.len() {
        return Err(TacticQCampaignError::InvalidState(
            "learner snapshot replay shape is invalid",
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.tactic-q-learner-replay/v1\0");
    hasher.update((transitions.len() as u64).to_le_bytes());
    for (transition, episode_group) in transitions.iter().zip(episode_groups) {
        hasher.update(transition.replay_identity_sha256()?.0);
        hasher.update(episode_group.to_le_bytes());
    }
    Ok(Digest(hasher.finalize().into()))
}

pub(crate) fn checkpoint_digest(
    checkpoint: &TacticQCampaignCheckpoint,
) -> Result<Digest, TacticQCampaignError> {
    let mut canonical = checkpoint.clone();
    canonical.content_sha256 = Digest::ZERO;
    let bytes = serde_cbor::to_vec(&canonical)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    Ok(sha256(&bytes))
}

pub(crate) fn validate_final_result(
    result: &TacticQFinalResult,
) -> Result<(), TacticQCampaignError> {
    result
        .route_tape
        .validate()
        .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
    result
        .terminal
        .validate()
        .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
    let route_bytes = result
        .route_tape
        .encode()
        .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
    let replay_bytes = serde_cbor::to_vec(&(&result.replay, &result.replay_routes))
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    if result.schema != TACTIC_Q_FINAL_RESULT_SCHEMA_V2
        || result.content_sha256 == Digest::ZERO
        || result.content_sha256 != final_result_digest(result)?
        || result.objective_sha256 == Digest::ZERO
        || result.root_checkpoint_sha256 == Digest::ZERO
        || result.route_tape_sha256 != sha256(&route_bytes)
        || result.replay_sha256 != sha256(&replay_bytes)
        || result.replay.len() != result.replay_routes.len()
        || result.replay.iter().any(|transition| {
            transition.execution_authority_sha256 != result.execution_authority_sha256
        })
        || result.terminal_state_sha256
            != result
                .terminal
                .content_sha256()
                .map_err(|error| TacticQCampaignError::Features(error.to_string()))?
        || result.terminal.terminal.configured != Some(true)
        || result.terminal.terminal.reached != Some(true)
        || result.terminal.tape_frame != result.route_tape.frames.len() as u64
        || result
            .replay
            .last()
            .map(|transition| transition.after_state_sha256)
            != Some(result.terminal_state_sha256)
    {
        return Err(TacticQCampaignError::InvalidState(
            "final tactic-Q result is not an authenticated terminal route",
        ));
    }
    for (transition, route) in result.replay.iter().zip(&result.replay_routes) {
        transition.validate()?;
        transition
            .execution
            .validate_against_tape(route)
            .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
    }
    Ok(())
}

fn final_result_digest(result: &TacticQFinalResult) -> Result<Digest, TacticQCampaignError> {
    // The component digests already bind the exact route, replay, and terminal
    // payloads. Seal the small identity tuple instead of serializing those
    // multi-megabyte values a second time merely to derive the outer identity.
    let bytes = if result.execution_authority_sha256 == Digest::ZERO {
        serde_cbor::to_vec(&(
            &result.schema,
            result.objective_sha256,
            result.root_checkpoint_sha256,
            result.route_tape_sha256,
            result.replay_sha256,
            result.terminal_state_sha256,
        ))
    } else {
        serde_cbor::to_vec(&(
            "dusklight-tactic-q-final-result-identity/v3",
            &result.schema,
            result.execution_authority_sha256,
            result.objective_sha256,
            result.root_checkpoint_sha256,
            result.route_tape_sha256,
            result.replay_sha256,
            result.terminal_state_sha256,
        ))
    }
    .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    Ok(sha256(&bytes))
}

fn tape_prefix(tape: &InputTape, frame_count: usize) -> InputTape {
    InputTape {
        boot: tape.boot.clone(),
        tick_rate_numerator: tape.tick_rate_numerator,
        tick_rate_denominator: tape.tick_rate_denominator,
        frames: tape.frames[..frame_count].to_vec(),
    }
}

fn extends(prefix: &InputTape, route: &InputTape) -> bool {
    prefix.boot == route.boot
        && prefix.tick_rate_numerator == route.tick_rate_numerator
        && prefix.tick_rate_denominator == route.tick_rate_denominator
        && route.frames.starts_with(&prefix.frames)
        && route.frames.len() > prefix.frames.len()
}

pub(crate) fn route_checkpoint(
    root_checkpoint_sha256: Digest,
    route: &InputTape,
) -> Result<Digest, TacticQCampaignError> {
    route
        .validate()
        .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
    let bytes = serde_json::to_vec(route)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(ROUTE_CHECKPOINT_SCHEMA_V1);
    hasher.update(root_checkpoint_sha256.0);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[derive(Debug)]
pub enum TacticQCampaignError {
    InvalidState(&'static str),
    Features(String),
    Tape(String),
    Io(String),
    Serialization(String),
    Frontier(String),
    LearnerState(LearnerStateError),
    Catalog(LiveTacticCatalogError),
    Exploration(TacticExplorationError),
    Transition(OptionTransitionError),
    Values(OptionValueError),
    Shaping(ShapingError),
    Hindsight(HindsightError),
    FrozenPolicy(TacticFrozenPolicyError),
    Native(NativeTacticWorkerError),
    GeneralizedValue(GeneralizedTacticValueError),
}

impl fmt::Display for TacticQCampaignError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidState(message) => {
                write!(formatter, "tactic-Q campaign invalid: {message}")
            }
            Self::Features(message) => write!(formatter, "tactic-Q features failed: {message}"),
            Self::Tape(message) => write!(formatter, "tactic-Q tape failed: {message}"),
            Self::Io(message) => write!(formatter, "tactic-Q checkpoint I/O failed: {message}"),
            Self::Serialization(message) => {
                write!(formatter, "tactic-Q serialization failed: {message}")
            }
            Self::Frontier(message) => write!(formatter, "tactic-Q frontier failed: {message}"),
            Self::LearnerState(error) => write!(formatter, "tactic-Q state failed: {error}"),
            Self::Catalog(error) => write!(formatter, "tactic-Q catalog failed: {error}"),
            Self::Exploration(error) => write!(formatter, "tactic-Q selection failed: {error}"),
            Self::Transition(error) => write!(formatter, "tactic-Q transition failed: {error}"),
            Self::Values(error) => write!(formatter, "tactic-Q refit failed: {error}"),
            Self::Shaping(error) => write!(formatter, "tactic-Q reward failed: {error}"),
            Self::Hindsight(error) => write!(formatter, "tactic-Q hindsight failed: {error}"),
            Self::FrozenPolicy(error) => write!(formatter, "tactic-Q freeze failed: {error}"),
            Self::Native(error) => write!(formatter, "tactic-Q native execution failed: {error}"),
            Self::GeneralizedValue(error) => {
                write!(formatter, "tactic-Q generalized value failed: {error}")
            }
        }
    }
}

impl Error for TacticQCampaignError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::LearnerState(error) => Some(error),
            Self::Catalog(error) => Some(error),
            Self::Exploration(error) => Some(error),
            Self::Transition(error) => Some(error),
            Self::Values(error) => Some(error),
            Self::Shaping(error) => Some(error),
            Self::Hindsight(error) => Some(error),
            Self::FrozenPolicy(error) => Some(error),
            Self::Native(error) => Some(error),
            Self::GeneralizedValue(error) => Some(error),
            _ => None,
        }
    }
}

impl From<LearnerStateError> for TacticQCampaignError {
    fn from(value: LearnerStateError) -> Self {
        Self::LearnerState(value)
    }
}

impl From<LiveTacticCatalogError> for TacticQCampaignError {
    fn from(value: LiveTacticCatalogError) -> Self {
        Self::Catalog(value)
    }
}

impl From<TacticExplorationError> for TacticQCampaignError {
    fn from(value: TacticExplorationError) -> Self {
        Self::Exploration(value)
    }
}

impl From<OptionTransitionError> for TacticQCampaignError {
    fn from(value: OptionTransitionError) -> Self {
        Self::Transition(value)
    }
}

impl From<OptionValueError> for TacticQCampaignError {
    fn from(value: OptionValueError) -> Self {
        Self::Values(value)
    }
}

impl From<ShapingError> for TacticQCampaignError {
    fn from(value: ShapingError) -> Self {
        Self::Shaping(value)
    }
}

impl From<NativeTacticWorkerError> for TacticQCampaignError {
    fn from(value: NativeTacticWorkerError) -> Self {
        Self::Native(value)
    }
}

impl From<GeneralizedTacticValueError> for TacticQCampaignError {
    fn from(value: GeneralizedTacticValueError) -> Self {
        Self::GeneralizedValue(value)
    }
}

#[cfg(test)]
#[path = "tactic_q_campaign/tests.rs"]
mod tests;
