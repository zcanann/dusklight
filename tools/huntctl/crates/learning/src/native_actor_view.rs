//! Complete relational actor views derived from authenticated native episodes.
//!
//! The native shard remains the canonical source of absolute dynamic state.
//! This view binds that state to the exact pointer-free profile catalog and
//! derives coordinate frames offline. It performs no live-game queries.

use crate::artifact::Digest;
use dusklight_evidence::native_episode_shard::{
    NativeActorObservation, NativeChannelStatus, NativeEpisodeShard, NativeLearningObservation,
    NativeObservationPhase,
};
use dusklight_world::actor_profile_catalog::ActorProfileCatalog;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::f32::consts::PI;
use std::fmt;

pub const NATIVE_ACTOR_VIEW_SCHEMA_V1: &str = "dusklight-native-actor-view/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorViewObservationPhase {
    PreInput,
    PostSimulation,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeActorRelation {
    pub runtime_generation: u64,
    pub parent_runtime_generation: u32,
    pub actor_name: i16,
    pub profile_name: i16,
    pub profile_slots: Vec<u32>,
    pub set_id: u16,
    pub home_room: i8,
    pub current_room: i8,
    pub group: u8,
    pub argument: i8,
    pub health: i16,
    pub parameters: u32,
    pub status: u32,
    pub absolute_position: [f32; 3],
    pub absolute_home_position: [f32; 3],
    pub absolute_velocity: [f32; 3],
    pub forward_speed: f32,
    pub current_angle: [i16; 3],
    pub shape_angle: [i16; 3],
    pub link_relative_position: Option<[f32; 3]>,
    pub link_relative_home_position: Option<[f32; 3]>,
    pub link_relative_velocity: Option<[f32; 3]>,
    pub link_distance: Option<f32>,
    pub current_yaw_relative_to_link: Option<[f32; 2]>,
    pub shape_yaw_relative_to_link: Option<[f32; 2]>,
    pub camera_relative_position: Option<[f32; 3]>,
    pub camera_relative_home_position: Option<[f32; 3]>,
    pub camera_relative_velocity: Option<[f32; 3]>,
    pub current_yaw_relative_to_camera: Option<[f32; 2]>,
    pub shape_yaw_relative_to_camera: Option<[f32; 2]>,
    pub parent_relative_position: Option<[f32; 3]>,
    pub parent_relative_velocity: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeActorViewObservation {
    pub episode_id: String,
    pub step_index: u32,
    pub phase: ActorViewObservationPhase,
    pub boundary_index: u64,
    pub state_identity_xxh3_128: String,
    pub stage: String,
    pub room: i8,
    pub player_present: bool,
    pub player_position: [f32; 3],
    pub player_yaw: i16,
    pub camera_frame_present: bool,
    pub actors: Vec<NativeActorRelation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEpisodeActorView {
    pub schema: String,
    pub native_shard_sha256: Digest,
    pub observation_schema: String,
    pub actor_profile_catalog_identity: String,
    pub actor_profile_catalog_sha256: Digest,
    pub observations: Vec<NativeActorViewObservation>,
    pub view_sha256: Digest,
}

impl NativeEpisodeActorView {
    pub fn build(
        shard: &NativeEpisodeShard,
        catalog: &ActorProfileCatalog,
    ) -> Result<Self, NativeActorViewError> {
        catalog
            .validate()
            .map_err(|error| NativeActorViewError::new(error.to_string()))?;
        let expected_catalog = shard
            .metadata
            .actor_profile_catalog_identity
            .as_deref()
            .ok_or_else(|| {
                NativeActorViewError::new(
                    "native actor view requires a shard-bound actor profile catalog",
                )
            })?;
        if expected_catalog != catalog.identity {
            return Err(NativeActorViewError::new(
                "native shard actor-profile identity does not match supplied catalog",
            ));
        }
        let mut profile_slots = BTreeMap::<i16, Vec<u32>>::new();
        for profile in catalog
            .profiles
            .iter()
            .filter(|profile| profile.present && profile.is_actor == Some(true))
        {
            profile_slots
                .entry(profile.profile_name.expect("validated present profile"))
                .or_default()
                .push(profile.slot);
        }

        let mut observations = Vec::new();
        for episode in &shard.episodes {
            for (step_index, step) in episode.steps.iter().enumerate() {
                let step_index = u32::try_from(step_index)
                    .map_err(|_| NativeActorViewError::new("episode step index overflowed"))?;
                observations.push(materialize_observation(
                    &episode.id,
                    step_index,
                    &step.pre_input,
                    &profile_slots,
                )?);
                observations.push(materialize_observation(
                    &episode.id,
                    step_index,
                    &step.post_simulation,
                    &profile_slots,
                )?);
            }
        }
        let mut view = Self {
            schema: NATIVE_ACTOR_VIEW_SCHEMA_V1.into(),
            native_shard_sha256: shard.content_sha256,
            observation_schema: shard.metadata.observation_schema.clone(),
            actor_profile_catalog_identity: catalog.identity.clone(),
            actor_profile_catalog_sha256: catalog
                .digest()
                .map_err(|error| NativeActorViewError::new(error.to_string()))?,
            observations,
            view_sha256: Digest::ZERO,
        };
        view.view_sha256 = view.compute_identity()?;
        view.validate()?;
        Ok(view)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, NativeActorViewError> {
        let view: Self = serde_json::from_slice(bytes)
            .map_err(|error| NativeActorViewError::new(error.to_string()))?;
        view.validate()?;
        if view.canonical_bytes()? != bytes {
            return Err(NativeActorViewError::new(
                "native actor view bytes are not canonical",
            ));
        }
        Ok(view)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, NativeActorViewError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| NativeActorViewError::new(error.to_string()))
    }

    pub fn validate(&self) -> Result<(), NativeActorViewError> {
        if self.schema != NATIVE_ACTOR_VIEW_SCHEMA_V1
            || self.native_shard_sha256 == Digest::ZERO
            || self.observation_schema.is_empty()
            || !valid_catalog_identity(&self.actor_profile_catalog_identity)
            || self.actor_profile_catalog_sha256 == Digest::ZERO
            || self.observations.is_empty()
            || self.view_sha256 != self.compute_identity()?
        {
            return Err(NativeActorViewError::new(
                "native actor view envelope or seal is invalid",
            ));
        }
        for observation in &self.observations {
            validate_observation(observation)?;
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, NativeActorViewError> {
        let mut canonical = self.clone();
        canonical.view_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| NativeActorViewError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-actor-view/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn materialize_observation(
    episode_id: &str,
    step_index: u32,
    observation: &NativeLearningObservation,
    profile_slots: &BTreeMap<i16, Vec<u32>>,
) -> Result<NativeActorViewObservation, NativeActorViewError> {
    let phase = match observation.phase {
        NativeObservationPhase::PreInput => ActorViewObservationPhase::PreInput,
        NativeObservationPhase::PostSimulation => ActorViewObservationPhase::PostSimulation,
    };
    let camera_frame = if observation.camera_status == NativeChannelStatus::Present {
        observation
            .camera
            .as_ref()
            .and_then(CameraFrame::from_observation)
    } else {
        None
    };
    let actors_by_generation = observation
        .actors
        .iter()
        .map(|actor| (actor.runtime_generation, actor))
        .collect::<BTreeMap<_, _>>();
    let mut actors = Vec::with_capacity(observation.actors.len());
    for actor in &observation.actors {
        let slots = profile_slots.get(&actor.profile_name).ok_or_else(|| {
            NativeActorViewError::new(format!(
                "actor profile {} is absent from supplied profile catalog",
                actor.profile_name
            ))
        })?;
        actors.push(materialize_actor(
            actor,
            slots,
            observation,
            camera_frame,
            &actors_by_generation,
        ));
    }
    Ok(NativeActorViewObservation {
        episode_id: episode_id.into(),
        step_index,
        phase,
        boundary_index: observation.boundary_index,
        state_identity_xxh3_128: lower_hex(&observation.state_identity),
        stage: observation.stage.clone(),
        room: observation.room,
        player_present: observation.player_present,
        player_position: observation.player_position,
        player_yaw: observation.player_shape_angle[1],
        camera_frame_present: camera_frame.is_some(),
        actors,
    })
}

fn materialize_actor(
    actor: &NativeActorObservation,
    profile_slots: &[u32],
    observation: &NativeLearningObservation,
    camera_frame: Option<CameraFrame>,
    actors_by_generation: &BTreeMap<u64, &NativeActorObservation>,
) -> NativeActorRelation {
    let link_relative_position = observation.player_present.then(|| {
        relative_yaw(
            actor.position,
            observation.player_position,
            observation.player_shape_angle[1],
        )
    });
    let link_relative_home_position = observation.player_present.then(|| {
        relative_yaw(
            actor.home_position,
            observation.player_position,
            observation.player_shape_angle[1],
        )
    });
    let link_relative_velocity = observation
        .player_present
        .then(|| direction_yaw(actor.velocity, observation.player_shape_angle[1]));
    let link_distance = link_relative_position.map(length);
    let current_yaw_relative_to_link = observation.player_present.then(|| {
        angle_pair(actor.current_angle[1].wrapping_sub(observation.player_shape_angle[1]))
    });
    let shape_yaw_relative_to_link = observation
        .player_present
        .then(|| angle_pair(actor.shape_angle[1].wrapping_sub(observation.player_shape_angle[1])));
    let parent = actors_by_generation.get(&u64::from(actor.parent_runtime_generation));
    NativeActorRelation {
        runtime_generation: actor.runtime_generation,
        parent_runtime_generation: actor.parent_runtime_generation,
        actor_name: actor.actor_name,
        profile_name: actor.profile_name,
        profile_slots: profile_slots.to_vec(),
        set_id: actor.set_id,
        home_room: actor.home_room,
        current_room: actor.current_room,
        group: actor.group,
        argument: actor.argument,
        health: actor.health,
        parameters: actor.parameters,
        status: actor.status,
        absolute_position: actor.position,
        absolute_home_position: actor.home_position,
        absolute_velocity: actor.velocity,
        forward_speed: actor.forward_speed,
        current_angle: actor.current_angle,
        shape_angle: actor.shape_angle,
        link_relative_position,
        link_relative_home_position,
        link_relative_velocity,
        link_distance,
        current_yaw_relative_to_link,
        shape_yaw_relative_to_link,
        camera_relative_position: camera_frame.map(|frame| frame.point(actor.position)),
        camera_relative_home_position: camera_frame.map(|frame| frame.point(actor.home_position)),
        camera_relative_velocity: camera_frame.map(|frame| frame.direction(actor.velocity)),
        current_yaw_relative_to_camera: camera_frame
            .map(|frame| angle_pair(actor.current_angle[1].wrapping_sub(frame.view_yaw))),
        shape_yaw_relative_to_camera: camera_frame
            .map(|frame| angle_pair(actor.shape_angle[1].wrapping_sub(frame.view_yaw))),
        parent_relative_position: parent.map(|parent| subtract(actor.position, parent.position)),
        parent_relative_velocity: parent.map(|parent| subtract(actor.velocity, parent.velocity)),
    }
}

#[derive(Clone, Copy)]
struct CameraFrame {
    eye: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
    view_yaw: i16,
}

impl CameraFrame {
    fn from_observation(
        camera: &dusklight_evidence::native_episode_shard::NativeCameraObservation,
    ) -> Option<Self> {
        let forward = normalize(subtract(camera.center, camera.eye))?;
        let right = normalize(cross(forward, camera.up))?;
        let up = normalize(cross(right, forward))?;
        Some(Self {
            eye: camera.eye,
            right,
            up,
            forward,
            view_yaw: camera.view_yaw,
        })
    }

    fn point(self, point: [f32; 3]) -> [f32; 3] {
        self.direction(subtract(point, self.eye))
    }

    fn direction(self, direction: [f32; 3]) -> [f32; 3] {
        [
            dot(direction, self.right),
            dot(direction, self.up),
            dot(direction, self.forward),
        ]
    }
}

fn validate_observation(
    observation: &NativeActorViewObservation,
) -> Result<(), NativeActorViewError> {
    if observation.episode_id.is_empty()
        || observation.stage.is_empty()
        || !is_lower_hex(&observation.state_identity_xxh3_128, 32)
        || observation
            .player_position
            .iter()
            .any(|value| !value.is_finite())
        || observation
            .actors
            .windows(2)
            .any(|pair| pair[0].runtime_generation >= pair[1].runtime_generation)
    {
        return Err(NativeActorViewError::new(
            "native actor observation envelope is invalid",
        ));
    }
    for actor in &observation.actors {
        let option_consistent = observation.player_present
            == actor.link_relative_position.is_some()
            && observation.player_present == actor.link_relative_home_position.is_some()
            && observation.player_present == actor.link_relative_velocity.is_some()
            && observation.player_present == actor.link_distance.is_some()
            && observation.player_present == actor.current_yaw_relative_to_link.is_some()
            && observation.player_present == actor.shape_yaw_relative_to_link.is_some()
            && observation.camera_frame_present == actor.camera_relative_position.is_some()
            && observation.camera_frame_present == actor.camera_relative_home_position.is_some()
            && observation.camera_frame_present == actor.camera_relative_velocity.is_some()
            && observation.camera_frame_present == actor.current_yaw_relative_to_camera.is_some()
            && observation.camera_frame_present == actor.shape_yaw_relative_to_camera.is_some();
        let finite = actor
            .absolute_position
            .iter()
            .chain(&actor.absolute_home_position)
            .chain(&actor.absolute_velocity)
            .chain(std::iter::once(&actor.forward_speed))
            .chain(actor.link_relative_position.iter().flatten())
            .chain(actor.link_relative_home_position.iter().flatten())
            .chain(actor.link_relative_velocity.iter().flatten())
            .chain(actor.link_distance.iter())
            .chain(actor.current_yaw_relative_to_link.iter().flatten())
            .chain(actor.shape_yaw_relative_to_link.iter().flatten())
            .chain(actor.camera_relative_position.iter().flatten())
            .chain(actor.camera_relative_home_position.iter().flatten())
            .chain(actor.camera_relative_velocity.iter().flatten())
            .chain(actor.current_yaw_relative_to_camera.iter().flatten())
            .chain(actor.shape_yaw_relative_to_camera.iter().flatten())
            .chain(actor.parent_relative_position.iter().flatten())
            .chain(actor.parent_relative_velocity.iter().flatten())
            .all(|value| value.is_finite());
        if actor.profile_slots.is_empty()
            || actor
                .profile_slots
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || !option_consistent
            || !finite
        {
            return Err(NativeActorViewError::new(
                "native actor relation is invalid",
            ));
        }
    }
    Ok(())
}

fn relative_yaw(target: [f32; 3], origin: [f32; 3], yaw: i16) -> [f32; 3] {
    direction_yaw(subtract(target, origin), yaw)
}

fn direction_yaw(direction: [f32; 3], yaw: i16) -> [f32; 3] {
    let radians = f32::from(yaw) * PI / 32768.0;
    let (sin, cos) = radians.sin_cos();
    [
        cos * direction[0] - sin * direction[2],
        direction[1],
        sin * direction[0] + cos * direction[2],
    ]
}

fn angle_pair(angle: i16) -> [f32; 2] {
    let radians = f32::from(angle) * PI / 32768.0;
    [radians.sin(), radians.cos()]
}

fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left.into_iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

fn length(value: [f32; 3]) -> f32 {
    dot(value, value).sqrt()
}

fn normalize(value: [f32; 3]) -> Option<[f32; 3]> {
    let length = length(value);
    (length.is_finite() && length > f32::EPSILON).then(|| value.map(|component| component / length))
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

fn valid_catalog_identity(value: &str) -> bool {
    value
        .strip_prefix("actor-profile-catalog:xxh3-128:")
        .is_some_and(|digest| is_lower_hex(digest, 32))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeActorViewError(String);

impl NativeActorViewError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeActorViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeActorViewError {}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_world::actor_profile_catalog::{ACTOR_PROFILE_CATALOG_SCHEMA, ActorProfileEntry};

    fn catalog_for(shard: &NativeEpisodeShard) -> ActorProfileCatalog {
        let mut names = shard
            .episodes
            .iter()
            .flat_map(|episode| &episode.steps)
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
            .flat_map(|observation| &observation.actors)
            .map(|actor| actor.profile_name)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        let mut catalog = ActorProfileCatalog {
            schema: ACTOR_PROFILE_CATALOG_SCHEMA.into(),
            identity: String::new(),
            profiles: names
                .into_iter()
                .enumerate()
                .map(|(slot, profile_name)| ActorProfileEntry {
                    slot: slot as u32,
                    present: true,
                    layer_id: Some(u32::MAX - 2),
                    list_id: Some(7),
                    list_priority: Some(u16::MAX - 2),
                    profile_name: Some(profile_name),
                    process_size: Some(512),
                    auxiliary_size: Some(0),
                    parameters: Some(0),
                    is_leaf: Some(true),
                    draw_priority: Some(slot as i16),
                    is_actor: Some(true),
                    status: Some(0),
                    group: Some(0),
                    cull_type: Some(0),
                })
                .collect(),
        };
        catalog.identity = catalog.computed_identity().unwrap();
        catalog
    }

    fn shard() -> NativeEpisodeShard {
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v5.dseps"
        ))
        .unwrap();
        shard.episodes.truncate(1);
        shard.episodes[0].steps.truncate(1);
        shard
    }

    #[test]
    fn builds_complete_absolute_link_camera_and_parent_relations() {
        let mut shard = shard();
        let catalog = catalog_for(&shard);
        shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
        let view = NativeEpisodeActorView::build(&shard, &catalog).unwrap();
        assert_eq!(view.observations.len(), 2);
        for observation in &view.observations {
            assert_eq!(
                observation.actors.len(),
                shard.episodes[0].steps[0].pre_input.actors.len()
            );
            assert!(observation.player_present);
            assert!(observation.camera_frame_present);
            assert!(observation.actors.iter().all(|actor| {
                actor.link_relative_position.is_some()
                    && actor.camera_relative_position.is_some()
                    && !actor.profile_slots.is_empty()
            }));
        }
        let bytes = view.canonical_bytes().unwrap();
        assert_eq!(
            NativeEpisodeActorView::decode_canonical(&bytes).unwrap(),
            view
        );
    }

    #[test]
    fn rejects_the_wrong_catalog_and_noncanonical_or_tampered_views() {
        let mut shard = shard();
        let catalog = catalog_for(&shard);
        shard.metadata.actor_profile_catalog_identity =
            Some("actor-profile-catalog:xxh3-128:00000000000000000000000000000001".into());
        assert!(NativeEpisodeActorView::build(&shard, &catalog).is_err());

        shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
        let view = NativeEpisodeActorView::build(&shard, &catalog).unwrap();
        let mut bytes = view.canonical_bytes().unwrap();
        bytes.push(b'\n');
        assert!(NativeEpisodeActorView::decode_canonical(&bytes).is_err());
        let mut tampered = view;
        tampered.observations[0].actors[0].absolute_position[0] += 1.0;
        assert!(tampered.validate().is_err());
    }
}
