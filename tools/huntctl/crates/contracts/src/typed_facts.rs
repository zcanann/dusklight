//! Portable serialization for the bounded native typed-fact response.

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

pub const TYPED_FACT_RESPONSE_SCHEMA_V1: &str = "dusklight-typed-fact-response/v1";
pub const TYPED_FACT_RESPONSE_MAJOR_VERSION: u16 = 1;
pub const TYPED_FACT_RESPONSE_MINOR_VERSION: u16 = 1;
pub const TYPED_FACT_MAXIMUM_ENTRIES: usize = 16;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TypedFactPhase {
    PreInput,
    PostSimulation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TypedFactStatus {
    Present,
    Absent,
    Unavailable,
    Truncated,
    Invalid,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TypedFactId {
    StageName,
    StageRoom,
    StageSpawn,
    PlayerExists,
    PlayerIsLink,
    PlayerPosition,
    EventRunning,
    EventId,
    PlayerDoStatus,
    TalkPartner,
    GrabbedActor,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TypedFactValueType {
    Boolean,
    I32,
    U32,
    Vec3F32,
    StageCode,
    ActorIdentity,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TypedFactActorIdentity {
    pub runtime_generation: u32,
    pub actor_name: i16,
    pub set_id: u16,
    pub home_room: i8,
    pub current_room: i8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_position: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TypedFactValue {
    Boolean(bool),
    I32(i32),
    U32(u32),
    Vec3F32([f32; 3]),
    StageCode(String),
    ActorIdentity(TypedFactActorIdentity),
}

impl TypedFactValue {
    fn value_type(&self) -> TypedFactValueType {
        match self {
            Self::Boolean(_) => TypedFactValueType::Boolean,
            Self::I32(_) => TypedFactValueType::I32,
            Self::U32(_) => TypedFactValueType::U32,
            Self::Vec3F32(_) => TypedFactValueType::Vec3F32,
            Self::StageCode(_) => TypedFactValueType::StageCode,
            Self::ActorIdentity(_) => TypedFactValueType::ActorIdentity,
        }
    }

    fn canonical(&self) -> bool {
        match self {
            Self::Vec3F32(values) => values
                .iter()
                .all(|value| value.is_finite() && value.to_bits() != (-0.0_f32).to_bits()),
            Self::StageCode(value) => {
                !value.is_empty()
                    && value.len() <= 8
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            }
            Self::ActorIdentity(identity) => identity.home_position.is_none_or(|values| {
                values
                    .iter()
                    .all(|value| value.is_finite() && value.to_bits() != (-0.0_f32).to_bits())
            }),
            _ => true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TypedFactEntry {
    pub id: TypedFactId,
    pub status: TypedFactStatus,
    pub value_type: TypedFactValueType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<TypedFactValue>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TypedFactResponse {
    pub schema: String,
    pub major_version: u16,
    pub minor_version: u16,
    pub phase: TypedFactPhase,
    pub simulation_tick: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tape_frame: Option<u64>,
    pub entries: Vec<TypedFactEntry>,
}

impl TypedFactResponse {
    pub fn validate(&self) -> Result<(), TypedFactError> {
        if self.schema != TYPED_FACT_RESPONSE_SCHEMA_V1
            || self.major_version != TYPED_FACT_RESPONSE_MAJOR_VERSION
            || self.minor_version != TYPED_FACT_RESPONSE_MINOR_VERSION
            || self.entries.is_empty()
            || self.entries.len() > TYPED_FACT_MAXIMUM_ENTRIES
            || self.entries.windows(2).any(|pair| pair[0].id >= pair[1].id)
        {
            return Err(TypedFactError(
                "invalid typed-fact response envelope".into(),
            ));
        }
        for entry in &self.entries {
            let value_is_valid = match (&entry.status, &entry.value) {
                (TypedFactStatus::Present, Some(value)) => {
                    value.value_type() == entry.value_type && value.canonical()
                }
                (TypedFactStatus::Present, None) => false,
                (_, None) => true,
                (_, Some(_)) => false,
            };
            if !value_is_valid {
                return Err(TypedFactError(format!(
                    "typed fact {:?} contradicts its status or value type",
                    entry.id
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct TypedFactError(String);

impl fmt::Display for TypedFactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for TypedFactError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> TypedFactResponse {
        TypedFactResponse {
            schema: TYPED_FACT_RESPONSE_SCHEMA_V1.into(),
            major_version: 1,
            minor_version: TYPED_FACT_RESPONSE_MINOR_VERSION,
            phase: TypedFactPhase::PreInput,
            simulation_tick: 12,
            tape_frame: Some(11),
            entries: vec![
                TypedFactEntry {
                    id: TypedFactId::StageName,
                    status: TypedFactStatus::Present,
                    value_type: TypedFactValueType::StageCode,
                    value: Some(TypedFactValue::StageCode("F_SP104".into())),
                },
                TypedFactEntry {
                    id: TypedFactId::GrabbedActor,
                    status: TypedFactStatus::Absent,
                    value_type: TypedFactValueType::ActorIdentity,
                    value: None,
                },
            ],
        }
    }

    #[test]
    fn round_trips_explicit_presence_and_missingness() {
        let response = response();
        response.validate().unwrap();
        let bytes = serde_json::to_vec(&response).unwrap();
        let decoded: TypedFactResponse = serde_json::from_slice(&bytes).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn rejects_values_attached_to_missing_facts() {
        let mut response = response();
        response.entries[1].value = Some(TypedFactValue::ActorIdentity(TypedFactActorIdentity {
            runtime_generation: 1,
            actor_name: 2,
            set_id: 3,
            home_room: 4,
            current_room: 4,
            home_position: Some([1.0, 2.0, 3.0]),
        }));
        assert!(response.validate().is_err());
    }
}
