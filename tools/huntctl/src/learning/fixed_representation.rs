//! Fixed, bounded model inputs assembled from authenticated observation facts.

use super::goal_conditioning::{COMPILED_OBJECTIVE_VECTOR_WIDTH, CompiledObjectiveVector};
use crate::artifact::Digest;
use crate::observation_view::{MissingnessPolicy, ObservationSpec, movement_state_v2_spec};
use crate::transition_evidence::{EntityActorEvidence, EntityFactsEvidence, EvidenceAvailability};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::error::Error;
use std::f32::consts::PI;
use std::fmt;

pub const FIXED_MODEL_REPRESENTATION_SCHEMA_V1: &str = "dusklight-fixed-model-representation/v1";
pub const DEFAULT_NEAREST_ACTOR_SLOTS: usize = 4;
pub const MAX_NEAREST_ACTOR_SLOTS: usize = 16;
pub const DEFAULT_CATEGORICAL_EMBEDDING_WIDTH: usize = 8;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixedRepresentationLayout {
    pub schema: String,
    pub observation_schema_sha256: Digest,
    pub observation_width: usize,
    pub continuous_indices: Vec<usize>,
    pub categorical_indices: Vec<usize>,
    pub missingness_mask_indices: Vec<usize>,
    pub local_geometry_indices: Vec<usize>,
    pub categorical_embedding_width: usize,
    pub nearest_actor_slots: usize,
    pub actor_slot_width: usize,
    pub objective_width: usize,
    pub layout_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixedRepresentationInput {
    pub layout_sha256: Digest,
    pub continuous: Vec<f32>,
    pub categorical_embeddings: Vec<CategoricalEmbeddingInput>,
    pub missingness_masks: Vec<f32>,
    pub local_geometry_probes: Vec<f32>,
    pub objective: Vec<f32>,
    pub actor_source_available: bool,
    pub actor_source_truncated: bool,
    pub actor_slots: Vec<SemanticActorSlot>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CategoricalEmbeddingInput {
    pub feature_index: usize,
    pub category_bits: u32,
    pub embedding_width: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticActorSlot {
    pub present: f32,
    pub actor_name: i16,
    pub set_id: u16,
    pub current_room: i8,
    pub health_normalized: f32,
    pub status: u32,
    pub relative_position: [f32; 3],
    pub distance_normalized: f32,
    pub current_yaw_sin_cos: [f32; 2],
    pub shape_yaw_sin_cos: [f32; 2],
}

impl FixedRepresentationLayout {
    pub fn movement_v2(
        nearest_actor_slots: usize,
        categorical_embedding_width: usize,
    ) -> Result<Self, FixedRepresentationError> {
        let spec = movement_state_v2_spec();
        spec.validate()
            .map_err(|error| FixedRepresentationError::new(error.to_string()))?;
        if nearest_actor_slots == 0
            || nearest_actor_slots > MAX_NEAREST_ACTOR_SLOTS
            || categorical_embedding_width == 0
            || categorical_embedding_width > 64
        {
            return Err(FixedRepresentationError::new(
                "fixed representation bounds are invalid",
            ));
        }
        let categorical_indices = spec.categorical_features();
        let categorical = categorical_indices.iter().copied().collect::<BTreeSet<_>>();
        let continuous_indices = (0..spec.features.len())
            .filter(|index| !categorical.contains(index))
            .collect();
        let missingness_mask_indices: Vec<usize> = spec
            .features
            .iter()
            .filter_map(|feature| match feature.missingness {
                MissingnessPolicy::MaskedBy { field_id } => Some(field_id as usize - 1),
                _ => None,
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let local_geometry_indices = spec
            .features
            .iter()
            .enumerate()
            .filter_map(|(index, feature)| {
                (feature.name.starts_with("collision.") || feature.name.starts_with("surface."))
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        if missingness_mask_indices.is_empty() || local_geometry_indices.is_empty() {
            return Err(FixedRepresentationError::new(
                "movement-state/v2 lacks masks or local geometry probes",
            ));
        }
        let mut layout = Self {
            schema: FIXED_MODEL_REPRESENTATION_SCHEMA_V1.into(),
            observation_schema_sha256: spec
                .digest()
                .map_err(|error| FixedRepresentationError::new(error.to_string()))?,
            observation_width: spec.features.len(),
            continuous_indices,
            categorical_indices,
            missingness_mask_indices,
            local_geometry_indices,
            categorical_embedding_width,
            nearest_actor_slots,
            actor_slot_width: 14,
            objective_width: COMPILED_OBJECTIVE_VECTOR_WIDTH,
            layout_sha256: Digest::ZERO,
        };
        layout.layout_sha256 = layout.identity()?;
        layout.validate(&spec)?;
        Ok(layout)
    }

    pub fn validate(&self, spec: &ObservationSpec) -> Result<(), FixedRepresentationError> {
        spec.validate()
            .map_err(|error| FixedRepresentationError::new(error.to_string()))?;
        let all_indices = self
            .continuous_indices
            .iter()
            .chain(&self.categorical_indices)
            .copied()
            .collect::<BTreeSet<_>>();
        if self.schema != FIXED_MODEL_REPRESENTATION_SCHEMA_V1
            || self.observation_schema_sha256
                != spec
                    .digest()
                    .map_err(|error| FixedRepresentationError::new(error.to_string()))?
            || self.observation_width != spec.features.len()
            || all_indices.len() != self.observation_width
            || all_indices.iter().copied().ne(0..self.observation_width)
            || self.nearest_actor_slots == 0
            || self.nearest_actor_slots > MAX_NEAREST_ACTOR_SLOTS
            || self.categorical_embedding_width == 0
            || self.objective_width != COMPILED_OBJECTIVE_VECTOR_WIDTH
            || self.actor_slot_width != 14
            || self.layout_sha256 != self.identity()?
        {
            return Err(FixedRepresentationError::new(
                "fixed representation layout is invalid",
            ));
        }
        Ok(())
    }

    pub fn encode(
        &self,
        state: &[f32],
        objective: &CompiledObjectiveVector,
        entities: &EntityFactsEvidence,
    ) -> Result<FixedRepresentationInput, FixedRepresentationError> {
        let spec = movement_state_v2_spec();
        self.validate(&spec)?;
        objective
            .validate()
            .map_err(|error| FixedRepresentationError::new(error.to_string()))?;
        if state.len() != self.observation_width || state.iter().any(|value| !value.is_finite()) {
            return Err(FixedRepresentationError::new(
                "state does not match fixed representation layout",
            ));
        }
        let actor_source_available = match entities.availability {
            EvidenceAvailability::Present => true,
            EvidenceAvailability::Absent => false,
            _ => {
                return Err(FixedRepresentationError::new(
                    "selected actor facts are unavailable or incomplete",
                ));
            }
        };
        if !actor_source_available && (!entities.actors.is_empty() || entities.observed_count != 0)
        {
            return Err(FixedRepresentationError::new(
                "absent actor channel contains actors",
            ));
        }
        let player_position = [
            named_state(&spec, state, "player.position_x")? * 8192.0,
            named_state(&spec, state, "player.position_y")? * 8192.0,
            named_state(&spec, state, "player.position_z")? * 8192.0,
        ];
        let player_room = named_state(&spec, state, "stage.room")? as i8;
        let mut actors = entities.actors.iter().collect::<Vec<_>>();
        actors.sort_by(|left, right| actor_order(left, right, player_room, player_position));
        let actor_slots = actors
            .into_iter()
            .take(self.nearest_actor_slots)
            .map(|actor| actor_slot(actor, player_position))
            .chain(
                std::iter::repeat_with(empty_actor_slot).take(
                    self.nearest_actor_slots
                        .saturating_sub(entities.actors.len()),
                ),
            )
            .collect::<Vec<_>>();
        Ok(FixedRepresentationInput {
            layout_sha256: self.layout_sha256,
            continuous: self
                .continuous_indices
                .iter()
                .map(|index| state[*index])
                .collect(),
            categorical_embeddings: self
                .categorical_indices
                .iter()
                .map(|index| CategoricalEmbeddingInput {
                    feature_index: *index,
                    category_bits: state[*index].to_bits(),
                    embedding_width: self.categorical_embedding_width,
                })
                .collect(),
            missingness_masks: self
                .missingness_mask_indices
                .iter()
                .map(|index| state[*index])
                .collect(),
            local_geometry_probes: self
                .local_geometry_indices
                .iter()
                .map(|index| state[*index])
                .collect(),
            objective: objective.values.clone(),
            actor_source_available,
            actor_source_truncated: entities.truncated,
            actor_slots,
        })
    }

    fn identity(&self) -> Result<Digest, FixedRepresentationError> {
        let mut copy = self.clone();
        copy.layout_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&copy)
            .map_err(|error| FixedRepresentationError::new(error.to_string()))?;
        Ok(Digest(Sha256::digest(bytes).into()))
    }
}

fn named_state(
    spec: &ObservationSpec,
    state: &[f32],
    name: &str,
) -> Result<f32, FixedRepresentationError> {
    spec.features
        .iter()
        .position(|feature| feature.name == name)
        .map(|index| state[index])
        .ok_or_else(|| FixedRepresentationError::new(format!("missing feature {name}")))
}

fn actor_order(
    left: &EntityActorEvidence,
    right: &EntityActorEvidence,
    player_room: i8,
    player_position: [f32; 3],
) -> Ordering {
    (left.current_room != player_room)
        .cmp(&(right.current_room != player_room))
        .then_with(|| {
            distance_squared(left.position, player_position)
                .total_cmp(&distance_squared(right.position, player_position))
        })
        .then(left.actor_name.cmp(&right.actor_name))
        .then(left.set_id.cmp(&right.set_id))
        .then(left.session_process_id.cmp(&right.session_process_id))
}

fn distance_squared(left: [f32; 3], right: [f32; 3]) -> f32 {
    left.into_iter()
        .zip(right)
        .map(|(left, right)| (left - right).powi(2))
        .sum()
}

fn actor_slot(actor: &EntityActorEvidence, player_position: [f32; 3]) -> SemanticActorSlot {
    let relative_position = [
        (actor.position[0] - player_position[0]) / 8192.0,
        (actor.position[1] - player_position[1]) / 8192.0,
        (actor.position[2] - player_position[2]) / 8192.0,
    ];
    SemanticActorSlot {
        present: 1.0,
        actor_name: actor.actor_name,
        set_id: actor.set_id,
        current_room: actor.current_room,
        health_normalized: f32::from(actor.health) / f32::from(i16::MAX),
        status: actor.status,
        relative_position,
        distance_normalized: relative_position
            .into_iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt(),
        current_yaw_sin_cos: angle_sin_cos(actor.current_angle[1]),
        shape_yaw_sin_cos: angle_sin_cos(actor.shape_angle[1]),
    }
}

fn angle_sin_cos(angle: i16) -> [f32; 2] {
    let radians = f32::from(angle) * PI / 32768.0;
    [radians.sin(), radians.cos()]
}

fn empty_actor_slot() -> SemanticActorSlot {
    SemanticActorSlot {
        present: 0.0,
        actor_name: 0,
        set_id: 0,
        current_room: 0,
        health_normalized: 0.0,
        status: 0,
        relative_position: [0.0; 3],
        distance_normalized: 0.0,
        current_yaw_sin_cos: [0.0; 2],
        shape_yaw_sin_cos: [0.0; 2],
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixedRepresentationError(String);

impl FixedRepresentationError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for FixedRepresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for FixedRepresentationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_dsl::compile_source;

    const OBJECTIVE: &str = r#"milestones 1.0
milestone target_room {
  phase post_sim
  when stage.room == 1
}
"#;

    fn actor(id: u32, name: i16, room: i8, position: [f32; 3]) -> EntityActorEvidence {
        EntityActorEvidence {
            session_process_id: id,
            actor_name: name,
            set_id: id as u16,
            home_room: room,
            current_room: room,
            health: 10,
            status: 3,
            position,
            current_angle: [0, 0, 0],
            shape_angle: [0, 16_384, 0],
        }
    }

    #[test]
    fn fixed_layout_retains_masks_categories_objective_geometry_and_actor_slots() {
        let layout = FixedRepresentationLayout::movement_v2(2, 8).unwrap();
        let spec = movement_state_v2_spec();
        let mut state = vec![0.0; spec.features.len()];
        let set = |state: &mut [f32], name: &str, value| {
            let index = spec
                .features
                .iter()
                .position(|feature| feature.name == name)
                .unwrap();
            state[index] = value;
        };
        set(&mut state, "stage.room", 1.0);
        set(&mut state, "player.position_x", 100.0 / 8192.0);
        set(&mut state, "player.position_y", 0.0);
        set(&mut state, "player.position_z", 0.0);
        let objective =
            CompiledObjectiveVector::from_compiled(&compile_source(OBJECTIVE).unwrap(), 0).unwrap();
        let entities = EntityFactsEvidence {
            availability: EvidenceAvailability::Present,
            observed_count: 3,
            truncated: false,
            actors: vec![
                actor(3, 30, 2, [101.0, 0.0, 0.0]),
                actor(2, 20, 1, [300.0, 0.0, 0.0]),
                actor(1, 10, 1, [110.0, 0.0, 0.0]),
            ],
        };
        let encoded = layout.encode(&state, &objective, &entities).unwrap();
        assert_eq!(encoded.layout_sha256, layout.layout_sha256);
        assert_eq!(encoded.objective, objective.values);
        assert_eq!(encoded.actor_slots.len(), 2);
        assert_eq!(encoded.actor_slots[0].actor_name, 10);
        assert_eq!(encoded.actor_slots[1].actor_name, 20);
        assert!(encoded.actor_slots.iter().all(|slot| slot.present == 1.0));
        assert_eq!(
            encoded.categorical_embeddings.len(),
            layout.categorical_indices.len()
        );
        assert_eq!(
            encoded.missingness_masks.len(),
            layout.missingness_mask_indices.len()
        );
        assert_eq!(
            encoded.local_geometry_probes.len(),
            layout.local_geometry_indices.len()
        );
        assert_ne!(layout.layout_sha256, Digest::ZERO);
    }

    #[test]
    fn actor_absence_is_masked_and_unavailable_actor_facts_fail_closed() {
        let layout = FixedRepresentationLayout::movement_v2(3, 4).unwrap();
        let spec = movement_state_v2_spec();
        let state = vec![0.0; spec.features.len()];
        let objective =
            CompiledObjectiveVector::from_compiled(&compile_source(OBJECTIVE).unwrap(), 0).unwrap();
        let absent = EntityFactsEvidence {
            availability: EvidenceAvailability::Absent,
            observed_count: 0,
            truncated: false,
            actors: Vec::new(),
        };
        let encoded = layout.encode(&state, &objective, &absent).unwrap();
        assert!(!encoded.actor_source_available);
        assert!(encoded.actor_slots.iter().all(|slot| slot.present == 0.0));

        let mut unavailable = absent;
        unavailable.availability = EvidenceAvailability::Unavailable;
        assert!(layout.encode(&state, &objective, &unavailable).is_err());
    }
}
