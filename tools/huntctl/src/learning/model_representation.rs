//! Fixed, masked model inputs for the first serious representation baseline.

use super::dataset::NormalizationStatistics;
use super::goal_conditioning::CompiledObjectiveVector;
use crate::artifact::Digest;
use crate::observation_view::{MissingnessPolicy, ObservationSpec};
use crate::trace::TraceSelectedActors;
use crate::world_spatial::WorldPointQueryReport;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::f32::consts::PI;
use std::fmt;

pub const FIXED_MODEL_REPRESENTATION_SCHEMA_V1: &str = "dusklight-fixed-model-representation/v1";
pub const DEFAULT_CATEGORICAL_EMBEDDING_WIDTH: usize = 4;
pub const DEFAULT_ACTOR_SLOTS: usize = 4;
pub const DEFAULT_GEOMETRY_SLOTS: usize = 4;
const MAX_CATEGORIES_PER_FIELD: usize = 4096;
const ACTOR_SLOT_WIDTH: usize = 17;
const GEOMETRY_SLOT_WIDTH: usize = 14;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LocalGeometryProbe {
    pub stable_id: String,
    pub attribute: u16,
    pub closest_point: [f32; 3],
    pub distance: f32,
    pub signed_plane_distance: f32,
    pub normal: [f32; 3],
    pub load_trigger: bool,
}

impl LocalGeometryProbe {
    pub fn from_point_report(
        report: &WorldPointQueryReport,
    ) -> Result<Vec<Self>, ModelRepresentationError> {
        report
            .results
            .iter()
            .map(|hit| {
                let probe = Self {
                    stable_id: hit.surface.authored.stable_id.clone(),
                    attribute: hit.surface.authored.attribute,
                    closest_point: [
                        hit.point_query.closest_point.x,
                        hit.point_query.closest_point.y,
                        hit.point_query.closest_point.z,
                    ],
                    distance: hit.point_query.distance as f32,
                    signed_plane_distance: hit.point_query.signed_plane_distance as f32,
                    normal: [
                        hit.surface.plane.normal.x,
                        hit.surface.plane.normal.y,
                        hit.surface.plane.normal.z,
                    ],
                    load_trigger: hit.surface.load_trigger.is_some(),
                };
                probe.validate()?;
                Ok(probe)
            })
            .collect()
    }

    fn validate(&self) -> Result<(), ModelRepresentationError> {
        if self.stable_id.is_empty()
            || self
                .closest_point
                .iter()
                .chain(&self.normal)
                .chain([&self.distance, &self.signed_plane_distance])
                .any(|value| !value.is_finite())
            || self.distance < 0.0
        {
            return Err(ModelRepresentationError::InvalidGeometry);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct RepresentationContext<'a> {
    pub state: &'a [f32],
    pub objective: &'a CompiledObjectiveVector,
    pub player_position: [f32; 3],
    pub player_yaw: i16,
    pub player_session_process_id: Option<u32>,
    pub selected_actors: Option<&'a TraceSelectedActors>,
    pub geometry: Option<&'a [LocalGeometryProbe]>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FixedModelInput {
    pub schema_sha256: Digest,
    pub values: Vec<f32>,
    pub missingness_mask: Vec<f32>,
}

impl FixedModelInput {
    pub fn concatenated(&self) -> Vec<f32> {
        self.values
            .iter()
            .chain(&self.missingness_mask)
            .copied()
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct CategoricalEmbeddingTable {
    field_id: u32,
    categories: BTreeMap<u32, Vec<f32>>,
    unknown: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct BaseFeatureLayout {
    field_id: u32,
    categorical: bool,
    mask_index: Option<usize>,
    mean: f32,
    inverse_standard_deviation: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FixedModelRepresentationEncoder {
    schema: &'static str,
    observation_schema_sha256: Digest,
    base_features: Vec<BaseFeatureLayout>,
    categorical_embeddings: Vec<CategoricalEmbeddingTable>,
    categorical_embedding_width: usize,
    objective_width: usize,
    actor_slots: usize,
    actor_slot_width: usize,
    geometry_slots: usize,
    geometry_slot_width: usize,
    value_width: usize,
    representation_sha256: Digest,
}

impl FixedModelRepresentationEncoder {
    pub fn fit(
        spec: &ObservationSpec,
        normalization: &NormalizationStatistics,
        training_states: &[Vec<f32>],
    ) -> Result<Self, ModelRepresentationError> {
        spec.validate()
            .map_err(|error| ModelRepresentationError::InvalidSpec(error.to_string()))?;
        let observation_schema_sha256 = spec
            .digest()
            .map_err(|error| ModelRepresentationError::InvalidSpec(error.to_string()))?;
        let width = spec.features.len();
        if normalization.schema != "dusklight-normalization/v1"
            || normalization.feature_schema_sha256 != observation_schema_sha256
            || normalization.sample_count == 0
            || normalization.means.len() != width
            || normalization.standard_deviations.len() != width
            || training_states.is_empty()
            || normalization.sample_count != training_states.len() as u64
            || training_states
                .iter()
                .any(|state| state.len() != width || state.iter().any(|value| !value.is_finite()))
        {
            return Err(ModelRepresentationError::InvalidNormalization);
        }
        let id_to_index = spec
            .features
            .iter()
            .enumerate()
            .map(|(index, feature)| (feature.field_id, index))
            .collect::<BTreeMap<_, _>>();
        let mut base_features = Vec::with_capacity(width);
        let mut categorical_embeddings = Vec::new();
        for (index, feature) in spec.features.iter().enumerate() {
            let mask_index = match feature.missingness {
                MissingnessPolicy::Required => None,
                MissingnessPolicy::MaskedBy { field_id } => Some(id_to_index[&field_id]),
            };
            let standard_deviation = normalization.standard_deviations[index];
            let inverse_standard_deviation =
                if standard_deviation.is_finite() && standard_deviation > f64::EPSILON {
                    (1.0 / standard_deviation) as f32
                } else {
                    1.0
                };
            let mean = normalization.means[index] as f32;
            if !mean.is_finite() || !inverse_standard_deviation.is_finite() {
                return Err(ModelRepresentationError::InvalidNormalization);
            }
            base_features.push(BaseFeatureLayout {
                field_id: feature.field_id,
                categorical: feature.categorical,
                mask_index,
                mean,
                inverse_standard_deviation,
            });
            if feature.categorical {
                let categories = training_states
                    .iter()
                    .filter(|state| mask_index.is_none_or(|mask| state[mask] > 0.5))
                    .map(|state| state[index].to_bits())
                    .collect::<BTreeSet<_>>();
                if categories.len() > MAX_CATEGORIES_PER_FIELD {
                    return Err(ModelRepresentationError::TooManyCategories {
                        field_id: feature.field_id,
                        count: categories.len(),
                    });
                }
                categorical_embeddings.push(CategoricalEmbeddingTable {
                    field_id: feature.field_id,
                    categories: categories
                        .into_iter()
                        .map(|category| {
                            (
                                category,
                                token_embedding(
                                    b"base-category",
                                    feature.field_id,
                                    &category.to_le_bytes(),
                                    DEFAULT_CATEGORICAL_EMBEDDING_WIDTH,
                                ),
                            )
                        })
                        .collect(),
                    unknown: token_embedding(
                        b"base-category-unknown",
                        feature.field_id,
                        &[],
                        DEFAULT_CATEGORICAL_EMBEDDING_WIDTH,
                    ),
                });
            }
        }
        let base_value_width = base_features
            .iter()
            .map(|feature| {
                if feature.categorical {
                    DEFAULT_CATEGORICAL_EMBEDDING_WIDTH
                } else {
                    1
                }
            })
            .sum::<usize>();
        let value_width = base_value_width
            + super::goal_conditioning::COMPILED_OBJECTIVE_VECTOR_WIDTH
            + 3
            + DEFAULT_ACTOR_SLOTS * ACTOR_SLOT_WIDTH
            + 3
            + DEFAULT_GEOMETRY_SLOTS * GEOMETRY_SLOT_WIDTH;
        let mut encoder = Self {
            schema: FIXED_MODEL_REPRESENTATION_SCHEMA_V1,
            observation_schema_sha256,
            base_features,
            categorical_embeddings,
            categorical_embedding_width: DEFAULT_CATEGORICAL_EMBEDDING_WIDTH,
            objective_width: super::goal_conditioning::COMPILED_OBJECTIVE_VECTOR_WIDTH,
            actor_slots: DEFAULT_ACTOR_SLOTS,
            actor_slot_width: ACTOR_SLOT_WIDTH,
            geometry_slots: DEFAULT_GEOMETRY_SLOTS,
            geometry_slot_width: GEOMETRY_SLOT_WIDTH,
            value_width,
            representation_sha256: Digest::ZERO,
        };
        encoder.representation_sha256 = encoder.digest()?;
        Ok(encoder)
    }

    pub fn encode(
        &self,
        context: RepresentationContext<'_>,
    ) -> Result<FixedModelInput, ModelRepresentationError> {
        if self.representation_sha256 != self.digest()?
            || context.state.len() != self.base_features.len()
        {
            return Err(ModelRepresentationError::InvalidEncoder);
        }
        if context.state.iter().any(|value| !value.is_finite())
            || context
                .player_position
                .iter()
                .any(|value| !value.is_finite())
        {
            return Err(ModelRepresentationError::NonFiniteInput);
        }
        context
            .objective
            .validate()
            .map_err(|error| ModelRepresentationError::InvalidObjective(error.to_string()))?;
        let tables = self
            .categorical_embeddings
            .iter()
            .map(|table| (table.field_id, table))
            .collect::<BTreeMap<_, _>>();
        let mut values = Vec::with_capacity(self.value_width);
        let mut missingness_mask = Vec::with_capacity(self.value_width);
        for (index, layout) in self.base_features.iter().enumerate() {
            let present = layout
                .mask_index
                .is_none_or(|mask_index| context.state[mask_index] > 0.5);
            if layout.categorical {
                let table = tables[&layout.field_id];
                let embedding = table
                    .categories
                    .get(&context.state[index].to_bits())
                    .unwrap_or(&table.unknown);
                append_masked(&mut values, &mut missingness_mask, embedding, present);
            } else {
                let normalized =
                    (context.state[index] - layout.mean) * layout.inverse_standard_deviation;
                append_masked(&mut values, &mut missingness_mask, &[normalized], present);
            }
        }
        append_masked(
            &mut values,
            &mut missingness_mask,
            &context.objective.values,
            true,
        );
        self.append_actor_slots(&mut values, &mut missingness_mask, &context)?;
        self.append_geometry_slots(&mut values, &mut missingness_mask, &context)?;
        if values.len() != self.value_width
            || missingness_mask.len() != self.value_width
            || values.iter().any(|value| !value.is_finite())
        {
            return Err(ModelRepresentationError::InvalidEncoder);
        }
        Ok(FixedModelInput {
            schema_sha256: self.representation_sha256,
            values,
            missingness_mask,
        })
    }

    pub fn value_width(&self) -> usize {
        self.value_width
    }

    pub fn model_input_width(&self) -> usize {
        self.value_width * 2
    }

    pub fn schema_sha256(&self) -> Digest {
        self.representation_sha256
    }

    fn append_actor_slots(
        &self,
        values: &mut Vec<f32>,
        masks: &mut Vec<f32>,
        context: &RepresentationContext<'_>,
    ) -> Result<(), ModelRepresentationError> {
        let (channel_present, truncated, observed_count) =
            context.selected_actors.map_or((false, false, 0), |actors| {
                (true, actors.truncated, actors.observed_count)
            });
        append_masked(
            values,
            masks,
            &[
                f32::from(channel_present),
                f32::from(truncated),
                observed_count.min(1024) as f32 / 1024.0,
            ],
            true,
        );
        let mut actors = context
            .selected_actors
            .into_iter()
            .flat_map(|selected| &selected.actors)
            .filter(|actor| Some(actor.session_process_id) != context.player_session_process_id)
            .collect::<Vec<_>>();
        actors.sort_by(|left, right| {
            squared_distance(left.position, context.player_position)
                .total_cmp(&squared_distance(right.position, context.player_position))
                .then_with(|| left.actor_name.cmp(&right.actor_name))
                .then_with(|| left.session_process_id.cmp(&right.session_process_id))
        });
        for slot in 0..self.actor_slots {
            let Some(actor) = actors.get(slot) else {
                append_masked(values, masks, &vec![0.0; self.actor_slot_width], false);
                continue;
            };
            if actor.position.iter().any(|value| !value.is_finite()) {
                return Err(ModelRepresentationError::InvalidActors);
            }
            let relative =
                relative_local(actor.position, context.player_position, context.player_yaw);
            let distance = squared_distance(actor.position, context.player_position).sqrt();
            let mut encoded = vec![1.0];
            encoded.extend(token_embedding(
                b"actor-name",
                0,
                &actor.actor_name.to_le_bytes(),
                self.categorical_embedding_width,
            ));
            encoded.extend(token_embedding(
                b"actor-room",
                0,
                &actor.current_room.to_le_bytes(),
                self.categorical_embedding_width,
            ));
            encoded.extend(relative.map(|value| value / 8192.0));
            encoded.push(distance / 8192.0);
            encoded.push(f32::from(actor.health) / 32768.0);
            encoded.push(actor.status as f32 / u32::MAX as f32);
            let yaw = yaw_radians(actor.current_angle[1]);
            encoded.extend([yaw.sin(), yaw.cos()]);
            debug_assert_eq!(encoded.len(), self.actor_slot_width);
            append_masked(values, masks, &encoded, true);
        }
        Ok(())
    }

    fn append_geometry_slots(
        &self,
        values: &mut Vec<f32>,
        masks: &mut Vec<f32>,
        context: &RepresentationContext<'_>,
    ) -> Result<(), ModelRepresentationError> {
        let probes = context.geometry.unwrap_or_default();
        for probe in probes {
            probe.validate()?;
        }
        append_masked(
            values,
            masks,
            &[
                f32::from(context.geometry.is_some()),
                f32::from(probes.len() > self.geometry_slots),
                probes.len().min(1024) as f32 / 1024.0,
            ],
            true,
        );
        let mut probes = probes.iter().collect::<Vec<_>>();
        probes.sort_by(|left, right| {
            left.distance
                .total_cmp(&right.distance)
                .then_with(|| left.stable_id.cmp(&right.stable_id))
        });
        for slot in 0..self.geometry_slots {
            let Some(probe) = probes.get(slot) else {
                append_masked(values, masks, &vec![0.0; self.geometry_slot_width], false);
                continue;
            };
            let relative = relative_local(
                probe.closest_point,
                context.player_position,
                context.player_yaw,
            );
            let mut encoded = vec![1.0];
            encoded.extend(token_embedding(
                b"surface-id",
                u32::from(probe.attribute),
                probe.stable_id.as_bytes(),
                self.categorical_embedding_width,
            ));
            encoded.extend(relative.map(|value| value / 8192.0));
            encoded.push(probe.distance / 8192.0);
            encoded.push(probe.signed_plane_distance / 8192.0);
            encoded.extend(probe.normal);
            encoded.push(f32::from(probe.load_trigger));
            debug_assert_eq!(encoded.len(), self.geometry_slot_width);
            append_masked(values, masks, &encoded, true);
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, ModelRepresentationError> {
        let bytes = serde_json::to_vec(&(
            self.schema,
            self.observation_schema_sha256,
            &self.base_features,
            &self.categorical_embeddings,
            self.categorical_embedding_width,
            self.objective_width,
            self.actor_slots,
            self.actor_slot_width,
            self.geometry_slots,
            self.geometry_slot_width,
            self.value_width,
        ))
        .map_err(|error| ModelRepresentationError::Serialization(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.fixed-model-representation/v1\0");
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn append_masked(values: &mut Vec<f32>, masks: &mut Vec<f32>, input: &[f32], present: bool) {
    values.extend(input.iter().map(|value| if present { *value } else { 0.0 }));
    masks.extend(std::iter::repeat_n(f32::from(present), input.len()));
}

fn token_embedding(domain: &[u8], field_id: u32, token: &[u8], width: usize) -> Vec<f32> {
    (0..width)
        .map(|dimension| {
            let mut hasher = Sha256::new();
            hasher.update(b"dusklight.categorical-embedding-init/v1\0");
            hasher.update((domain.len() as u64).to_le_bytes());
            hasher.update(domain);
            hasher.update(field_id.to_le_bytes());
            hasher.update((token.len() as u64).to_le_bytes());
            hasher.update(token);
            hasher.update((dimension as u64).to_le_bytes());
            let digest = hasher.finalize();
            let raw = u32::from_le_bytes(digest[..4].try_into().expect("four bytes"));
            (raw as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect()
}

fn squared_distance(left: [f32; 3], right: [f32; 3]) -> f32 {
    left.into_iter()
        .zip(right)
        .map(|(left, right)| (left - right).powi(2))
        .sum()
}

fn relative_local(target: [f32; 3], origin: [f32; 3], yaw: i16) -> [f32; 3] {
    let delta = [
        target[0] - origin[0],
        target[1] - origin[1],
        target[2] - origin[2],
    ];
    let yaw = yaw_radians(yaw);
    let (sin, cos) = yaw.sin_cos();
    [
        cos * delta[0] - sin * delta[2],
        delta[1],
        sin * delta[0] + cos * delta[2],
    ]
}

fn yaw_radians(yaw: i16) -> f32 {
    f32::from(yaw) * PI / 32768.0
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelRepresentationError {
    InvalidSpec(String),
    InvalidNormalization,
    TooManyCategories { field_id: u32, count: usize },
    InvalidEncoder,
    InvalidObjective(String),
    NonFiniteInput,
    InvalidActors,
    InvalidGeometry,
    Serialization(String),
}

impl fmt::Display for ModelRepresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "model representation rejected: {self:?}")
    }
}

impl Error for ModelRepresentationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::goal_conditioning::CompiledObjectiveVector;
    use crate::milestone_dsl::compile_source;
    use crate::observation_view::movement_state_v2_spec;
    use crate::trace::TraceSelectedActor;

    fn objective() -> CompiledObjectiveVector {
        let compiled = compile_source(
            r#"milestones 1.0
milestone target_room {
  phase post_sim
  when stage.room == 1
}
"#,
        )
        .unwrap();
        CompiledObjectiveVector::from_compiled(&compiled, 0).unwrap()
    }

    fn actor(process: u32, name: i16, x: f32) -> TraceSelectedActor {
        TraceSelectedActor {
            session_process_id: process,
            actor_name: name,
            set_id: process as u16,
            home_room: 1,
            current_room: 1,
            health: 10,
            status: 3,
            position: [x, 0.0, 0.0],
            current_angle: [0, 0, 0],
            shape_angle: [0, 0, 0],
        }
    }

    fn probe(id: &str, x: f32) -> LocalGeometryProbe {
        LocalGeometryProbe {
            stable_id: id.into(),
            attribute: 2,
            closest_point: [x, 0.0, 0.0],
            distance: x.abs(),
            signed_plane_distance: x,
            normal: [0.0, 1.0, 0.0],
            load_trigger: id == "near",
        }
    }

    fn fixture() -> (
        FixedModelRepresentationEncoder,
        Vec<f32>,
        CompiledObjectiveVector,
    ) {
        let spec = movement_state_v2_spec();
        let width = spec.features.len();
        let states = vec![vec![0.0; width], vec![0.0; width]];
        let normalization = NormalizationStatistics {
            schema: "dusklight-normalization/v1".into(),
            feature_schema_sha256: spec.digest().unwrap(),
            training_episode_sha256: vec![Digest([9; 32])],
            sample_count: 2,
            means: vec![0.0; width],
            standard_deviations: vec![1.0; width],
        };
        (
            FixedModelRepresentationEncoder::fit(&spec, &normalization, &states).unwrap(),
            states[0].clone(),
            objective(),
        )
    }

    #[test]
    fn fixed_input_has_parallel_missingness_and_goal_conditioning() {
        let (encoder, mut state, objective) = fixture();
        // player.procedure is masked by player.procedure_present (field 18).
        state[17] = 0.0;
        state[18] = 42.0;
        let input = encoder
            .encode(RepresentationContext {
                state: &state,
                objective: &objective,
                player_position: [0.0; 3],
                player_yaw: 0,
                player_session_process_id: None,
                selected_actors: None,
                geometry: None,
            })
            .unwrap();
        assert_eq!(input.values.len(), encoder.value_width());
        assert_eq!(input.missingness_mask.len(), encoder.value_width());
        assert_eq!(input.concatenated().len(), encoder.model_input_width());
        assert_ne!(input.schema_sha256, Digest::ZERO);

        let procedure_offset = encoder
            .base_features
            .iter()
            .take(18)
            .map(|feature| {
                if feature.categorical {
                    encoder.categorical_embedding_width
                } else {
                    1
                }
            })
            .sum::<usize>();
        assert_eq!(
            &input.values[procedure_offset..procedure_offset + encoder.categorical_embedding_width],
            &[0.0; DEFAULT_CATEGORICAL_EMBEDDING_WIDTH]
        );
        assert_eq!(
            &input.missingness_mask
                [procedure_offset..procedure_offset + encoder.categorical_embedding_width],
            &[0.0; DEFAULT_CATEGORICAL_EMBEDDING_WIDTH]
        );

        let mut changed = objective.clone();
        changed.values[40] += 0.25;
        changed.vector_sha256 = {
            // Invalid objective vectors fail closed rather than becoming detached labels.
            Digest::ZERO
        };
        assert!(
            encoder
                .encode(RepresentationContext {
                    objective: &changed,
                    state: &state,
                    player_position: [0.0; 3],
                    player_yaw: 0,
                    player_session_process_id: None,
                    selected_actors: None,
                    geometry: None,
                })
                .is_err()
        );
    }

    #[test]
    fn normalization_must_describe_the_exact_training_state_count() {
        let spec = movement_state_v2_spec();
        let width = spec.features.len();
        let states = vec![vec![0.0; width], vec![0.0; width]];
        let detached = NormalizationStatistics {
            schema: "dusklight-normalization/v1".into(),
            feature_schema_sha256: spec.digest().unwrap(),
            training_episode_sha256: vec![Digest([9; 32])],
            sample_count: 3,
            means: vec![0.0; width],
            standard_deviations: vec![1.0; width],
        };
        assert_eq!(
            FixedModelRepresentationEncoder::fit(&spec, &detached, &states),
            Err(ModelRepresentationError::InvalidNormalization)
        );
    }

    #[test]
    fn actor_and_geometry_slots_are_nearest_k_and_order_invariant() {
        let (encoder, state, objective) = fixture();
        let actors = TraceSelectedActors {
            observed_count: 3,
            truncated: false,
            actors: vec![actor(10, 1, 0.0), actor(20, 2, 80.0), actor(30, 3, 8.0)],
        };
        let reversed = TraceSelectedActors {
            actors: actors.actors.iter().cloned().rev().collect(),
            ..actors.clone()
        };
        let geometry = vec![probe("far", 90.0), probe("near", 9.0)];
        let reversed_geometry = geometry.iter().cloned().rev().collect::<Vec<_>>();
        let encode = |actors: &TraceSelectedActors, geometry: &[LocalGeometryProbe]| {
            encoder
                .encode(RepresentationContext {
                    state: &state,
                    objective: &objective,
                    player_position: [0.0; 3],
                    player_yaw: 0,
                    player_session_process_id: Some(10),
                    selected_actors: Some(actors),
                    geometry: Some(geometry),
                })
                .unwrap()
        };
        let first = encode(&actors, &geometry);
        let second = encode(&reversed, &reversed_geometry);
        assert_eq!(first, second);

        let base_width = encoder
            .base_features
            .iter()
            .map(|feature| {
                if feature.categorical {
                    encoder.categorical_embedding_width
                } else {
                    1
                }
            })
            .sum::<usize>();
        let actor_start = base_width + encoder.objective_width + 3;
        assert!((first.values[actor_start + 9] - 8.0 / 8192.0).abs() < f32::EPSILON);
        let geometry_start = actor_start + encoder.actor_slots * encoder.actor_slot_width + 3;
        assert!((first.values[geometry_start + 5] - 9.0 / 8192.0).abs() < f32::EPSILON);
        assert_eq!(first.values[geometry_start + 13], 1.0);
        assert!(
            first.missingness_mask[actor_start + 2 * encoder.actor_slot_width
                ..actor_start + 3 * encoder.actor_slot_width]
                .iter()
                .all(|mask| *mask == 0.0)
        );
    }
}
