//! Typed dynamic-collision sets derived from authenticated native episodes.
//!
//! The view never calls collision code. It retains every collider captured by
//! the preceding native collision pass and resolves owner/hit-partner IDs into
//! explicit edges to the complete actor population when possible.

use crate::artifact::Digest;
use crate::native_actor_view::ActorViewObservationPhase;
use dusklight_evidence::native_episode_shard::{
    NativeChannelStatus, NativeDynamicColliderObservation, NativeDynamicColliderShape,
    NativeEpisodeShard, NativeLearningObservation, NativeObservationPhase,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::f32::consts::PI;
use std::fmt;

pub const NATIVE_COLLISION_VIEW_SCHEMA_V1: &str = "dusklight-native-collision-view/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CollisionSetStatus {
    NotSampled,
    Unavailable,
    Absent,
    Present,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicColliderShape {
    Unknown,
    Sphere,
    Cylinder,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionActorEdge {
    /// `None` means the native collider exposed no owner. `Some` plus a missing
    /// actor index means the reference was real but outside this actor set.
    pub runtime_generation: Option<u32>,
    pub actor_index: Option<u16>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicColliderStatusFeature {
    pub attack_set: bool,
    pub target_set: bool,
    pub correction_set: bool,
    pub attack_hit: bool,
    pub target_hit: bool,
    pub correction_hit: bool,
    pub attack_type: u32,
    pub target_type: u32,
    pub attack_source_parameters: u32,
    pub attack_result_parameters: u32,
    pub target_source_parameters: u32,
    pub target_result_parameters: u32,
    pub correction_source_parameters: u32,
    pub correction_result_parameters: u32,
    pub attack_power: u8,
    pub weight: u8,
    pub damage: u8,
    pub correction: [f32; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicColliderShapeFeature {
    pub kind: DynamicColliderShape,
    pub center: [f32; 3],
    pub radius: f32,
    pub height: f32,
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
    pub link_relative_center: Option<[f32; 3]>,
    pub owner_relative_center: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeDynamicColliderFeature {
    pub registration_index: u16,
    pub owner: CollisionActorEdge,
    pub attack_hit_owner: CollisionActorEdge,
    pub target_hit_owner: CollisionActorEdge,
    pub correction_hit_owner: CollisionActorEdge,
    pub status_present: bool,
    pub status: DynamicColliderStatusFeature,
    pub shape: Option<DynamicColliderShapeFeature>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCollisionObservation {
    pub episode_id: String,
    pub step_index: u32,
    pub phase: ActorViewObservationPhase,
    pub boundary_index: u64,
    pub state_identity_xxh3_128: String,
    pub stage: String,
    pub room: i8,
    pub player_present: bool,
    /// Exact actor generations used to validate collider graph edges. The
    /// actor payload itself remains in the independently sealed actor view.
    pub actor_runtime_generations: Vec<u64>,
    pub status: CollisionSetStatus,
    pub colliders: Vec<NativeDynamicColliderFeature>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEpisodeCollisionView {
    pub schema: String,
    pub native_shard_sha256: Digest,
    pub observation_schema: String,
    pub observations: Vec<NativeCollisionObservation>,
    pub view_sha256: Digest,
}

impl NativeEpisodeCollisionView {
    pub fn build(shard: &NativeEpisodeShard) -> Result<Self, NativeCollisionViewError> {
        if shard.content_sha256 == Digest::ZERO || shard.episodes.is_empty() {
            return Err(NativeCollisionViewError::new(
                "native collision view requires an authenticated nonempty shard",
            ));
        }
        let mut observations = Vec::new();
        for episode in &shard.episodes {
            for (step_index, step) in episode.steps.iter().enumerate() {
                let step_index = u32::try_from(step_index)
                    .map_err(|_| NativeCollisionViewError::new("step index overflowed"))?;
                observations.push(materialize_observation(
                    &episode.id,
                    step_index,
                    &step.pre_input,
                )?);
                observations.push(materialize_observation(
                    &episode.id,
                    step_index,
                    &step.post_simulation,
                )?);
            }
        }
        let mut view = Self {
            schema: NATIVE_COLLISION_VIEW_SCHEMA_V1.into(),
            native_shard_sha256: shard.content_sha256,
            observation_schema: shard.metadata.observation_schema.clone(),
            observations,
            view_sha256: Digest::ZERO,
        };
        view.view_sha256 = view.compute_identity()?;
        view.validate()?;
        Ok(view)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, NativeCollisionViewError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| NativeCollisionViewError::new(error.to_string()))
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, NativeCollisionViewError> {
        let view: Self = serde_json::from_slice(bytes)
            .map_err(|error| NativeCollisionViewError::new(error.to_string()))?;
        view.validate()?;
        if view.canonical_bytes()? != bytes {
            return Err(NativeCollisionViewError::new(
                "native collision view bytes are not canonical",
            ));
        }
        Ok(view)
    }

    pub fn validate(&self) -> Result<(), NativeCollisionViewError> {
        if self.schema != NATIVE_COLLISION_VIEW_SCHEMA_V1
            || self.native_shard_sha256 == Digest::ZERO
            || self.observation_schema.is_empty()
            || self.observations.is_empty()
            || self.view_sha256 != self.compute_identity()?
        {
            return Err(NativeCollisionViewError::new(
                "native collision view envelope or seal is invalid",
            ));
        }
        for observation in &self.observations {
            validate_observation(observation)?;
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, NativeCollisionViewError> {
        let mut canonical = self.clone();
        canonical.view_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| NativeCollisionViewError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-collision-view/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn materialize_observation(
    episode_id: &str,
    step_index: u32,
    source: &NativeLearningObservation,
) -> Result<NativeCollisionObservation, NativeCollisionViewError> {
    let phase = match source.phase {
        NativeObservationPhase::PreInput => ActorViewObservationPhase::PreInput,
        NativeObservationPhase::PostSimulation => ActorViewObservationPhase::PostSimulation,
    };
    let status = match source.dynamic_colliders_status {
        NativeChannelStatus::NotSampled => CollisionSetStatus::NotSampled,
        NativeChannelStatus::Unavailable => CollisionSetStatus::Unavailable,
        NativeChannelStatus::Absent => CollisionSetStatus::Absent,
        NativeChannelStatus::Present => CollisionSetStatus::Present,
    };
    let actor_runtime_generations = source
        .actors
        .iter()
        .map(|actor| actor.runtime_generation)
        .collect::<Vec<_>>();
    if actor_runtime_generations.len() > usize::from(u16::MAX) {
        return Err(NativeCollisionViewError::new(
            "actor population exceeds collision-edge index width",
        ));
    }
    let colliders = source
        .dynamic_colliders
        .iter()
        .map(|collider| materialize_collider(collider, source, &actor_runtime_generations))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(NativeCollisionObservation {
        episode_id: episode_id.into(),
        step_index,
        phase,
        boundary_index: source.boundary_index,
        state_identity_xxh3_128: lower_hex(&source.state_identity),
        stage: source.stage.clone(),
        room: source.room,
        player_present: source.player_present,
        actor_runtime_generations,
        status,
        colliders,
    })
}

fn materialize_collider(
    collider: &NativeDynamicColliderObservation,
    observation: &NativeLearningObservation,
    actor_generations: &[u64],
) -> Result<NativeDynamicColliderFeature, NativeCollisionViewError> {
    let owner = actor_edge(collider.owner_runtime_generation, actor_generations)?;
    let owner_position = owner
        .actor_index
        .map(|index| observation.actors[usize::from(index)].position);
    let status = DynamicColliderStatusFeature {
        attack_set: collider.attack_set,
        target_set: collider.target_set,
        correction_set: collider.correction_set,
        attack_hit: collider.attack_hit,
        target_hit: collider.target_hit,
        correction_hit: collider.correction_hit,
        attack_type: collider.attack_type,
        target_type: collider.target_type,
        attack_source_parameters: collider.attack_source_parameters,
        attack_result_parameters: collider.attack_result_parameters,
        target_source_parameters: collider.target_source_parameters,
        target_result_parameters: collider.target_result_parameters,
        correction_source_parameters: collider.correction_source_parameters,
        correction_result_parameters: collider.correction_result_parameters,
        attack_power: collider.attack_power,
        weight: collider.weight,
        damage: collider.damage,
        correction: collider.correction,
    };
    let shape = collider
        .shape_present
        .then_some(DynamicColliderShapeFeature {
            kind: match collider.shape {
                NativeDynamicColliderShape::Unknown => DynamicColliderShape::Unknown,
                NativeDynamicColliderShape::Sphere => DynamicColliderShape::Sphere,
                NativeDynamicColliderShape::Cylinder => DynamicColliderShape::Cylinder,
            },
            center: collider.center,
            radius: collider.radius,
            height: collider.height,
            aabb_min: collider.aabb_min,
            aabb_max: collider.aabb_max,
            link_relative_center: observation.player_present.then_some({
                relative_yaw(
                    collider.center,
                    observation.player_position,
                    observation.player_shape_angle[1],
                )
            }),
            owner_relative_center: owner_position
                .map(|position| subtract(collider.center, position)),
        });
    Ok(NativeDynamicColliderFeature {
        registration_index: collider.registration_index,
        owner,
        attack_hit_owner: actor_edge(
            collider.attack_hit_owner_runtime_generation,
            actor_generations,
        )?,
        target_hit_owner: actor_edge(
            collider.target_hit_owner_runtime_generation,
            actor_generations,
        )?,
        correction_hit_owner: actor_edge(
            collider.correction_hit_owner_runtime_generation,
            actor_generations,
        )?,
        status_present: collider.status_present,
        status,
        shape,
    })
}

fn actor_edge(
    runtime_generation: Option<u32>,
    actor_generations: &[u64],
) -> Result<CollisionActorEdge, NativeCollisionViewError> {
    let actor_index = runtime_generation
        .and_then(|generation| actor_generations.binary_search(&u64::from(generation)).ok())
        .map(u16::try_from)
        .transpose()
        .map_err(|_| NativeCollisionViewError::new("actor edge index overflowed"))?;
    Ok(CollisionActorEdge {
        runtime_generation,
        actor_index,
    })
}

fn validate_observation(
    observation: &NativeCollisionObservation,
) -> Result<(), NativeCollisionViewError> {
    if observation.episode_id.is_empty()
        || observation.stage.is_empty()
        || !is_lower_hex(&observation.state_identity_xxh3_128, 32)
        || observation
            .actor_runtime_generations
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || (observation.status != CollisionSetStatus::Present && !observation.colliders.is_empty())
    {
        return Err(NativeCollisionViewError::new(
            "native collision observation is invalid",
        ));
    }
    for (index, collider) in observation.colliders.iter().enumerate() {
        if usize::from(collider.registration_index) != index {
            return Err(NativeCollisionViewError::new(
                "dynamic collider registration order is noncanonical",
            ));
        }
        validate_edge(collider.owner, &observation.actor_runtime_generations)?;
        validate_edge(
            collider.attack_hit_owner,
            &observation.actor_runtime_generations,
        )?;
        validate_edge(
            collider.target_hit_owner,
            &observation.actor_runtime_generations,
        )?;
        validate_edge(
            collider.correction_hit_owner,
            &observation.actor_runtime_generations,
        )?;
        if collider
            .status
            .correction
            .iter()
            .any(|value| !value.is_finite())
            || !collider.status_present
                && (collider.status.weight != 0
                    || collider.status.damage != 0
                    || collider.status.correction != [0.0; 3])
            || collider.attack_hit_owner.runtime_generation.is_some() && !collider.status.attack_hit
            || collider.target_hit_owner.runtime_generation.is_some() && !collider.status.target_hit
            || collider.correction_hit_owner.runtime_generation.is_some()
                && !collider.status.correction_hit
        {
            return Err(NativeCollisionViewError::new(
                "dynamic collider status is inconsistent",
            ));
        }
        if let Some(shape) = &collider.shape
            && (shape
                .center
                .iter()
                .chain(&shape.aabb_min)
                .chain(&shape.aabb_max)
                .chain(shape.link_relative_center.iter().flatten())
                .chain(shape.owner_relative_center.iter().flatten())
                .chain([&shape.radius, &shape.height])
                .any(|value| !value.is_finite())
                || shape.radius < 0.0
                || shape.height < 0.0
                || shape
                    .aabb_min
                    .iter()
                    .zip(&shape.aabb_max)
                    .any(|(minimum, maximum)| minimum > maximum)
                || observation.player_present != shape.link_relative_center.is_some()
                || collider.owner.actor_index.is_some() != shape.owner_relative_center.is_some())
        {
            return Err(NativeCollisionViewError::new(
                "dynamic collider shape is invalid",
            ));
        }
    }
    Ok(())
}

fn validate_edge(
    edge: CollisionActorEdge,
    actor_generations: &[u64],
) -> Result<(), NativeCollisionViewError> {
    match (edge.runtime_generation, edge.actor_index) {
        (None, None) => Ok(()),
        (Some(_), None) => Ok(()),
        (Some(generation), Some(index))
            if actor_generations.get(usize::from(index)) == Some(&u64::from(generation)) =>
        {
            Ok(())
        }
        _ => Err(NativeCollisionViewError::new(
            "dynamic collider actor edge is invalid",
        )),
    }
}

fn relative_yaw(target: [f32; 3], origin: [f32; 3], yaw: i16) -> [f32; 3] {
    let delta = subtract(target, origin);
    let radians = f32::from(yaw) * PI / 32768.0;
    let (sin, cos) = radians.sin_cos();
    [
        cos * delta[0] - sin * delta[2],
        delta[1],
        sin * delta[0] + cos * delta[2],
    ]
}

fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn lower_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[usize::from(byte >> 4)] as char);
        encoded.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeCollisionViewError(String);

impl NativeCollisionViewError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeCollisionViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeCollisionViewError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn shard_v8() -> NativeEpisodeShard {
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v8.dseps"
        ))
        .unwrap();
        shard.episodes.truncate(1);
        shard.episodes[0].steps.truncate(1);
        shard
    }

    fn shard_v7() -> NativeEpisodeShard {
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v7.dseps"
        ))
        .unwrap();
        shard.episodes.truncate(1);
        shard.episodes[0].steps.truncate(1);
        shard
    }

    #[test]
    fn v8_collision_set_preserves_values_relative_geometry_and_actor_edges() {
        let shard = shard_v8();
        let source = &shard.episodes[0].steps[0].pre_input;
        let view = NativeEpisodeCollisionView::build(&shard).unwrap();
        assert_eq!(view.observations.len(), 2);
        let observation = &view.observations[0];
        assert_eq!(observation.status, CollisionSetStatus::Present);
        assert_eq!(observation.colliders.len(), 1);
        let collider = &observation.colliders[0];
        assert_eq!(collider.registration_index, 0);
        assert_eq!(collider.owner.runtime_generation, Some(7));
        let expected_owner_index = source
            .actors
            .iter()
            .position(|actor| actor.runtime_generation == 7)
            .map(|index| index as u16);
        assert_eq!(collider.owner.actor_index, expected_owner_index);
        assert_eq!(collider.attack_hit_owner.runtime_generation, Some(9));
        let expected_attack_index = source
            .actors
            .iter()
            .position(|actor| actor.runtime_generation == 9)
            .map(|index| index as u16);
        assert_eq!(collider.attack_hit_owner.actor_index, expected_attack_index);
        assert!(collider.status_present);
        assert!(collider.status.attack_set);
        assert!(collider.status.target_set);
        assert!(collider.status.correction_set);
        assert!(collider.status.attack_hit);
        assert_eq!(collider.status.attack_type, 0x20);
        assert_eq!(collider.status.target_type, 0xd8fbfdff);
        assert_eq!(collider.status.attack_power, 4);
        assert_eq!(collider.status.weight, 120);
        assert_eq!(collider.status.damage, 3);
        assert_eq!(collider.status.correction, [0.25, 0.0, -0.5]);

        let shape = collider.shape.as_ref().unwrap();
        assert_eq!(shape.kind, DynamicColliderShape::Cylinder);
        assert_eq!(shape.center, [12.5, 2.0, -8.0]);
        assert_eq!(shape.radius, 35.0);
        assert_eq!(shape.height, 80.0);
        assert_eq!(shape.aabb_min, [-22.5, 2.0, -43.0]);
        assert_eq!(shape.aabb_max, [47.5, 82.0, 27.0]);
        assert_eq!(shape.link_relative_center.is_some(), source.player_present);
        assert_eq!(
            shape.owner_relative_center.is_some(),
            expected_owner_index.is_some()
        );
        if let Some(index) = expected_owner_index {
            let owner_position = source.actors[usize::from(index)].position;
            assert_eq!(
                shape.owner_relative_center,
                Some(subtract(shape.center, owner_position))
            );
        }

        let bytes = view.canonical_bytes().unwrap();
        assert_eq!(
            NativeEpisodeCollisionView::decode_canonical(&bytes).unwrap(),
            view
        );
    }

    #[test]
    fn legacy_shards_preserve_not_sampled_instead_of_fabricating_an_empty_scan() {
        let view = NativeEpisodeCollisionView::build(&shard_v7()).unwrap();
        assert!(view.observations.iter().all(|observation| {
            observation.status == CollisionSetStatus::NotSampled && observation.colliders.is_empty()
        }));
    }

    #[test]
    fn malformed_edges_status_and_geometry_fail_after_resealing() {
        let view = NativeEpisodeCollisionView::build(&shard_v8()).unwrap();

        let mut wrong_edge = view.clone();
        wrong_edge.observations[0].colliders[0].owner.actor_index = Some(u16::MAX);
        wrong_edge.view_sha256 = wrong_edge.compute_identity().unwrap();
        assert!(wrong_edge.validate().is_err());

        let mut detached_hit = view.clone();
        detached_hit.observations[0].colliders[0].status.attack_hit = false;
        detached_hit.view_sha256 = detached_hit.compute_identity().unwrap();
        assert!(detached_hit.validate().is_err());

        let mut masked_payload = view.clone();
        masked_payload.observations[0].colliders[0].status_present = false;
        masked_payload.view_sha256 = masked_payload.compute_identity().unwrap();
        assert!(masked_payload.validate().is_err());

        let mut nonfinite = view;
        nonfinite.observations[0].colliders[0]
            .shape
            .as_mut()
            .unwrap()
            .center[0] = f32::NAN;
        nonfinite.view_sha256 = nonfinite.compute_identity().unwrap();
        assert!(nonfinite.validate().is_err());
    }
}
