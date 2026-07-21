//! Declarative, typed actor feature views derived from authenticated actor sets.
//!
//! The native actor view remains the source of truth. This artifact separates
//! exact categorical values, continuous values, presence masks, structural
//! process relations, and goal-relative vectors without selecting a fixed
//! number of actors or coercing integer identities into lossy floats.

use crate::artifact::Digest;
use crate::native_actor_view::{
    ActorViewChannelStatus, ActorViewObservationPhase, NativeActorRelation, NativeEpisodeActorView,
    NativePlayerRelationshipEdge, NativePlayerRelationshipRole,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const NATIVE_ACTOR_FEATURE_VIEW_SCHEMA_V3: &str = "dusklight-native-actor-feature-view/v3";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorFeatureFamily {
    Identity,
    AbsoluteMotion,
    BaseLifecycle,
    BasePhysics,
    LinkRelative,
    CameraRelative,
    ParentRelative,
    Attention,
    EventParticipation,
    EnemyBase,
    GoalRelative,
    PlayerRelationships,
}

impl ActorFeatureFamily {
    const ALL: [Self; 12] = [
        Self::Identity,
        Self::AbsoluteMotion,
        Self::BaseLifecycle,
        Self::BasePhysics,
        Self::LinkRelative,
        Self::CameraRelative,
        Self::ParentRelative,
        Self::Attention,
        Self::EventParticipation,
        Self::EnemyBase,
        Self::GoalRelative,
        Self::PlayerRelationships,
    ];
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorFeatureSpec {
    pub families: Vec<ActorFeatureFamily>,
}

impl ActorFeatureSpec {
    pub fn all() -> Self {
        Self {
            families: ActorFeatureFamily::ALL.into(),
        }
    }

    pub fn new(
        families: impl IntoIterator<Item = ActorFeatureFamily>,
    ) -> Result<Self, NativeActorFeatureError> {
        let families = families
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let spec = Self { families };
        spec.validate()?;
        Ok(spec)
    }

    pub fn contains(&self, family: ActorFeatureFamily) -> bool {
        self.families.binary_search(&family).is_ok()
    }

    fn validate(&self) -> Result<(), NativeActorFeatureError> {
        if self.families.is_empty() || self.families.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(NativeActorFeatureError::new(
                "actor feature families must be nonempty, unique, and canonical",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorFeatureColumns {
    pub categorical: Vec<String>,
    pub continuous: Vec<String>,
    pub binary: Vec<String>,
    pub goal_anchor_count: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeActorFeatureRow {
    /// Structural provenance only; deliberately excluded from feature columns.
    pub runtime_generation: u64,
    pub parent_runtime_generation: u32,
    pub profile_slots: Vec<u32>,
    pub categorical: Vec<i64>,
    pub categorical_present: Vec<bool>,
    pub continuous: Vec<f32>,
    pub continuous_present: Vec<bool>,
    pub binary: Vec<bool>,
    pub binary_present: Vec<bool>,
    pub goal_relative_positions: Vec<Option<[f32; 3]>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeActorFeatureObservation {
    pub episode_id: String,
    pub step_index: u32,
    pub phase: ActorViewObservationPhase,
    pub boundary_index: u64,
    pub state_identity_xxh3_128: String,
    pub stage: String,
    pub room: i8,
    pub player_present: bool,
    pub camera_frame_present: bool,
    pub player_relationships_status: Option<ActorViewChannelStatus>,
    pub actors: Vec<NativeActorFeatureRow>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeActorFeatureView {
    pub schema: String,
    pub source_actor_view_sha256: Digest,
    pub spec: ActorFeatureSpec,
    pub columns: ActorFeatureColumns,
    pub feature_schema_sha256: Digest,
    pub observations: Vec<NativeActorFeatureObservation>,
    pub view_sha256: Digest,
}

impl NativeActorFeatureView {
    pub fn build(
        source: &NativeEpisodeActorView,
        spec: ActorFeatureSpec,
    ) -> Result<Self, NativeActorFeatureError> {
        source
            .validate()
            .map_err(|error| NativeActorFeatureError::new(error.to_string()))?;
        spec.validate()?;
        let goal_anchor_count = source
            .observations
            .first()
            .map_or(0, |observation| observation.goal_anchors.len());
        if source
            .observations
            .iter()
            .any(|observation| observation.goal_anchors.len() != goal_anchor_count)
        {
            return Err(NativeActorFeatureError::new(
                "actor source has inconsistent goal-anchor widths",
            ));
        }
        let columns = columns_for(&spec, goal_anchor_count);
        let feature_schema_sha256 = feature_schema_digest(&spec, &columns)?;
        let observations = source
            .observations
            .iter()
            .map(|observation| NativeActorFeatureObservation {
                episode_id: observation.episode_id.clone(),
                step_index: observation.step_index,
                phase: observation.phase,
                boundary_index: observation.boundary_index,
                state_identity_xxh3_128: observation.state_identity_xxh3_128.clone(),
                stage: observation.stage.clone(),
                room: observation.room,
                player_present: observation.player_present,
                camera_frame_present: observation.camera_frame_present,
                player_relationships_status: spec
                    .contains(ActorFeatureFamily::PlayerRelationships)
                    .then_some(observation.player_relationships_status),
                actors: observation
                    .actors
                    .iter()
                    .map(|actor| {
                        materialize_actor(
                            actor,
                            &spec,
                            observation.player_relationships_status,
                            &observation.player_relationships,
                        )
                    })
                    .collect(),
            })
            .collect();
        let mut view = Self {
            schema: NATIVE_ACTOR_FEATURE_VIEW_SCHEMA_V3.into(),
            source_actor_view_sha256: source.view_sha256,
            spec,
            columns,
            feature_schema_sha256,
            observations,
            view_sha256: Digest::ZERO,
        };
        view.view_sha256 = view.compute_identity()?;
        view.validate()?;
        Ok(view)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, NativeActorFeatureError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| NativeActorFeatureError::new(error.to_string()))
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, NativeActorFeatureError> {
        let view: Self = serde_json::from_slice(bytes)
            .map_err(|error| NativeActorFeatureError::new(error.to_string()))?;
        view.validate()?;
        if view.canonical_bytes()? != bytes {
            return Err(NativeActorFeatureError::new(
                "native actor feature bytes are not canonical",
            ));
        }
        Ok(view)
    }

    pub fn validate(&self) -> Result<(), NativeActorFeatureError> {
        self.spec.validate()?;
        let expected_columns = columns_for(&self.spec, self.columns.goal_anchor_count);
        if self.schema != NATIVE_ACTOR_FEATURE_VIEW_SCHEMA_V3
            || self.source_actor_view_sha256 == Digest::ZERO
            || self.observations.is_empty()
            || self.columns != expected_columns
            || self.feature_schema_sha256 != feature_schema_digest(&self.spec, &self.columns)?
            || self.view_sha256 != self.compute_identity()?
        {
            return Err(NativeActorFeatureError::new(
                "native actor feature envelope or seal is invalid",
            ));
        }
        for observation in &self.observations {
            if observation.episode_id.is_empty()
                || observation.stage.is_empty()
                || !is_lower_hex(&observation.state_identity_xxh3_128, 32)
                || observation
                    .actors
                    .windows(2)
                    .any(|pair| pair[0].runtime_generation >= pair[1].runtime_generation)
            {
                return Err(NativeActorFeatureError::new(
                    "native actor feature observation is invalid",
                ));
            }
            for actor in &observation.actors {
                validate_actor(actor, &self.columns)?;
            }
            validate_player_relationship_features(observation, &self.spec, &self.columns)?;
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, NativeActorFeatureError> {
        let mut canonical = self.clone();
        canonical.view_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.native-actor-feature-view/v3\0", &canonical)
    }
}

fn columns_for(spec: &ActorFeatureSpec, goal_anchor_count: usize) -> ActorFeatureColumns {
    let mut categorical = Vec::new();
    let mut continuous = Vec::new();
    let mut binary = Vec::new();
    if spec.contains(ActorFeatureFamily::Identity) {
        extend_names(
            &mut categorical,
            &[
                "actor_name",
                "profile_name",
                "group",
                "set_id",
                "home_room",
                "current_room",
                "argument",
                "parameters",
                "status",
            ],
        );
        continuous.push("health".into());
    }
    if spec.contains(ActorFeatureFamily::AbsoluteMotion) {
        extend_vec3_names(&mut continuous, "absolute_position");
        extend_vec3_names(&mut continuous, "absolute_home_position");
        extend_vec3_names(&mut continuous, "absolute_velocity");
        continuous.push("forward_speed".into());
        extend_vec3_names(&mut continuous, "current_angle_s16");
        extend_vec3_names(&mut continuous, "shape_angle_s16");
    }
    if spec.contains(ActorFeatureFamily::BaseLifecycle) {
        extend_names(
            &mut categorical,
            &[
                "actor_type",
                "process_subtype",
                "condition",
                "old_room",
                "pause_flag",
                "process_init_state",
                "process_create_phase",
                "cull_type",
                "demo_actor_id",
                "carry_type",
            ],
        );
        extend_names(
            &mut binary,
            &["heap_present", "model_present", "joint_collision_present"],
        );
    }
    if spec.contains(ActorFeatureFamily::BasePhysics) {
        extend_vec3_names(&mut continuous, "absolute_old_position");
        extend_vec3_names(&mut continuous, "scale");
        continuous.push("gravity".into());
        continuous.push("max_fall_speed".into());
        extend_vec3_names(&mut continuous, "absolute_eye_position");
        extend_vec3_names(&mut continuous, "home_angle_s16");
        extend_vec3_names(&mut continuous, "old_angle_s16");
    }
    if spec.contains(ActorFeatureFamily::LinkRelative) {
        extend_vec3_names(&mut continuous, "link_relative_position");
        extend_vec3_names(&mut continuous, "link_relative_home_position");
        extend_vec3_names(&mut continuous, "link_relative_velocity");
        continuous.push("link_distance".into());
        extend_vec2_names(&mut continuous, "current_yaw_relative_to_link_sin_cos");
        extend_vec2_names(&mut continuous, "shape_yaw_relative_to_link_sin_cos");
    }
    if spec.contains(ActorFeatureFamily::CameraRelative) {
        extend_vec3_names(&mut continuous, "camera_relative_position");
        extend_vec3_names(&mut continuous, "camera_relative_home_position");
        extend_vec3_names(&mut continuous, "camera_relative_velocity");
        extend_vec2_names(&mut continuous, "current_yaw_relative_to_camera_sin_cos");
        extend_vec2_names(&mut continuous, "shape_yaw_relative_to_camera_sin_cos");
    }
    if spec.contains(ActorFeatureFamily::ParentRelative) {
        extend_vec3_names(&mut continuous, "parent_relative_position");
        extend_vec3_names(&mut continuous, "parent_relative_velocity");
    }
    if spec.contains(ActorFeatureFamily::Attention) {
        categorical.push("attention_flags".into());
        for index in 0..9 {
            categorical.push(format!("attention_distance_index_{index}"));
        }
        categorical.push("attention_auxiliary".into());
        extend_vec3_names(&mut continuous, "attention_absolute_position");
        extend_vec3_names(&mut continuous, "attention_link_relative_position");
        extend_vec3_names(&mut continuous, "attention_camera_relative_position");
    }
    if spec.contains(ActorFeatureFamily::EventParticipation) {
        extend_names(
            &mut categorical,
            &[
                "event_command",
                "event_condition",
                "event_id",
                "event_map_tool_id",
                "event_index",
            ],
        );
    }
    if spec.contains(ActorFeatureFamily::EnemyBase) {
        extend_names(&mut categorical, &["enemy_flags", "enemy_throw_mode"]);
        extend_vec3_names(&mut continuous, "enemy_absolute_down_position");
        extend_vec3_names(&mut continuous, "enemy_absolute_head_lock_position");
    }
    if spec.contains(ActorFeatureFamily::PlayerRelationships) {
        extend_names(
            &mut binary,
            &[
                "player_targeted_actor",
                "player_ride_actor",
                "player_held_item_actor",
                "player_grabbed_actor",
                "player_thrown_boomerang_actor",
                "player_copy_rod_actor",
                "player_hookshot_roof_wait_actor",
                "player_chain_grab_actor",
                "player_attention_hint_actor",
                "player_attention_catch_actor",
                "player_attention_look_actor",
            ],
        );
    }
    ActorFeatureColumns {
        categorical,
        continuous,
        binary,
        goal_anchor_count: if spec.contains(ActorFeatureFamily::GoalRelative) {
            goal_anchor_count
        } else {
            0
        },
    }
}

fn materialize_actor(
    actor: &NativeActorRelation,
    spec: &ActorFeatureSpec,
    player_relationships_status: ActorViewChannelStatus,
    player_relationships: &[NativePlayerRelationshipEdge],
) -> NativeActorFeatureRow {
    let mut row = NativeActorFeatureRow {
        runtime_generation: actor.runtime_generation,
        parent_runtime_generation: actor.parent_runtime_generation,
        profile_slots: actor.profile_slots.clone(),
        categorical: Vec::new(),
        categorical_present: Vec::new(),
        continuous: Vec::new(),
        continuous_present: Vec::new(),
        binary: Vec::new(),
        binary_present: Vec::new(),
        goal_relative_positions: Vec::new(),
    };
    if spec.contains(ActorFeatureFamily::Identity) {
        extend_categories(
            &mut row,
            &[
                i64::from(actor.actor_name),
                i64::from(actor.profile_name),
                i64::from(actor.group),
                i64::from(actor.set_id),
                i64::from(actor.home_room),
                i64::from(actor.current_room),
                i64::from(actor.argument),
                i64::from(actor.parameters),
                i64::from(actor.status),
            ],
            true,
        );
        push_continuous(&mut row, f32::from(actor.health), true);
    }
    if spec.contains(ActorFeatureFamily::AbsoluteMotion) {
        extend_continuous(&mut row, &actor.absolute_position, true);
        extend_continuous(&mut row, &actor.absolute_home_position, true);
        extend_continuous(&mut row, &actor.absolute_velocity, true);
        push_continuous(&mut row, actor.forward_speed, true);
        extend_angles(&mut row, actor.current_angle, true);
        extend_angles(&mut row, actor.shape_angle, true);
    }
    if spec.contains(ActorFeatureFamily::BaseLifecycle) {
        if let Some(base) = &actor.base_state {
            extend_categories(
                &mut row,
                &[
                    i64::from(base.actor_type),
                    i64::from(base.process_subtype),
                    i64::from(base.condition),
                    i64::from(base.old_room),
                    i64::from(base.pause_flag),
                    i64::from(base.process_init_state),
                    i64::from(base.process_create_phase),
                    i64::from(base.cull_type),
                    i64::from(base.demo_actor_id),
                    i64::from(base.carry_type),
                ],
                true,
            );
            extend_binary(
                &mut row,
                &[
                    base.heap_present,
                    base.model_present,
                    base.joint_collision_present,
                ],
                true,
            );
        } else {
            extend_categories(&mut row, &[0; 10], false);
            extend_binary(&mut row, &[false; 3], false);
        }
    }
    if spec.contains(ActorFeatureFamily::BasePhysics) {
        if let Some(base) = &actor.base_state {
            extend_continuous(&mut row, &base.absolute_old_position, true);
            extend_continuous(&mut row, &base.scale, true);
            push_continuous(&mut row, base.gravity, true);
            push_continuous(&mut row, base.max_fall_speed, true);
            extend_continuous(&mut row, &base.absolute_eye_position, true);
            extend_angles(&mut row, base.home_angle, true);
            extend_angles(&mut row, base.old_angle, true);
        } else {
            extend_continuous(&mut row, &[0.0; 17], false);
        }
    }
    if spec.contains(ActorFeatureFamily::LinkRelative) {
        extend_optional_vec3(&mut row, actor.link_relative_position);
        extend_optional_vec3(&mut row, actor.link_relative_home_position);
        extend_optional_vec3(&mut row, actor.link_relative_velocity);
        push_optional_scalar(&mut row, actor.link_distance);
        extend_optional_vec2(&mut row, actor.current_yaw_relative_to_link);
        extend_optional_vec2(&mut row, actor.shape_yaw_relative_to_link);
    }
    if spec.contains(ActorFeatureFamily::CameraRelative) {
        extend_optional_vec3(&mut row, actor.camera_relative_position);
        extend_optional_vec3(&mut row, actor.camera_relative_home_position);
        extend_optional_vec3(&mut row, actor.camera_relative_velocity);
        extend_optional_vec2(&mut row, actor.current_yaw_relative_to_camera);
        extend_optional_vec2(&mut row, actor.shape_yaw_relative_to_camera);
    }
    if spec.contains(ActorFeatureFamily::ParentRelative) {
        extend_optional_vec3(&mut row, actor.parent_relative_position);
        extend_optional_vec3(&mut row, actor.parent_relative_velocity);
    }
    if spec.contains(ActorFeatureFamily::Attention) {
        if let Some(attention) = &actor.attention {
            push_category(&mut row, i64::from(attention.flags), true);
            extend_categories(&mut row, &attention.distance_indices.map(i64::from), true);
            push_category(&mut row, i64::from(attention.auxiliary), true);
            extend_continuous(&mut row, &attention.absolute_position, true);
            extend_optional_vec3(&mut row, attention.link_relative_position);
            extend_optional_vec3(&mut row, attention.camera_relative_position);
        } else {
            extend_categories(&mut row, &[0; 11], false);
            extend_continuous(&mut row, &[0.0; 9], false);
        }
    }
    if spec.contains(ActorFeatureFamily::EventParticipation) {
        if let Some(event) = &actor.event_participation {
            extend_categories(
                &mut row,
                &[
                    i64::from(event.command),
                    i64::from(event.condition),
                    i64::from(event.event_id),
                    i64::from(event.map_tool_id),
                    i64::from(event.index),
                ],
                true,
            );
        } else {
            extend_categories(&mut row, &[0; 5], false);
        }
    }
    if spec.contains(ActorFeatureFamily::EnemyBase) {
        if let Some(enemy) = &actor.enemy_base {
            extend_categories(
                &mut row,
                &[i64::from(enemy.flags), i64::from(enemy.throw_mode)],
                true,
            );
            extend_continuous(&mut row, &enemy.absolute_down_position, true);
            extend_continuous(&mut row, &enemy.absolute_head_lock_position, true);
        } else {
            extend_categories(&mut row, &[0; 2], false);
            extend_continuous(&mut row, &[0.0; 6], false);
        }
    }
    if spec.contains(ActorFeatureFamily::GoalRelative) {
        row.goal_relative_positions = actor.goal_relative_positions.clone();
    }
    if spec.contains(ActorFeatureFamily::PlayerRelationships) {
        let present = player_relationships_status == ActorViewChannelStatus::Present;
        for role in NativePlayerRelationshipRole::ALL {
            let related = player_relationships.iter().any(|edge| {
                edge.role == role && edge.actor_runtime_generation == actor.runtime_generation
            });
            push_binary(&mut row, related, present);
        }
    }
    row
}

fn validate_player_relationship_features(
    observation: &NativeActorFeatureObservation,
    spec: &ActorFeatureSpec,
    columns: &ActorFeatureColumns,
) -> Result<(), NativeActorFeatureError> {
    let enabled = spec.contains(ActorFeatureFamily::PlayerRelationships);
    if enabled != observation.player_relationships_status.is_some() {
        return Err(NativeActorFeatureError::new(
            "player-relationship feature status disagrees with its feature family",
        ));
    }
    if !enabled {
        return Ok(());
    }
    let first = columns
        .binary
        .iter()
        .position(|name| name == "player_targeted_actor")
        .ok_or_else(|| NativeActorFeatureError::new("player-relationship columns are absent"))?;
    let expected_present =
        observation.player_relationships_status == Some(ActorViewChannelStatus::Present);
    for role_offset in 0..NativePlayerRelationshipRole::ALL.len() {
        let column = first + role_offset;
        let mut related_count = 0usize;
        for actor in &observation.actors {
            if actor.binary_present[column] != expected_present {
                return Err(NativeActorFeatureError::new(
                    "player-relationship feature mask disagrees with channel status",
                ));
            }
            related_count += usize::from(actor.binary[column]);
        }
        if related_count > 1 {
            return Err(NativeActorFeatureError::new(
                "player-relationship role selects multiple actors",
            ));
        }
    }
    Ok(())
}

fn validate_actor(
    actor: &NativeActorFeatureRow,
    columns: &ActorFeatureColumns,
) -> Result<(), NativeActorFeatureError> {
    let canonical_absence = actor
        .categorical
        .iter()
        .zip(&actor.categorical_present)
        .all(|(value, present)| *present || *value == 0)
        && actor
            .continuous
            .iter()
            .zip(&actor.continuous_present)
            .all(|(value, present)| *present || *value == 0.0)
        && actor
            .binary
            .iter()
            .zip(&actor.binary_present)
            .all(|(value, present)| *present || !*value);
    if actor.runtime_generation == 0
        || actor.profile_slots.is_empty()
        || actor
            .profile_slots
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || actor.categorical.len() != columns.categorical.len()
        || actor.categorical_present.len() != columns.categorical.len()
        || actor.continuous.len() != columns.continuous.len()
        || actor.continuous_present.len() != columns.continuous.len()
        || actor.binary.len() != columns.binary.len()
        || actor.binary_present.len() != columns.binary.len()
        || actor.goal_relative_positions.len() != columns.goal_anchor_count
        || actor.continuous.iter().any(|value| !value.is_finite())
        || actor
            .goal_relative_positions
            .iter()
            .flatten()
            .flatten()
            .any(|value| !value.is_finite())
        || !canonical_absence
    {
        return Err(NativeActorFeatureError::new(
            "native actor feature row is invalid",
        ));
    }
    Ok(())
}

fn feature_schema_digest(
    spec: &ActorFeatureSpec,
    columns: &ActorFeatureColumns,
) -> Result<Digest, NativeActorFeatureError> {
    canonical_digest(
        b"dusklight.native-actor-feature-schema/v2\0",
        &(spec, columns),
    )
}

fn canonical_digest<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Digest, NativeActorFeatureError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| NativeActorFeatureError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

fn extend_names(output: &mut Vec<String>, values: &[&str]) {
    output.extend(values.iter().map(|value| (*value).into()));
}

fn extend_vec3_names(output: &mut Vec<String>, prefix: &str) {
    output.extend(["x", "y", "z"].map(|axis| format!("{prefix}_{axis}")));
}

fn extend_vec2_names(output: &mut Vec<String>, prefix: &str) {
    output.extend(["0", "1"].map(|axis| format!("{prefix}_{axis}")));
}

fn push_category(row: &mut NativeActorFeatureRow, value: i64, present: bool) {
    row.categorical.push(if present { value } else { 0 });
    row.categorical_present.push(present);
}

fn extend_categories(row: &mut NativeActorFeatureRow, values: &[i64], present: bool) {
    for value in values {
        push_category(row, *value, present);
    }
}

fn push_continuous(row: &mut NativeActorFeatureRow, value: f32, present: bool) {
    row.continuous.push(if present { value } else { 0.0 });
    row.continuous_present.push(present);
}

fn extend_continuous(row: &mut NativeActorFeatureRow, values: &[f32], present: bool) {
    for value in values {
        push_continuous(row, *value, present);
    }
}

fn extend_angles(row: &mut NativeActorFeatureRow, values: [i16; 3], present: bool) {
    extend_continuous(row, &values.map(f32::from), present);
}

fn extend_binary(row: &mut NativeActorFeatureRow, values: &[bool], present: bool) {
    row.binary
        .extend(values.iter().map(|value| present && *value));
    row.binary_present
        .extend(std::iter::repeat_n(present, values.len()));
}

fn push_binary(row: &mut NativeActorFeatureRow, value: bool, present: bool) {
    row.binary.push(present && value);
    row.binary_present.push(present);
}

fn extend_optional_vec3(row: &mut NativeActorFeatureRow, value: Option<[f32; 3]>) {
    extend_continuous(row, &value.unwrap_or_default(), value.is_some());
}

fn extend_optional_vec2(row: &mut NativeActorFeatureRow, value: Option<[f32; 2]>) {
    extend_continuous(row, &value.unwrap_or_default(), value.is_some());
}

fn push_optional_scalar(row: &mut NativeActorFeatureRow, value: Option<f32>) {
    push_continuous(row, value.unwrap_or_default(), value.is_some());
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeActorFeatureError(String);

impl NativeActorFeatureError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeActorFeatureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeActorFeatureError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_actor_view::NativeEpisodeActorView;
    use crate::trainable_set_encoder::TypedSetSample;
    use dusklight_evidence::native_episode_shard::{NativeEpisodeShard, NativeLearningObservation};
    use dusklight_world::actor_profile_catalog::{
        ACTOR_PROFILE_CATALOG_SCHEMA, ActorProfileCatalog, ActorProfileEntry,
    };

    fn catalog_for(shard: &NativeEpisodeShard) -> ActorProfileCatalog {
        let mut names = shard
            .episodes
            .iter()
            .flat_map(|episode| &episode.steps)
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
            .flat_map(|observation: &NativeLearningObservation| &observation.actors)
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

    fn actor_view(fixture: &[u8]) -> NativeEpisodeActorView {
        let mut shard = NativeEpisodeShard::decode(fixture).unwrap();
        shard.episodes.truncate(1);
        shard.episodes[0].steps.truncate(1);
        let catalog = catalog_for(&shard);
        shard.metadata.actor_profile_catalog_identity = Some(catalog.identity.clone());
        NativeEpisodeActorView::build(&shard, &catalog).unwrap()
    }

    fn categorical(view: &NativeActorFeatureView, name: &str) -> usize {
        view.columns
            .categorical
            .iter()
            .position(|column| column == name)
            .unwrap()
    }

    fn continuous(view: &NativeActorFeatureView, name: &str) -> usize {
        view.columns
            .continuous
            .iter()
            .position(|column| column == name)
            .unwrap()
    }

    fn binary(view: &NativeActorFeatureView, name: &str) -> usize {
        view.columns
            .binary
            .iter()
            .position(|column| column == name)
            .unwrap()
    }

    #[test]
    fn all_families_preserve_v7_types_values_masks_and_round_trip() {
        let source = actor_view(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v7.dseps"
        ));
        let view = NativeActorFeatureView::build(&source, ActorFeatureSpec::all()).unwrap();
        assert_eq!(view.observations.len(), 2);
        assert_eq!(view.source_actor_view_sha256, source.view_sha256);
        assert_ne!(view.feature_schema_sha256, Digest::ZERO);
        let actor = &view.observations[0].actors[0];

        let actor_type = categorical(&view, "actor_type");
        assert_eq!(actor.categorical[actor_type], 5);
        assert!(actor.categorical_present[actor_type]);
        let gravity = continuous(&view, "gravity");
        assert_eq!(actor.continuous[gravity], -3.0);
        assert!(actor.continuous_present[gravity]);
        let heap = binary(&view, "heap_present");
        assert!(actor.binary[heap]);
        assert!(actor.binary_present[heap]);

        let bytes = view.canonical_bytes().unwrap();
        assert_eq!(
            NativeActorFeatureView::decode_canonical(&bytes).unwrap(),
            view
        );
    }

    #[test]
    fn sealed_feature_observation_materializes_a_complete_trainable_set() {
        let source = actor_view(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v7.dseps"
        ));
        let view = NativeActorFeatureView::build(&source, ActorFeatureSpec::all()).unwrap();
        let sample = TypedSetSample::from_native_actor_observation(
            &view,
            0,
            Digest([9; 32]),
            vec![1.0],
            vec![true],
            0.5,
        )
        .unwrap();
        assert_eq!(
            sample.actor_feature_schema_sha256,
            view.feature_schema_sha256
        );
        assert_eq!(sample.nodes.len(), view.observations[0].actors.len());
        assert_eq!(
            sample.nodes[0].categorical,
            view.observations[0].actors[0].categorical
        );
        assert_eq!(
            sample.nodes[0].continuous_present.len(),
            view.columns.continuous.len() + view.columns.goal_anchor_count * 3
        );
    }

    #[test]
    fn legacy_base_state_is_masked_without_fabricated_presence() {
        let source = actor_view(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v6.dseps"
        ));
        let view = NativeActorFeatureView::build(&source, ActorFeatureSpec::all()).unwrap();
        let actor = &view.observations[0].actors[0];
        let actor_type = categorical(&view, "actor_type");
        let gravity = continuous(&view, "gravity");
        let heap = binary(&view, "heap_present");
        assert_eq!(actor.categorical[actor_type], 0);
        assert!(!actor.categorical_present[actor_type]);
        assert_eq!(actor.continuous[gravity], 0.0);
        assert!(!actor.continuous_present[gravity]);
        assert!(!actor.binary[heap]);
        assert!(!actor.binary_present[heap]);

        assert_eq!(
            view.observations[0].player_relationships_status,
            Some(ActorViewChannelStatus::NotSampled)
        );
        let targeted = binary(&view, "player_targeted_actor");
        assert!(
            view.observations[0]
                .actors
                .iter()
                .all(|actor| !actor.binary[targeted] && !actor.binary_present[targeted])
        );
    }

    #[test]
    fn v15_enemy_base_is_selectable_and_legacy_values_are_masked() {
        let source = actor_view(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v15.dseps"
        ));
        let view = NativeActorFeatureView::build(
            &source,
            ActorFeatureSpec::new([ActorFeatureFamily::EnemyBase]).unwrap(),
        )
        .unwrap();
        let actor = &view.observations[0].actors[0];
        let flags = categorical(&view, "enemy_flags");
        let throw_mode = categorical(&view, "enemy_throw_mode");
        let down_x = continuous(&view, "enemy_absolute_down_position_x");
        let head_y = continuous(&view, "enemy_absolute_head_lock_position_y");
        assert_eq!(actor.categorical[flags], 0x89);
        assert_eq!(actor.categorical[throw_mode], 0x04);
        assert!(actor.categorical_present[flags]);
        assert_eq!(actor.continuous[down_x], 12.0);
        assert_eq!(actor.continuous[head_y], 7.0);
        assert!(actor.continuous_present[head_y]);

        let legacy = actor_view(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v10.dseps"
        ));
        let legacy = NativeActorFeatureView::build(
            &legacy,
            ActorFeatureSpec::new([ActorFeatureFamily::EnemyBase]).unwrap(),
        )
        .unwrap();
        let actor = &legacy.observations[0].actors[0];
        assert_eq!(actor.categorical[flags], 0);
        assert!(!actor.categorical_present[flags]);
        assert_eq!(actor.continuous[down_x], 0.0);
        assert!(!actor.continuous_present[down_x]);
    }

    #[test]
    fn v10_relationship_edges_become_masked_per_actor_role_features() {
        let source = actor_view(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v10.dseps"
        ));
        let view = NativeActorFeatureView::build(&source, ActorFeatureSpec::all()).unwrap();
        let observation = &view.observations[0];
        assert_eq!(
            observation.player_relationships_status,
            Some(ActorViewChannelStatus::Present)
        );
        let targeted = binary(&view, "player_targeted_actor");
        let ride = binary(&view, "player_ride_actor");
        for actor in &observation.actors {
            assert!(actor.binary_present[targeted]);
            assert!(actor.binary_present[ride]);
            assert_eq!(
                actor.binary[targeted],
                actor.runtime_generation == 7,
                "target role must join the exact actor generation"
            );
            assert!(!actor.binary[ride]);
        }

        let without_relationships = NativeActorFeatureView::build(
            &source,
            ActorFeatureSpec::new([ActorFeatureFamily::Identity]).unwrap(),
        )
        .unwrap();
        assert!(
            without_relationships.observations[0]
                .player_relationships_status
                .is_none()
        );
        assert!(
            !without_relationships
                .columns
                .binary
                .iter()
                .any(|name| name.starts_with("player_"))
        );
    }

    #[test]
    fn feature_families_are_declarative_schema_changes_not_detached_vectors() {
        let source = actor_view(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v7.dseps"
        ));
        let identity = NativeActorFeatureView::build(
            &source,
            ActorFeatureSpec::new([ActorFeatureFamily::Identity]).unwrap(),
        )
        .unwrap();
        let relational = NativeActorFeatureView::build(
            &source,
            ActorFeatureSpec::new([
                ActorFeatureFamily::Identity,
                ActorFeatureFamily::LinkRelative,
            ])
            .unwrap(),
        )
        .unwrap();
        assert_ne!(
            identity.feature_schema_sha256,
            relational.feature_schema_sha256
        );
        assert!(
            !identity
                .columns
                .continuous
                .iter()
                .any(|name| name.starts_with("link_"))
        );
        assert!(
            relational
                .columns
                .continuous
                .iter()
                .any(|name| name == "link_relative_position_x")
        );
        assert_eq!(
            identity.observations[0].actors.len(),
            source.observations[0].actors.len()
        );
        assert_eq!(
            relational.observations[0].actors.len(),
            source.observations[0].actors.len()
        );

        let mut tampered = relational.clone();
        tampered.observations[0].actors[0].continuous[0] += 1.0;
        assert!(tampered.validate().is_err());
    }
}
