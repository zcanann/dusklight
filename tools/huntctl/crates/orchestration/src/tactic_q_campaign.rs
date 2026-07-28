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
    ensure_generalized_value_acquisition, ensure_terminal_support_type_acquisitions,
    retain_generalized_value_acquisition,
};
use dusklight_learning::tactic_frozen_policy::{TacticFrozenPolicy, TacticFrozenPolicyError};
use dusklight_proposals::behavior_archive::{
    BehaviorArchive, MAX_BEHAVIOR_ARCHIVE_ENTRIES, TacticEndpointDescriptor, TacticStateDescriptor,
    tactic_state_descriptor,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

pub const TACTIC_Q_CAMPAIGN_SCHEMA_V1: &str = "dusklight-tactic-q-campaign/v1";
pub const TACTIC_Q_CHECKPOINT_SCHEMA_V2: &str = "dusklight-tactic-q-checkpoint/v2";
pub const TACTIC_Q_CHECKPOINT_SCHEMA_V3: &str = "dusklight-tactic-q-checkpoint/v3";
pub const TACTIC_Q_CHECKPOINT_EXTENSION: &str = "dtqz";
pub const TACTIC_Q_CHECKPOINT_SERIALIZATION_BENCHMARK_SCHEMA_V1: &str =
    "dusklight-tactic-q-checkpoint-serialization-benchmark/v1";
pub const TACTIC_Q_FINAL_RESULT_SCHEMA_V1: &str = "dusklight-tactic-q-final-result/v1";
pub const TACTIC_Q_FINAL_RESULT_SCHEMA_V2: &str = "dusklight-tactic-q-final-result/v2";
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
    pub model_config: OptionValueConfig,
    pub exploration: TacticExplorationConfig,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticQFinalResult {
    pub schema: String,
    pub content_sha256: Digest,
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
    pub reward: f32,
    pub best_mean_q: Option<f64>,
    /// Learned terminal-supported cost from the frontier through first hit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicted_terminal_ticks_to_go: Option<f64>,
    /// Root-relative route cost: replayed prefix plus learned ticks-to-go.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicted_total_terminal_ticks: Option<f64>,
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
    model_config: OptionValueConfig,
    exploration: TacticExplorationConfig,
    model: Option<OptionValueModel>,
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
            model_config,
            exploration,
            model: None,
            visited_states,
            hindsight,
        })
    }

    pub fn model(&self) -> Option<&OptionValueModel> {
        self.model.as_ref()
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
        let mut training_replay = self.training_replay.clone();
        let mut training_replay_routes = self.training_replay_routes.clone();
        let mut training_episode_groups = self.training_episode_groups.clone();
        let mut identities = self.training_identities.clone();
        let mut visited_states = self.visited_states.clone();
        let mut admitted = 0_usize;

        for corpus in corpora {
            if corpus.feature_schema_sha256 != self.feature_schema_sha256
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
                    self.feature_schema_sha256,
                    self.root_checkpoint_sha256,
                    transition,
                    route,
                )?;
                let identity = transition.replay_identity_sha256()?;
                if identities.insert(identity) {
                    training_replay.push(transition.clone());
                    training_replay_routes.push(route.clone());
                    training_episode_groups.push(*episode_group);
                    visited_states.insert(tactic_state_descriptor(
                        &transition.before,
                        transition.before.terminal.reached == Some(true),
                    ));
                    visited_states.insert(tactic_state_descriptor(
                        &transition.after,
                        transition.value_sample.terminal,
                    ));
                    admitted = admitted.saturating_add(1);
                }
            }
        }

        let model = replay_model(
            self.feature_schema_sha256,
            self.objective_sha256,
            &training_replay,
            &training_episode_groups,
            &self.model_config,
        )?;
        self.training_replay = training_replay;
        self.training_replay_routes = training_replay_routes;
        self.training_episode_groups = training_episode_groups;
        self.training_identities = identities;
        self.visited_states = visited_states;
        self.model = model;
        Ok(admitted)
    }

    pub fn visited_state_count(&self) -> usize {
        self.visited_states.len()
    }

    pub fn hindsight_replay(&self) -> &HindsightOptionReplay {
        &self.hindsight
    }

    pub fn frontier_archive(&self) -> Result<BehaviorArchive, TacticQCampaignError> {
        let mut archive = BehaviorArchive::default();
        // Every evaluated proposal is attached to an authenticated route from
        // the campaign root, even though only one winner advances the current
        // executable path. Preserve those alternatives as branchable frontier
        // evidence instead of collapsing exploration to the winner lineage.
        // BehaviorArchive keeps one short elite per semantic state cell and
        // selects cells by novelty, so repeated choke outcomes cannot crowd
        // out distinct progress, terminal, or low-cost endpoints.
        for (index, ((transition, route), episode_group)) in self
            .training_replay
            .iter()
            .zip(&self.training_replay_routes)
            .zip(&self.training_episode_groups)
            .enumerate()
        {
            // Model-only evidence teaches return without granting the policy a
            // route to replay. Terminal endpoints are leaves; refinement must
            // branch from their source state, never execute beyond the goal.
            if *episode_group == TACTIC_Q_MODEL_ONLY_EPISODE_GROUP
                || transition.value_sample.terminal
            {
                continue;
            }
            archive
                .consider_tactic_endpoint(
                    self.root_checkpoint_sha256,
                    transition.clone(),
                    route.clone(),
                    index as u64,
                )
                .map_err(|error| TacticQCampaignError::Frontier(error.to_string()))?;
        }
        Ok(archive)
    }

    /// Count the bounded semantic frontier without cloning every retained
    /// transition and complete route tape.
    ///
    /// Native orchestration records this count after every decision. Building
    /// the executable archive there made a diagnostic integer perform the
    /// same allocation-heavy reconstruction used for an actual branch. The
    /// archive owns one elite per state descriptor, so the count can be
    /// derived directly and capped by the same archive bound.
    pub fn frontier_cell_count(&self) -> usize {
        self.training_replay
            .iter()
            .zip(&self.training_episode_groups)
            .filter(|(transition, episode_group)| {
                **episode_group != TACTIC_Q_MODEL_ONLY_EPISODE_GROUP
                    && !transition.value_sample.terminal
            })
            .map(|(transition, _)| tactic_state_descriptor(&transition.after, false))
            .collect::<BTreeSet<_>>()
            .len()
            .min(MAX_BEHAVIOR_ARCHIVE_ENTRIES)
    }

    pub fn demonstration_frontier_count(&self) -> usize {
        self.training_replay
            .iter()
            .zip(&self.training_episode_groups)
            .filter(|(transition, episode_group)| {
                **episode_group == TACTIC_Q_DEMONSTRATION_EPISODE_GROUP
                    && !transition.value_sample.terminal
            })
            .map(|(transition, _)| tactic_state_descriptor(&transition.after, false))
            .collect::<BTreeSet<_>>()
            .len()
            .min(MAX_BEHAVIOR_ARCHIVE_ENTRIES)
    }

    pub fn graph(&self) -> Result<TacticCampaignGraph, TacticQCampaignError> {
        let root = self
            .replay
            .first()
            .ok_or(TacticQCampaignError::InvalidState(
                "campaign graph requires replay",
            ))?;
        let root_checkpoint_sha256 = root.source_checkpoint_sha256;
        // One realized PAD checkpoint can legitimately have multiple
        // learner-facing snapshots when distinct tactic labels compile to the
        // same input. `recent_option` records that provenance, so graph nodes
        // are identified by both the restorable checkpoint and fact snapshot.
        let mut nodes = BTreeMap::<(Digest, Digest), TacticCampaignGraphNode>::new();
        let mut edges = Vec::with_capacity(self.replay.len());
        for ((transition, route), episode_group) in self
            .replay
            .iter()
            .zip(&self.replay_routes)
            .zip(&self.episode_groups)
        {
            let start = usize::try_from(transition.execution.realized_tape_range.start_frame)
                .map_err(|_| TacticQCampaignError::InvalidState("graph tape range overflows"))?;
            let before_node = TacticCampaignGraphNode {
                checkpoint_sha256: transition.source_checkpoint_sha256,
                state_sha256: transition.before_state_sha256,
                state: transition.before.clone(),
                route_tape: tape_prefix(route, start),
            };
            let after_node = TacticCampaignGraphNode {
                checkpoint_sha256: transition.next_checkpoint_sha256,
                state_sha256: transition.after_state_sha256,
                state: transition.after.clone(),
                route_tape: route.clone(),
            };
            insert_graph_node(&mut nodes, before_node)?;
            insert_graph_node(&mut nodes, after_node)?;
            edges.push(TacticCampaignGraphEdge {
                episode_group: *episode_group,
                before_state_sha256: transition.before_state_sha256,
                after_state_sha256: transition.after_state_sha256,
                source_checkpoint_sha256: transition.source_checkpoint_sha256,
                next_checkpoint_sha256: transition.next_checkpoint_sha256,
                action: transition.value_sample.action.clone(),
                execution: transition.execution.clone(),
                reward: transition.value_sample.reward,
                terminal: transition.value_sample.terminal,
                route_tape: route.clone(),
            });
        }
        let root_state_sha256 = root.before_state_sha256;
        let mut reachable = BTreeSet::from([(root_checkpoint_sha256, root.before_state_sha256)]);
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
        Ok(TacticCampaignGraph {
            schema: "dusklight-tactic-campaign-graph/v1".into(),
            root_checkpoint_sha256,
            root_state_sha256,
            root_connected: reachable.len() == nodes.len(),
            nodes: nodes.into_values().collect(),
            edges,
        })
    }

    pub fn graph_projection(&self) -> Result<TacticCampaignGraphProjection, TacticQCampaignError> {
        let root = self
            .replay
            .first()
            .ok_or(TacticQCampaignError::InvalidState(
                "campaign graph requires replay",
            ))?;
        let root_checkpoint_sha256 = root.source_checkpoint_sha256;
        let root_state_sha256 = root.before_state_sha256;
        let current_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &self.route_tape)?;
        let retained = self
            .frontier_archive()?
            .tactic_route_checkpoints()
            .collect::<BTreeSet<_>>();
        let mut nodes = BTreeMap::<(Digest, Digest), TacticCampaignGraphProjectionNode>::new();
        let mut edges = Vec::with_capacity(self.replay.len());
        for (edge_index, (transition, episode_group)) in
            self.replay.iter().zip(&self.episode_groups).enumerate()
        {
            for (checkpoint_sha256, state_sha256, state) in [
                (
                    transition.source_checkpoint_sha256,
                    transition.before_state_sha256,
                    &transition.before,
                ),
                (
                    transition.next_checkpoint_sha256,
                    transition.after_state_sha256,
                    &transition.after,
                ),
            ] {
                let node = TacticCampaignGraphProjectionNode {
                    checkpoint_sha256,
                    state_sha256,
                    stage: state.world.stage.clone(),
                    room: state.world.room,
                    player_position: state.player.position_f32_bits.map(f32::from_bits),
                    terminal: state.terminal.reached == Some(true),
                    retained_frontier: retained.contains(&checkpoint_sha256),
                    current: checkpoint_sha256 == current_checkpoint_sha256
                        && state_sha256 == self.current.snapshot_sha256,
                };
                let identity = (checkpoint_sha256, state_sha256);
                if nodes
                    .get(&identity)
                    .is_some_and(|existing| existing != &node)
                {
                    return Err(TacticQCampaignError::InvalidState(
                        "one checkpoint-state identity has conflicting projected graph nodes",
                    ));
                }
                nodes.entry(identity).or_insert(node);
            }
            edges.push(TacticCampaignGraphProjectionEdge {
                edge_index: edge_index as u64,
                episode_group: *episode_group,
                before_state_sha256: transition.before_state_sha256,
                after_state_sha256: transition.after_state_sha256,
                source_checkpoint_sha256: transition.source_checkpoint_sha256,
                next_checkpoint_sha256: transition.next_checkpoint_sha256,
                option_id: transition.value_sample.action.option_id.clone(),
                reward: transition.value_sample.reward,
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
        Ok(TacticCampaignGraphProjection {
            schema: "dusklight-tactic-campaign-graph-projection/v1".into(),
            root_checkpoint_sha256,
            root_state_sha256,
            root_connected: reachable.len() == nodes.len(),
            frontier_cells: retained.len(),
            nodes: nodes.into_values().collect(),
            edges,
        })
    }

    pub fn diagnostics(&self) -> Result<TacticCampaignDiagnostics, TacticQCampaignError> {
        let archive = self.frontier_archive()?;
        let graph = self.graph_projection()?;
        let mut compositions = BTreeMap::<u64, Vec<Digest>>::new();
        let mut selected_actions = BTreeSet::new();
        for (transition, episode_group) in self.replay.iter().zip(&self.episode_groups) {
            let digest = action_digest(&transition.value_sample.action)?;
            selected_actions.insert(digest);
            compositions.entry(*episode_group).or_default().push(digest);
        }
        let mut composition_counts = BTreeMap::<Vec<Digest>, usize>::new();
        for composition in compositions.into_values().filter(|row| !row.is_empty()) {
            *composition_counts.entry(composition).or_default() += 1;
        }
        Ok(TacticCampaignDiagnostics {
            replay_rows: self.replay.len(),
            frontier_cells: archive.tactic_len(),
            logical_frontier_records: graph.nodes.len(),
            directly_restorable_native_frontiers: 0,
            replay_only_frontiers: archive.tactic_len(),
            unique_selected_actions: selected_actions.len(),
            zero_diversity_selection: self.replay.len() >= 2 && selected_actions.len() <= 1,
            repeated_identical_compositions: composition_counts.values().any(|count| *count > 1),
            no_progress_loop: has_no_progress_loop(&self.replay, &self.episode_groups)?,
            frontier_lost_root_connectivity: !graph.root_connected,
        })
    }

    /// Returns one root and one retained frontier branch on every call. The
    /// retained choices rotate from a seeded offset across every eligible
    /// archive cell; root connectivity is sampled explicitly instead of being
    /// left to archive luck.
    pub fn sample_root_and_frontier(
        &self,
        seed: u64,
        round: u64,
        reference: &[TacticEndpointDescriptor],
        maximum_route_frames: usize,
    ) -> Result<[TacticCampaignBranch; 2], TacticQCampaignError> {
        let first = self
            .replay
            .first()
            .ok_or(TacticQCampaignError::InvalidState(
                "frontier sampling requires replay",
            ))?;
        let first_route = &self.replay_routes[0];
        let root_frames = usize::try_from(first.execution.realized_tape_range.start_frame)
            .map_err(|_| TacticQCampaignError::InvalidState("root tape range overflows"))?;
        let root_route = tape_prefix(first_route, root_frames);
        let root_identity = route_checkpoint(self.root_checkpoint_sha256, &root_route)?;
        let root = TacticCampaignBranch {
            kind: TacticBranchKind::Root,
            logical_frontier: LogicalTacticFrontierRecord {
                identity_sha256: root_identity,
                state_sha256: first.before_state_sha256,
                route_frames: root_route.frames.len() as u64,
                replayed_prefix_ticks: 0,
            },
            restorable_native_checkpoint: None,
            acquisition: None,
            state: first.before.clone(),
            route_tape: root_route,
            descriptor: None,
        };
        let archive = self.frontier_archive()?;
        let choices = archive
            .select_tactic_frontier(reference, archive.tactic_len())
            .into_iter()
            .filter(|entry| entry.route_tape.frames.len() <= maximum_route_frames)
            .collect::<Vec<_>>();
        if choices.is_empty() {
            return Err(TacticQCampaignError::InvalidState(
                "frontier archive has no eligible restorable endpoint",
            ));
        }
        let index = seeded_frontier_index(seed, round, choices.len());
        let selected = &choices[index];
        let replayed_prefix_ticks = selected
            .route_tape
            .frames
            .len()
            .checked_sub(root_frames)
            .ok_or(TacticQCampaignError::InvalidState(
                "frontier route precedes its native root",
            ))? as u64;
        let frontier = TacticCampaignBranch {
            kind: TacticBranchKind::RetainedFrontier,
            logical_frontier: LogicalTacticFrontierRecord {
                identity_sha256: selected.route_checkpoint_sha256,
                state_sha256: selected.transition.after_state_sha256,
                route_frames: selected.route_tape.frames.len() as u64,
                replayed_prefix_ticks,
            },
            restorable_native_checkpoint: None,
            acquisition: None,
            state: selected.transition.after.clone(),
            route_tape: selected.route_tape.clone(),
            descriptor: Some(selected.descriptor.clone()),
        };
        Ok([root, frontier])
    }

    /// Return the authenticated root plus one learned frontier acquisition.
    ///
    /// Authenticated terminal state and shared predicted future return over the
    /// actions executable at the endpoint determine exploitation. Expansion
    /// count, model distance, and exact-critic variance then preserve generic
    /// exploration pressure. Keeping coverage behind learned return is
    /// important because every batch adds several fresh siblings: a hard
    /// least-expanded tier otherwise prevents the learner from revisiting and
    /// deepening a valuable frontier. The last edge's immediate cost is
    /// evidence only, not a myopic frontier-ordering rule.
    pub fn sample_root_and_ranked_frontier<E, AE, F, A>(
        &self,
        seed: u64,
        round: u64,
        reference: &[TacticEndpointDescriptor],
        maximum_route_frames: usize,
        demonstration_curriculum: bool,
        goal_distance_feature: usize,
        encode: &F,
        applicable_actions: &A,
    ) -> Result<[TacticCampaignBranch; 2], TacticQCampaignError>
    where
        E: fmt::Display,
        AE: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(&FactSnapshot) -> Result<Vec<OptionActionDescriptor>, AE>,
    {
        let [root, _] =
            self.sample_root_and_frontier(seed, round, reference, maximum_route_frames)?;
        let root_frames = root.route_tape.frames.len();
        let archive = self.frontier_archive()?;
        let mut choices = archive
            .select_tactic_frontier(reference, archive.tactic_len())
            .into_iter()
            .filter(|entry| entry.route_tape.frames.len() <= maximum_route_frames)
            .collect::<Vec<_>>();
        if demonstration_curriculum {
            let demonstration_endpoints = self
                .training_replay
                .iter()
                .zip(&self.training_episode_groups)
                .filter(|(_, group)| **group == TACTIC_Q_DEMONSTRATION_EPISODE_GROUP)
                .map(|(transition, _)| {
                    (
                        transition.next_checkpoint_sha256,
                        transition.after_state_sha256,
                    )
                })
                .collect::<BTreeSet<_>>();
            let demonstration_choices = choices
                .iter()
                .filter(|entry| {
                    demonstration_endpoints.contains(&(
                        entry.route_checkpoint_sha256,
                        entry.transition.after_state_sha256,
                    ))
                })
                .cloned()
                .collect::<Vec<_>>();
            if !demonstration_choices.is_empty() {
                choices = demonstration_choices;
            }
        }
        if choices.is_empty() {
            return Err(TacticQCampaignError::InvalidState(
                "frontier archive has no eligible learned acquisition",
            ));
        }
        if !demonstration_curriculum && choices.len() > MAX_RANKED_FRONTIER_CANDIDATES {
            let offset = seeded_frontier_index(seed, round, choices.len());
            choices.rotate_left(offset);
            choices.truncate(MAX_RANKED_FRONTIER_CANDIDATES);
        }
        let tie_offset = seeded_frontier_index(seed, round, choices.len());
        let choice_count = choices.len();
        let generalized_model = (!demonstration_curriculum && self.training_replay.len() >= 2)
            .then(|| {
                GeneralizedTacticValueModel::fit_fitted_q_transitions(
                    &self.training_replay,
                    goal_distance_feature,
                    self.model_config.fitted_q.iterations,
                    self.model_config.fitted_q.discount,
                )
            })
            .transpose()?;
        let mut ranked = choices
            .into_iter()
            .enumerate()
            .map(|(novelty_rank, entry)| {
                let acquisition_estimates = if demonstration_curriculum {
                    (None, None, None, None)
                } else {
                    let features = encode(&entry.transition.after)
                        .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
                    if features.is_empty() || features.iter().any(|value| !value.is_finite()) {
                        return Err(TacticQCampaignError::Features(
                            "frontier encoding is empty or non-finite".into(),
                        ));
                    }
                    let applicable = applicable_actions(&entry.transition.after)
                        .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
                    if applicable.is_empty() {
                        return Err(TacticQCampaignError::InvalidState(
                            "frontier has no applicable executable actions".into(),
                        ));
                    }
                    if let Some(model) = generalized_model.as_ref() {
                        let context =
                            GeneralizedTacticContext::from_facts(&entry.transition.after)?;
                        let estimates = model.rank(&features, &context, &applicable)?;
                        (
                            estimates
                                .first()
                                .map(|value| f64::from(value.outcome.reward)),
                            estimates.first().and_then(|value| {
                                (value.outcome.terminal > 0.0
                                    && value.outcome.duration_ticks.is_finite()
                                    && value.outcome.duration_ticks > 0.0)
                                    .then_some(f64::from(value.outcome.duration_ticks))
                            }),
                            None,
                            estimates
                                .iter()
                                .map(|value| value.nearest_distance)
                                .max_by(f32::total_cmp),
                        )
                    } else {
                        let estimates = self
                            .model
                            .as_ref()
                            .map(|model| model.rank_available_options(&features, &applicable))
                            .transpose()?;
                        (
                            estimates
                                .as_ref()
                                .and_then(|values| values.ranked.first())
                                .map(|value| value.mean_q),
                            None,
                            estimates.as_ref().and_then(|values| {
                                values
                                    .ranked
                                    .iter()
                                    .map(|value| value.ensemble_variance)
                                    .max_by(f64::total_cmp)
                            }),
                            None,
                        )
                    }
                };
                let (
                    best_mean_q,
                    predicted_terminal_ticks_to_go,
                    maximum_ensemble_variance,
                    generalized_nearest_distance,
                ) = acquisition_estimates;
                let expansion_count = self
                    .replay
                    .iter()
                    .filter(|transition| {
                        transition.before_state_sha256 == entry.transition.after_state_sha256
                            && transition.source_checkpoint_sha256 == entry.route_checkpoint_sha256
                    })
                    .count() as u64;
                let replayed_prefix_ticks = entry
                    .route_tape
                    .frames
                    .len()
                    .checked_sub(root_frames)
                    .ok_or(TacticQCampaignError::InvalidState(
                    "learned frontier route precedes its native root",
                ))? as u64;
                let acquisition = TacticFrontierAcquisition {
                    expansion_count,
                    terminal: entry.transition.value_sample.terminal,
                    reward: entry.transition.value_sample.reward,
                    best_mean_q,
                    predicted_terminal_ticks_to_go,
                    predicted_total_terminal_ticks: predicted_terminal_ticks_to_go
                        .map(|ticks| replayed_prefix_ticks as f64 + ticks),
                    maximum_ensemble_variance,
                    generalized_nearest_distance,
                    novelty_rank: novelty_rank as u64,
                    replayed_prefix_ticks,
                };
                let tie_rank = (novelty_rank + choice_count - tie_offset) % choice_count;
                Ok((entry, acquisition, tie_rank))
            })
            .collect::<Result<Vec<_>, TacticQCampaignError>>()?;
        ranked.sort_by(|left, right| {
            if demonstration_curriculum {
                // This lane is coverage over human-connected states, not an
                // imitation-policy score. Every checkpoint receives a native
                // alternative-action trial before any one is repeated.
                left.1
                    .expansion_count
                    .cmp(&right.1.expansion_count)
                    .then_with(|| left.2.cmp(&right.2))
            } else {
                compare_frontier_acquisition(&left.1, &right.1).then_with(|| left.2.cmp(&right.2))
            }
            .then_with(|| left.1.novelty_rank.cmp(&right.1.novelty_rank))
            .then_with(|| left.0.descriptor.cmp(&right.0.descriptor))
        });
        let (selected, acquisition, _) = ranked
            .into_iter()
            .next()
            .expect("nonempty learned frontier ranking");
        let frontier = TacticCampaignBranch {
            kind: TacticBranchKind::RetainedFrontier,
            logical_frontier: LogicalTacticFrontierRecord {
                identity_sha256: selected.route_checkpoint_sha256,
                state_sha256: selected.transition.after_state_sha256,
                route_frames: selected.route_tape.frames.len() as u64,
                replayed_prefix_ticks: acquisition.replayed_prefix_ticks,
            },
            restorable_native_checkpoint: None,
            acquisition: Some(acquisition),
            state: selected.transition.after.clone(),
            route_tape: selected.route_tape,
            descriptor: Some(selected.descriptor),
        };
        Ok([root, frontier])
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore_branch<A>(
        &mut self,
        branch: &TacticCampaignBranch,
        episode_group: u64,
        registry: &FactRegistry,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        entry_applicable: A,
    ) -> Result<(), TacticQCampaignError>
    where
        A: Fn(&TacticAssetDescription) -> bool,
    {
        branch
            .state
            .validate()
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        branch
            .route_tape
            .validate()
            .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
        let frontier = self.frontier_archive()?;
        let native_root_frames = self
            .replay
            .first()
            .and_then(|first| usize::try_from(first.execution.realized_tape_range.start_frame).ok())
            .ok_or(TacticQCampaignError::InvalidState(
                "campaign has no native root boundary",
            ))?;
        let expected_replayed_prefix_ticks = branch
            .route_tape
            .frames
            .len()
            .checked_sub(native_root_frames)
            .ok_or(TacticQCampaignError::InvalidState(
                "frontier route precedes its native root",
            ))? as u64;
        let admitted = match branch.kind {
            TacticBranchKind::Root => self.replay.first().is_some_and(|first| {
                first.before_state_sha256 == branch.logical_frontier.state_sha256
                    && first.source_checkpoint_sha256 == branch.logical_frontier.identity_sha256
            }),
            TacticBranchKind::RetainedFrontier => frontier
                .select_tactic_frontier(&[], frontier.tactic_len())
                .iter()
                .any(|entry| {
                    entry.transition.after_state_sha256 == branch.logical_frontier.state_sha256
                        && entry.route_checkpoint_sha256 == branch.logical_frontier.identity_sha256
                }),
        };
        if !admitted
            || self.episode_groups.contains(&episode_group)
            || branch.restorable_native_checkpoint.is_some()
            || branch.logical_frontier.state_sha256
                != branch
                    .state
                    .content_sha256()
                    .map_err(|error| TacticQCampaignError::Features(error.to_string()))?
            || branch.state.tape_frame != branch.route_tape.frames.len() as u64
            || branch.logical_frontier.route_frames != branch.route_tape.frames.len() as u64
            || branch.logical_frontier.replayed_prefix_ticks != expected_replayed_prefix_ticks
            || branch.logical_frontier.identity_sha256
                != route_checkpoint(self.root_checkpoint_sha256, &branch.route_tape)?
        {
            return Err(TacticQCampaignError::InvalidState(
                "frontier branch is detached or reuses an episode group",
            ));
        }
        self.current = LearnerState::build(
            branch.state.clone(),
            registry,
            catalog,
            blueprints,
            entry_applicable,
        )?;
        self.route_tape = branch.route_tape.clone();
        self.episode_group = episode_group;
        Ok(())
    }

    pub fn checkpoint(&self) -> Result<TacticQCampaignCheckpoint, TacticQCampaignError> {
        let mut checkpoint = TacticQCampaignCheckpoint {
            schema: TACTIC_Q_CHECKPOINT_SCHEMA_V3.into(),
            content_sha256: Digest::ZERO,
            feature_schema_sha256: self.feature_schema_sha256,
            objective_sha256: self.objective_sha256,
            root_checkpoint_sha256: self.root_checkpoint_sha256,
            episode_group: self.episode_group,
            decision_index: self.decision_index,
            current: self.current.clone(),
            route_tape: self.route_tape.clone(),
            replay: self.replay.clone(),
            replay_routes: self.replay_routes.clone(),
            episode_groups: self.episode_groups.clone(),
            training_replay: self.training_replay.clone(),
            training_replay_routes: self.training_replay_routes.clone(),
            training_episode_groups: self.training_episode_groups.clone(),
            model_config: self.model_config.clone(),
            exploration: self.exploration,
        };
        checkpoint.content_sha256 = checkpoint_digest(&checkpoint)?;
        validate_checkpoint(&checkpoint)?;
        Ok(checkpoint)
    }

    /// Seal the fitted critic as an independently reloadable greedy policy.
    ///
    /// The executable action schema is derived from the checkpoint's complete
    /// action mask. This is intentionally not supplied by the caller: campaigns
    /// may extend the default catalog with goal-conditioned tactics, and
    /// freezing against a separately reconstructed default catalog would stamp
    /// the policy with the wrong executable universe.
    pub fn freeze_greedy_policy(&self) -> Result<TacticFrozenPolicy, TacticQCampaignError> {
        let checkpoint = self.checkpoint()?;
        let first = self
            .replay
            .first()
            .ok_or(TacticQCampaignError::InvalidState(
                "freezing a tactic policy requires replay",
            ))?;
        let training_batch = OptionValueBatch::new(
            self.feature_schema_sha256,
            self.objective_sha256,
            first.value_sample.state.len(),
            self.training_replay
                .iter()
                .map(|transition| transition.value_sample.clone())
                .collect(),
            self.training_episode_groups.clone(),
        )?;
        let action_universe_sha256 = Digest(
            Sha256::digest(
                serde_json::to_vec(
                    &self
                        .current
                        .action_mask
                        .iter()
                        .map(|entry| &entry.descriptor)
                        .collect::<Vec<_>>(),
                )
                .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?,
            )
            .into(),
        );
        TacticFrozenPolicy::freeze(
            checkpoint.content_sha256,
            self.root_checkpoint_sha256,
            first.before_state_sha256,
            self.feature_schema_sha256,
            action_universe_sha256,
            self.objective_sha256,
            training_batch,
            checkpoint.model_config,
        )
        .map_err(TacticQCampaignError::FrozenPolicy)
    }

    /// Writes one immutable, content-addressed checkpoint. A completed file is
    /// always resumable; a crash can leave only an unreferenced `.partial`
    /// file, never a half-written checkpoint at the final path.
    pub fn write_checkpoint(&self, directory: &Path) -> Result<PathBuf, TacticQCampaignError> {
        let checkpoint = self.checkpoint()?;
        tactic_q_checkpoint_store::write_checkpoint_with_local_store(&checkpoint, directory)
    }

    pub fn write_checkpoint_with_store(
        &self,
        directory: &Path,
        content_root: &Path,
    ) -> Result<PathBuf, TacticQCampaignError> {
        let checkpoint = self.checkpoint()?;
        tactic_q_checkpoint_store::write_checkpoint(&checkpoint, directory, content_root)
    }

    pub fn read_checkpoint(path: &Path) -> Result<Self, TacticQCampaignError> {
        Self::resume(Self::read_checkpoint_payload(path)?)
    }

    /// Reads and validates the durable checkpoint payload without rebuilding
    /// the fitted model. Orchestration uses this to authenticate run-specific
    /// identities before calling `resume`.
    pub fn read_checkpoint_payload(
        path: &Path,
    ) -> Result<TacticQCampaignCheckpoint, TacticQCampaignError> {
        tactic_q_checkpoint_store::read_checkpoint(path)
    }

    /// Measures only deterministic checkpoint-root serialization. Native
    /// simulation, object-store writes, filesystem sync, and report projection
    /// are deliberately outside this codec boundary.
    pub fn benchmark_checkpoint_serialization(
        legacy_json_path: &Path,
        current_checkpoint_path: &Path,
        iterations: u64,
    ) -> Result<TacticQCheckpointSerializationBenchmark, TacticQCampaignError> {
        tactic_q_checkpoint_store::benchmark_checkpoint_serialization(
            legacy_json_path,
            current_checkpoint_path,
            iterations,
        )
    }

    pub fn resume(mut checkpoint: TacticQCampaignCheckpoint) -> Result<Self, TacticQCampaignError> {
        validate_checkpoint(&checkpoint)?;
        if checkpoint.schema == TACTIC_Q_CHECKPOINT_SCHEMA_V2 {
            checkpoint.training_replay = checkpoint.replay.clone();
            checkpoint.training_replay_routes = checkpoint.replay_routes.clone();
            checkpoint.training_episode_groups = checkpoint.episode_groups.clone();
        }
        let model = replay_model(
            checkpoint.feature_schema_sha256,
            checkpoint.objective_sha256,
            &checkpoint.training_replay,
            &checkpoint.training_episode_groups,
            &checkpoint.model_config,
        )?;
        let mut visited_states = BTreeSet::from([tactic_state_descriptor(
            &checkpoint.current.snapshot,
            checkpoint.current.snapshot.terminal.reached == Some(true),
        )]);
        for transition in &checkpoint.training_replay {
            visited_states.insert(tactic_state_descriptor(
                &transition.before,
                transition.before.terminal.reached == Some(true),
            ));
            visited_states.insert(tactic_state_descriptor(
                &transition.after,
                transition.value_sample.terminal,
            ));
        }
        let hindsight = HindsightOptionReplay::new(checkpoint.feature_schema_sha256)
            .map_err(TacticQCampaignError::Hindsight)?;
        let training_identities = checkpoint
            .training_replay
            .iter()
            .map(OptionTransitionSample::replay_identity_sha256)
            .collect::<Result<BTreeSet<_>, _>>()?;
        Ok(Self {
            schema: TACTIC_Q_CAMPAIGN_SCHEMA_V1.into(),
            feature_schema_sha256: checkpoint.feature_schema_sha256,
            objective_sha256: checkpoint.objective_sha256,
            root_checkpoint_sha256: checkpoint.root_checkpoint_sha256,
            episode_group: checkpoint.episode_group,
            decision_index: checkpoint.decision_index,
            current: checkpoint.current,
            route_tape: checkpoint.route_tape,
            replay: checkpoint.replay,
            replay_routes: checkpoint.replay_routes,
            episode_groups: checkpoint.episode_groups,
            training_replay: checkpoint.training_replay,
            training_replay_routes: checkpoint.training_replay_routes,
            training_episode_groups: checkpoint.training_episode_groups,
            training_identities,
            model_config: checkpoint.model_config,
            exploration: checkpoint.exploration,
            model,
            visited_states,
            hindsight,
        })
    }

    pub fn final_result(&self) -> Result<TacticQFinalResult, TacticQCampaignError> {
        if self.current.snapshot.terminal.configured != Some(true)
            || self.current.snapshot.terminal.reached != Some(true)
            || self.replay.last().map(|row| row.after_state_sha256)
                != Some(self.current.snapshot_sha256)
        {
            return Err(TacticQCampaignError::InvalidState(
                "final result requires a native-authorized terminal replay boundary",
            ));
        }
        self.build_final_result(
            self.route_tape.clone(),
            self.replay.clone(),
            self.replay_routes.clone(),
            self.current.snapshot.clone(),
        )
    }

    /// Seal an authenticated terminal sibling evaluated at the current
    /// frontier without changing the policy-selected campaign trajectory.
    ///
    /// Native proposal batches are both learning evidence and a real bounded
    /// candidate search. A terminal sibling is therefore eligible route
    /// evidence even though it must not retroactively replace the learner's
    /// selected action.
    pub fn final_result_from_evaluated_terminal(
        &self,
        evaluated: &EvaluatedRewardedTacticOutcome,
    ) -> Result<TacticQFinalResult, TacticQCampaignError> {
        let outcome = &evaluated.outcome;
        evaluated.transition.validate()?;
        let source_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &self.route_tape)?;
        let terminal_state_sha256 = outcome
            .next_facts
            .content_sha256()
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        if !outcome.terminal
            || outcome.next_facts.terminal.configured != Some(true)
            || outcome.next_facts.terminal.reached != Some(true)
            || outcome.selected.decision_index != self.decision_index
            || outcome.selected.learner_snapshot_sha256 != self.current.snapshot_sha256
            || outcome.source_checkpoint_sha256 != self.root_checkpoint_sha256
            || !extends(&self.route_tape, &outcome.route_tape)
            || evaluated.transition.before_state_sha256 != self.current.snapshot_sha256
            || evaluated.transition.after_state_sha256 != terminal_state_sha256
            || evaluated.transition.source_checkpoint_sha256 != source_checkpoint_sha256
            || evaluated.transition.next_checkpoint_sha256
                != route_checkpoint(self.root_checkpoint_sha256, &outcome.route_tape)?
            || !evaluated.transition.value_sample.terminal
            || evaluated.transition.value_sample.action != outcome.selected.descriptor
            || evaluated.transition.execution != outcome.execution
            || evaluated.transition.value_sample.reward.to_bits()
                != evaluated.reward.training_reward.to_bits()
        {
            return Err(TacticQCampaignError::InvalidState(
                "evaluated terminal tactic is detached from the current campaign frontier",
            ));
        }
        let mut replay = self.replay.clone();
        replay.push(evaluated.transition.clone());
        let mut replay_routes = self.replay_routes.clone();
        replay_routes.push(outcome.route_tape.clone());
        self.build_final_result(
            outcome.route_tape.clone(),
            replay,
            replay_routes,
            outcome.next_facts.clone(),
        )
    }

    fn build_final_result(
        &self,
        route_tape: InputTape,
        replay: Vec<OptionTransitionSample>,
        replay_routes: Vec<InputTape>,
        terminal: FactSnapshot,
    ) -> Result<TacticQFinalResult, TacticQCampaignError> {
        let route_bytes = route_tape
            .encode()
            .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
        let replay_bytes = serde_cbor::to_vec(&(&replay, &replay_routes))
            .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
        let terminal_state_sha256 = terminal
            .content_sha256()
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        let mut result = TacticQFinalResult {
            schema: TACTIC_Q_FINAL_RESULT_SCHEMA_V2.into(),
            content_sha256: Digest::ZERO,
            objective_sha256: self.objective_sha256,
            root_checkpoint_sha256: self.root_checkpoint_sha256,
            route_tape_sha256: sha256(&route_bytes),
            replay_sha256: sha256(&replay_bytes),
            terminal_state_sha256,
            route_tape,
            replay,
            replay_routes,
            terminal,
        };
        result.content_sha256 = final_result_digest(&result)?;
        validate_final_result(&result)?;
        Ok(result)
    }

    /// Admit a native-evaluated false-to-true hindsight row only when it
    /// relabels an exact primary replay transition from this campaign. The row
    /// is refit under its own compiled objective, never the primary critic.
    pub fn admit_hindsight(
        &mut self,
        row: RelabeledHindsightOption,
    ) -> Result<&OptionValueModel, TacticQCampaignError> {
        let Some((index, _)) = self.replay.iter().enumerate().find(|(_, original)| {
            if original.value_sample.reward.to_bits() != row.original_reward.to_bits()
                || original.value_sample.terminal
            {
                return false;
            }
            let mut expected = original.value_sample.clone();
            expected.reward = row.transition.reward;
            expected.terminal = true;
            expected == row.transition
        }) else {
            return Err(TacticQCampaignError::InvalidState(
                "hindsight row does not relabel campaign replay",
            ));
        };
        self.hindsight
            .admit_and_refit(row, self.episode_groups[index], &self.model_config)
            .map_err(TacticQCampaignError::Hindsight)
    }

    pub fn decide<E, F>(
        &self,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        encode: &F,
    ) -> Result<TacticQDecision, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
    {
        let mut batch = self.decide_batch(catalog, blueprints, encode, 1)?;
        let selected = batch
            .proposals
            .pop()
            .ok_or(TacticQCampaignError::InvalidState(
                "tactic proposal batch is empty",
            ))?;
        Ok(TacticQDecision {
            ranking: batch.ranking,
            selected,
        })
    }

    pub fn decide_batch<E, F>(
        &self,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        encode: &F,
        maximum_proposals: usize,
    ) -> Result<TacticQProposalBatch, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
    {
        let live = LiveTacticCatalog::build(&self.current, catalog, blueprints)?;
        let features = encode(&self.current.snapshot)
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        if features.is_empty() || features.iter().any(|value| !value.is_finite()) {
            return Err(TacticQCampaignError::Features(
                "state encoding is empty or non-finite".into(),
            ));
        }
        let ranking = if let Some(model) = &self.model {
            live.rank(model, &features)?
        } else {
            LiveTacticRanking {
                learner_snapshot_sha256: live.learner_snapshot_sha256,
                action_universe_sha256: live.action_universe_sha256,
                choices: live.choices.clone(),
                values: AvailableOptionRanking {
                    ranked: Vec::new(),
                    unsupported: live.descriptors().cloned().collect(),
                },
            }
        };
        let current_cell = tactic_state_descriptor(&self.current.snapshot, false);
        let tried_here = self
            .training_replay
            .iter()
            .filter(|transition| tactic_state_descriptor(&transition.before, false) == current_cell)
            .map(|transition| transition.value_sample.action.option_id.as_str())
            .collect::<BTreeSet<_>>();
        let state_untried = live
            .descriptors()
            .filter(|descriptor| !tried_here.contains(descriptor.option_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let mut proposals = choose_tactic_batch_with_state_untried(
            &ranking,
            self.decision_index,
            self.exploration,
            &state_untried,
            maximum_proposals,
        )?;
        ensure_blueprint_proposal(&ranking, maximum_proposals, &mut proposals)?;
        Ok(TacticQProposalBatch { ranking, proposals })
    }

    /// Rank an ephemeral set of bounded instances under a stable tactic-family
    /// schema. The executable instances may be new at this decision; the
    /// option-value model scores exact instances it has seen and leaves new
    /// parameter combinations explicit for exploration.
    pub fn decide_parameterized_batch<E, F>(
        &self,
        proposal_catalog: &TacticAssetCatalog,
        proposal_blueprints: &[TacticBlueprint],
        family_schema_sha256: Digest,
        encode: &F,
        maximum_proposals: usize,
    ) -> Result<TacticQProposalBatch, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
    {
        self.decide_parameterized_batch_with_policy(
            proposal_catalog,
            proposal_blueprints,
            family_schema_sha256,
            encode,
            maximum_proposals,
            0,
            TacticProposalPolicy::Learned,
            None,
            false,
        )
    }

    pub fn decide_parameterized_batch_with_policy<E, F>(
        &self,
        proposal_catalog: &TacticAssetCatalog,
        proposal_blueprints: &[TacticBlueprint],
        family_schema_sha256: Digest,
        encode: &F,
        maximum_proposals: usize,
        acquisition_partition: u64,
        policy: TacticProposalPolicy,
        goal_distance_feature: Option<usize>,
        force_exploration: bool,
    ) -> Result<TacticQProposalBatch, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
    {
        self.current.validate()?;
        if family_schema_sha256 == Digest::ZERO || maximum_proposals == 0 {
            return Err(TacticQCampaignError::InvalidState(
                "parameterized tactic proposal schema or capacity is invalid",
            ));
        }
        for blueprint in proposal_blueprints {
            blueprint
                .compile_static(proposal_catalog)
                .map_err(LiveTacticCatalogError::Blueprint)?;
        }
        let applicable = ApplicableTacticChoices::enumerate(
            proposal_catalog,
            proposal_blueprints,
            |description| tactic_intrinsically_applicable(description, &self.current.snapshot),
            |_| Some(false),
        )
        .map_err(LiveTacticCatalogError::Blueprint)?;
        let mut choices = Vec::with_capacity(applicable.candidates.len());
        for entry in proposal_catalog.entries() {
            proposal_catalog
                .prepare_execution(entry.option_id())
                .map_err(LiveTacticCatalogError::Asset)?;
        }
        for (candidate, applicable) in applicable
            .candidates
            .into_iter()
            .zip(applicable.applicable_mask)
        {
            choices.push(LearnerActionMaskEntry {
                choice_id: candidate.choice_id,
                kind: candidate.kind,
                descriptor: candidate.descriptor,
                duration: candidate.duration,
                applicable,
            });
        }
        let descriptors = choices
            .iter()
            .map(|choice| choice.descriptor.clone())
            .collect::<Vec<_>>();
        let features = encode(&self.current.snapshot)
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        if features.is_empty() || features.iter().any(|value| !value.is_finite()) {
            return Err(TacticQCampaignError::Features(
                "state encoding is empty or non-finite".into(),
            ));
        }
        let values = if let Some(model) = &self.model {
            model.rank_available_options(&features, &descriptors)?
        } else {
            AvailableOptionRanking {
                ranked: Vec::new(),
                unsupported: descriptors.clone(),
            }
        };
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: self.current.snapshot_sha256,
            action_universe_sha256: family_schema_sha256,
            choices,
            values,
        };
        let current_cell = tactic_state_descriptor(&self.current.snapshot, false);
        let tried_here = self
            .training_replay
            .iter()
            .filter(|transition| tactic_state_descriptor(&transition.before, false) == current_cell)
            .map(|transition| transition.value_sample.action.option_id.as_str())
            .collect::<BTreeSet<_>>();
        let state_untried = applicable_untried_descriptors(&ranking.choices, &tried_here);
        let exploration = if force_exploration {
            TacticExplorationConfig {
                seed: self.exploration.seed,
                epsilon_per_million: 1_000_000,
            }
        } else {
            self.exploration
        };
        let mut proposals = choose_tactic_batch_for_policy(
            &ranking,
            self.decision_index,
            exploration,
            &state_untried,
            maximum_proposals,
            policy,
        )?;
        let forced_primary = if force_exploration {
            let proposal = proposals
                .first()
                .cloned()
                .ok_or(TacticQCampaignError::InvalidState(
                    "forced exploration did not produce a primary proposal",
                ))?;
            if !matches!(
                proposal.reason,
                TacticSelectionReason::Epsilon | TacticSelectionReason::UnsupportedBootstrap
            ) {
                return Err(TacticQCampaignError::InvalidState(
                    "forced exploration primary is value-selected",
                ));
            }
            Some(proposal)
        } else {
            None
        };
        if policy != TacticProposalPolicy::RandomValid {
            ensure_blueprint_proposal(&ranking, maximum_proposals, &mut proposals)?;
        }
        if policy == TacticProposalPolicy::Learned
            && self.training_replay.len() >= 2
            && let Some(goal_distance_feature) = goal_distance_feature
        {
            let model = GeneralizedTacticValueModel::fit_fitted_q_transitions(
                &self.training_replay,
                goal_distance_feature,
                self.model_config.fitted_q.iterations,
                self.model_config.fitted_q.discount,
            )?;
            let context = GeneralizedTacticContext::from_facts(&self.current.snapshot)?;
            let applicable_descriptors = ranking
                .choices
                .iter()
                .filter(|choice| choice.applicable)
                .map(|choice| choice.descriptor.clone())
                .collect::<Vec<_>>();
            // Partition zero is the dedicated terminal-support policy lane.
            // It behavior-clones the closest action at the nearest phase and
            // physical state on any authenticated successful trajectory.
            // Remaining partitions stay Q-ranked, preserving independent
            // improvement and exploration.
            let ranked_applicable = if acquisition_partition == 0 {
                model.rank_terminal_support(&features, &context, &applicable_descriptors)?
            } else {
                model.rank(&features, &context, &applicable_descriptors)?
            }
            .into_iter()
            .map(|estimate| estimate.descriptor)
            .collect::<Vec<_>>();
            ensure_generalized_value_acquisition(
                &ranked_applicable,
                acquisition_partition,
                maximum_proposals,
                &mut proposals,
            )?;
            if acquisition_partition == 0 {
                ensure_terminal_support_type_acquisitions(
                    &ranked_applicable,
                    maximum_proposals,
                    &mut proposals,
                )?;
            }
            retain_generalized_value_acquisition(&mut proposals)?;
        }
        if let Some(primary) = forced_primary {
            proposals.retain(|proposal| proposal.descriptor != primary.descriptor);
            proposals.insert(0, primary);
            proposals.truncate(maximum_proposals);
        }
        if proposals.iter().any(|proposal| {
            !ranking.choices.iter().any(|choice| {
                choice.applicable
                    && choice.choice_id == proposal.descriptor.option_id
                    && choice.descriptor == proposal.descriptor
            })
        }) {
            return Err(TacticQCampaignError::InvalidState(
                "parameterized proposal batch contains an inapplicable tactic".into(),
            ));
        }
        Ok(TacticQProposalBatch { ranking, proposals })
    }

    /// Score and capture a native proposal without mutating the retained
    /// campaign path. Callers can evaluate several outcomes from this exact
    /// boundary, choose one deterministically, and admit only that winner.
    pub fn evaluate_rewarded_outcome<E, F>(
        &self,
        outcome: NativeTacticWorkerOutcome,
        encode: &F,
        reward_spec: &TacticRewardSpec,
    ) -> Result<EvaluatedRewardedTacticOutcome, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
    {
        if outcome.selected.decision_index != self.decision_index
            || outcome.selected.learner_snapshot_sha256 != self.current.snapshot_sha256
            || outcome.source_checkpoint_sha256 != self.root_checkpoint_sha256
            || !extends(&self.route_tape, &outcome.route_tape)
        {
            return Err(TacticQCampaignError::InvalidState(
                "native proposal outcome is detached from the campaign boundary",
            ));
        }
        let state = encode(&self.current.snapshot)
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        let next_state = encode(&outcome.next_facts)
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        let endpoint = tactic_state_descriptor(&outcome.next_facts, outcome.terminal);
        let reward = reward_spec.evaluate_with_motion(
            self.feature_schema_sha256,
            &state,
            &next_state,
            outcome.execution.duration.realized_ticks,
            outcome.terminal,
            !self.visited_states.contains(&endpoint),
            outcome
                .next_facts
                .recent_option
                .as_ref()
                .and_then(|option| option.trajectory),
        )?;
        let source_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &self.route_tape)?;
        let next_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &outcome.route_tape)?;
        let transition = OptionTransitionSample::capture(
            self.feature_schema_sha256,
            source_checkpoint_sha256,
            next_checkpoint_sha256,
            self.current.snapshot.clone(),
            outcome.next_facts.clone(),
            outcome.execution.clone(),
            &outcome.route_tape,
            reward.training_reward,
            outcome.terminal,
            encode,
        )?;
        Ok(EvaluatedRewardedTacticOutcome {
            outcome,
            transition,
            reward,
        })
    }

    /// Admit every native-evaluated alternative into the deduplicated training
    /// replay without changing the executable retained path. The subsequent
    /// winner admission performs the scheduled critic refit over this complete
    /// batch.
    pub fn admit_evaluated_replay(
        &mut self,
        evaluated: &[EvaluatedRewardedTacticOutcome],
    ) -> Result<usize, TacticQCampaignError> {
        if evaluated.is_empty() {
            return Err(TacticQCampaignError::InvalidState(
                "evaluated tactic replay batch is empty",
            ));
        }
        let source_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &self.route_tape)?;
        let mut training_replay = self.training_replay.clone();
        let mut training_replay_routes = self.training_replay_routes.clone();
        let mut training_episode_groups = self.training_episode_groups.clone();
        let mut identities = self.training_identities.clone();
        let mut admitted = 0;
        for evaluated in evaluated {
            evaluated.transition.validate()?;
            if evaluated.outcome.selected.decision_index != self.decision_index
                || evaluated.outcome.selected.learner_snapshot_sha256
                    != self.current.snapshot_sha256
                || evaluated.outcome.source_checkpoint_sha256 != self.root_checkpoint_sha256
                || evaluated.transition.before_state_sha256 != self.current.snapshot_sha256
                || evaluated.transition.source_checkpoint_sha256 != source_checkpoint_sha256
                || evaluated.transition.after_state_sha256
                    != evaluated
                        .outcome
                        .next_facts
                        .content_sha256()
                        .map_err(|error| TacticQCampaignError::Features(error.to_string()))?
                || evaluated.transition.next_checkpoint_sha256
                    != route_checkpoint(self.root_checkpoint_sha256, &evaluated.outcome.route_tape)?
                || evaluated.transition.value_sample.action != evaluated.outcome.selected.descriptor
                || evaluated.transition.value_sample.reward.to_bits()
                    != evaluated.reward.training_reward.to_bits()
                || !extends(&self.route_tape, &evaluated.outcome.route_tape)
            {
                return Err(TacticQCampaignError::InvalidState(
                    "evaluated tactic replay is detached from its shared frontier",
                ));
            }
            let identity = evaluated.transition.replay_identity_sha256()?;
            if identities.insert(identity) {
                training_replay.push(evaluated.transition.clone());
                training_replay_routes.push(evaluated.outcome.route_tape.clone());
                training_episode_groups.push(self.episode_group);
                admitted += 1;
            }
        }
        self.training_replay = training_replay;
        self.training_replay_routes = training_replay_routes;
        self.training_episode_groups = training_episode_groups;
        self.training_identities = identities;
        Ok(admitted)
    }

    /// Execute and retain one native tactic boundary, then rebuild the Q model
    /// from every replay row accumulated so far.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_and_refit<W, E, F, A, R>(
        &mut self,
        worker: &mut W,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        registry: &FactRegistry,
        paths: &NativeTacticWorkerPaths,
        encode: &F,
        entry_applicable: A,
        reward: R,
    ) -> Result<TacticQCampaignStep, TacticQCampaignError>
    where
        W: PersistentTacticBatchWorker,
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(&TacticAssetDescription) -> bool,
        R: Fn(&FactSnapshot, &FactSnapshot, &OptionExecution) -> f32,
    {
        let decision = self.decide(catalog, blueprints, encode)?;
        let outcome = execute_selected_tactic(
            worker,
            &decision.selected,
            catalog,
            blueprints,
            &self.current.snapshot,
            &self.route_tape,
            None,
            paths,
        )?;
        self.retain_and_refit(
            decision,
            outcome,
            catalog,
            blueprints,
            registry,
            encode,
            entry_applicable,
            reward,
            true,
        )
    }

    /// Reward-policy variant of [`Self::execute_and_refit`]. It composes
    /// terminal bonus, exact tick cost, first-visit novelty, and optional
    /// potential shaping without granting any of them terminal authority.
    /// Replay is retained on every call; callers may batch the fitted-Q rebuild
    /// after the first model exists. A terminal outcome always forces a refit.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_and_refit_rewarded<W, E, F, A>(
        &mut self,
        worker: &mut W,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        registry: &FactRegistry,
        paths: &NativeTacticWorkerPaths,
        encode: &F,
        entry_applicable: A,
        reward_spec: &TacticRewardSpec,
        refit_model: bool,
    ) -> Result<RewardedTacticQCampaignStep, TacticQCampaignError>
    where
        W: PersistentTacticBatchWorker,
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(&TacticAssetDescription) -> bool,
    {
        let decision = self.decide(catalog, blueprints, encode)?;
        let outcome = execute_selected_tactic(
            worker,
            &decision.selected,
            catalog,
            blueprints,
            &self.current.snapshot,
            &self.route_tape,
            None,
            paths,
        )?;
        self.retain_and_refit_rewarded(
            decision,
            outcome,
            catalog,
            blueprints,
            registry,
            encode,
            entry_applicable,
            reward_spec,
            refit_model,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn retain_and_refit_rewarded<E, F, A>(
        &mut self,
        decision: TacticQDecision,
        outcome: NativeTacticWorkerOutcome,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        registry: &FactRegistry,
        encode: &F,
        entry_applicable: A,
        reward_spec: &TacticRewardSpec,
        refit_model: bool,
    ) -> Result<RewardedTacticQCampaignStep, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(&TacticAssetDescription) -> bool,
    {
        let state = encode(&self.current.snapshot)
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        let next_state = encode(&outcome.next_facts)
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        let endpoint = tactic_state_descriptor(&outcome.next_facts, outcome.terminal);
        let reward = reward_spec.evaluate_with_motion(
            self.feature_schema_sha256,
            &state,
            &next_state,
            outcome.execution.duration.realized_ticks,
            outcome.terminal,
            !self.visited_states.contains(&endpoint),
            outcome
                .next_facts
                .recent_option
                .as_ref()
                .and_then(|option| option.trajectory),
        )?;
        let training_reward = reward.training_reward;
        let refit_model = refit_model || outcome.terminal;
        let step = self.retain_and_refit(
            decision,
            outcome,
            catalog,
            blueprints,
            registry,
            encode,
            entry_applicable,
            move |_, _, _| training_reward,
            refit_model,
        )?;
        Ok(RewardedTacticQCampaignStep { step, reward })
    }

    /// Admit an already executed native outcome. This is public so alternate
    /// executors (including observation-loop workers) can share exactly the
    /// same replay and refit path.
    #[allow(clippy::too_many_arguments)]
    pub fn retain_and_refit<E, F, A, R>(
        &mut self,
        decision: TacticQDecision,
        outcome: NativeTacticWorkerOutcome,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        registry: &FactRegistry,
        encode: &F,
        entry_applicable: A,
        reward: R,
        refit_model: bool,
    ) -> Result<TacticQCampaignStep, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(&TacticAssetDescription) -> bool,
        R: Fn(&FactSnapshot, &FactSnapshot, &OptionExecution) -> f32,
    {
        if !refit_model && self.model.is_none() {
            return Err(TacticQCampaignError::InvalidState(
                "a tactic campaign needs an initial fitted model before batching refits",
            ));
        }
        if decision.selected != outcome.selected
            || decision.selected.decision_index != self.decision_index
            || decision.selected.learner_snapshot_sha256 != self.current.snapshot_sha256
            || outcome.source_checkpoint_sha256 != self.root_checkpoint_sha256
            || !extends(&self.route_tape, &outcome.route_tape)
        {
            return Err(TacticQCampaignError::InvalidState(
                "native outcome is detached from the selected campaign boundary",
            ));
        }
        let next = LearnerState::build(
            outcome.next_facts.clone(),
            registry,
            catalog,
            blueprints,
            entry_applicable,
        )?;
        let reward_value = reward(&self.current.snapshot, &next.snapshot, &outcome.execution);
        if !reward_value.is_finite() {
            return Err(TacticQCampaignError::InvalidState(
                "campaign reward is non-finite",
            ));
        }
        let source_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &self.route_tape)?;
        let next_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &outcome.route_tape)?;
        let transition = OptionTransitionSample::capture(
            self.feature_schema_sha256,
            source_checkpoint_sha256,
            next_checkpoint_sha256,
            self.current.snapshot.clone(),
            next.snapshot.clone(),
            outcome.execution,
            &outcome.route_tape,
            reward_value,
            outcome.terminal,
            encode,
        )?;

        let mut replay = self.replay.clone();
        replay.push(transition.clone());
        let mut replay_routes = self.replay_routes.clone();
        replay_routes.push(outcome.route_tape.clone());
        let mut episode_groups = self.episode_groups.clone();
        episode_groups.push(self.episode_group);
        let mut training_replay = self.training_replay.clone();
        let mut training_replay_routes = self.training_replay_routes.clone();
        let mut training_episode_groups = self.training_episode_groups.clone();
        let mut training_identities = self.training_identities.clone();
        if training_identities.insert(transition.replay_identity_sha256()?) {
            training_replay.push(transition.clone());
            training_replay_routes.push(outcome.route_tape.clone());
            training_episode_groups.push(self.episode_group);
        }
        let model_update = if refit_model {
            Some(replay_model(
                self.feature_schema_sha256,
                self.objective_sha256,
                &training_replay,
                &training_episode_groups,
                &self.model_config,
            )?)
        } else {
            None
        };

        self.visited_states.insert(tactic_state_descriptor(
            &next.snapshot,
            transition.value_sample.terminal,
        ));
        self.current = next;
        self.route_tape = outcome.route_tape;
        self.replay = replay;
        self.replay_routes = replay_routes;
        self.episode_groups = episode_groups;
        self.training_replay = training_replay;
        self.training_replay_routes = training_replay_routes;
        self.training_episode_groups = training_episode_groups;
        self.training_identities = training_identities;
        if let Some(model) = model_update {
            // Exact-descriptor FQI is a small-data control, not the scalable
            // action representation. Clear it once a dynamic controller
            // universe exceeds its categorical capacity; the shared
            // state-action outcome model continues to consume every row.
            self.model = model;
        }
        self.decision_index =
            self.decision_index
                .checked_add(1)
                .ok_or(TacticQCampaignError::InvalidState(
                    "campaign decision index overflowed",
                ))?;
        Ok(TacticQCampaignStep {
            decision,
            reward: reward_value,
            replay_rows: self.replay.len(),
            transition,
        })
    }
}

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
        _ => option_f64(right.best_mean_q)
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
    let current = checkpoint.schema == TACTIC_Q_CHECKPOINT_SCHEMA_V3;
    if (!legacy && !current)
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
        || (current
            && (checkpoint.training_replay.len() != checkpoint.training_replay_routes.len()
                || checkpoint.training_replay.len() != checkpoint.training_episode_groups.len()
                || checkpoint.training_replay.len() < checkpoint.replay.len()))
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
        if transition.feature_schema_sha256 != checkpoint.feature_schema_sha256
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
    replay_model(
        checkpoint.feature_schema_sha256,
        checkpoint.objective_sha256,
        training_replay,
        training_groups,
        &checkpoint.model_config,
    )?;
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
    if transition.feature_schema_sha256 != feature_schema_sha256
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
    let bytes = serde_cbor::to_vec(&(
        &result.schema,
        result.objective_sha256,
        result.root_checkpoint_sha256,
        result.route_tape_sha256,
        result.replay_sha256,
        result.terminal_state_sha256,
    ))
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
mod tests {
    use super::*;
    use dusklight_automation_contracts::tape::{InputFrame, RawPadState};
    use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
    use dusklight_control::option_execution::{OptionCondition, OptionEndReason, TapeRange};
    use dusklight_evidence::native_episode_shard::{NativeEpisodeShard, NativeObservationPhase};
    use dusklight_learning::parameterized_tactic_proposals::{
        ParameterizedTacticProposalContext, parameterized_tactic_family_schema_sha256,
        propose_parameterized_tactics,
    };
    use dusklight_learning::reward_shaping::{
        POTENTIAL_SHAPING_SCHEMA_V1, PotentialShapingSpec, PotentialTerm,
        TACTIC_REWARD_SPEC_SCHEMA_V1,
    };
    use dusklight_learning::tactic_asset::{TacticAssetSource, TacticCatalogEntry};
    use dusklight_learning::tactic_exploration::TacticSelectionReason;
    use std::fs;

    #[test]
    fn seeded_frontier_rotation_visits_every_eligible_cell_before_repeating() {
        let choice_count = 35;
        let visited = (0..choice_count as u64)
            .map(|round| seeded_frontier_index(104_729, round, choice_count))
            .collect::<BTreeSet<_>>();
        assert_eq!(visited, (0..choice_count).collect());
        assert_eq!(
            seeded_frontier_index(104_729, choice_count as u64, choice_count),
            seeded_frontier_index(104_729, 0, choice_count)
        );
    }

    #[test]
    fn frontier_learning_value_precedes_the_last_edges_immediate_cost() {
        let valuable = TacticFrontierAcquisition {
            expansion_count: 0,
            terminal: false,
            reward: -0.4,
            best_mean_q: Some(10.0),
            predicted_terminal_ticks_to_go: None,
            predicted_total_terminal_ticks: None,
            maximum_ensemble_variance: None,
            generalized_nearest_distance: Some(0.1),
            novelty_rank: 1,
            replayed_prefix_ticks: 40,
        };
        let cheap_dead_end = TacticFrontierAcquisition {
            reward: -0.04,
            best_mean_q: Some(1.0),
            replayed_prefix_ticks: 4,
            ..valuable.clone()
        };

        assert_eq!(
            compare_frontier_acquisition(&valuable, &cheap_dead_end),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn frontier_learning_value_precedes_coverage_count() {
        let valuable = TacticFrontierAcquisition {
            expansion_count: 3,
            terminal: false,
            reward: -0.4,
            best_mean_q: Some(10.0),
            predicted_terminal_ticks_to_go: None,
            predicted_total_terminal_ticks: None,
            maximum_ensemble_variance: None,
            generalized_nearest_distance: Some(0.1),
            novelty_rank: 1,
            replayed_prefix_ticks: 40,
        };
        let fresh_dead_end = TacticFrontierAcquisition {
            expansion_count: 0,
            best_mean_q: Some(1.0),
            ..valuable.clone()
        };

        assert_eq!(
            compare_frontier_acquisition(&valuable, &fresh_dead_end),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn frontier_terminal_cost_includes_the_replayed_prefix() {
        let earlier = TacticFrontierAcquisition {
            expansion_count: 0,
            terminal: false,
            reward: -0.4,
            best_mean_q: Some(99.0),
            predicted_terminal_ticks_to_go: Some(84.0),
            predicted_total_terminal_ticks: Some(124.0),
            maximum_ensemble_variance: None,
            generalized_nearest_distance: Some(0.1),
            novelty_rank: 1,
            replayed_prefix_ticks: 40,
        };
        let late = TacticFrontierAcquisition {
            best_mean_q: Some(99.98),
            predicted_terminal_ticks_to_go: Some(2.0),
            predicted_total_terminal_ticks: Some(126.0),
            replayed_prefix_ticks: 124,
            ..earlier.clone()
        };

        assert_eq!(
            compare_frontier_acquisition(&earlier, &late),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn equal_terminal_cost_prefers_the_less_expanded_frontier() {
        let fresh = TacticFrontierAcquisition {
            expansion_count: 0,
            terminal: false,
            reward: -0.4,
            best_mean_q: Some(99.0),
            predicted_terminal_ticks_to_go: Some(86.0),
            predicted_total_terminal_ticks: Some(126.0),
            maximum_ensemble_variance: None,
            generalized_nearest_distance: Some(0.1),
            novelty_rank: 1,
            replayed_prefix_ticks: 40,
        };
        let expanded = TacticFrontierAcquisition {
            expansion_count: 1,
            best_mean_q: Some(99.98),
            predicted_terminal_ticks_to_go: Some(2.0),
            replayed_prefix_ticks: 124,
            ..fresh.clone()
        };

        assert_eq!(
            compare_frontier_acquisition(&fresh, &expanded),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn novelty_identity_ignores_bookkeeping_and_micro_motion_but_not_new_cells() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let original = FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            vec![],
        )
        .unwrap();
        let mut later_observation = original.clone();
        later_observation.boundary_index += 9;
        later_observation.simulation_tick += 9;
        later_observation.tape_frame += 9;
        later_observation.state_identity = [0x5a; 16];

        assert_ne!(
            original.content_sha256().unwrap(),
            later_observation.content_sha256().unwrap()
        );
        assert_eq!(
            semantic_state_digest(&original).unwrap(),
            semantic_state_digest(&later_observation).unwrap()
        );

        let mut moved = later_observation;
        let mut position = moved.player.position_f32_bits;
        let original_x = f32::from_bits(position[0]);
        position[0] = ((original_x / 256.0).floor() * 256.0 + 128.0).to_bits();
        moved.player.position_f32_bits = position;
        assert_ne!(
            semantic_state_digest(&original).unwrap(),
            semantic_state_digest(&moved).unwrap()
        );
        assert_eq!(
            tactic_state_descriptor(&original, false),
            tactic_state_descriptor(&moved, false)
        );

        let mut new_cell = moved;
        let mut position = new_cell.player.position_f32_bits;
        position[0] = (f32::from_bits(position[0]) + 512.0).to_bits();
        new_cell.player.position_f32_bits = position;
        assert_ne!(
            tactic_state_descriptor(&original, false),
            tactic_state_descriptor(&new_cell, false)
        );
    }

    #[test]
    fn parameterized_batch_uses_family_instances_absent_from_the_state_catalog() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let before = FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap();
        let bootstrap = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new(
                "shield",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 1,
                })),
            )
            .unwrap(),
        ])
        .unwrap();
        let current = LearnerState::build(
            before.clone(),
            &FactRegistry::canonical(),
            &bootstrap,
            &[],
            |_| true,
        )
        .unwrap();
        let campaign = TacticQCampaign::new(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            0,
            current,
            InputTape {
                frames: vec![InputFrame::default(); before.tape_frame as usize],
                ..InputTape::default()
            },
            OptionValueConfig::default(),
            TacticExplorationConfig {
                seed: 17,
                epsilon_per_million: 0,
            },
        )
        .unwrap();
        let proposals = propose_parameterized_tactics(ParameterizedTacticProposalContext {
            seed: 17,
            decision_index: 0,
            state_sha256: campaign.current.snapshot_sha256,
            player_position: before.player.position_f32_bits.map(f32::from_bits),
            camera_yaw_radians: before
                .player
                .camera_yaw_radians_f32_bits
                .map(f32::from_bits),
            goal_coordinate: [100.0, 20.0, -50.0],
            maximum_ticks: 40,
            feedback: None,
        })
        .unwrap();
        let batch = campaign
            .decide_parameterized_batch(
                &proposals.catalog,
                &proposals.blueprints,
                parameterized_tactic_family_schema_sha256(),
                &|_: &FactSnapshot| Ok::<_, &'static str>(vec![0.0]),
                32,
            )
            .unwrap();

        assert_eq!(
            batch.ranking.action_universe_sha256,
            parameterized_tactic_family_schema_sha256()
        );
        assert!(batch.proposals.len() > 4);
        assert!(
            batch
                .proposals
                .iter()
                .all(|proposal| { proposal.descriptor.option_id.starts_with("family/") })
        );
        assert!(batch.proposals.iter().all(|proposal| {
            proposals
                .catalog
                .prepare_execution(&proposal.descriptor.option_id)
                .is_ok()
        }));
        assert!(
            batch
                .proposals
                .iter()
                .all(|proposal| proposal.descriptor.option_id != "shield")
        );
        assert!(batch.proposals.iter().any(|proposal| {
            proposal.descriptor.option_type == dusklight_control::option_execution::OptionType::Roll
        }));

        let mut choices = batch.ranking.choices.clone();
        let excluded = choices[0].descriptor.clone();
        choices[0].applicable = false;
        let tried = choices[1].descriptor.option_id.as_str();
        let untried = applicable_untried_descriptors(&choices, &BTreeSet::from([tried]));
        assert!(!untried.contains(&excluded));
        assert!(
            !untried
                .iter()
                .any(|descriptor| descriptor.option_id == tried)
        );
        assert!(untried.iter().all(|descriptor| {
            choices
                .iter()
                .any(|choice| choice.applicable && choice.descriptor == *descriptor)
        }));
    }

    #[test]
    fn cold_start_retains_refits_and_ranks_the_next_boundary() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let native_step = &shard.episodes[0].steps[0];
        let before =
            FactSnapshot::from_native_learning(&native_step.pre_input, &[], None, Vec::new())
                .unwrap();
        let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new(
                "shield",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 1,
                })),
            )
            .unwrap(),
        ])
        .unwrap();
        let registry = FactRegistry::canonical();
        let current =
            LearnerState::build(before.clone(), &registry, &catalog, &[], |_| true).unwrap();
        let route_prefix = InputTape {
            frames: vec![InputFrame::default(); before.tape_frame as usize],
            ..InputTape::default()
        };
        let root_checkpoint_sha256 = Digest([7; 32]);
        let mut campaign = TacticQCampaign::new(
            Digest([1; 32]),
            Digest([2; 32]),
            root_checkpoint_sha256,
            11,
            current,
            route_prefix.clone(),
            OptionValueConfig::default(),
            TacticExplorationConfig {
                seed: 41,
                epsilon_per_million: 0,
            },
        )
        .unwrap();
        let encode = |facts: &FactSnapshot| Ok::<_, &'static str>(vec![facts.tape_frame as f32]);

        let decision = campaign.decide(&catalog, &[], &encode).unwrap();
        assert_eq!(
            decision.selected.reason,
            TacticSelectionReason::UnsupportedBootstrap
        );
        assert!(decision.ranking.values.ranked.is_empty());
        assert_eq!(decision.ranking.values.unsupported.len(), 1);

        let mut frame = InputFrame {
            owned_ports: 1,
            ..InputFrame::default()
        };
        frame.pads[0] = RawPadState {
            buttons: native_step.chosen_pad.buttons,
            stick_x: native_step.chosen_pad.stick_x,
            stick_y: native_step.chosen_pad.stick_y,
            substick_x: native_step.chosen_pad.substick_x,
            substick_y: native_step.chosen_pad.substick_y,
            trigger_left: native_step.chosen_pad.trigger_left,
            trigger_right: native_step.chosen_pad.trigger_right,
            analog_a: native_step.chosen_pad.analog_a,
            analog_b: native_step.chosen_pad.analog_b,
            connected: native_step.chosen_pad.connected,
            error: native_step.chosen_pad.error,
        };
        let mut route_tape = route_prefix;
        route_tape.frames.push(frame);
        let execution = OptionExecution::capture(
            decision.selected.descriptor.option_id.clone(),
            decision.selected.descriptor.option_type.clone(),
            decision.selected.descriptor.parameters.clone(),
            1,
            1,
            OptionCondition::DurationElapsed,
            Vec::new(),
            OptionEndReason::Completed,
            &route_tape,
            TapeRange {
                start_frame: before.tape_frame,
                end_frame_exclusive: before.tape_frame + 1,
            },
        )
        .unwrap();
        let mut next_boundary = native_step.post_simulation.clone();
        next_boundary.phase = NativeObservationPhase::PreInput;
        next_boundary.simulation_tick += 1;
        next_boundary.tape_frame += 1;
        let after = FactSnapshot::from_native_learning(
            &next_boundary,
            std::slice::from_ref(&native_step.pre_input),
            Some(&execution),
            Vec::new(),
        )
        .unwrap();
        let terminal = after.terminal.reached.unwrap();
        let outcome = NativeTacticWorkerOutcome {
            schema: crate::native_tactic_worker::NATIVE_TACTIC_WORKER_OUTCOME_SCHEMA_V2.into(),
            source_checkpoint_sha256: root_checkpoint_sha256,
            checkpoint_identity: "fixture-checkpoint".into(),
            episode_shard_sha256: shard.content_sha256,
            selected: decision.selected.clone(),
            execution,
            native_queries: Vec::new(),
            route_tape,
            next_facts: after,
            terminal,
            retained_native_checkpoint: None,
            retained_native_boundary_fingerprint: None,
        };
        let reward_spec = TacticRewardSpec {
            schema: TACTIC_REWARD_SPEC_SCHEMA_V1.into(),
            terminal_reward: 5.0,
            tick_cost: 0.25,
            novelty_reward: 1.0,
            per_tick_discount: 0.9,
            potential: Some(PotentialShapingSpec {
                schema: POTENTIAL_SHAPING_SCHEMA_V1.into(),
                feature_schema: Digest([1; 32]),
                terms: vec![PotentialTerm::CorridorProgress {
                    name: "tape-progress".into(),
                    feature: 0,
                    start: before.tape_frame as f32,
                    end: before.tape_frame as f32 + 1.0,
                    weight: 2.0,
                    unavailable_value: None,
                }],
            }),
            motion_cost: None,
        };
        let evaluated = campaign
            .evaluate_rewarded_outcome(outcome.clone(), &encode, &reward_spec)
            .unwrap();
        assert_eq!(campaign.decision_index, 0);
        assert!(campaign.replay.is_empty());
        assert_eq!(
            campaign
                .admit_evaluated_replay(&[evaluated.clone(), evaluated.clone()])
                .unwrap(),
            1
        );
        assert_eq!(campaign.training_replay_len(), 1);
        let retained = campaign
            .retain_and_refit_rewarded(
                decision,
                outcome,
                &catalog,
                &[],
                &registry,
                &encode,
                |_| true,
                &reward_spec,
                true,
            )
            .unwrap();

        assert_eq!(evaluated.transition, retained.step.transition);
        assert_eq!(evaluated.reward, retained.reward);
        assert_eq!(retained.step.replay_rows, 1);
        assert_eq!(retained.reward.terminal_observed, terminal);
        assert!(!retained.reward.endpoint_novel);
        assert_eq!(retained.reward.tick_cost_component, -0.25);
        assert_eq!(retained.reward.novelty_component, 0.0);
        assert!(retained.reward.potential.is_some());
        assert!(retained.reward.terminal_objective_unchanged);
        assert!(!retained.reward.promotion_authority);
        assert_eq!(campaign.replay.len(), 1);
        assert_eq!(campaign.training_replay_len(), 1);
        assert_eq!(campaign.episode_groups, vec![11]);
        assert!(campaign.model().is_some());
        assert_eq!(campaign.current.snapshot.tape_frame, before.tape_frame + 1);
        assert_eq!(
            campaign.route_tape.frames.len() as u64,
            campaign.current.snapshot.tape_frame
        );
        assert_eq!(campaign.visited_state_count(), 1);

        let checkpoint = campaign.checkpoint().unwrap();
        assert_eq!(checkpoint.schema, TACTIC_Q_CHECKPOINT_SCHEMA_V3);
        assert_eq!(checkpoint.training_replay.len(), 1);
        let restored = TacticQCampaign::resume(checkpoint.clone()).unwrap();
        assert_eq!(restored.decision_index, campaign.decision_index);
        assert_eq!(restored.training_replay_len(), 1);
        assert_eq!(restored.route_tape, campaign.route_tape);
        assert_eq!(restored.replay, campaign.replay);
        assert_eq!(restored.replay_routes, campaign.replay_routes);
        assert!(restored.model().is_some());
        let corpus = campaign.training_corpus();
        let corpus_root = std::env::temp_dir().join(format!(
            "dusklight-tactic-training-corpus-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&corpus_root);
        let corpus_path = corpus_root.join("seed-000-41/generated-training.dtqc");
        corpus
            .write(&corpus_path, &corpus_root.join("objects"))
            .unwrap();
        assert_eq!(TacticQTrainingCorpus::read(&corpus_path).unwrap(), corpus);
        let mut tampered = fs::read(&corpus_path).unwrap();
        *tampered.last_mut().unwrap() ^= 0x01;
        let tampered_path = corpus_root.join("tampered.dtqc");
        fs::write(&tampered_path, tampered).unwrap();
        assert!(TacticQTrainingCorpus::read(&tampered_path).is_err());
        fs::remove_dir_all(corpus_root).unwrap();
        let fresh_current =
            LearnerState::build(before.clone(), &registry, &catalog, &[], |_| true).unwrap();
        let mut fresh_episode = TacticQCampaign::new(
            Digest([1; 32]),
            Digest([2; 32]),
            root_checkpoint_sha256,
            99,
            fresh_current,
            tape_prefix(&campaign.replay_routes[0], before.tape_frame as usize),
            OptionValueConfig::default(),
            TacticExplorationConfig {
                seed: 43,
                epsilon_per_million: 0,
            },
        )
        .unwrap();
        assert!(fresh_episode.model().is_none());
        assert_eq!(
            fresh_episode
                .import_training_corpora(std::slice::from_ref(&corpus))
                .unwrap(),
            1
        );
        assert!(fresh_episode.model().is_some());
        assert!(fresh_episode.replay.is_empty());
        assert_eq!(fresh_episode.training_replay_len(), 1);
        assert_eq!(
            fresh_episode
                .import_training_corpora(std::slice::from_ref(&corpus))
                .unwrap(),
            0
        );
        let mut detached = corpus.clone();
        detached.root_checkpoint_sha256 = Digest([9; 32]);
        assert!(
            fresh_episode
                .import_training_corpora(std::slice::from_ref(&detached))
                .is_err()
        );
        assert_eq!(fresh_episode.training_replay_len(), 1);
        let policy = restored.freeze_greedy_policy().unwrap();
        assert_eq!(
            policy.action_universe_sha256,
            catalog.action_schema_sha256()
        );
        let archive = restored.frontier_archive().unwrap();
        assert_eq!(archive.tactic_len(), 1);
        let graph = restored.graph().unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert!(graph.root_connected);
        assert_eq!(
            graph.root_checkpoint_sha256,
            campaign.replay[0].source_checkpoint_sha256
        );
        let root_node = graph
            .nodes
            .iter()
            .find(|node| node.checkpoint_sha256 == graph.root_checkpoint_sha256)
            .unwrap();
        assert_eq!(root_node.route_tape.frames.len() as u64, before.tape_frame);
        assert!(
            graph.nodes.iter().any(|node| {
                node.checkpoint_sha256 == campaign.replay[0].next_checkpoint_sha256
            })
        );
        let projection = restored.graph_projection().unwrap();
        assert_eq!(projection.nodes.len(), 2);
        assert_eq!(projection.edges.len(), 1);
        assert_eq!(projection.edges[0].edge_index, 0);
        assert_eq!(projection.frontier_cells, 1);
        assert!(projection.root_connected);
        assert!(
            projection
                .nodes
                .iter()
                .any(|node| node.current && node.retained_frontier)
        );
        let projection_json = serde_json::to_vec(&projection).unwrap();
        assert!(
            !projection_json
                .windows(10)
                .any(|bytes| bytes == b"route_tape")
        );
        assert!(projection_json.len() < 4_096);
        let mut equivalent_pad_projection = campaign.replay[0].clone();
        equivalent_pad_projection
            .after
            .recent_option
            .as_mut()
            .unwrap()
            .option_id = "equivalent-pad-tactic".into();
        equivalent_pad_projection.after_state_sha256 =
            equivalent_pad_projection.after.content_sha256().unwrap();
        equivalent_pad_projection.value_sample.after_state_sha256 =
            equivalent_pad_projection.after_state_sha256;
        let mut equivalent_graph = TacticQCampaign::resume(campaign.checkpoint().unwrap()).unwrap();
        equivalent_graph.replay.push(equivalent_pad_projection);
        equivalent_graph
            .replay_routes
            .push(campaign.replay_routes[0].clone());
        equivalent_graph.episode_groups.push(77);
        let graph = equivalent_graph.graph().unwrap();
        assert_eq!(graph.nodes.len(), 3);
        assert!(graph.root_connected);
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| {
                    node.checkpoint_sha256 == campaign.replay[0].next_checkpoint_sha256
                })
                .count(),
            2
        );
        let diagnostics = restored.diagnostics().unwrap();
        assert_eq!(diagnostics.unique_selected_actions, 1);
        assert!(!diagnostics.zero_diversity_selection);
        assert!(!diagnostics.repeated_identical_compositions);
        assert!(!diagnostics.no_progress_loop);
        assert!(!diagnostics.frontier_lost_root_connectivity);
        let mut stagnant = campaign.replay[0].clone();
        stagnant.after = stagnant.before.clone();
        stagnant.after.boundary_index += 1;
        stagnant.after.simulation_tick += 1;
        stagnant.after.tape_frame += 1;
        stagnant.value_sample.terminal = false;
        assert!(has_no_progress_loop(&[stagnant], &[99]).unwrap());
        let mut collapsed = TacticQCampaign::resume(campaign.checkpoint().unwrap()).unwrap();
        collapsed.replay.push(campaign.replay[0].clone());
        collapsed
            .replay_routes
            .push(campaign.replay_routes[0].clone());
        collapsed.episode_groups.push(77);
        let collapsed_diagnostics = collapsed.diagnostics().unwrap();
        assert!(collapsed_diagnostics.zero_diversity_selection);
        assert!(collapsed_diagnostics.repeated_identical_compositions);
        let diagnostics = restored.diagnostics().unwrap();
        assert_eq!(diagnostics.logical_frontier_records, 2);
        assert_eq!(diagnostics.directly_restorable_native_frontiers, 0);
        assert_eq!(diagnostics.replay_only_frontiers, 1);
        let [root_branch, frontier_branch] = restored
            .sample_root_and_frontier(5, 0, &[], usize::MAX)
            .unwrap();
        assert_eq!(root_branch.kind, TacticBranchKind::Root);
        assert_eq!(frontier_branch.kind, TacticBranchKind::RetainedFrontier);
        assert_eq!(
            frontier_branch.logical_frontier.state_sha256,
            campaign.current.snapshot_sha256
        );
        assert!(root_branch.restorable_native_checkpoint.is_none());
        assert!(frontier_branch.restorable_native_checkpoint.is_none());
        assert_eq!(root_branch.logical_frontier.replayed_prefix_ticks, 0);
        assert!(frontier_branch.logical_frontier.replayed_prefix_ticks > 0);
        let [ranked_root, ranked_frontier] = restored
            .sample_root_and_ranked_frontier(
                5,
                0,
                &[],
                usize::MAX,
                false,
                0,
                &encode,
                &|_: &FactSnapshot| {
                    Ok::<_, &'static str>(catalog.option_descriptors().cloned().collect())
                },
            )
            .unwrap();
        assert!(ranked_root.acquisition.is_none());
        let acquisition = ranked_frontier.acquisition.as_ref().unwrap();
        assert_eq!(acquisition.expansion_count, 0);
        assert_eq!(
            acquisition.replayed_prefix_ticks,
            ranked_frontier.logical_frontier.replayed_prefix_ticks
        );
        assert!(acquisition.best_mean_q.is_some());
        assert!(
            acquisition.maximum_ensemble_variance.is_some()
                || acquisition.generalized_nearest_distance.is_some()
        );
        let mut model_only = TacticQCampaign::resume(restored.checkpoint().unwrap()).unwrap();
        model_only
            .training_episode_groups
            .fill(TACTIC_Q_MODEL_ONLY_EPISODE_GROUP);
        assert_eq!(model_only.frontier_archive().unwrap().tactic_len(), 0);
        assert_eq!(model_only.frontier_cell_count(), 0);
        assert_eq!(model_only.demonstration_frontier_count(), 0);
        let mut demonstration = TacticQCampaign::resume(restored.checkpoint().unwrap()).unwrap();
        demonstration
            .training_episode_groups
            .fill(TACTIC_Q_DEMONSTRATION_EPISODE_GROUP);
        assert_eq!(demonstration.frontier_archive().unwrap().tactic_len(), 1);
        assert_eq!(demonstration.frontier_cell_count(), 1);
        assert_eq!(demonstration.demonstration_frontier_count(), 1);
        let mut terminal_leaf = TacticQCampaign::resume(restored.checkpoint().unwrap()).unwrap();
        terminal_leaf
            .training_episode_groups
            .fill(TACTIC_Q_DEMONSTRATION_EPISODE_GROUP);
        terminal_leaf.training_replay[0].value_sample.terminal = true;
        assert_eq!(terminal_leaf.frontier_archive().unwrap().tactic_len(), 0);
        assert_eq!(terminal_leaf.frontier_cell_count(), 0);
        assert_eq!(terminal_leaf.demonstration_frontier_count(), 0);
        let mut forged_native_frontier = frontier_branch.clone();
        forged_native_frontier.restorable_native_checkpoint =
            Some(RestorableNativeTacticCheckpoint {
                worker_slot: 0,
                native_source_sha256: campaign.root_checkpoint_sha256,
                logical_frontier_sha256: frontier_branch.logical_frontier.identity_sha256,
                state_sha256: frontier_branch.logical_frontier.state_sha256,
                restore_identity: "unadmitted-process-local-handle".into(),
                checkpoint_bytes: 4096,
            });
        let mut rejects_forged_native =
            TacticQCampaign::resume(campaign.checkpoint().unwrap()).unwrap();
        assert!(
            rejects_forged_native
                .restore_branch(
                    &forged_native_frontier,
                    23,
                    &registry,
                    &catalog,
                    &[],
                    |_| true,
                )
                .is_err()
        );
        assert!(
            restored
                .sample_root_and_frontier(5, 0, &[], frontier_branch.route_tape.frames.len() - 1,)
                .is_err()
        );
        let mut branched = TacticQCampaign::resume(checkpoint.clone()).unwrap();
        branched
            .restore_branch(&root_branch, 22, &registry, &catalog, &[], |_| true)
            .unwrap();
        assert_eq!(branched.episode_group, 22);
        assert_eq!(
            branched.current.snapshot_sha256,
            root_branch.logical_frontier.state_sha256
        );
        assert!(branched.model().is_some());
        branched.checkpoint().unwrap();
        let mut tampered = checkpoint;
        tampered.decision_index += 1;
        assert!(TacticQCampaign::resume(tampered).is_err());

        let directory = std::env::temp_dir().join(format!(
            "dusklight-tactic-q-checkpoint-{}-{}",
            std::process::id(),
            campaign.current.snapshot_sha256
        ));
        let path = campaign.write_checkpoint(&directory).unwrap();
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some(TACTIC_Q_CHECKPOINT_EXTENSION)
        );
        let stored = fs::read(&path).unwrap();
        assert_eq!(&stored[..8], b"DSKTQZ01");
        assert_ne!(stored.first(), Some(&b'{'));
        assert!(
            stored.len()
                < serde_json::to_vec(&campaign.checkpoint().unwrap())
                    .unwrap()
                    .len()
        );
        let payload = TacticQCampaign::read_checkpoint_payload(&path).unwrap();
        assert_eq!(
            payload.content_sha256,
            campaign.checkpoint().unwrap().content_sha256
        );
        let from_file = TacticQCampaign::read_checkpoint(&path).unwrap();
        assert_eq!(from_file.replay, campaign.replay);
        let mut legacy_checkpoint = campaign.checkpoint().unwrap();
        legacy_checkpoint.schema = "dusklight-tactic-q-checkpoint/v1".into();
        let legacy_path = directory.join("legacy-checkpoint.json");
        fs::write(
            &legacy_path,
            serde_json::to_vec(&legacy_checkpoint).unwrap(),
        )
        .unwrap();
        let codec_benchmark =
            TacticQCampaign::benchmark_checkpoint_serialization(&legacy_path, &path, 2).unwrap();
        assert_eq!(codec_benchmark.iterations, 2);
        assert_eq!(codec_benchmark.decision_index, campaign.decision_index);
        assert_eq!(
            codec_benchmark.replay_transitions,
            campaign.replay.len() as u64
        );
        assert!(
            codec_benchmark.legacy_json_bytes_per_iteration
                > codec_benchmark.current_manifest_envelope_bytes_per_iteration
        );
        fs::remove_file(legacy_path).unwrap();
        let objects = directory.join("objects");
        let hidden_objects = directory.join("objects-unavailable");
        fs::rename(&objects, &hidden_objects).unwrap();
        assert!(TacticQCampaign::read_checkpoint(&path).is_err());
        fs::rename(&hidden_objects, &objects).unwrap();
        let mut tampered_envelope = stored;
        let last = tampered_envelope.len() - 1;
        tampered_envelope[last] ^= 1;
        let tampered_path = path.with_file_name("tampered.dtqz");
        fs::write(&tampered_path, tampered_envelope).unwrap();
        assert!(TacticQCampaign::read_checkpoint_payload(&tampered_path).is_err());
        fs::remove_file(tampered_path).unwrap();
        fs::remove_file(&path).unwrap();
        fs::remove_dir_all(&directory).unwrap();

        if terminal {
            let final_result = campaign.final_result().unwrap();
            validate_final_result(&final_result).unwrap();
            let final_directory = std::env::temp_dir().join(format!(
                "dusklight-tactic-final-result-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&final_directory);
            let final_path = final_directory.join("result.dtqz");
            final_result.write(&final_path).unwrap();
            assert_eq!(TacticQFinalResult::read(&final_path).unwrap(), final_result);
            fs::remove_dir_all(final_directory).unwrap();
            let mut tampered = final_result;
            tampered.route_tape.frames[0].owned_ports ^= 1;
            assert!(validate_final_result(&tampered).is_err());
        } else {
            assert!(campaign.final_result().is_err());
        }

        let next = restored.decide(&catalog, &[], &encode).unwrap();
        assert_eq!(next.selected.reason, TacticSelectionReason::Greedy);
        assert_eq!(next.ranking.values.ranked.len(), 1);
        assert!(next.ranking.values.unsupported.is_empty());

        // Continue the original in-memory campaign and the campaign loaded
        // from the sealed checkpoint through the same terminal outcome. This
        // makes interruption equivalence cover selection, refit, frontier,
        // tape, and final proof identities rather than only decoding.
        let mut uninterrupted = campaign;
        let mut resumed = from_file;
        let uninterrupted_decision = uninterrupted.decide(&catalog, &[], &encode).unwrap();
        let resumed_decision = resumed.decide(&catalog, &[], &encode).unwrap();
        assert_eq!(uninterrupted_decision, resumed_decision);
        assert_eq!(uninterrupted_decision, next);

        let mut terminal_route = uninterrupted.route_tape.clone();
        terminal_route.frames.push(InputFrame {
            owned_ports: 1,
            ..InputFrame::default()
        });
        let start_frame = uninterrupted.current.snapshot.tape_frame;
        let terminal_execution = OptionExecution::capture(
            uninterrupted_decision.selected.descriptor.option_id.clone(),
            uninterrupted_decision
                .selected
                .descriptor
                .option_type
                .clone(),
            uninterrupted_decision
                .selected
                .descriptor
                .parameters
                .clone(),
            1,
            1,
            OptionCondition::DurationElapsed,
            Vec::new(),
            OptionEndReason::Completed,
            &terminal_route,
            TapeRange {
                start_frame,
                end_frame_exclusive: start_frame + 1,
            },
        )
        .unwrap();
        let mut terminal_facts = uninterrupted.current.snapshot.clone();
        terminal_facts.boundary_index += 1;
        terminal_facts.simulation_tick += 1;
        terminal_facts.tape_frame += 1;
        terminal_facts.state_identity = [0x5a; 16];
        terminal_facts.player.position_f32_bits[0] =
            (f32::from_bits(terminal_facts.player.position_f32_bits[0]) + 512.0).to_bits();
        terminal_facts.terminal.configured = Some(true);
        terminal_facts.terminal.reached = Some(true);
        terminal_facts.terminal.reason =
            dusklight_learning::fact_snapshot::FactTerminalReason::GoalReached;
        terminal_facts.terminal.first_hit_tick = Some(terminal_facts.simulation_tick);
        let terminal_outcome = NativeTacticWorkerOutcome {
            schema: crate::native_tactic_worker::NATIVE_TACTIC_WORKER_OUTCOME_SCHEMA_V2.into(),
            source_checkpoint_sha256: uninterrupted.root_checkpoint_sha256,
            checkpoint_identity: "resume-equivalence-terminal".into(),
            episode_shard_sha256: shard.content_sha256,
            selected: uninterrupted_decision.selected.clone(),
            execution: terminal_execution,
            native_queries: Vec::new(),
            route_tape: terminal_route,
            next_facts: terminal_facts,
            terminal: true,
            retained_native_checkpoint: None,
            retained_native_boundary_fingerprint: None,
        };
        let evaluated_terminal = uninterrupted
            .evaluate_rewarded_outcome(terminal_outcome.clone(), &encode, &reward_spec)
            .unwrap();
        let evaluated_terminal_result = uninterrupted
            .final_result_from_evaluated_terminal(&evaluated_terminal)
            .unwrap();
        validate_final_result(&evaluated_terminal_result).unwrap();
        assert!(uninterrupted.final_result().is_err());
        let uninterrupted_step = uninterrupted
            .retain_and_refit_rewarded(
                uninterrupted_decision,
                terminal_outcome.clone(),
                &catalog,
                &[],
                &registry,
                &encode,
                |_| true,
                &reward_spec,
                true,
            )
            .unwrap();
        let resumed_step = resumed
            .retain_and_refit_rewarded(
                resumed_decision,
                terminal_outcome,
                &catalog,
                &[],
                &registry,
                &encode,
                |_| true,
                &reward_spec,
                true,
            )
            .unwrap();
        assert_eq!(uninterrupted_step, resumed_step);
        assert_eq!(
            serde_cbor::to_vec(&uninterrupted.model()).unwrap(),
            serde_cbor::to_vec(&resumed.model()).unwrap()
        );
        assert_eq!(
            uninterrupted.graph_projection().unwrap(),
            resumed.graph_projection().unwrap()
        );
        assert_eq!(
            uninterrupted
                .sample_root_and_frontier(8, 0, &[], usize::MAX)
                .unwrap(),
            resumed
                .sample_root_and_frontier(8, 0, &[], usize::MAX)
                .unwrap()
        );
        assert_eq!(uninterrupted.route_tape, resumed.route_tape);
        assert_eq!(
            uninterrupted.checkpoint().unwrap(),
            resumed.checkpoint().unwrap()
        );
        assert_eq!(
            uninterrupted.final_result().unwrap(),
            resumed.final_result().unwrap()
        );
        assert_eq!(
            uninterrupted.final_result().unwrap(),
            evaluated_terminal_result
        );
    }
}
