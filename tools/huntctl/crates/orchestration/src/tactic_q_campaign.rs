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
use dusklight_learning::fact_snapshot::FactSnapshot;
use dusklight_learning::hindsight::{
    HindsightError, HindsightOptionReplay, RelabeledHindsightOption,
};
use dusklight_learning::learner_state::{LearnerState, LearnerStateError};
use dusklight_learning::live_tactic_catalog::{
    LiveTacticCatalog, LiveTacticCatalogError, LiveTacticRanking,
};
use dusklight_learning::option_transition::{OptionTransitionError, OptionTransitionSample};
use dusklight_learning::option_values::{
    AvailableOptionRanking, OptionActionDescriptor, OptionValueBatch, OptionValueConfig,
    OptionValueError, OptionValueModel,
};
use dusklight_learning::reward_shaping::{ShapingError, TacticRewardBreakdown, TacticRewardSpec};
use dusklight_learning::tactic_asset::{TacticAssetCatalog, TacticAssetDescription};
use dusklight_learning::tactic_blueprint::TacticBlueprint;
use dusklight_learning::tactic_exploration::{
    SelectedTactic, TacticExplorationConfig, TacticExplorationError,
    choose_tactic_batch_with_state_untried,
};
use dusklight_learning::tactic_frozen_policy::{TacticFrozenPolicy, TacticFrozenPolicyError};
use dusklight_proposals::behavior_archive::{
    BehaviorArchive, TacticEndpointDescriptor, TacticStateDescriptor, tactic_state_descriptor,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

pub const TACTIC_Q_CAMPAIGN_SCHEMA_V1: &str = "dusklight-tactic-q-campaign/v1";
pub const TACTIC_Q_CHECKPOINT_SCHEMA_V2: &str = "dusklight-tactic-q-checkpoint/v2";
pub const TACTIC_Q_CHECKPOINT_EXTENSION: &str = "dtqz";
pub const TACTIC_Q_CHECKPOINT_SERIALIZATION_BENCHMARK_SCHEMA_V1: &str =
    "dusklight-tactic-q-checkpoint-serialization-benchmark/v1";
pub const TACTIC_Q_FINAL_RESULT_SCHEMA_V1: &str = "dusklight-tactic-q-final-result/v1";
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
    pub state: FactSnapshot,
    pub route_tape: InputTape,
    pub descriptor: Option<TacticEndpointDescriptor>,
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
    model_config: OptionValueConfig,
    exploration: TacticExplorationConfig,
    model: Option<OptionValueModel>,
    visited_states: BTreeSet<TacticStateDescriptor>,
    hindsight: HindsightOptionReplay,
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

    pub fn visited_state_count(&self) -> usize {
        self.visited_states.len()
    }

    pub fn hindsight_replay(&self) -> &HindsightOptionReplay {
        &self.hindsight
    }

    pub fn frontier_archive(&self) -> Result<BehaviorArchive, TacticQCampaignError> {
        let mut archive = BehaviorArchive::default();
        for (index, (transition, route)) in self.replay.iter().zip(&self.replay_routes).enumerate()
        {
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
            state: selected.transition.after.clone(),
            route_tape: selected.route_tape.clone(),
            descriptor: Some(selected.descriptor.clone()),
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
            schema: TACTIC_Q_CHECKPOINT_SCHEMA_V2.into(),
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
            self.replay
                .iter()
                .map(|transition| transition.value_sample.clone())
                .collect(),
            self.episode_groups.clone(),
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

    pub fn resume(checkpoint: TacticQCampaignCheckpoint) -> Result<Self, TacticQCampaignError> {
        validate_checkpoint(&checkpoint)?;
        let model = replay_model(
            checkpoint.feature_schema_sha256,
            checkpoint.objective_sha256,
            &checkpoint.replay,
            &checkpoint.episode_groups,
            &checkpoint.model_config,
        )?;
        let mut visited_states = BTreeSet::from([tactic_state_descriptor(
            &checkpoint.current.snapshot,
            checkpoint.current.snapshot.terminal.reached == Some(true),
        )]);
        for transition in &checkpoint.replay {
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
        let route_bytes = serde_json::to_vec(&self.route_tape)
            .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
        let replay_bytes = serde_json::to_vec(&(&self.replay, &self.replay_routes))
            .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
        let mut result = TacticQFinalResult {
            schema: TACTIC_Q_FINAL_RESULT_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            objective_sha256: self.objective_sha256,
            root_checkpoint_sha256: self.root_checkpoint_sha256,
            route_tape_sha256: sha256(&route_bytes),
            replay_sha256: sha256(&replay_bytes),
            terminal_state_sha256: self.current.snapshot_sha256,
            route_tape: self.route_tape.clone(),
            replay: self.replay.clone(),
            replay_routes: self.replay_routes.clone(),
            terminal: self.current.snapshot.clone(),
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
            .replay
            .iter()
            .filter(|transition| tactic_state_descriptor(&transition.before, false) == current_cell)
            .map(|transition| transition.value_sample.action.option_id.as_str())
            .collect::<BTreeSet<_>>();
        let state_untried = live
            .descriptors()
            .filter(|descriptor| !tried_here.contains(descriptor.option_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let proposals = choose_tactic_batch_with_state_untried(
            &ranking,
            self.decision_index,
            self.exploration,
            &state_untried,
            maximum_proposals,
        )?;
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
        let reward = reward_spec.evaluate(
            self.feature_schema_sha256,
            &state,
            &next_state,
            outcome.execution.duration.realized_ticks,
            outcome.terminal,
            !self.visited_states.contains(&endpoint),
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
        let reward = reward_spec.evaluate(
            self.feature_schema_sha256,
            &state,
            &next_state,
            outcome.execution.duration.realized_ticks,
            outcome.terminal,
            !self.visited_states.contains(&endpoint),
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
        let model = if refit_model {
            let feature_width = transition.value_sample.state.len();
            let batch = OptionValueBatch::new(
                self.feature_schema_sha256,
                self.objective_sha256,
                feature_width,
                replay
                    .iter()
                    .map(|sample| sample.value_sample.clone())
                    .collect(),
                episode_groups.clone(),
            )?;
            Some(OptionValueModel::fit_batch(&batch, &self.model_config)?)
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
        if let Some(model) = model {
            self.model = Some(model);
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
    if checkpoint.schema != TACTIC_Q_CHECKPOINT_SCHEMA_V2
        || checkpoint.content_sha256 == Digest::ZERO
        || checkpoint.content_sha256 != checkpoint_digest(checkpoint)?
        || checkpoint.feature_schema_sha256 == Digest::ZERO
        || checkpoint.objective_sha256 == Digest::ZERO
        || checkpoint.root_checkpoint_sha256 == Digest::ZERO
        || checkpoint.exploration.epsilon_per_million > 1_000_000
        || checkpoint.replay.len() != checkpoint.episode_groups.len()
        || checkpoint.replay.len() != checkpoint.replay_routes.len()
        || checkpoint.decision_index != checkpoint.replay.len() as u64
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
    replay_model(
        checkpoint.feature_schema_sha256,
        checkpoint.objective_sha256,
        &checkpoint.replay,
        &checkpoint.episode_groups,
        &checkpoint.model_config,
    )?;
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

fn validate_final_result(result: &TacticQFinalResult) -> Result<(), TacticQCampaignError> {
    result
        .route_tape
        .validate()
        .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
    result
        .terminal
        .validate()
        .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
    let route_bytes = serde_json::to_vec(&result.route_tape)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    let replay_bytes = serde_json::to_vec(&(&result.replay, &result.replay_routes))
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    if result.schema != TACTIC_Q_FINAL_RESULT_SCHEMA_V1
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
    let mut canonical = result.clone();
    canonical.content_sha256 = Digest::ZERO;
    let bytes = serde_json::to_vec(&canonical)
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

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_automation_contracts::tape::{InputFrame, RawPadState};
    use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
    use dusklight_control::option_execution::{OptionCondition, OptionEndReason, TapeRange};
    use dusklight_evidence::native_episode_shard::{NativeEpisodeShard, NativeObservationPhase};
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
        };
        let evaluated = campaign
            .evaluate_rewarded_outcome(outcome.clone(), &encode, &reward_spec)
            .unwrap();
        assert_eq!(campaign.decision_index, 0);
        assert!(campaign.replay.is_empty());
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
        assert_eq!(campaign.episode_groups, vec![11]);
        assert!(campaign.model().is_some());
        assert_eq!(campaign.current.snapshot.tape_frame, before.tape_frame + 1);
        assert_eq!(
            campaign.route_tape.frames.len() as u64,
            campaign.current.snapshot.tape_frame
        );
        assert_eq!(campaign.visited_state_count(), 1);

        let checkpoint = campaign.checkpoint().unwrap();
        let restored = TacticQCampaign::resume(checkpoint.clone()).unwrap();
        assert_eq!(restored.decision_index, campaign.decision_index);
        assert_eq!(restored.route_tape, campaign.route_tape);
        assert_eq!(restored.replay, campaign.replay);
        assert_eq!(restored.replay_routes, campaign.replay_routes);
        assert!(restored.model().is_some());
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
        };
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
    }
}
