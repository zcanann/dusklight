//! Complete relational actor views derived from authenticated native episodes.
//!
//! The native shard remains the canonical source of absolute dynamic state.
//! This view binds that state to the exact pointer-free profile catalog and
//! derives coordinate frames offline. It performs no live-game queries.

use crate::artifact::Digest;
use crate::compiled_goal_graph::{CompiledGoalGraph, GoalSpatialAnchor};
use dusklight_evidence::native_episode_shard::{
    NativeActorObservation, NativeChannelStatus, NativeEpisodeShard, NativeLearningObservation,
    NativeObservationPhase,
};
use dusklight_objectives::milestone_dsl::{CompiledMilestones, decode};
use dusklight_world::actor_profile_catalog::ActorProfileCatalog;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::f32::consts::PI;
use std::fmt;

pub const NATIVE_ACTOR_VIEW_SCHEMA_V4: &str = "dusklight-native-actor-view/v4";

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
    pub base_state: Option<NativeActorBaseState>,
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
    pub attention: Option<NativeActorAttentionRelation>,
    pub event_participation: Option<NativeActorEventParticipation>,
    /// One entry per semantic goal-graph spatial anchor, preserving explicit
    /// unresolved values.
    pub goal_relative_positions: Vec<Option<[f32; 3]>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeActorBaseState {
    pub actor_type: i32,
    pub process_subtype: i32,
    pub condition: u32,
    pub old_room: i8,
    pub pause_flag: u8,
    pub process_init_state: i8,
    pub process_create_phase: u8,
    pub cull_type: u8,
    pub demo_actor_id: u8,
    pub carry_type: u8,
    pub heap_present: bool,
    pub model_present: bool,
    pub joint_collision_present: bool,
    pub absolute_old_position: [f32; 3],
    pub scale: [f32; 3],
    pub gravity: f32,
    pub max_fall_speed: f32,
    pub absolute_eye_position: [f32; 3],
    pub home_angle: [i16; 3],
    pub old_angle: [i16; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeActorAttentionRelation {
    pub flags: u32,
    pub absolute_position: [f32; 3],
    pub distance_indices: [u8; 9],
    pub auxiliary: i16,
    pub link_relative_position: Option<[f32; 3]>,
    pub camera_relative_position: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeActorEventParticipation {
    pub command: u16,
    pub condition: u16,
    pub event_id: i16,
    pub map_tool_id: u8,
    pub index: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeGoalAnchorStatus {
    Static,
    ResolvedActor,
    StageMismatch,
    ActorAbsent,
    ActorAmbiguous,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalAnchorObservation {
    /// Exact predicate node in `NativeEpisodeActorView::goal_graph`.
    pub node_index: u16,
    /// Zero is `when`; subsequent values are ordered `then` steps.
    pub sequence_step: u16,
    pub status: NativeGoalAnchorStatus,
    pub absolute_position: Option<[f32; 3]>,
    pub link_relative_position: Option<[f32; 3]>,
    pub actor_runtime_generation: Option<u64>,
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
    pub goal_anchors: Vec<NativeGoalAnchorObservation>,
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
    pub goal_graph: Option<CompiledGoalGraph>,
    pub observations: Vec<NativeActorViewObservation>,
    pub view_sha256: Digest,
}

impl NativeEpisodeActorView {
    pub fn build(
        shard: &NativeEpisodeShard,
        catalog: &ActorProfileCatalog,
    ) -> Result<Self, NativeActorViewError> {
        Self::build_internal(shard, catalog, None)
    }

    pub fn build_for_goal(
        shard: &NativeEpisodeShard,
        catalog: &ActorProfileCatalog,
        milestone_program: &[u8],
        definition_name: &str,
    ) -> Result<Self, NativeActorViewError> {
        let decoded = decode(milestone_program)
            .map_err(|error| NativeActorViewError::new(error.to_string()))?;
        let definition_index = decoded
            .definitions
            .iter()
            .position(|definition| definition.name == definition_name)
            .ok_or_else(|| {
                NativeActorViewError::new(format!(
                    "compiled milestone definition {definition_name} does not exist"
                ))
            })?;
        if shard.metadata.objective != definition_name {
            return Err(NativeActorViewError::new(format!(
                "native shard objective {} does not match selected definition {definition_name}",
                shard.metadata.objective
            )));
        }
        let program_sha256 = Digest(decoded.program_sha256);
        let definition_sha256 = Digest(decoded.definitions[definition_index].sha256);
        shard
            .verify_authored_objective(&program_sha256.to_string(), &definition_sha256.to_string())
            .map_err(|error| NativeActorViewError::new(error.to_string()))?;
        let compiled = CompiledMilestones {
            bytes: milestone_program.to_vec(),
            program_sha256: decoded.program_sha256,
            definitions: decoded.definitions,
        };
        let goal_graph = CompiledGoalGraph::from_compiled(&compiled, definition_index)
            .map_err(|error| NativeActorViewError::new(error.to_string()))?;
        Self::build_internal(shard, catalog, Some(goal_graph))
    }

    fn build_internal(
        shard: &NativeEpisodeShard,
        catalog: &ActorProfileCatalog,
        goal_graph: Option<CompiledGoalGraph>,
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
                    goal_graph.as_ref(),
                )?);
                observations.push(materialize_observation(
                    &episode.id,
                    step_index,
                    &step.post_simulation,
                    &profile_slots,
                    goal_graph.as_ref(),
                )?);
            }
        }
        let mut view = Self {
            schema: NATIVE_ACTOR_VIEW_SCHEMA_V4.into(),
            native_shard_sha256: shard.content_sha256,
            observation_schema: shard.metadata.observation_schema.clone(),
            actor_profile_catalog_identity: catalog.identity.clone(),
            actor_profile_catalog_sha256: catalog
                .digest()
                .map_err(|error| NativeActorViewError::new(error.to_string()))?,
            goal_graph,
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
        if self.schema != NATIVE_ACTOR_VIEW_SCHEMA_V4
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
        if let Some(graph) = &self.goal_graph {
            graph
                .validate()
                .map_err(|error| NativeActorViewError::new(error.to_string()))?;
        }
        let goal_anchors = self
            .goal_graph
            .as_ref()
            .map_or_else(Vec::new, CompiledGoalGraph::spatial_anchors);
        for observation in &self.observations {
            validate_observation(observation, &goal_anchors)?;
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, NativeActorViewError> {
        let mut canonical = self.clone();
        canonical.view_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| NativeActorViewError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-actor-view/v4\0");
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
    goal_graph: Option<&CompiledGoalGraph>,
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
    let goal_anchors = goal_graph
        .map(|graph| materialize_goal_anchors(&graph.spatial_anchors(), observation))
        .unwrap_or_default();
    let goal_positions = goal_anchors
        .iter()
        .map(|anchor| anchor.absolute_position)
        .collect::<Vec<_>>();
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
            &goal_positions,
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
        goal_anchors,
        actors,
    })
}

fn materialize_goal_anchors(
    anchors: &[GoalSpatialAnchor],
    observation: &NativeLearningObservation,
) -> Vec<NativeGoalAnchorObservation> {
    anchors
        .iter()
        .map(|anchor| {
            let (node_index, sequence_step) = goal_anchor_identity(anchor);
            let (status, absolute_position, actor_runtime_generation) = match anchor {
                GoalSpatialAnchor::PlayerAabb {
                    minimum, maximum, ..
                } => (
                    NativeGoalAnchorStatus::Static,
                    Some([
                        (minimum[0] + maximum[0]) * 0.5,
                        (minimum[1] + maximum[1]) * 0.5,
                        (minimum[2] + maximum[2]) * 0.5,
                    ]),
                    None,
                ),
                GoalSpatialAnchor::PlayerPlane { point, .. } => {
                    (NativeGoalAnchorStatus::Static, Some(*point), None)
                }
                GoalSpatialAnchor::PlacedActor { selector, .. }
                    if selector.stage != observation.stage =>
                {
                    (NativeGoalAnchorStatus::StageMismatch, None, None)
                }
                GoalSpatialAnchor::PlacedActor { selector, .. } => {
                    let mut matches = observation.actors.iter().filter(|actor| {
                        actor.home_room == selector.home_room
                            && actor.set_id == selector.set_id
                            && actor.actor_name == selector.actor_name
                    });
                    match (matches.next(), matches.next()) {
                        (None, _) => (NativeGoalAnchorStatus::ActorAbsent, None, None),
                        (Some(actor), None) => (
                            NativeGoalAnchorStatus::ResolvedActor,
                            Some(actor.position),
                            Some(actor.runtime_generation),
                        ),
                        (Some(_), Some(_)) => (NativeGoalAnchorStatus::ActorAmbiguous, None, None),
                    }
                }
            };
            let link_relative_position = absolute_position.and_then(|position| {
                observation.player_present.then(|| {
                    relative_yaw(
                        position,
                        observation.player_position,
                        observation.player_shape_angle[1],
                    )
                })
            });
            NativeGoalAnchorObservation {
                node_index,
                sequence_step,
                status,
                absolute_position,
                link_relative_position,
                actor_runtime_generation,
            }
        })
        .collect()
}

fn goal_anchor_identity(anchor: &GoalSpatialAnchor) -> (u16, u16) {
    match anchor {
        GoalSpatialAnchor::PlacedActor {
            node_index,
            sequence_step,
            ..
        }
        | GoalSpatialAnchor::PlayerAabb {
            node_index,
            sequence_step,
            ..
        }
        | GoalSpatialAnchor::PlayerPlane {
            node_index,
            sequence_step,
            ..
        } => (*node_index, *sequence_step),
    }
}

fn materialize_actor(
    actor: &NativeActorObservation,
    profile_slots: &[u32],
    observation: &NativeLearningObservation,
    camera_frame: Option<CameraFrame>,
    actors_by_generation: &BTreeMap<u64, &NativeActorObservation>,
    goal_positions: &[Option<[f32; 3]>],
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
        base_state: actor.base_state_available.then_some(NativeActorBaseState {
            actor_type: actor.actor_type,
            process_subtype: actor.process_subtype,
            condition: actor.condition,
            old_room: actor.old_room,
            pause_flag: actor.pause_flag,
            process_init_state: actor.process_init_state,
            process_create_phase: actor.process_create_phase,
            cull_type: actor.cull_type,
            demo_actor_id: actor.demo_actor_id,
            carry_type: actor.carry_type,
            heap_present: actor.heap_present,
            model_present: actor.model_present,
            joint_collision_present: actor.joint_collision_present,
            absolute_old_position: actor.old_position,
            scale: actor.scale,
            gravity: actor.gravity,
            max_fall_speed: actor.max_fall_speed,
            absolute_eye_position: actor.eye_position,
            home_angle: actor.home_angle,
            old_angle: actor.old_angle,
        }),
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
        attention: actor
            .attention
            .as_ref()
            .map(|attention| NativeActorAttentionRelation {
                flags: attention.flags,
                absolute_position: attention.position,
                distance_indices: attention.distance_indices,
                auxiliary: attention.auxiliary,
                link_relative_position: observation.player_present.then(|| {
                    relative_yaw(
                        attention.position,
                        observation.player_position,
                        observation.player_shape_angle[1],
                    )
                }),
                camera_relative_position: camera_frame.map(|frame| frame.point(attention.position)),
            }),
        event_participation: actor.event_participation.as_ref().map(|event| {
            NativeActorEventParticipation {
                command: event.command,
                condition: event.condition,
                event_id: event.event_id,
                map_tool_id: event.map_tool_id,
                index: event.index,
            }
        }),
        goal_relative_positions: goal_positions
            .iter()
            .map(|position| position.map(|position| subtract(actor.position, position)))
            .collect(),
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
    expected_goal_anchors: &[GoalSpatialAnchor],
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
        || observation.goal_anchors.len() != expected_goal_anchors.len()
    {
        return Err(NativeActorViewError::new(
            "native actor observation envelope is invalid",
        ));
    }
    for (anchor, expected) in observation.goal_anchors.iter().zip(expected_goal_anchors) {
        let payload_consistent = match anchor.status {
            NativeGoalAnchorStatus::Static => {
                anchor.absolute_position.is_some() && anchor.actor_runtime_generation.is_none()
            }
            NativeGoalAnchorStatus::ResolvedActor => {
                anchor.absolute_position.is_some() && anchor.actor_runtime_generation.is_some()
            }
            NativeGoalAnchorStatus::StageMismatch
            | NativeGoalAnchorStatus::ActorAbsent
            | NativeGoalAnchorStatus::ActorAmbiguous => {
                anchor.absolute_position.is_none() && anchor.actor_runtime_generation.is_none()
            }
        };
        if (anchor.node_index, anchor.sequence_step) != goal_anchor_identity(expected)
            || !payload_consistent
            || (anchor.absolute_position.is_some() && observation.player_present)
                != anchor.link_relative_position.is_some()
            || anchor
                .absolute_position
                .iter()
                .flatten()
                .chain(anchor.link_relative_position.iter().flatten())
                .any(|value| !value.is_finite())
        {
            return Err(NativeActorViewError::new(
                "goal anchor observation is invalid",
            ));
        }
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
            .chain(
                actor
                    .attention
                    .iter()
                    .flat_map(|attention| attention.absolute_position.iter()),
            )
            .chain(
                actor
                    .attention
                    .iter()
                    .flat_map(|attention| attention.link_relative_position.iter().flatten()),
            )
            .chain(
                actor
                    .attention
                    .iter()
                    .flat_map(|attention| attention.camera_relative_position.iter().flatten()),
            )
            .chain(actor.goal_relative_positions.iter().flatten().flatten())
            .all(|value| value.is_finite());
        let base_state_finite = actor.base_state.as_ref().is_none_or(|base_state| {
            base_state
                .absolute_old_position
                .iter()
                .chain(&base_state.scale)
                .chain(std::iter::once(&base_state.gravity))
                .chain(std::iter::once(&base_state.max_fall_speed))
                .chain(&base_state.absolute_eye_position)
                .all(|value| value.is_finite())
        });
        let attention_consistent = actor.attention.as_ref().is_none_or(|attention| {
            attention.flags != 0
                && observation.player_present == attention.link_relative_position.is_some()
                && observation.camera_frame_present == attention.camera_relative_position.is_some()
        });
        if actor.profile_slots.is_empty()
            || actor.goal_relative_positions.len() != expected_goal_anchors.len()
            || actor
                .profile_slots
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || !option_consistent
            || !attention_consistent
            || !base_state_finite
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
    use dusklight_evidence::native_episode_shard::authored_milestone_objective_identity;
    use dusklight_objectives::milestone_dsl::compile_source;
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
            "../../../../../tests/fixtures/automation/native_episode_v6.dseps"
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
    fn builds_complete_absolute_link_camera_and_parent_relations() {
        let mut shard = shard();
        let catalog = catalog_for(&shard);
        shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
        let view = NativeEpisodeActorView::build(&shard, &catalog).unwrap();
        assert_eq!(view.observations.len(), 2);
        for observation in &view.observations {
            assert!(view.goal_graph.is_none());
            assert!(observation.goal_anchors.is_empty());
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
                    && actor.goal_relative_positions.is_empty()
                    && actor.base_state.is_none()
            }));
            let attention = observation.actors[0].attention.as_ref().unwrap();
            assert_eq!(attention.flags, 0x20000002);
            assert!(attention.link_relative_position.is_some());
            assert!(attention.camera_relative_position.is_some());
            assert_eq!(
                observation.actors[0]
                    .event_participation
                    .as_ref()
                    .unwrap()
                    .event_id,
                27
            );
        }
        let bytes = view.canonical_bytes().unwrap();
        assert_eq!(
            NativeEpisodeActorView::decode_canonical(&bytes).unwrap(),
            view
        );
    }

    #[test]
    fn exposes_v7_universal_base_state_without_fabricating_legacy_values() {
        let mut v7_shard = shard_v7();
        let catalog = catalog_for(&v7_shard);
        v7_shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
        let view = NativeEpisodeActorView::build(&v7_shard, &catalog).unwrap();
        for observation in &view.observations {
            let state = observation.actors[0]
                .base_state
                .as_ref()
                .expect("v7 actor base state");
            assert_eq!(state.actor_type, 5);
            assert_eq!(state.process_subtype, 6);
            assert_eq!(state.condition, 0x12);
            assert_eq!(state.old_room, 1);
            assert_eq!(state.pause_flag, 4);
            assert_eq!(state.process_init_state, -2);
            assert_eq!(state.process_create_phase, 7);
            assert_eq!(state.cull_type, 8);
            assert_eq!(state.demo_actor_id, 9);
            assert_eq!(state.carry_type, 10);
            assert!(state.heap_present);
            assert!(state.model_present);
            assert!(state.joint_collision_present);
            assert_eq!(state.absolute_old_position, [12.0, 2.5, -8.5]);
            assert_eq!(state.scale, [1.0, 2.0, 3.0]);
            assert_eq!(state.gravity, -3.0);
            assert_eq!(state.max_fall_speed, -20.0);
            assert_eq!(state.absolute_eye_position, [12.5, 7.0, -8.0]);
            assert_eq!(state.home_angle, [11, 12, 13]);
            assert_eq!(state.old_angle, [14, 15, 16]);
        }

        let mut legacy = shard();
        let legacy_catalog = catalog_for(&legacy);
        legacy.metadata.actor_profile_catalog_identity = Some(legacy_catalog.identity.clone());
        let legacy_view = NativeEpisodeActorView::build(&legacy, &legacy_catalog).unwrap();
        assert!(legacy_view.observations.iter().all(|observation| {
            observation
                .actors
                .iter()
                .all(|actor| actor.base_state.is_none())
        }));
    }

    #[test]
    fn binds_exact_compiled_goal_and_derives_only_real_spatial_anchors() {
        const SOURCE: &str = r#"milestones 1.8
milestone spatial_goal {
  phase post_sim
  when player.in_aabb(10.0, 0.0, -10.0, 14.0, 6.0, -6.0) &&
       actor.placed.exists("F_SP103", 0, 4, 291) &&
       player.plane_signed_distance(1.0, 2.0, 3.0, 1.0, 0.0, 0.0) >= 0.0
}
"#;
        let compiled = compile_source(SOURCE).unwrap();
        let definition = &compiled.definitions[0];
        let mut shard = shard();
        shard.metadata.objective = definition.name.clone();
        shard.metadata.objective_identity = authored_milestone_objective_identity(
            &Digest(compiled.program_sha256).to_string(),
            &Digest(definition.sha256).to_string(),
        )
        .unwrap();
        for observation in shard.episodes.iter_mut().flat_map(|episode| {
            episode
                .steps
                .iter_mut()
                .flat_map(|step| [&mut step.pre_input, &mut step.post_simulation])
        }) {
            for actor in observation.actors.iter_mut().skip(1) {
                actor.set_id = 5;
            }
        }
        let catalog = catalog_for(&shard);
        shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
        let view = NativeEpisodeActorView::build_for_goal(
            &shard,
            &catalog,
            &compiled.bytes,
            "spatial_goal",
        )
        .unwrap();
        let graph = view.goal_graph.as_ref().unwrap();
        assert_eq!(graph.program_sha256, Digest(compiled.program_sha256));
        assert_eq!(graph.definition_sha256, Digest(definition.sha256));
        assert_eq!(graph.spatial_anchors().len(), 3);
        for observation in &view.observations {
            assert_eq!(observation.goal_anchors.len(), 3);
            assert_eq!(observation.goal_anchors[0].node_index, 0);
            assert_eq!(observation.goal_anchors[0].sequence_step, 0);
            assert_eq!(
                observation.goal_anchors[0].status,
                NativeGoalAnchorStatus::Static
            );
            assert_eq!(
                observation.goal_anchors[0].absolute_position,
                Some([12.0, 3.0, -8.0])
            );
            assert_eq!(
                observation.goal_anchors[1].status,
                NativeGoalAnchorStatus::ResolvedActor
            );
            assert_eq!(
                observation.goal_anchors[1].actor_runtime_generation,
                Some(1)
            );
            assert_eq!(
                observation.actors[0].goal_relative_positions,
                [
                    Some([0.5, 0.0, 0.0]),
                    Some([0.0, 0.0, 0.0]),
                    Some([11.5, 1.0, -11.0])
                ]
            );
        }

        let mut wrong_identity = shard.clone();
        wrong_identity.metadata.objective_identity = "00000000000000000000000000000000".into();
        assert!(
            NativeEpisodeActorView::build_for_goal(
                &wrong_identity,
                &catalog,
                &compiled.bytes,
                "spatial_goal"
            )
            .is_err()
        );
    }

    #[test]
    fn goal_actor_resolution_preserves_ambiguous_and_stage_missingness() {
        const SOURCE: &str = r#"milestones 1.8
milestone actor_goal {
  phase post_sim
  when actor.placed.exists("F_SP103", 0, 4, 291)
}
"#;
        let compiled = compile_source(SOURCE).unwrap();
        let definition = &compiled.definitions[0];
        let mut shard = shard();
        shard.metadata.objective = definition.name.clone();
        shard.metadata.objective_identity = authored_milestone_objective_identity(
            &Digest(compiled.program_sha256).to_string(),
            &Digest(definition.sha256).to_string(),
        )
        .unwrap();
        let catalog = catalog_for(&shard);
        shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());

        let ambiguous =
            NativeEpisodeActorView::build_for_goal(&shard, &catalog, &compiled.bytes, "actor_goal")
                .unwrap();
        assert!(ambiguous.observations.iter().all(|observation| {
            observation.goal_anchors[0].status == NativeGoalAnchorStatus::ActorAmbiguous
                && observation.goal_anchors[0].absolute_position.is_none()
                && observation
                    .actors
                    .iter()
                    .all(|actor| actor.goal_relative_positions == [None])
        }));

        for observation in shard.episodes.iter_mut().flat_map(|episode| {
            episode
                .steps
                .iter_mut()
                .flat_map(|step| [&mut step.pre_input, &mut step.post_simulation])
        }) {
            observation.stage = "F_SP104".into();
        }
        let wrong_stage =
            NativeEpisodeActorView::build_for_goal(&shard, &catalog, &compiled.bytes, "actor_goal")
                .unwrap();
        assert!(wrong_stage.observations.iter().all(|observation| {
            observation.goal_anchors[0].status == NativeGoalAnchorStatus::StageMismatch
                && observation.goal_anchors[0].absolute_position.is_none()
        }));
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
