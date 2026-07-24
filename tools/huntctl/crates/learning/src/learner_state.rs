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
use dusklight_control::option_execution::{OptionCondition, OptionParameter};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt::{self, Write as _};

pub const LEARNER_STATE_SCHEMA_V1: &str = "dusklight-learner-state/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
        let applicable = ApplicableTacticChoices::enumerate(
            catalog,
            blueprints,
            |description| entry_applicable(description),
            |condition| condition_value(registry, &snapshot, condition),
        )
        .map_err(LearnerStateError::Blueprint)?;
        let action_mask = applicable
            .candidates
            .into_iter()
            .zip(applicable.applicable_mask)
            .map(|(choice, is_applicable)| LearnerActionMaskEntry {
                applicable: is_applicable,
                choice_id: choice.choice_id,
                kind: choice.kind,
                descriptor: choice.descriptor,
                duration: choice.duration,
            })
            .collect::<Vec<_>>();
        let action_universe_sha256 = digest_universe(&action_mask)?;
        let state = Self {
            schema: LEARNER_STATE_SCHEMA_V1.into(),
            snapshot_sha256: snapshot
                .content_sha256()
                .map_err(|error| LearnerStateError::Snapshot(error.to_string()))?,
            fact_registry_sha256: registry.schema_sha256,
            action_universe_sha256,
            applicable_choice_schema_sha256: digest_applicability(
                action_universe_sha256,
                &action_mask,
            )?,
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
            || digest_applicability(self.action_universe_sha256, &self.action_mask)?
                != self.applicable_choice_schema_sha256
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

    /// Compact inspection text derived only from this authenticated state.
    pub fn infodump(&self) -> Result<String, LearnerStateError> {
        self.validate()?;
        const ACTOR_LIMIT: usize = 12;
        const ACTION_LIMIT: usize = 32;
        let mut output = String::new();
        writeln!(
            output,
            "State: {} room {} | tick {} | tape {}",
            self.snapshot.world.stage,
            self.snapshot.world.room,
            self.snapshot.simulation_tick,
            self.snapshot.tape_frame,
        )
        .unwrap();
        let position = self.snapshot.player.position_f32_bits.map(f32::from_bits);
        writeln!(
            output,
            "Player: position [{:.2}, {:.2}, {:.2}] | procedure {} | contacts {:#04x}",
            position[0],
            position[1],
            position[2],
            self.snapshot
                .player
                .procedure
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            self.snapshot.player.contacts.unwrap_or_default(),
        )
        .unwrap();
        writeln!(
            output,
            "Goal: configured {} | reached {} | terminal {:?}",
            readable_optional(self.snapshot.terminal.configured),
            readable_optional(self.snapshot.terminal.reached),
            self.snapshot.terminal.reason,
        )
        .unwrap();
        if let Some(event) = &self.snapshot.event {
            writeln!(
                output,
                "Event: running {} | id {} | mode {}",
                event.running, event.event_id, event.mode
            )
            .unwrap();
        } else {
            writeln!(output, "Event: unavailable").unwrap();
        }
        writeln!(output, "Actors: {} complete", self.snapshot.actors.len()).unwrap();
        for actor in self.snapshot.actors.iter().take(ACTOR_LIMIT) {
            let actor_position = actor.position_f32_bits.map(f32::from_bits);
            writeln!(
                output,
                "  - actor {} set {} room {} at [{:.1}, {:.1}, {:.1}]",
                actor.actor_name,
                actor.set_id,
                actor.current_room,
                actor_position[0],
                actor_position[1],
                actor_position[2],
            )
            .unwrap();
        }
        if self.snapshot.actors.len() > ACTOR_LIMIT {
            writeln!(
                output,
                "  … {} more actors",
                self.snapshot.actors.len() - ACTOR_LIMIT
            )
            .unwrap();
        }
        let applicable_count = self
            .action_mask
            .iter()
            .filter(|entry| entry.applicable)
            .count();
        writeln!(
            output,
            "Actions: {applicable_count}/{} applicable",
            self.action_mask.len()
        )
        .unwrap();
        for action in self.action_mask.iter().take(ACTION_LIMIT) {
            write!(
                output,
                "  {} {} ({}..{} ticks)",
                if action.applicable { "+" } else { "-" },
                action.choice_id,
                action.duration.minimum_ticks,
                action.duration.maximum_ticks,
            )
            .unwrap();
            if !action.descriptor.parameters.is_empty() {
                write!(output, " | ").unwrap();
                for (index, (name, value)) in action.descriptor.parameters.iter().enumerate() {
                    if index != 0 {
                        write!(output, ", ").unwrap();
                    }
                    write!(output, "{name}={}", readable_parameter(value)).unwrap();
                }
            }
            writeln!(output).unwrap();
        }
        if self.action_mask.len() > ACTION_LIMIT {
            writeln!(
                output,
                "  … {} more actions",
                self.action_mask.len() - ACTION_LIMIT
            )
            .unwrap();
        }
        writeln!(
            output,
            "History: {} prior boundaries",
            self.snapshot.recent_history.len()
        )
        .unwrap();
        Ok(output)
    }
}

fn readable_optional(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unavailable",
    }
}

fn readable_parameter(value: &OptionParameter) -> String {
    match value {
        OptionParameter::Bool(value) => value.to_string(),
        OptionParameter::Signed(value) => value.to_string(),
        OptionParameter::Unsigned(value) => value.to_string(),
        OptionParameter::F32Bits(bits) => format!("{:.4}", f32::from_bits(*bits)),
        OptionParameter::Vec3F32Bits(bits) => {
            let values = bits.map(f32::from_bits);
            format!("[{:.3}, {:.3}, {:.3}]", values[0], values[1], values[2])
        }
        OptionParameter::Text(value) => value.clone(),
        OptionParameter::Digest(value) => {
            let hex = value
                .0
                .iter()
                .take(4)
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            format!("{hex}…")
        }
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

fn digest_applicability(
    action_universe_sha256: Digest,
    mask: &[LearnerActionMaskEntry],
) -> Result<Digest, LearnerStateError> {
    Ok(Digest(
        Sha256::digest(
            serde_json::to_vec(&(
                LEARNER_STATE_SCHEMA_V1,
                action_universe_sha256,
                mask.iter()
                    .map(|entry| entry.applicable)
                    .collect::<Vec<_>>(),
            ))
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
        let infodump = state.infodump().unwrap();
        assert!(infodump.contains("State:"));
        assert!(infodump.contains("Actions: 2/3 applicable"));
        assert!(infodump.contains("+ shield.short"));
        assert!(infodump.contains("- shield.long"));
        assert!(!infodump.contains("\"schema\""));
        state.validate().unwrap();

        let mut tampered = state.clone();
        tampered.action_mask[0].applicable = !tampered.action_mask[0].applicable;
        assert!(matches!(
            tampered.validate(),
            Err(LearnerStateError::InvalidState)
        ));
    }
}
