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
use dusklight_learning::learner_state::{LearnerState, LearnerStateError};
use dusklight_learning::live_tactic_catalog::{
    LiveTacticCatalog, LiveTacticCatalogError, LiveTacticRanking,
};
use dusklight_learning::option_transition::{OptionTransitionError, OptionTransitionSample};
use dusklight_learning::option_values::{
    AvailableOptionRanking, OptionValueBatch, OptionValueConfig, OptionValueError, OptionValueModel,
};
use dusklight_learning::tactic_asset::{TacticAssetCatalog, TacticAssetDescription};
use dusklight_learning::tactic_blueprint::TacticBlueprint;
use dusklight_learning::tactic_exploration::{
    SelectedTactic, TacticExplorationConfig, TacticExplorationError, choose_tactic,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const TACTIC_Q_CAMPAIGN_SCHEMA_V1: &str = "dusklight-tactic-q-campaign/v1";
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
        })
    }

    pub fn model(&self) -> Option<&OptionValueModel> {
        self.model.as_ref()
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

#[derive(Debug)]
pub enum TacticQCampaignError {
    InvalidState(&'static str),
    Features(String),
    Tape(String),
    Serialization(String),
    LearnerState(LearnerStateError),
    Catalog(LiveTacticCatalogError),
    Exploration(TacticExplorationError),
    Transition(OptionTransitionError),
    Values(OptionValueError),
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
            Self::Serialization(message) => {
                write!(formatter, "tactic-Q serialization failed: {message}")
            }
            Self::LearnerState(error) => write!(formatter, "tactic-Q state failed: {error}"),
            Self::Catalog(error) => write!(formatter, "tactic-Q catalog failed: {error}"),
            Self::Exploration(error) => write!(formatter, "tactic-Q selection failed: {error}"),
            Self::Transition(error) => write!(formatter, "tactic-Q transition failed: {error}"),
            Self::Values(error) => write!(formatter, "tactic-Q refit failed: {error}"),
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
            schema: crate::native_tactic_worker::NATIVE_TACTIC_WORKER_OUTCOME_SCHEMA_V1.into(),
            source_checkpoint_sha256: root_checkpoint_sha256,
            checkpoint_identity: "fixture-checkpoint".into(),
            episode_shard_sha256: shard.content_sha256,
            selected: decision.selected.clone(),
            execution,
            route_tape,
            next_facts: after,
            terminal,
        };
        let retained = campaign
            .retain_and_refit(
                decision,
                outcome,
                &catalog,
                &[],
                &registry,
                &encode,
                |_| true,
                |_, _, _| 5.0,
            )
            .unwrap();

        assert_eq!(retained.replay_rows, 1);
        assert_eq!(campaign.replay.len(), 1);
        assert_eq!(campaign.episode_groups, vec![11]);
        assert!(campaign.model().is_some());
        assert_eq!(campaign.current.snapshot.tape_frame, before.tape_frame + 1);
        assert_eq!(
            campaign.route_tape.frames.len() as u64,
            campaign.current.snapshot.tape_frame
        );

        let next = campaign.decide(&catalog, &[], &encode).unwrap();
        assert_eq!(next.selected.reason, TacticSelectionReason::Greedy);
        assert_eq!(next.ranking.values.ranked.len(), 1);
        assert!(next.ranking.values.unsupported.is_empty());
    }
}
