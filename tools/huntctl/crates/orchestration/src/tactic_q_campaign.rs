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
    AvailableOptionRanking, OptionValueBatch, OptionValueConfig, OptionValueError, OptionValueModel,
};
use dusklight_learning::reward_shaping::{ShapingError, TacticRewardBreakdown, TacticRewardSpec};
use dusklight_learning::tactic_asset::{TacticAssetCatalog, TacticAssetDescription};
use dusklight_learning::tactic_blueprint::TacticBlueprint;
use dusklight_learning::tactic_exploration::{
    SelectedTactic, TacticExplorationConfig, TacticExplorationError, choose_tactic,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
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
    pub terminal: FactSnapshot,
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
    pub episode_groups: Vec<u64>,
    model_config: OptionValueConfig,
    exploration: TacticExplorationConfig,
    model: Option<OptionValueModel>,
    visited_states: BTreeSet<Digest>,
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
        let visited_states = BTreeSet::from([current.snapshot_sha256]);
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
            episode_groups: self.episode_groups.clone(),
            model_config: self.model_config.clone(),
            exploration: self.exploration,
        };
        checkpoint.content_sha256 = checkpoint_digest(&checkpoint)?;
        validate_checkpoint(&checkpoint)?;
        Ok(checkpoint)
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
        let mut visited_states = BTreeSet::from([checkpoint.current.snapshot_sha256]);
        for transition in &checkpoint.replay {
            visited_states.insert(transition.before_state_sha256);
            visited_states.insert(transition.after_state_sha256);
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
        let replay_bytes = serde_json::to_vec(&self.replay)
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
        let endpoint_sha256 = outcome
            .next_facts
            .content_sha256()
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        let reward = reward_spec.evaluate(
            self.feature_schema_sha256,
            &state,
            &next_state,
            outcome.execution.duration.realized_ticks,
            outcome.terminal,
            !self.visited_states.contains(&endpoint_sha256),
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

        self.visited_states.insert(next.snapshot_sha256);
        self.current = next;
        self.route_tape = outcome.route_tape;
        self.replay = replay;
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
        || checkpoint.decision_index != checkpoint.replay.len() as u64
        || checkpoint.current.snapshot.tape_frame != checkpoint.route_tape.frames.len() as u64
    {
        return Err(TacticQCampaignError::InvalidState(
            "campaign checkpoint identity or shape is invalid",
        ));
    }
    let mut prior_after = None;
    let mut prior_checkpoint = None;
    for transition in &checkpoint.replay {
        transition.validate()?;
        if transition.feature_schema_sha256 != checkpoint.feature_schema_sha256
            || prior_after.is_some_and(|digest| digest != transition.before_state_sha256)
            || prior_checkpoint.is_some_and(|digest| digest != transition.source_checkpoint_sha256)
        {
            return Err(TacticQCampaignError::InvalidState(
                "campaign checkpoint replay chain is detached",
            ));
        }
        transition
            .execution
            .validate_against_tape(&checkpoint.route_tape)
            .map_err(|error| TacticQCampaignError::Tape(error.to_string()))?;
        let start = usize::try_from(transition.execution.realized_tape_range.start_frame)
            .map_err(|_| TacticQCampaignError::InvalidState("replay tape range overflows"))?;
        let end = usize::try_from(transition.execution.realized_tape_range.end_frame_exclusive)
            .map_err(|_| TacticQCampaignError::InvalidState("replay tape range overflows"))?;
        if end > checkpoint.route_tape.frames.len()
            || transition.source_checkpoint_sha256
                != route_checkpoint(
                    checkpoint.root_checkpoint_sha256,
                    &tape_prefix(&checkpoint.route_tape, start),
                )?
            || transition.next_checkpoint_sha256
                != route_checkpoint(
                    checkpoint.root_checkpoint_sha256,
                    &tape_prefix(&checkpoint.route_tape, end),
                )?
        {
            return Err(TacticQCampaignError::InvalidState(
                "campaign checkpoint replay route is detached",
            ));
        }
        prior_after = Some(transition.after_state_sha256);
        prior_checkpoint = Some(transition.next_checkpoint_sha256);
    }
    if let Some(after) = prior_after
        && after != checkpoint.current.snapshot_sha256
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
    let replay_bytes = serde_json::to_vec(&result.replay)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    if result.schema != TACTIC_Q_FINAL_RESULT_SCHEMA_V1
        || result.content_sha256 == Digest::ZERO
        || result.content_sha256 != final_result_digest(result)?
        || result.objective_sha256 == Digest::ZERO
        || result.root_checkpoint_sha256 == Digest::ZERO
        || result.route_tape_sha256 != sha256(&route_bytes)
        || result.replay_sha256 != sha256(&replay_bytes)
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
    for transition in &result.replay {
        transition.validate()?;
        transition
            .execution
            .validate_against_tape(&result.route_tape)
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
    LearnerState(LearnerStateError),
    Catalog(LiveTacticCatalogError),
    Exploration(TacticExplorationError),
    Transition(OptionTransitionError),
    Values(OptionValueError),
    Shaping(ShapingError),
    Hindsight(HindsightError),
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
            Self::LearnerState(error) => write!(formatter, "tactic-Q state failed: {error}"),
            Self::Catalog(error) => write!(formatter, "tactic-Q catalog failed: {error}"),
            Self::Exploration(error) => write!(formatter, "tactic-Q selection failed: {error}"),
            Self::Transition(error) => write!(formatter, "tactic-Q transition failed: {error}"),
            Self::Values(error) => write!(formatter, "tactic-Q refit failed: {error}"),
            Self::Shaping(error) => write!(formatter, "tactic-Q reward failed: {error}"),
            Self::Hindsight(error) => write!(formatter, "tactic-Q hindsight failed: {error}"),
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
        assert!(retained.reward.endpoint_novel);
        assert_eq!(retained.reward.tick_cost_component, -0.25);
        assert_eq!(retained.reward.novelty_component, 1.0);
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
        assert_eq!(campaign.visited_state_count(), 2);

        let checkpoint = campaign.checkpoint().unwrap();
        let restored = TacticQCampaign::resume(checkpoint.clone()).unwrap();
        assert_eq!(restored.decision_index, campaign.decision_index);
        assert_eq!(restored.route_tape, campaign.route_tape);
        assert_eq!(restored.replay, campaign.replay);
        assert!(restored.model().is_some());
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
