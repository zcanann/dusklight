//! Shared typed queries over the learner-facing `FactSnapshot`.

use crate::artifact::Digest;
use crate::fact_snapshot::{
    ActorFactSnapshot, ByteBankFactSnapshot, FactAvailability, FactSnapshot,
};
use dusklight_control::option_execution::OptionCondition;
use dusklight_objectives::milestone_dsl::{
    ActorFact, Comparison, Expression, Field, FlagDomain, FlagSelector, QueryFact, Value,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::cmp::Ordering;
use std::error::Error;
use std::f32::consts::{PI, TAU};
use std::fmt;

pub const FACT_REGISTRY_SCHEMA_V1: &str = "dusklight-fact-registry/v1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreFact {
    BoundaryIndex,
    SimulationTick,
    TapeFrame,
    StateIdentity,
    Stage,
    Room,
    Layer,
    Point,
    PlayerPresent,
    PlayerPosition,
    PlayerVelocity,
    PlayerProcedure,
    PlayerModeFlags,
    PlayerContacts,
    PlayerCameraYaw,
    EventRunning,
    TerminalReached,
    ActorCount,
    ActorsComplete,
    RecentOptionId,
    RecentOptionTicks,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", content = "query", rename_all = "snake_case")]
pub enum FactQuery {
    Core(CoreFact),
    MilestoneField(Field),
    MilestoneQuery(QueryFact),
    Condition(OptionCondition),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum FactValue {
    Bool(bool),
    U32(u32),
    U64(u64),
    I32(i32),
    F32Bits(u32),
    Symbol(String),
    Vec3F32Bits([u32; 3]),
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum FactRead {
    Available(FactValue),
    Absent,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "target", rename_all = "snake_case")]
pub enum MeasurementQuery {
    DistanceToActor(dusklight_automation_contracts::actor_identity::PlacedActorSelector),
    RelativeAngleToActor(dusklight_automation_contracts::actor_identity::PlacedActorSelector),
    RelativeVelocityToActor(dusklight_automation_contracts::actor_identity::PlacedActorSelector),
    ContactSurfaceChange,
    StateChange,
    EventChange,
    ElapsedTicks,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum MeasurementValue {
    F32Bits(u32),
    Vec3F32Bits([u32; 3]),
    ContactSurface(ContactSurfaceMeasurement),
    StateChange(StateChangeMeasurement),
    EventChange(EventChangeMeasurement),
    U64(u64),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum MeasurementRead {
    Available(MeasurementValue),
    Absent,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContactSurfaceMeasurement {
    pub before_contacts: u8,
    pub after_contacts: u8,
    pub activated_contacts: u8,
    pub cleared_contacts: u8,
    pub before_ground_clearance_f32_bits: Option<u32>,
    pub after_ground_clearance_f32_bits: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateChangeMeasurement {
    pub identity_changed: bool,
    pub stage_changed: bool,
    pub room_changed: bool,
    pub player_position_changed: bool,
    pub player_procedure_changed: bool,
    pub actor_population_changed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventChangeMeasurement {
    pub availability_changed: bool,
    pub running_changed: bool,
    pub event_id_changed: bool,
    pub event_name_changed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactRegistry {
    pub schema: String,
    pub core_queries: Vec<CoreFact>,
    pub schema_sha256: Digest,
}

impl FactRegistry {
    pub fn canonical() -> Self {
        let core_queries = vec![
            CoreFact::BoundaryIndex,
            CoreFact::SimulationTick,
            CoreFact::TapeFrame,
            CoreFact::StateIdentity,
            CoreFact::Stage,
            CoreFact::Room,
            CoreFact::Layer,
            CoreFact::Point,
            CoreFact::PlayerPresent,
            CoreFact::PlayerPosition,
            CoreFact::PlayerVelocity,
            CoreFact::PlayerProcedure,
            CoreFact::PlayerModeFlags,
            CoreFact::PlayerContacts,
            CoreFact::PlayerCameraYaw,
            CoreFact::EventRunning,
            CoreFact::TerminalReached,
            CoreFact::ActorCount,
            CoreFact::ActorsComplete,
            CoreFact::RecentOptionId,
            CoreFact::RecentOptionTicks,
        ];
        let schema_sha256 =
            Digest(Sha256::digest(serde_json::to_vec(&core_queries).unwrap()).into());
        Self {
            schema: FACT_REGISTRY_SCHEMA_V1.into(),
            core_queries,
            schema_sha256,
        }
    }

    pub fn validate(&self) -> Result<(), FactRegistryError> {
        let canonical = Self::canonical();
        if self.schema != canonical.schema
            || self.core_queries != canonical.core_queries
            || self.schema_sha256 != canonical.schema_sha256
        {
            return Err(FactRegistryError::InvalidRegistry);
        }
        Ok(())
    }

    pub fn read(
        &self,
        snapshot: &FactSnapshot,
        query: &FactQuery,
    ) -> Result<FactRead, FactRegistryError> {
        self.validate()?;
        snapshot
            .validate()
            .map_err(|error| FactRegistryError::InvalidSnapshot(error.to_string()))?;
        match query {
            FactQuery::Core(fact) => Ok(read_core(snapshot, *fact)),
            FactQuery::MilestoneField(field) => Ok(read_milestone_field(snapshot, *field)),
            FactQuery::MilestoneQuery(fact) => read_milestone_query(snapshot, fact),
            FactQuery::Condition(condition) => Ok(snapshot
                .conditions
                .iter()
                .find(|evaluation| &evaluation.condition == condition)
                .map_or(FactRead::Unavailable, |evaluation| {
                    evaluation.value.map_or(FactRead::Unavailable, |value| {
                        FactRead::Available(FactValue::Bool(value))
                    })
                })),
        }
    }

    pub fn available(
        &self,
        snapshot: &FactSnapshot,
        query: &FactQuery,
    ) -> Result<bool, FactRegistryError> {
        Ok(matches!(
            self.read(snapshot, query)?,
            FactRead::Available(_)
        ))
    }

    pub fn condition_available(
        &self,
        snapshot: &FactSnapshot,
        condition: &OptionCondition,
    ) -> Result<bool, FactRegistryError> {
        self.available(snapshot, &FactQuery::Condition(condition.clone()))
    }

    pub fn evaluate_expression(
        &self,
        snapshot: &FactSnapshot,
        expression: &Expression,
    ) -> Result<Option<bool>, FactRegistryError> {
        match expression {
            Expression::Compare {
                field,
                operator,
                value,
            } => self.evaluate_comparison(
                snapshot,
                &FactQuery::MilestoneField(*field),
                *operator,
                value,
            ),
            Expression::Query {
                fact,
                operator,
                value,
            } => self.evaluate_comparison(
                snapshot,
                &FactQuery::MilestoneQuery(fact.clone()),
                *operator,
                value,
            ),
            Expression::Not(child) => Ok(self
                .evaluate_expression(snapshot, child)?
                .map(|value| !value)),
            Expression::And(left, right) => combine_boolean(
                self.evaluate_expression(snapshot, left)?,
                self.evaluate_expression(snapshot, right)?,
                false,
            ),
            Expression::Or(left, right) => combine_boolean(
                self.evaluate_expression(snapshot, left)?,
                self.evaluate_expression(snapshot, right)?,
                true,
            ),
        }
    }

    pub fn measure(
        &self,
        before: &FactSnapshot,
        after: &FactSnapshot,
        query: &MeasurementQuery,
    ) -> Result<MeasurementRead, FactRegistryError> {
        self.validate()?;
        before
            .validate()
            .map_err(|error| FactRegistryError::InvalidSnapshot(error.to_string()))?;
        after
            .validate()
            .map_err(|error| FactRegistryError::InvalidSnapshot(error.to_string()))?;
        if after.simulation_tick < before.simulation_tick
            || after.boundary_index < before.boundary_index
        {
            return Err(FactRegistryError::InvalidMeasurementOrder);
        }
        match query {
            MeasurementQuery::DistanceToActor(selector) => {
                let Some(actor) = unique_actor(after, selector)? else {
                    return Ok(MeasurementRead::Absent);
                };
                let actor = actor.position_f32_bits.map(f32::from_bits);
                let player = after.player.position_f32_bits.map(f32::from_bits);
                let distance = ((actor[0] - player[0]).powi(2)
                    + (actor[1] - player[1]).powi(2)
                    + (actor[2] - player[2]).powi(2))
                .sqrt();
                finite_measurement(distance)
            }
            MeasurementQuery::RelativeAngleToActor(selector) => {
                let Some(actor) = unique_actor(after, selector)? else {
                    return Ok(MeasurementRead::Absent);
                };
                let Some(current_angle) = after.player.current_angle else {
                    return Ok(MeasurementRead::Unavailable);
                };
                let actor = actor.position_f32_bits.map(f32::from_bits);
                let player = after.player.position_f32_bits.map(f32::from_bits);
                let world_bearing = (actor[0] - player[0]).atan2(actor[2] - player[2]);
                let player_yaw = f32::from(current_angle[1]) * PI / 32768.0;
                let relative = (world_bearing - player_yaw + PI).rem_euclid(TAU) - PI;
                finite_measurement(relative)
            }
            MeasurementQuery::RelativeVelocityToActor(selector) => {
                let Some(actor) = unique_actor(after, selector)? else {
                    return Ok(MeasurementRead::Absent);
                };
                let (Some(actor_velocity), Some(player_velocity)) =
                    (actor.velocity_f32_bits, after.player.velocity_f32_bits)
                else {
                    return Ok(MeasurementRead::Unavailable);
                };
                let actor_velocity = actor_velocity.map(f32::from_bits);
                let player_velocity = player_velocity.map(f32::from_bits);
                let relative = [
                    actor_velocity[0] - player_velocity[0],
                    actor_velocity[1] - player_velocity[1],
                    actor_velocity[2] - player_velocity[2],
                ];
                if relative.iter().any(|value| !value.is_finite()) {
                    return Err(FactRegistryError::InvalidMeasurement);
                }
                Ok(MeasurementRead::Available(MeasurementValue::Vec3F32Bits(
                    relative.map(f32::to_bits),
                )))
            }
            MeasurementQuery::ContactSurfaceChange => {
                let (Some(before_contacts), Some(after_contacts)) =
                    (before.player.contacts, after.player.contacts)
                else {
                    return Ok(MeasurementRead::Unavailable);
                };
                Ok(MeasurementRead::Available(
                    MeasurementValue::ContactSurface(ContactSurfaceMeasurement {
                        before_contacts,
                        after_contacts,
                        activated_contacts: after_contacts & !before_contacts,
                        cleared_contacts: before_contacts & !after_contacts,
                        before_ground_clearance_f32_bits: ground_clearance(before)?,
                        after_ground_clearance_f32_bits: ground_clearance(after)?,
                    }),
                ))
            }
            MeasurementQuery::StateChange => Ok(MeasurementRead::Available(
                MeasurementValue::StateChange(StateChangeMeasurement {
                    identity_changed: before.state_identity != after.state_identity,
                    stage_changed: before.world.stage != after.world.stage,
                    room_changed: before.world.room != after.world.room,
                    player_position_changed: before.player.position_f32_bits
                        != after.player.position_f32_bits,
                    player_procedure_changed: before.player.procedure != after.player.procedure,
                    actor_population_changed: actor_population_identity(before)
                        != actor_population_identity(after),
                }),
            )),
            MeasurementQuery::EventChange => Ok(MeasurementRead::Available(
                MeasurementValue::EventChange(EventChangeMeasurement {
                    availability_changed: before.event.is_some() != after.event.is_some(),
                    running_changed: before.event.as_ref().map(|event| event.running)
                        != after.event.as_ref().map(|event| event.running),
                    event_id_changed: before.event.as_ref().map(|event| event.event_id)
                        != after.event.as_ref().map(|event| event.event_id),
                    event_name_changed: before.event.as_ref().and_then(|event| event.name_hash)
                        != after.event.as_ref().and_then(|event| event.name_hash),
                }),
            )),
            MeasurementQuery::ElapsedTicks => Ok(MeasurementRead::Available(
                MeasurementValue::U64(after.simulation_tick - before.simulation_tick),
            )),
        }
    }

    fn evaluate_comparison(
        &self,
        snapshot: &FactSnapshot,
        query: &FactQuery,
        operator: Comparison,
        expected: &Value,
    ) -> Result<Option<bool>, FactRegistryError> {
        let FactRead::Available(actual) = self.read(snapshot, query)? else {
            return Ok(None);
        };
        Ok(compare(&actual, operator, expected))
    }
}

fn unique_actor<'a>(
    snapshot: &'a FactSnapshot,
    selector: &dusklight_automation_contracts::actor_identity::PlacedActorSelector,
) -> Result<Option<&'a ActorFactSnapshot>, FactRegistryError> {
    selector
        .validate()
        .map_err(|_| FactRegistryError::InvalidQuery)?;
    let mut matches = snapshot
        .actors
        .iter()
        .filter(|actor| actor.portable_selector.as_ref() == Some(selector));
    let actor = matches.next();
    if matches.next().is_some() {
        return Err(FactRegistryError::AmbiguousActor);
    }
    Ok(actor)
}

fn finite_measurement(value: f32) -> Result<MeasurementRead, FactRegistryError> {
    if !value.is_finite() {
        return Err(FactRegistryError::InvalidMeasurement);
    }
    Ok(MeasurementRead::Available(MeasurementValue::F32Bits(
        value.to_bits(),
    )))
}

fn ground_clearance(snapshot: &FactSnapshot) -> Result<Option<u32>, FactRegistryError> {
    let Some(ground_height) = snapshot.player.ground_height_f32_bits else {
        return Ok(None);
    };
    let clearance =
        f32::from_bits(snapshot.player.position_f32_bits[1]) - f32::from_bits(ground_height);
    if !clearance.is_finite() {
        return Err(FactRegistryError::InvalidMeasurement);
    }
    Ok(Some(clearance.to_bits()))
}

fn actor_population_identity(snapshot: &FactSnapshot) -> Vec<(u64, i16, u16, i8)> {
    snapshot
        .actors
        .iter()
        .map(|actor| {
            (
                actor.runtime_generation,
                actor.actor_name,
                actor.set_id,
                actor.current_room,
            )
        })
        .collect()
}

fn read_core(snapshot: &FactSnapshot, fact: CoreFact) -> FactRead {
    use CoreFact as Core;
    match fact {
        Core::BoundaryIndex => available(FactValue::U64(snapshot.boundary_index)),
        Core::SimulationTick => available(FactValue::U64(snapshot.simulation_tick)),
        Core::TapeFrame => available(FactValue::U64(snapshot.tape_frame)),
        Core::StateIdentity => available(FactValue::Bytes(snapshot.state_identity.to_vec())),
        Core::Stage => available(FactValue::Symbol(snapshot.world.stage.clone())),
        Core::Room => available(FactValue::I32(snapshot.world.room.into())),
        Core::Layer => optional(
            snapshot
                .world
                .layer
                .map(|value| FactValue::I32(value.into())),
        ),
        Core::Point => optional(
            snapshot
                .world
                .point
                .map(|value| FactValue::I32(value.into())),
        ),
        Core::PlayerPresent => available(FactValue::Bool(snapshot.player.present)),
        Core::PlayerPosition => {
            available(FactValue::Vec3F32Bits(snapshot.player.position_f32_bits))
        }
        Core::PlayerVelocity => optional(
            snapshot
                .player
                .velocity_f32_bits
                .map(FactValue::Vec3F32Bits),
        ),
        Core::PlayerProcedure => optional(
            snapshot
                .player
                .procedure
                .map(|value| FactValue::U32(value.into())),
        ),
        Core::PlayerModeFlags => optional(snapshot.player.mode_flags.map(FactValue::U32)),
        Core::PlayerContacts => optional(
            snapshot
                .player
                .contacts
                .map(|value| FactValue::U32(value.into())),
        ),
        Core::PlayerCameraYaw => optional(
            snapshot
                .player
                .camera_yaw_radians_f32_bits
                .map(FactValue::F32Bits),
        ),
        Core::EventRunning => snapshot
            .event
            .as_ref()
            .map_or(FactRead::Unavailable, |event| {
                available(FactValue::Bool(event.running))
            }),
        Core::TerminalReached => optional(snapshot.terminal.reached.map(FactValue::Bool)),
        Core::ActorCount => available(FactValue::U32(snapshot.actors.len() as u32)),
        Core::ActorsComplete => available(FactValue::Bool(snapshot.actors_complete)),
        Core::RecentOptionId => snapshot
            .recent_option
            .as_ref()
            .map_or(FactRead::Absent, |option| {
                available(FactValue::Symbol(option.option_id.clone()))
            }),
        Core::RecentOptionTicks => snapshot
            .recent_option
            .as_ref()
            .map_or(FactRead::Absent, |option| {
                available(FactValue::U32(option.realized_ticks))
            }),
    }
}

fn read_milestone_field(snapshot: &FactSnapshot, field: Field) -> FactRead {
    let player = &snapshot.player;
    let event = snapshot.event.as_ref();
    use Field as F;
    match field {
        F::BoundaryKind => available(FactValue::U32(u32::from(snapshot.boundary_index != 0))),
        F::BoundaryIndex => available(FactValue::U64(snapshot.boundary_index)),
        F::TapeFrame => available(FactValue::U64(snapshot.tape_frame)),
        F::StageName => available(FactValue::Symbol(snapshot.world.stage.clone())),
        F::StageRoom => available(FactValue::I32(snapshot.world.room.into())),
        F::StageLayer => optional(
            snapshot
                .world
                .layer
                .map(|value| FactValue::I32(value.into())),
        ),
        F::StageSpawn => optional(
            snapshot
                .world
                .point
                .map(|value| FactValue::I32(value.into())),
        ),
        F::PlayerExists => available(FactValue::Bool(player.present)),
        F::PlayerPositionX => float_component(player.position_f32_bits, 0),
        F::PlayerPositionY => float_component(player.position_f32_bits, 1),
        F::PlayerPositionZ => float_component(player.position_f32_bits, 2),
        F::PlayerSpeed => optional(player.forward_speed_f32_bits.map(FactValue::F32Bits)),
        F::PlayerProcedure => optional(player.procedure.map(|value| FactValue::U32(value.into()))),
        F::EventRunning => event.map_or(FactRead::Unavailable, |value| {
            available(FactValue::Bool(value.running))
        }),
        F::EventId => event.map_or(FactRead::Unavailable, |value| {
            available(FactValue::I32(value.event_id.into()))
        }),
        F::NextStageName => optional(snapshot.world.next_stage.clone().map(FactValue::Symbol)),
        F::NextStageRoom => optional(
            snapshot
                .world
                .next_room
                .map(|value| FactValue::I32(value.into())),
        ),
        F::NextStageLayer => optional(
            snapshot
                .world
                .next_layer
                .map(|value| FactValue::I32(value.into())),
        ),
        F::NextStageSpawn => optional(
            snapshot
                .world
                .next_point
                .map(|value| FactValue::I32(value.into())),
        ),
        F::BoundaryReached => available(FactValue::Bool(true)),
        F::PlayerIsLink => optional(player.is_link.map(FactValue::Bool)),
        F::NextStageEnabled => available(FactValue::Bool(snapshot.world.next_stage.is_some())),
        F::PlayerProcessId => optional(player.process_id.map(FactValue::U32)),
        F::PlayerActorName => optional(player.actor_name.map(|value| FactValue::I32(value.into()))),
        F::PlayerVelocityX => optional_float_component(player.velocity_f32_bits, 0),
        F::PlayerVelocityY => optional_float_component(player.velocity_f32_bits, 1),
        F::PlayerVelocityZ => optional_float_component(player.velocity_f32_bits, 2),
        F::PlayerCurrentAngleX => optional_angle(player.current_angle, 0),
        F::PlayerCurrentAngleY => optional_angle(player.current_angle, 1),
        F::PlayerCurrentAngleZ => optional_angle(player.current_angle, 2),
        F::PlayerShapeAngleX => optional_angle(player.shape_angle, 0),
        F::PlayerShapeAngleY => optional_angle(player.shape_angle, 1),
        F::PlayerShapeAngleZ => optional_angle(player.shape_angle, 2),
        F::PlayerModeFlags => optional(player.mode_flags.map(FactValue::U32)),
        F::EventMode => event.map_or(FactRead::Unavailable, |value| {
            available(FactValue::U32(value.mode.into()))
        }),
        F::EventStatus => event.map_or(FactRead::Unavailable, |value| {
            available(FactValue::U32(value.status.into()))
        }),
        F::EventMapToolId => event.map_or(FactRead::Unavailable, |value| {
            available(FactValue::U32(value.map_tool_id.into()))
        }),
        F::EventNameHashPresent => event.map_or(FactRead::Unavailable, |value| {
            available(FactValue::Bool(value.name_hash.is_some()))
        }),
        F::EventNameHash => event.map_or(FactRead::Unavailable, |value| {
            optional(value.name_hash.map(FactValue::U32))
        }),
        F::CollisionGroundHeight => optional(player.ground_height_f32_bits.map(FactValue::F32Bits)),
        F::CollisionRoofHeight => optional(player.roof_height_f32_bits.map(FactValue::F32Bits)),
        _ => FactRead::Unavailable,
    }
}

fn read_milestone_query(
    snapshot: &FactSnapshot,
    fact: &QueryFact,
) -> Result<FactRead, FactRegistryError> {
    match fact {
        QueryFact::PlacedActor { selector, field } => read_actor(snapshot, selector, *field),
        QueryFact::Flag { selector } => read_flag(snapshot, selector),
        QueryFact::TemporaryEventByte { index } => {
            let bank = &snapshot.flag_banks.temporary_event;
            if bank.availability != FactAvailability::Available {
                return Ok(FactRead::Unavailable);
            }
            Ok(bank
                .bytes
                .get(usize::from(*index))
                .map_or(FactRead::Absent, |value| {
                    available(FactValue::U32((*value).into()))
                }))
        }
        QueryFact::PlayerInAabb { minimum, maximum } => {
            validate_vec3(*minimum)?;
            validate_vec3(*maximum)?;
            let position = snapshot.player.position_f32_bits.map(f32::from_bits);
            Ok(available(FactValue::Bool((0..3).all(|axis| {
                position[axis] >= minimum[axis] && position[axis] <= maximum[axis]
            }))))
        }
        QueryFact::PlayerPlaneSignedDistance { point, normal } => {
            validate_vec3(*point)?;
            validate_vec3(*normal)?;
            let length_squared =
                normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2];
            if !length_squared.is_finite() || length_squared <= f32::EPSILON {
                return Err(FactRegistryError::InvalidQuery);
            }
            let position = snapshot.player.position_f32_bits.map(f32::from_bits);
            let distance = ((position[0] - point[0]) * normal[0]
                + (position[1] - point[1]) * normal[1]
                + (position[2] - point[2]) * normal[2])
                / length_squared.sqrt();
            if !distance.is_finite() {
                return Err(FactRegistryError::InvalidQuery);
            }
            Ok(available(FactValue::F32Bits(distance.to_bits())))
        }
    }
}

fn read_actor(
    snapshot: &FactSnapshot,
    selector: &dusklight_automation_contracts::actor_identity::PlacedActorSelector,
    field: ActorFact,
) -> Result<FactRead, FactRegistryError> {
    selector
        .validate()
        .map_err(|_| FactRegistryError::InvalidQuery)?;
    let mut matches = snapshot
        .actors
        .iter()
        .filter(|actor| actor.portable_selector.as_ref() == Some(selector));
    let Some(actor) = matches.next() else {
        return Ok(if matches!(field, ActorFact::Exists) {
            available(FactValue::Bool(false))
        } else {
            FactRead::Absent
        });
    };
    if matches.next().is_some() {
        return Err(FactRegistryError::AmbiguousActor);
    }
    Ok(actor_value(snapshot, actor, field))
}

fn actor_value(snapshot: &FactSnapshot, actor: &ActorFactSnapshot, field: ActorFact) -> FactRead {
    match field {
        ActorFact::Exists => available(FactValue::Bool(true)),
        ActorFact::PositionX => float_component(actor.position_f32_bits, 0),
        ActorFact::PositionY => float_component(actor.position_f32_bits, 1),
        ActorFact::PositionZ => float_component(actor.position_f32_bits, 2),
        ActorFact::DistanceToPlayer => {
            let actor = actor.position_f32_bits.map(f32::from_bits);
            let player = snapshot.player.position_f32_bits.map(f32::from_bits);
            let distance = ((actor[0] - player[0]).powi(2)
                + (actor[1] - player[1]).powi(2)
                + (actor[2] - player[2]).powi(2))
            .sqrt();
            available(FactValue::F32Bits(distance.to_bits()))
        }
        ActorFact::CurrentRoom => available(FactValue::I32(actor.current_room.into())),
        ActorFact::Health => optional(actor.health.map(|value| FactValue::I32(value.into()))),
        ActorFact::Status => optional(actor.status.map(FactValue::U32)),
    }
}

fn read_flag(
    snapshot: &FactSnapshot,
    selector: &FlagSelector,
) -> Result<FactRead, FactRegistryError> {
    let (bank, room_matches): (&ByteBankFactSnapshot, bool) = match selector.domain {
        FlagDomain::Event => (&snapshot.flag_banks.event, true),
        FlagDomain::Temporary => (&snapshot.flag_banks.temporary, true),
        FlagDomain::Dungeon => (&snapshot.flag_banks.dungeon, true),
        FlagDomain::Switch => (
            &snapshot.flag_banks.switch,
            snapshot.flag_banks.switch_room == Some(selector.room),
        ),
    };
    if bank.availability != FactAvailability::Available || !room_matches {
        return Ok(FactRead::Unavailable);
    }
    let byte_index = usize::from(selector.index / 8);
    let bit = (selector.index % 8) as u8;
    Ok(bank.bytes.get(byte_index).map_or(FactRead::Absent, |byte| {
        available(FactValue::Bool(byte & (1 << bit) != 0))
    }))
}

fn compare(actual: &FactValue, operator: Comparison, expected: &Value) -> Option<bool> {
    match (actual, expected) {
        (FactValue::Bool(left), Value::Bool(right)) => ordered(*left, *right, operator),
        (FactValue::U32(left), Value::U32(right))
        | (FactValue::U32(left), Value::ProcedureNumber(right)) => {
            integer_compare(u64::from(*left), u64::from(*right), operator)
        }
        (FactValue::U64(left), Value::U64(right)) => integer_compare(*left, *right, operator),
        (FactValue::I32(left), Value::I32(right)) => {
            signed_compare(i64::from(*left), i64::from(*right), operator)
        }
        (FactValue::F32Bits(left), Value::F32(right)) if right.is_finite() => {
            float_compare(f32::from_bits(*left), *right, operator)
        }
        (FactValue::Symbol(left), Value::Symbol(right))
        | (FactValue::Symbol(left), Value::ProcedureSymbol(right)) => {
            ordered(left, right, operator)
        }
        _ => None,
    }
}

fn integer_compare(left: u64, right: u64, operator: Comparison) -> Option<bool> {
    Some(match operator {
        Comparison::HasAll => left & right == right,
        Comparison::HasAny => left & right != 0,
        _ => ordering_compare(left.cmp(&right), operator)?,
    })
}

fn signed_compare(left: i64, right: i64, operator: Comparison) -> Option<bool> {
    Some(match operator {
        Comparison::HasAll => left & right == right,
        Comparison::HasAny => left & right != 0,
        _ => ordering_compare(left.cmp(&right), operator)?,
    })
}

fn float_compare(left: f32, right: f32, operator: Comparison) -> Option<bool> {
    ordering_compare(left.partial_cmp(&right)?, operator)
}

fn ordered<T: Ord>(left: T, right: T, operator: Comparison) -> Option<bool> {
    ordering_compare(left.cmp(&right), operator)
}

fn ordering_compare(ordering: Ordering, operator: Comparison) -> Option<bool> {
    Some(match operator {
        Comparison::Equal => ordering == Ordering::Equal,
        Comparison::NotEqual => ordering != Ordering::Equal,
        Comparison::Less => ordering == Ordering::Less,
        Comparison::LessEqual => ordering != Ordering::Greater,
        Comparison::Greater => ordering == Ordering::Greater,
        Comparison::GreaterEqual => ordering != Ordering::Less,
        Comparison::HasAll | Comparison::HasAny => return None,
    })
}

fn combine_boolean(
    left: Option<bool>,
    right: Option<bool>,
    is_or: bool,
) -> Result<Option<bool>, FactRegistryError> {
    Ok(if is_or {
        match (left, right) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        }
    } else {
        match (left, right) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        }
    })
}

fn validate_vec3(values: [f32; 3]) -> Result<(), FactRegistryError> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(FactRegistryError::InvalidQuery)
    }
}

fn float_component(values: [u32; 3], index: usize) -> FactRead {
    available(FactValue::F32Bits(values[index]))
}

fn optional_float_component(values: Option<[u32; 3]>, index: usize) -> FactRead {
    optional(values.map(|values| FactValue::F32Bits(values[index])))
}

fn optional_angle(values: Option<[i16; 3]>, index: usize) -> FactRead {
    optional(values.map(|values| FactValue::I32(values[index].into())))
}

fn available(value: FactValue) -> FactRead {
    FactRead::Available(value)
}

fn optional(value: Option<FactValue>) -> FactRead {
    value.map_or(FactRead::Unavailable, FactRead::Available)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FactRegistryError {
    InvalidRegistry,
    InvalidSnapshot(String),
    InvalidQuery,
    AmbiguousActor,
    InvalidMeasurementOrder,
    InvalidMeasurement,
}

impl fmt::Display for FactRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRegistry => formatter.write_str("fact registry identity is invalid"),
            Self::InvalidSnapshot(message) => {
                write!(formatter, "fact snapshot is invalid: {message}")
            }
            Self::InvalidQuery => formatter.write_str("fact query is invalid"),
            Self::AmbiguousActor => formatter.write_str("fact query matched multiple actors"),
            Self::InvalidMeasurementOrder => {
                formatter.write_str("fact measurement snapshots are out of order")
            }
            Self::InvalidMeasurement => formatter.write_str("fact measurement is invalid"),
        }
    }
}

impl Error for FactRegistryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fact_snapshot::FactSnapshot;
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;

    fn snapshot() -> FactSnapshot {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap()
    }

    #[test]
    fn core_and_milestone_queries_share_one_snapshot() {
        let registry = FactRegistry::canonical();
        let snapshot = snapshot();
        assert_eq!(
            registry
                .read(&snapshot, &FactQuery::Core(CoreFact::Stage))
                .unwrap(),
            registry
                .read(&snapshot, &FactQuery::MilestoneField(Field::StageName))
                .unwrap()
        );
        let position = snapshot.player.position_f32_bits.map(f32::from_bits);
        assert_eq!(
            registry
                .read(
                    &snapshot,
                    &FactQuery::MilestoneQuery(QueryFact::PlayerInAabb {
                        minimum: position.map(|value| value - 1.0),
                        maximum: position.map(|value| value + 1.0),
                    }),
                )
                .unwrap(),
            FactRead::Available(FactValue::Bool(true))
        );
    }

    #[test]
    fn actor_queries_use_the_same_complete_actor_population() {
        let registry = FactRegistry::canonical();
        let snapshot = snapshot();
        let selector = snapshot.actors[0].portable_selector.clone().unwrap();
        assert_eq!(
            registry
                .read(
                    &snapshot,
                    &FactQuery::MilestoneQuery(QueryFact::PlacedActor {
                        selector,
                        field: ActorFact::Exists,
                    }),
                )
                .unwrap(),
            FactRead::Available(FactValue::Bool(true))
        );
    }

    #[test]
    fn goal_expressions_evaluate_without_a_private_fact_path() {
        let registry = FactRegistry::canonical();
        let snapshot = snapshot();
        let expression = Expression::Compare {
            field: Field::StageName,
            operator: Comparison::Equal,
            value: Value::Symbol(snapshot.world.stage.clone()),
        };
        assert_eq!(
            registry
                .evaluate_expression(&snapshot, &expression)
                .unwrap(),
            Some(true)
        );
    }

    #[test]
    fn generic_measurements_share_the_same_before_after_snapshots() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let step = &shard.episodes[0].steps[0];
        let before =
            FactSnapshot::from_native_learning(&step.pre_input, &[], None, Vec::new()).unwrap();
        let after =
            FactSnapshot::from_native_learning(&step.post_simulation, &[], None, Vec::new())
                .unwrap();
        let registry = FactRegistry::canonical();
        let selector = after.actors[0].portable_selector.clone().unwrap();

        for query in [
            MeasurementQuery::DistanceToActor(selector.clone()),
            MeasurementQuery::RelativeAngleToActor(selector.clone()),
            MeasurementQuery::RelativeVelocityToActor(selector),
            MeasurementQuery::ContactSurfaceChange,
            MeasurementQuery::StateChange,
            MeasurementQuery::EventChange,
            MeasurementQuery::ElapsedTicks,
        ] {
            assert!(matches!(
                registry.measure(&before, &after, &query).unwrap(),
                MeasurementRead::Available(_)
            ));
        }
        assert_eq!(
            registry
                .measure(&before, &after, &MeasurementQuery::ElapsedTicks)
                .unwrap(),
            MeasurementRead::Available(MeasurementValue::U64(
                after.simulation_tick - before.simulation_tick
            ))
        );
    }

    #[test]
    fn measurements_reject_reverse_time() {
        let snapshot = snapshot();
        let mut later = snapshot.clone();
        later.simulation_tick += 1;
        later.boundary_index += 1;
        assert_eq!(
            FactRegistry::canonical()
                .measure(&later, &snapshot, &MeasurementQuery::ElapsedTicks)
                .unwrap_err(),
            FactRegistryError::InvalidMeasurementOrder
        );
    }
}
