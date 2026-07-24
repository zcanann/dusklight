//! Learner state = one fact snapshot plus the exact concrete action universe
//! and its current applicability mask.

use crate::artifact::Digest;
use crate::fact_registry::{FactQuery, FactRead, FactRegistry, FactValue};
use crate::fact_snapshot::FactSnapshot;
use crate::option_values::OptionActionDescriptor;
use crate::tactic_asset::{TacticAssetCatalog, TacticAssetDescription, TacticDurationBounds};
use crate::tactic_blueprint::{
    ApplicableTacticChoices, ConcreteTacticChoiceKind, TacticBlueprint, TacticBlueprintError,
};
use dusklight_control::option_execution::OptionCondition;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const LEARNER_STATE_SCHEMA_V1: &str = "dusklight-learner-state/v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearnerState {
    pub schema: String,
    pub snapshot_sha256: Digest,
    pub fact_registry_sha256: Digest,
    pub action_universe_sha256: Digest,
    pub applicable_choice_schema_sha256: Digest,
    pub snapshot: FactSnapshot,
    pub action_mask: Vec<LearnerActionMaskEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearnerActionMaskEntry {
    pub choice_id: String,
    pub kind: ConcreteTacticChoiceKind,
    pub descriptor: OptionActionDescriptor,
    pub duration: TacticDurationBounds,
    pub applicable: bool,
}

impl LearnerState {
    pub fn build<F>(
        snapshot: FactSnapshot,
        registry: &FactRegistry,
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        entry_applicable: F,
    ) -> Result<Self, LearnerStateError>
    where
        F: Fn(&TacticAssetDescription) -> bool,
    {
        snapshot
            .validate()
            .map_err(|error| LearnerStateError::Snapshot(error.to_string()))?;
        registry
            .validate()
            .map_err(|error| LearnerStateError::Registry(error.to_string()))?;
        let universe =
            ApplicableTacticChoices::enumerate(catalog, blueprints, |_| true, |_| Some(false))
                .map_err(LearnerStateError::Blueprint)?;
        let applicable = ApplicableTacticChoices::enumerate(
            catalog,
            blueprints,
            |description| entry_applicable(description),
            |condition| condition_value(registry, &snapshot, condition),
        )
        .map_err(LearnerStateError::Blueprint)?;
        let applicable_ids = applicable
            .choices
            .iter()
            .map(|choice| choice.choice_id.as_str())
            .collect::<BTreeSet<_>>();
        let action_mask = universe
            .choices
            .into_iter()
            .map(|choice| LearnerActionMaskEntry {
                applicable: applicable_ids.contains(choice.choice_id.as_str()),
                choice_id: choice.choice_id,
                kind: choice.kind,
                descriptor: choice.descriptor,
                duration: choice.duration,
            })
            .collect::<Vec<_>>();
        let state = Self {
            schema: LEARNER_STATE_SCHEMA_V1.into(),
            snapshot_sha256: snapshot
                .content_sha256()
                .map_err(|error| LearnerStateError::Snapshot(error.to_string()))?,
            fact_registry_sha256: registry.schema_sha256,
            action_universe_sha256: digest_universe(&action_mask)?,
            applicable_choice_schema_sha256: applicable.choice_schema_sha256,
            snapshot,
            action_mask,
        };
        state.validate()?;
        Ok(state)
    }

    pub fn validate(&self) -> Result<(), LearnerStateError> {
        if self.schema != LEARNER_STATE_SCHEMA_V1
            || self.snapshot_sha256 == Digest::ZERO
            || self.fact_registry_sha256 == Digest::ZERO
            || self.action_universe_sha256 == Digest::ZERO
            || self.applicable_choice_schema_sha256 == Digest::ZERO
            || self.action_mask.is_empty()
            || self
                .action_mask
                .windows(2)
                .any(|pair| pair[0].choice_id >= pair[1].choice_id)
            || digest_universe(&self.action_mask)? != self.action_universe_sha256
        {
            return Err(LearnerStateError::InvalidState);
        }
        self.snapshot
            .validate()
            .map_err(|error| LearnerStateError::Snapshot(error.to_string()))?;
        if self
            .snapshot
            .content_sha256()
            .map_err(|error| LearnerStateError::Snapshot(error.to_string()))?
            != self.snapshot_sha256
        {
            return Err(LearnerStateError::InvalidState);
        }
        for entry in &self.action_mask {
            entry
                .descriptor
                .validate()
                .map_err(|error| LearnerStateError::Descriptor(error.to_string()))?;
            if entry.choice_id != entry.descriptor.option_id
                || entry.duration.minimum_ticks == 0
                || entry.duration.minimum_ticks > entry.duration.maximum_ticks
            {
                return Err(LearnerStateError::InvalidState);
            }
        }
        Ok(())
    }

    pub fn applicable_descriptors(&self) -> impl Iterator<Item = &OptionActionDescriptor> {
        self.action_mask
            .iter()
            .filter(|entry| entry.applicable)
            .map(|entry| &entry.descriptor)
    }
}

fn condition_value(
    registry: &FactRegistry,
    snapshot: &FactSnapshot,
    condition: &OptionCondition,
) -> Option<bool> {
    match registry
        .read(snapshot, &FactQuery::Condition(condition.clone()))
        .ok()?
    {
        FactRead::Available(FactValue::Bool(value)) => Some(value),
        _ => None,
    }
}

fn digest_universe(mask: &[LearnerActionMaskEntry]) -> Result<Digest, LearnerStateError> {
    Ok(Digest(
        Sha256::digest(
            serde_json::to_vec(
                &mask
                    .iter()
                    .map(|entry| {
                        (
                            &entry.choice_id,
                            entry.kind,
                            &entry.descriptor,
                            entry.duration,
                        )
                    })
                    .collect::<Vec<_>>(),
            )
            .map_err(|error| LearnerStateError::Serialization(error.to_string()))?,
        )
        .into(),
    ))
}

#[derive(Debug)]
pub enum LearnerStateError {
    Snapshot(String),
    Registry(String),
    Blueprint(TacticBlueprintError),
    Descriptor(String),
    InvalidState,
    Serialization(String),
}

impl fmt::Display for LearnerStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Snapshot(message) => write!(formatter, "learner fact snapshot failed: {message}"),
            Self::Registry(message) => write!(formatter, "learner fact registry failed: {message}"),
            Self::Blueprint(error) => {
                write!(formatter, "learner action enumeration failed: {error}")
            }
            Self::Descriptor(message) => {
                write!(formatter, "learner action descriptor failed: {message}")
            }
            Self::InvalidState => formatter.write_str("learner state is invalid"),
            Self::Serialization(message) => {
                write!(formatter, "learner state serialization failed: {message}")
            }
        }
    }
}

impl Error for LearnerStateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Blueprint(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fact_snapshot::FactSnapshot;
    use crate::tactic_asset::{TacticAssetSource, TacticCatalogEntry};
    use crate::tactic_blueprint::TacticBlueprintNode;
    use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;

    #[test]
    fn learner_state_carries_full_universe_mask_and_concrete_parameters() {
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
        let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new(
                "shield.short",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 1,
                })),
            )
            .unwrap(),
            TacticCatalogEntry::new(
                "shield.long",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 4,
                })),
            )
            .unwrap(),
        ])
        .unwrap();
        let blueprint = TacticBlueprint::new(
            "use.short",
            TacticBlueprintNode::Invoke {
                option_id: "shield.short".into(),
            },
        )
        .unwrap();

        let state = LearnerState::build(
            snapshot,
            &FactRegistry::canonical(),
            &catalog,
            &[blueprint],
            |description| description.option.option_id != "shield.long",
        )
        .unwrap();
        assert_eq!(state.action_mask.len(), 3);
        assert_eq!(
            state
                .action_mask
                .iter()
                .map(|entry| (entry.choice_id.as_str(), entry.applicable))
                .collect::<Vec<_>>(),
            vec![
                ("blueprint/use.short", true),
                ("shield.long", false),
                ("shield.short", true),
            ]
        );
        assert_eq!(state.applicable_descriptors().count(), 2);
        assert_ne!(
            state.action_mask[1].descriptor.parameters,
            state.action_mask[2].descriptor.parameters
        );
        state.validate().unwrap();
    }
}
