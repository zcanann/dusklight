//! State-local executable tactic catalog and existing-Q ranking.

use crate::artifact::Digest;
use crate::learner_state::{LearnerActionMaskEntry, LearnerState, LearnerStateError};
use crate::option_values::{
    AvailableOptionRanking, OptionActionDescriptor, OptionValueError, OptionValueModel,
};
use crate::tactic_asset::{TacticAssetCatalog, TacticAssetError};
use crate::tactic_blueprint::{
    ApplicableTacticChoices, ConcreteTacticChoiceKind, TacticBlueprint, TacticBlueprintError,
};
use serde::Serialize;
use std::error::Error;
use std::fmt;

pub const LIVE_TACTIC_CATALOG_SCHEMA_V1: &str = "dusklight-live-tactic-catalog/v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LiveTacticCatalog {
    pub schema: String,
    pub learner_snapshot_sha256: Digest,
    pub action_universe_sha256: Digest,
    pub applicability_sha256: Digest,
    pub choices: Vec<LearnerActionMaskEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LiveTacticRanking {
    pub learner_snapshot_sha256: Digest,
    pub action_universe_sha256: Digest,
    pub choices: Vec<LearnerActionMaskEntry>,
    pub values: AvailableOptionRanking,
}

impl LiveTacticCatalog {
    pub fn build(
        state: &LearnerState,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
    ) -> Result<Self, LiveTacticCatalogError> {
        state.validate()?;
        let mut choices = Vec::new();
        for entry in state.action_mask.iter().filter(|entry| entry.applicable) {
            match entry.kind {
                ConcreteTacticChoiceKind::CatalogEntry => {
                    let catalog_entry = catalog.entry(&entry.choice_id).ok_or_else(|| {
                        LiveTacticCatalogError::MissingExecutor(entry.choice_id.clone())
                    })?;
                    if catalog_entry.description().option != entry.descriptor
                        || catalog_entry.description().duration != entry.duration
                    {
                        return Err(LiveTacticCatalogError::DetachedChoice(
                            entry.choice_id.clone(),
                        ));
                    }
                    catalog.prepare_execution(&entry.choice_id)?;
                }
                ConcreteTacticChoiceKind::Blueprint => {
                    let asset_id = entry.choice_id.strip_prefix("blueprint/").ok_or_else(|| {
                        LiveTacticCatalogError::DetachedChoice(entry.choice_id.clone())
                    })?;
                    let blueprint = blueprints
                        .iter()
                        .find(|blueprint| blueprint.asset_id == asset_id)
                        .ok_or_else(|| {
                            LiveTacticCatalogError::MissingExecutor(entry.choice_id.clone())
                        })?;
                    blueprint.compile_static(catalog)?;
                    let enumerated = ApplicableTacticChoices::enumerate(
                        catalog,
                        std::slice::from_ref(blueprint),
                        |_| true,
                        |_| Some(false),
                    )?;
                    let expected = enumerated
                        .candidates
                        .iter()
                        .find(|candidate| candidate.choice_id == entry.choice_id)
                        .ok_or_else(|| {
                            LiveTacticCatalogError::DetachedChoice(entry.choice_id.clone())
                        })?;
                    if expected.descriptor != entry.descriptor
                        || expected.duration != entry.duration
                    {
                        return Err(LiveTacticCatalogError::DetachedChoice(
                            entry.choice_id.clone(),
                        ));
                    }
                }
            }
            choices.push(entry.clone());
        }
        Ok(Self {
            schema: LIVE_TACTIC_CATALOG_SCHEMA_V1.into(),
            learner_snapshot_sha256: state.snapshot_sha256,
            action_universe_sha256: state.action_universe_sha256,
            applicability_sha256: state.applicable_choice_schema_sha256,
            choices,
        })
    }

    pub fn descriptors(&self) -> impl ExactSizeIterator<Item = &OptionActionDescriptor> {
        self.choices.iter().map(|entry| &entry.descriptor)
    }

    pub fn rank(
        &self,
        model: &OptionValueModel,
        state_features: &[f32],
    ) -> Result<LiveTacticRanking, LiveTacticCatalogError> {
        let descriptors = self.descriptors().cloned().collect::<Vec<_>>();
        let values = model.rank_available_options(state_features, &descriptors)?;
        Ok(LiveTacticRanking {
            learner_snapshot_sha256: self.learner_snapshot_sha256,
            action_universe_sha256: self.action_universe_sha256,
            choices: self.choices.clone(),
            values,
        })
    }
}

#[derive(Debug)]
pub enum LiveTacticCatalogError {
    State(LearnerStateError),
    Asset(TacticAssetError),
    Blueprint(TacticBlueprintError),
    Values(OptionValueError),
    MissingExecutor(String),
    DetachedChoice(String),
}

impl fmt::Display for LiveTacticCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::State(error) => write!(formatter, "live tactic state failed: {error}"),
            Self::Asset(error) => write!(formatter, "live tactic asset failed: {error}"),
            Self::Blueprint(error) => write!(formatter, "live tactic blueprint failed: {error}"),
            Self::Values(error) => write!(formatter, "live tactic ranking failed: {error}"),
            Self::MissingExecutor(choice) => {
                write!(formatter, "live tactic {choice:?} has no exact executor")
            }
            Self::DetachedChoice(choice) => {
                write!(
                    formatter,
                    "live tactic {choice:?} differs from its executor"
                )
            }
        }
    }
}

impl Error for LiveTacticCatalogError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::State(error) => Some(error),
            Self::Asset(error) => Some(error),
            Self::Blueprint(error) => Some(error),
            Self::Values(error) => Some(error),
            _ => None,
        }
    }
}

impl From<LearnerStateError> for LiveTacticCatalogError {
    fn from(value: LearnerStateError) -> Self {
        Self::State(value)
    }
}

impl From<TacticAssetError> for LiveTacticCatalogError {
    fn from(value: TacticAssetError) -> Self {
        Self::Asset(value)
    }
}

impl From<TacticBlueprintError> for LiveTacticCatalogError {
    fn from(value: TacticBlueprintError) -> Self {
        Self::Blueprint(value)
    }
}

impl From<OptionValueError> for LiveTacticCatalogError {
    fn from(value: OptionValueError) -> Self {
        Self::Values(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fact_registry::FactRegistry;
    use crate::fact_snapshot::FactSnapshot;
    use crate::option_values::{OptionValueConfig, OptionValueSample};
    use crate::tactic_asset::{TacticAssetSource, TacticCatalogEntry};
    use crate::tactic_blueprint::TacticBlueprint;
    use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
    use dusklight_control::option_execution::TapeRange;
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;

    fn sample(action: OptionActionDescriptor, reward: f32, digest: u8) -> OptionValueSample {
        OptionValueSample {
            action,
            state: vec![0.0],
            duration_ticks: 1,
            reward,
            next_state: vec![1.0],
            terminal: true,
            before_state_sha256: Digest([digest; 32]),
            after_state_sha256: Digest([digest + 1; 32]),
            source_checkpoint_sha256: Digest([digest + 2; 32]),
            next_checkpoint_sha256: Digest([digest + 3; 32]),
            realized_tape_range: TapeRange {
                start_frame: 0,
                end_frame_exclusive: 1,
            },
            realized_tape_sha256: Digest([digest + 4; 32]),
        }
    }

    #[test]
    fn ranks_only_the_current_exact_executable_catalog() {
        let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new(
                "shield",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 1,
                })),
            )
            .unwrap(),
            TacticCatalogEntry::new(
                "attack",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Target {
                    frames: 1,
                })),
            )
            .unwrap(),
        ])
        .unwrap();
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let snapshot = FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap();
        let state = LearnerState::build(
            snapshot,
            &FactRegistry::canonical(),
            &catalog,
            &[] as &[TacticBlueprint],
            |description| description.option.option_id == "shield",
        )
        .unwrap();
        let live = LiveTacticCatalog::build(&state, &catalog, &[]).unwrap();
        assert_eq!(live.choices.len(), 1);
        assert_eq!(live.choices[0].choice_id, "shield");

        let shield = catalog
            .entry("shield")
            .unwrap()
            .description()
            .option
            .clone();
        let attack = catalog
            .entry("attack")
            .unwrap()
            .description()
            .option
            .clone();
        let model = OptionValueModel::fit(
            1,
            &[sample(shield, 1.0, 1), sample(attack, 100.0, 10)],
            &[0, 1],
            &OptionValueConfig::default(),
        )
        .unwrap();
        let ranking = live.rank(&model, &[0.0]).unwrap();
        assert_eq!(ranking.values.ranked.len(), 1);
        assert_eq!(ranking.values.ranked[0].descriptor.option_id, "shield");
        assert!(ranking.values.unsupported.is_empty());
    }
}
