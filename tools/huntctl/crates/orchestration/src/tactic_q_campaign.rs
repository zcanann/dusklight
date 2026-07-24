//! Online option-Q campaign over authenticated learner states and native tactic
//! boundaries.

use crate::native_tactic_worker::{
    NativeTacticWorkerError, NativeTacticWorkerOutcome, NativeTacticWorkerPaths,
    PersistentTacticBatchWorker, execute_selected_tactic,
};
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
    SelectedTactic, TacticExplorationConfig, TacticExplorationError, choose_tactic,
};
use dusklight_learning::tactic_frozen_policy::{TacticFrozenPolicy, TacticFrozenPolicyError};
use dusklight_proposals::behavior_archive::{
    BehaviorArchive, TacticEndpointDescriptor, TacticFrontierEntry, TacticStateDescriptor,
    tactic_state_descriptor,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const TACTIC_Q_CAMPAIGN_SCHEMA_V1: &str = "dusklight-tactic-q-campaign/v1";
pub const TACTIC_Q_CHECKPOINT_SCHEMA_V1: &str = "dusklight-tactic-q-checkpoint/v1";
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

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticCampaignBranch {
    pub kind: TacticBranchKind,
    pub state_sha256: Digest,
    pub route_checkpoint_sha256: Digest,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticCampaignDiagnostics {
    pub replay_rows: usize,
    pub frontier_cells: usize,
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
        let mut nodes = BTreeMap::<Digest, TacticCampaignGraphNode>::new();
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
        let mut reachable = BTreeSet::from([root_checkpoint_sha256]);
        loop {
            let before = reachable.len();
            for edge in &edges {
                if reachable.contains(&edge.source_checkpoint_sha256) {
                    reachable.insert(edge.next_checkpoint_sha256);
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

    pub fn diagnostics(&self) -> Result<TacticCampaignDiagnostics, TacticQCampaignError> {
        let archive = self.frontier_archive()?;
        let graph = self.graph()?;
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
            unique_selected_actions: selected_actions.len(),
            zero_diversity_selection: self.replay.len() >= 2 && selected_actions.len() <= 1,
            repeated_identical_compositions: composition_counts.values().any(|count| *count > 1),
            no_progress_loop: has_no_progress_loop(&self.replay, &self.episode_groups)?,
            frontier_lost_root_connectivity: !graph.root_connected,
        })
    }

    /// Returns one root and one retained frontier branch on every call. The
    /// retained choice is seeded; root connectivity is therefore sampled
    /// explicitly instead of being left to archive luck.
    pub fn sample_root_and_frontier(
        &self,
        seed: u64,
        round: u64,
        reference: &[TacticEndpointDescriptor],
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
        let root = TacticCampaignBranch {
            kind: TacticBranchKind::Root,
            state_sha256: first.before_state_sha256,
            route_checkpoint_sha256: route_checkpoint(self.root_checkpoint_sha256, &root_route)?,
            state: first.before.clone(),
            route_tape: root_route,
            descriptor: None,
        };
        let archive = self.frontier_archive()?;
        let choices = archive.select_tactic_frontier(reference, archive.tactic_len());
        if choices.is_empty() {
            return Err(TacticQCampaignError::InvalidState(
                "frontier archive has no restorable endpoint",
            ));
        }
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight-tactic-frontier-sample/v1");
        hasher.update(seed.to_le_bytes());
        hasher.update(round.to_le_bytes());
        let digest = hasher.finalize();
        let index =
            (u64::from_le_bytes(digest[..8].try_into().unwrap()) % choices.len() as u64) as usize;
        let selected: &TacticFrontierEntry = &choices[index];
        let frontier = TacticCampaignBranch {
            kind: TacticBranchKind::RetainedFrontier,
            state_sha256: selected.transition.after_state_sha256,
            route_checkpoint_sha256: selected.route_checkpoint_sha256,
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
        let admitted = match branch.kind {
            TacticBranchKind::Root => self.replay.first().is_some_and(|first| {
                first.before_state_sha256 == branch.state_sha256
                    && first.source_checkpoint_sha256 == branch.route_checkpoint_sha256
            }),
            TacticBranchKind::RetainedFrontier => frontier
                .select_tactic_frontier(&[], frontier.tactic_len())
                .iter()
                .any(|entry| {
                    entry.transition.after_state_sha256 == branch.state_sha256
                        && entry.route_checkpoint_sha256 == branch.route_checkpoint_sha256
                }),
        };
        if !admitted
            || self.episode_groups.contains(&episode_group)
            || branch.state_sha256
                != branch
                    .state
                    .content_sha256()
                    .map_err(|error| TacticQCampaignError::Features(error.to_string()))?
            || branch.state.tape_frame != branch.route_tape.frames.len() as u64
            || branch.route_checkpoint_sha256
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
            schema: TACTIC_Q_CHECKPOINT_SCHEMA_V1.into(),
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
    /// The supplied digest identifies the complete executable catalog, while
    /// the training payload remains the existing semi-Markov value batch.
    pub fn freeze_greedy_policy(
        &self,
        action_universe_sha256: Digest,
    ) -> Result<TacticFrozenPolicy, TacticQCampaignError> {
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
        fs::create_dir_all(directory)
            .map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
        let final_path = directory.join(format!("tactic-q-{}.json", checkpoint.content_sha256));
        let bytes = serde_json::to_vec_pretty(&checkpoint)
            .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
        if final_path.exists() {
            let existing = fs::read(&final_path)
                .map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
            if existing == bytes {
                return Ok(final_path);
            }
            return Err(TacticQCampaignError::InvalidState(
                "content-addressed checkpoint path contains different bytes",
            ));
        }
        let partial_path = directory.join(format!(
            ".tactic-q-{}.{}.partial",
            checkpoint.content_sha256,
            std::process::id()
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial_path)
            .map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
        fs::rename(&partial_path, &final_path)
            .map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
        Ok(final_path)
    }

    pub fn read_checkpoint(path: &Path) -> Result<Self, TacticQCampaignError> {
        let bytes = fs::read(path).map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
        let checkpoint: TacticQCampaignCheckpoint = serde_json::from_slice(&bytes)
            .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
        Self::resume(checkpoint)
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
        let selected = choose_tactic(&ranking, self.decision_index, self.exploration)?;
        Ok(TacticQDecision { ranking, selected })
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
        )
    }

    /// Reward-policy variant of [`Self::execute_and_refit`]. It composes
    /// terminal bonus, exact tick cost, first-visit novelty, and optional
    /// potential shaping without granting any of them terminal authority.
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
        let step = self.retain_and_refit(
            decision,
            outcome,
            catalog,
            blueprints,
            registry,
            encode,
            entry_applicable,
            move |_, _, _| training_reward,
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
    ) -> Result<TacticQCampaignStep, TacticQCampaignError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
        A: Fn(&TacticAssetDescription) -> bool,
        R: Fn(&FactSnapshot, &FactSnapshot, &OptionExecution) -> f32,
    {
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
        let model = OptionValueModel::fit_batch(&batch, &self.model_config)?;

        self.visited_states.insert(tactic_state_descriptor(
            &next.snapshot,
            transition.value_sample.terminal,
        ));
        self.current = next;
        self.route_tape = outcome.route_tape;
        self.replay = replay;
        self.replay_routes = replay_routes;
        self.episode_groups = episode_groups;
        self.model = Some(model);
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

fn action_digest(action: &OptionActionDescriptor) -> Result<Digest, TacticQCampaignError> {
    let bytes = serde_json::to_vec(action)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    Ok(sha256(&bytes))
}

fn insert_graph_node(
    nodes: &mut BTreeMap<Digest, TacticCampaignGraphNode>,
    node: TacticCampaignGraphNode,
) -> Result<(), TacticQCampaignError> {
    if let Some(existing) = nodes.get(&node.checkpoint_sha256) {
        if existing != &node {
            return Err(TacticQCampaignError::InvalidState(
                "one checkpoint identifies conflicting campaign graph nodes",
            ));
        }
    } else {
        nodes.insert(node.checkpoint_sha256, node);
    }
    Ok(())
}

fn has_no_progress_loop(
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

fn validate_checkpoint(checkpoint: &TacticQCampaignCheckpoint) -> Result<(), TacticQCampaignError> {
    checkpoint.current.validate()?;
    checkpoint
        .route_tape
        .validate()
        .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
    if checkpoint.schema != TACTIC_Q_CHECKPOINT_SCHEMA_V1
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

fn checkpoint_digest(
    checkpoint: &TacticQCampaignCheckpoint,
) -> Result<Digest, TacticQCampaignError> {
    let mut canonical = checkpoint.clone();
    canonical.content_sha256 = Digest::ZERO;
    let bytes = serde_json::to_vec(&canonical)
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

fn route_checkpoint(
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
            )
            .unwrap();

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
        let [root_branch, frontier_branch] = restored.sample_root_and_frontier(5, 0, &[]).unwrap();
        assert_eq!(root_branch.kind, TacticBranchKind::Root);
        assert_eq!(frontier_branch.kind, TacticBranchKind::RetainedFrontier);
        assert_eq!(
            frontier_branch.state_sha256,
            campaign.current.snapshot_sha256
        );
        let mut branched = TacticQCampaign::resume(checkpoint.clone()).unwrap();
        branched
            .restore_branch(&root_branch, 22, &registry, &catalog, &[], |_| true)
            .unwrap();
        assert_eq!(branched.episode_group, 22);
        assert_eq!(branched.current.snapshot_sha256, root_branch.state_sha256);
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
        let from_file = TacticQCampaign::read_checkpoint(&path).unwrap();
        assert_eq!(from_file.replay, campaign.replay);
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&directory).unwrap();

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
    }
}
