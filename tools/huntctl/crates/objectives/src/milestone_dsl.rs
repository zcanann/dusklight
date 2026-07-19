//! A bounded, deterministic milestone language for native per-tick evaluation.
//!
//! The source AST is intentionally suitable for visual editors. Compilation
//! produces canonical postfix bytecode: it has no jumps, loops, or mutable
//! state other than the evaluator-owned `stable` counter for each definition.

use serde::{Deserialize, Serialize};

pub use crate::actor_identity::PlacedActorSelector;
use crate::trace_typed_facts::typed_facts_from_trace_record;
use dusklight_automation_contracts::typed_facts::{
    TypedFactActorIdentity, TypedFactId, TypedFactResponse, TypedFactStatus, TypedFactValue,
};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const MAGIC: [u8; 4] = *b"DMSP";
pub const WIRE_VERSION: (u16, u16) = (1, 6);
pub const LANGUAGE_VERSION: (u16, u16) = (1, 6);
pub const MAX_DEFINITIONS: usize = 256;
pub const MAX_NAME_BYTES: usize = 96;
pub const MAX_SYMBOL_BYTES: usize = 64;
pub const MAX_OPS: usize = 256;
pub const MAX_PROJECTIONS: usize = 8;
pub const MAX_PROJECTION_ITEMS: usize = 32;
pub const MAX_EXPRESSION_DEPTH: usize = 32;
pub const MAX_BINARY_BYTES: usize = 1024 * 1024;

const DEFINITION_DOMAIN: &[u8] = b"dusklight.milestone.definition/v1\0";
const PROGRAM_DOMAIN: &[u8] = b"dusklight.milestone.program/v1\0";
const PROJECTION_DOMAIN: &[u8] = b"dusklight.value-projection.identity/v1\0";
const HEADER_BYTES: usize = 52;
const RECORD_FIXED_BYTES: usize = 44;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DslError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for DslError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(output, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl Error for DslError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryError(pub String);

impl fmt::Display for BinaryError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(&self.0)
    }
}

impl Error for BinaryError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LanguageVersion {
    pub major: u16,
    pub minor: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MilestoneProgram {
    pub version: LanguageVersion,
    pub definitions: Vec<MilestoneDefinition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MilestoneDefinition {
    pub name: String,
    pub phase: EvaluationPhase,
    /// Consecutive evaluations required before the milestone fires.
    pub stable_ticks: u16,
    pub when: Expression,
    /// Ordered predicates observed on strictly later evaluations.
    #[serde(default)]
    pub then: Vec<Expression>,
    /// Maximum matching-phase evaluations after the first step.
    #[serde(default)]
    pub within_ticks: Option<u16>,
    /// Named, exact value sets captured from the same observation as the hit.
    #[serde(default)]
    pub projections: Vec<ValueProjection>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValueProjection {
    pub name: String,
    pub items: Vec<ValueProjectionItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ValueProjectionItem {
    Rng { stream: RngStream },
    ActorPopulation { stage: String, room: i8 },
    Flag { selector: FlagSelector },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum RngStream {
    Primary = 0,
    Secondary = 1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationPhase {
    PreInput = 0,
    PostSim = 1,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "arguments", rename_all = "snake_case")]
pub enum Expression {
    Compare {
        field: Field,
        operator: Comparison,
        value: Value,
    },
    Query {
        fact: QueryFact,
        operator: Comparison,
        value: Value,
    },
    Not(Box<Expression>),
    And(Box<Expression>, Box<Expression>),
    Or(Box<Expression>, Box<Expression>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueryFact {
    PlacedActor {
        selector: PlacedActorSelector,
        field: ActorFact,
    },
    Flag {
        selector: FlagSelector,
    },
    PlayerInAabb {
        minimum: [f32; 3],
        maximum: [f32; 3],
    },
    PlayerPlaneSignedDistance {
        point: [f32; 3],
        normal: [f32; 3],
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum ActorFact {
    Exists = 1,
    PositionX = 2,
    PositionY = 3,
    PositionZ = 4,
    DistanceToPlayer = 5,
    CurrentRoom = 6,
    Health = 7,
    Status = 8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FlagSelector {
    pub domain: FlagDomain,
    pub room: i8,
    pub index: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum FlagDomain {
    Event = 0,
    Temporary = 1,
    Dungeon = 2,
    Switch = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    Equal = 0x20,
    NotEqual = 0x21,
    Less = 0x22,
    LessEqual = 0x23,
    Greater = 0x24,
    GreaterEqual = 0x25,
    HasAll = 0x26,
    HasAny = 0x27,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Value {
    Bool(bool),
    U32(u32),
    U64(u64),
    I32(i32),
    F32(f32),
    Symbol(String),
    ProcedureNumber(u32),
    ProcedureSymbol(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    BoundaryKind = 1,
    BoundaryIndex = 2,
    TapeFrame = 3,
    StageName = 4,
    StageRoom = 5,
    StageLayer = 6,
    StageSpawn = 7,
    PlayerExists = 8,
    PlayerPositionX = 9,
    PlayerPositionY = 10,
    PlayerPositionZ = 11,
    PlayerSpeed = 12,
    PlayerProcedure = 13,
    EventRunning = 14,
    EventId = 15,
    NextStageName = 16,
    NextStageRoom = 17,
    NextStageLayer = 18,
    NextStageSpawn = 19,
    BoundaryReached = 20,
    PlayerIsLink = 21,
    NextStageEnabled = 22,
    PlayerProcessId = 23,
    PlayerActorName = 24,
    PlayerVelocityX = 25,
    PlayerVelocityY = 26,
    PlayerVelocityZ = 27,
    PlayerCurrentAngleX = 28,
    PlayerCurrentAngleY = 29,
    PlayerCurrentAngleZ = 30,
    PlayerShapeAngleX = 31,
    PlayerShapeAngleY = 32,
    PlayerShapeAngleZ = 33,
    PlayerModeFlags = 34,
    PlayerDamageWaitTimer = 35,
    PlayerIceDamageWaitTimer = 36,
    PlayerSwordChangeWaitTimer = 37,
    EventMode = 38,
    EventStatus = 39,
    EventMapToolId = 40,
    EventNameHashPresent = 41,
    EventNameHash = 42,
    RngPrimaryState0 = 43,
    RngPrimaryState1 = 44,
    RngPrimaryState2 = 45,
    RngPrimaryCalls = 46,
    RngSecondaryState0 = 47,
    RngSecondaryState1 = 48,
    RngSecondaryState2 = 49,
    RngSecondaryCalls = 50,
    CollisionGroundContact = 51,
    CollisionWallContact = 52,
    CollisionRoofContact = 53,
    CollisionWaterContact = 54,
    CollisionWaterIn = 55,
    CollisionGroundHeight = 56,
    CollisionRoofHeight = 57,
    CollisionGroundClearance = 58,
    PlayerDoStatus = 59,
    TalkPartnerExists = 60,
    TalkPartnerActorName = 61,
    TalkPartnerSetId = 62,
    TalkPartnerHomeRoom = 63,
    TalkPartnerCurrentRoom = 64,
    GrabbedActorExists = 65,
    GrabbedActorActorName = 66,
    GrabbedActorSetId = 67,
    GrabbedActorHomeRoom = 68,
    GrabbedActorCurrentRoom = 69,
    TalkPartnerHomePositionX = 70,
    TalkPartnerHomePositionY = 71,
    TalkPartnerHomePositionZ = 72,
    GrabbedActorHomePositionX = 73,
    GrabbedActorHomePositionY = 74,
    GrabbedActorHomePositionZ = 75,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FieldType {
    Bool,
    U32,
    U64,
    I32,
    F32,
    Symbol,
    Enum,
    Procedure,
}

impl ActorFact {
    fn parse(path: &str) -> Option<Self> {
        Some(match path {
            "actor.placed.exists" => Self::Exists,
            "actor.placed.position.x" => Self::PositionX,
            "actor.placed.position.y" => Self::PositionY,
            "actor.placed.position.z" => Self::PositionZ,
            "actor.placed.distance_to_player" => Self::DistanceToPlayer,
            "actor.placed.current_room" => Self::CurrentRoom,
            "actor.placed.health" => Self::Health,
            "actor.placed.status" => Self::Status,
            _ => return None,
        })
    }

    fn path(self) -> &'static str {
        match self {
            Self::Exists => "actor.placed.exists",
            Self::PositionX => "actor.placed.position.x",
            Self::PositionY => "actor.placed.position.y",
            Self::PositionZ => "actor.placed.position.z",
            Self::DistanceToPlayer => "actor.placed.distance_to_player",
            Self::CurrentRoom => "actor.placed.current_room",
            Self::Health => "actor.placed.health",
            Self::Status => "actor.placed.status",
        }
    }

    fn field_type(self) -> FieldType {
        match self {
            Self::Exists => FieldType::Bool,
            Self::PositionX | Self::PositionY | Self::PositionZ | Self::DistanceToPlayer => {
                FieldType::F32
            }
            Self::CurrentRoom | Self::Health => FieldType::I32,
            Self::Status => FieldType::U32,
        }
    }

    fn from_id(id: u8) -> Option<Self> {
        Self::parse(match id {
            1 => "actor.placed.exists",
            2 => "actor.placed.position.x",
            3 => "actor.placed.position.y",
            4 => "actor.placed.position.z",
            5 => "actor.placed.distance_to_player",
            6 => "actor.placed.current_room",
            7 => "actor.placed.health",
            8 => "actor.placed.status",
            _ => return None,
        })
    }
}

impl FlagDomain {
    fn parse(path: &str) -> Option<Self> {
        Some(match path {
            "flag.event" => Self::Event,
            "flag.temporary" => Self::Temporary,
            "flag.dungeon" => Self::Dungeon,
            "flag.switch" => Self::Switch,
            _ => return None,
        })
    }

    fn path(self) -> &'static str {
        match self {
            Self::Event => "flag.event",
            Self::Temporary => "flag.temporary",
            Self::Dungeon => "flag.dungeon",
            Self::Switch => "flag.switch",
        }
    }

    fn from_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(Self::Event),
            1 => Some(Self::Temporary),
            2 => Some(Self::Dungeon),
            3 => Some(Self::Switch),
            _ => None,
        }
    }
}

impl QueryFact {
    fn field_type(&self) -> FieldType {
        match self {
            Self::PlacedActor { field, .. } => field.field_type(),
            Self::Flag { .. } => FieldType::Bool,
            Self::PlayerInAabb { .. } => FieldType::Bool,
            Self::PlayerPlaneSignedDistance { .. } => FieldType::F32,
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            Self::PlacedActor { field, .. } => field.path(),
            Self::Flag { selector } => selector.domain.path(),
            Self::PlayerInAabb { .. } => "player.in_aabb",
            Self::PlayerPlaneSignedDistance { .. } => "player.plane_signed_distance",
        }
    }
}

impl Field {
    pub fn path(self) -> &'static str {
        match self {
            Self::BoundaryKind => "boundary.kind",
            Self::BoundaryIndex => "boundary.index",
            Self::TapeFrame => "tape.frame",
            Self::StageName => "stage.name",
            Self::StageRoom => "stage.room",
            Self::StageLayer => "stage.layer",
            Self::StageSpawn => "stage.spawn",
            Self::PlayerExists => "player.exists",
            Self::PlayerPositionX => "player.position.x",
            Self::PlayerPositionY => "player.position.y",
            Self::PlayerPositionZ => "player.position.z",
            Self::PlayerSpeed => "player.speed",
            Self::PlayerProcedure => "player.procedure",
            Self::EventRunning => "event.running",
            Self::EventId => "event.id",
            Self::NextStageName => "next_stage.name",
            Self::NextStageRoom => "next_stage.room",
            Self::NextStageLayer => "next_stage.layer",
            Self::NextStageSpawn => "next_stage.spawn",
            Self::BoundaryReached => "boundary.reached",
            Self::PlayerIsLink => "player.is_link",
            Self::NextStageEnabled => "next_stage.enabled",
            Self::PlayerProcessId => "player.process_id",
            Self::PlayerActorName => "player.actor_name",
            Self::PlayerVelocityX => "player.velocity.x",
            Self::PlayerVelocityY => "player.velocity.y",
            Self::PlayerVelocityZ => "player.velocity.z",
            Self::PlayerCurrentAngleX => "player.current_angle.x",
            Self::PlayerCurrentAngleY => "player.current_angle.y",
            Self::PlayerCurrentAngleZ => "player.current_angle.z",
            Self::PlayerShapeAngleX => "player.shape_angle.x",
            Self::PlayerShapeAngleY => "player.shape_angle.y",
            Self::PlayerShapeAngleZ => "player.shape_angle.z",
            Self::PlayerModeFlags => "player.mode_flags",
            Self::PlayerDamageWaitTimer => "player.timer.damage_wait",
            Self::PlayerIceDamageWaitTimer => "player.timer.ice_damage_wait",
            Self::PlayerSwordChangeWaitTimer => "player.timer.sword_change_wait",
            Self::EventMode => "event.mode",
            Self::EventStatus => "event.status",
            Self::EventMapToolId => "event.map_tool_id",
            Self::EventNameHashPresent => "event.name_hash.present",
            Self::EventNameHash => "event.name_hash.fnv1a32",
            Self::RngPrimaryState0 => "rng.primary.state0",
            Self::RngPrimaryState1 => "rng.primary.state1",
            Self::RngPrimaryState2 => "rng.primary.state2",
            Self::RngPrimaryCalls => "rng.primary.calls",
            Self::RngSecondaryState0 => "rng.secondary.state0",
            Self::RngSecondaryState1 => "rng.secondary.state1",
            Self::RngSecondaryState2 => "rng.secondary.state2",
            Self::RngSecondaryCalls => "rng.secondary.calls",
            Self::CollisionGroundContact => "collision.ground.contact",
            Self::CollisionWallContact => "collision.wall.contact",
            Self::CollisionRoofContact => "collision.roof.contact",
            Self::CollisionWaterContact => "collision.water.contact",
            Self::CollisionWaterIn => "collision.water.in",
            Self::CollisionGroundHeight => "collision.ground.height",
            Self::CollisionRoofHeight => "collision.roof.height",
            Self::CollisionGroundClearance => "collision.ground.clearance",
            Self::PlayerDoStatus => "player.interaction.do_status",
            Self::TalkPartnerExists => "player.interaction.talk_partner.exists",
            Self::TalkPartnerActorName => "player.interaction.talk_partner.actor_name",
            Self::TalkPartnerSetId => "player.interaction.talk_partner.set_id",
            Self::TalkPartnerHomeRoom => "player.interaction.talk_partner.home_room",
            Self::TalkPartnerCurrentRoom => "player.interaction.talk_partner.current_room",
            Self::GrabbedActorExists => "player.interaction.grabbed_actor.exists",
            Self::GrabbedActorActorName => "player.interaction.grabbed_actor.actor_name",
            Self::GrabbedActorSetId => "player.interaction.grabbed_actor.set_id",
            Self::GrabbedActorHomeRoom => "player.interaction.grabbed_actor.home_room",
            Self::GrabbedActorCurrentRoom => "player.interaction.grabbed_actor.current_room",
            Self::TalkPartnerHomePositionX => "player.interaction.talk_partner.home_position.x",
            Self::TalkPartnerHomePositionY => "player.interaction.talk_partner.home_position.y",
            Self::TalkPartnerHomePositionZ => "player.interaction.talk_partner.home_position.z",
            Self::GrabbedActorHomePositionX => "player.interaction.grabbed_actor.home_position.x",
            Self::GrabbedActorHomePositionY => "player.interaction.grabbed_actor.home_position.y",
            Self::GrabbedActorHomePositionZ => "player.interaction.grabbed_actor.home_position.z",
        }
    }

    fn field_type(self) -> FieldType {
        match self {
            Self::BoundaryKind => FieldType::Enum,
            Self::BoundaryIndex
            | Self::TapeFrame
            | Self::RngPrimaryCalls
            | Self::RngSecondaryCalls => FieldType::U64,
            Self::PlayerProcessId
            | Self::PlayerModeFlags
            | Self::PlayerSwordChangeWaitTimer
            | Self::EventMode
            | Self::EventStatus
            | Self::EventMapToolId
            | Self::EventNameHash => FieldType::U32,
            Self::PlayerDoStatus | Self::TalkPartnerSetId | Self::GrabbedActorSetId => {
                FieldType::U32
            }
            Self::StageName | Self::NextStageName => FieldType::Symbol,
            Self::StageRoom
            | Self::StageLayer
            | Self::StageSpawn
            | Self::NextStageRoom
            | Self::NextStageLayer
            | Self::NextStageSpawn => FieldType::I32,
            Self::PlayerActorName
            | Self::PlayerCurrentAngleX
            | Self::PlayerCurrentAngleY
            | Self::PlayerCurrentAngleZ
            | Self::PlayerShapeAngleX
            | Self::PlayerShapeAngleY
            | Self::PlayerShapeAngleZ
            | Self::PlayerDamageWaitTimer
            | Self::PlayerIceDamageWaitTimer
            | Self::RngPrimaryState0
            | Self::RngPrimaryState1
            | Self::RngPrimaryState2
            | Self::RngSecondaryState0
            | Self::RngSecondaryState1
            | Self::RngSecondaryState2 => FieldType::I32,
            Self::TalkPartnerActorName
            | Self::TalkPartnerHomeRoom
            | Self::TalkPartnerCurrentRoom
            | Self::GrabbedActorActorName
            | Self::GrabbedActorHomeRoom
            | Self::GrabbedActorCurrentRoom => FieldType::I32,
            Self::PlayerExists
            | Self::EventRunning
            | Self::BoundaryReached
            | Self::PlayerIsLink
            | Self::NextStageEnabled => FieldType::Bool,
            Self::TalkPartnerExists | Self::GrabbedActorExists => FieldType::Bool,
            Self::EventNameHashPresent
            | Self::CollisionGroundContact
            | Self::CollisionWallContact
            | Self::CollisionRoofContact
            | Self::CollisionWaterContact
            | Self::CollisionWaterIn => FieldType::Bool,
            Self::PlayerPositionX
            | Self::PlayerPositionY
            | Self::PlayerPositionZ
            | Self::PlayerSpeed => FieldType::F32,
            Self::PlayerVelocityX
            | Self::PlayerVelocityY
            | Self::PlayerVelocityZ
            | Self::CollisionGroundHeight
            | Self::CollisionRoofHeight
            | Self::CollisionGroundClearance => FieldType::F32,
            Self::TalkPartnerHomePositionX
            | Self::TalkPartnerHomePositionY
            | Self::TalkPartnerHomePositionZ
            | Self::GrabbedActorHomePositionX
            | Self::GrabbedActorHomePositionY
            | Self::GrabbedActorHomePositionZ => FieldType::F32,
            Self::PlayerProcedure => FieldType::Procedure,
            Self::EventId => FieldType::I32,
        }
    }

    fn parse(path: &str) -> Option<Self> {
        (1..=75).find_map(|id| {
            let field = Self::from_id(id)?;
            (field.path() == path).then_some(field)
        })
    }

    fn from_id(id: u8) -> Option<Self> {
        Some(match id {
            1 => Self::BoundaryKind,
            2 => Self::BoundaryIndex,
            3 => Self::TapeFrame,
            4 => Self::StageName,
            5 => Self::StageRoom,
            6 => Self::StageLayer,
            7 => Self::StageSpawn,
            8 => Self::PlayerExists,
            9 => Self::PlayerPositionX,
            10 => Self::PlayerPositionY,
            11 => Self::PlayerPositionZ,
            12 => Self::PlayerSpeed,
            13 => Self::PlayerProcedure,
            14 => Self::EventRunning,
            15 => Self::EventId,
            16 => Self::NextStageName,
            17 => Self::NextStageRoom,
            18 => Self::NextStageLayer,
            19 => Self::NextStageSpawn,
            20 => Self::BoundaryReached,
            21 => Self::PlayerIsLink,
            22 => Self::NextStageEnabled,
            23 => Self::PlayerProcessId,
            24 => Self::PlayerActorName,
            25 => Self::PlayerVelocityX,
            26 => Self::PlayerVelocityY,
            27 => Self::PlayerVelocityZ,
            28 => Self::PlayerCurrentAngleX,
            29 => Self::PlayerCurrentAngleY,
            30 => Self::PlayerCurrentAngleZ,
            31 => Self::PlayerShapeAngleX,
            32 => Self::PlayerShapeAngleY,
            33 => Self::PlayerShapeAngleZ,
            34 => Self::PlayerModeFlags,
            35 => Self::PlayerDamageWaitTimer,
            36 => Self::PlayerIceDamageWaitTimer,
            37 => Self::PlayerSwordChangeWaitTimer,
            38 => Self::EventMode,
            39 => Self::EventStatus,
            40 => Self::EventMapToolId,
            41 => Self::EventNameHashPresent,
            42 => Self::EventNameHash,
            43 => Self::RngPrimaryState0,
            44 => Self::RngPrimaryState1,
            45 => Self::RngPrimaryState2,
            46 => Self::RngPrimaryCalls,
            47 => Self::RngSecondaryState0,
            48 => Self::RngSecondaryState1,
            49 => Self::RngSecondaryState2,
            50 => Self::RngSecondaryCalls,
            51 => Self::CollisionGroundContact,
            52 => Self::CollisionWallContact,
            53 => Self::CollisionRoofContact,
            54 => Self::CollisionWaterContact,
            55 => Self::CollisionWaterIn,
            56 => Self::CollisionGroundHeight,
            57 => Self::CollisionRoofHeight,
            58 => Self::CollisionGroundClearance,
            59 => Self::PlayerDoStatus,
            60 => Self::TalkPartnerExists,
            61 => Self::TalkPartnerActorName,
            62 => Self::TalkPartnerSetId,
            63 => Self::TalkPartnerHomeRoom,
            64 => Self::TalkPartnerCurrentRoom,
            65 => Self::GrabbedActorExists,
            66 => Self::GrabbedActorActorName,
            67 => Self::GrabbedActorSetId,
            68 => Self::GrabbedActorHomeRoom,
            69 => Self::GrabbedActorCurrentRoom,
            70 => Self::TalkPartnerHomePositionX,
            71 => Self::TalkPartnerHomePositionY,
            72 => Self::TalkPartnerHomePositionZ,
            73 => Self::GrabbedActorHomePositionX,
            74 => Self::GrabbedActorHomePositionY,
            75 => Self::GrabbedActorHomePositionZ,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledMilestones {
    pub bytes: Vec<u8>,
    pub program_sha256: [u8; 32],
    pub definitions: Vec<CompiledDefinitionIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledDefinitionIdentity {
    pub name: String,
    pub sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedMilestones {
    pub program: MilestoneProgram,
    pub program_sha256: [u8; 32],
    pub definitions: Vec<CompiledDefinitionIdentity>,
}

#[derive(Clone, Debug, PartialEq)]
enum TokenKind {
    Word(String),
    Number(String),
    String(String),
    LeftBrace,
    RightBrace,
    LeftParen,
    RightParen,
    Comma,
    Not,
    And,
    Or,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Eof,
}

#[derive(Clone, Debug)]
struct Token {
    kind: TokenKind,
    line: usize,
    column: usize,
}

/// Parse and validate milestone source without compiling it.
pub fn parse(source: &str) -> Result<MilestoneProgram, DslError> {
    Parser::new(lex(source)?).program()
}

fn lex(source: &str) -> Result<Vec<Token>, DslError> {
    let chars = source.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let (mut at, mut line, mut column) = (0, 1, 1);
    while at < chars.len() {
        let (start_line, start_column) = (line, column);
        match chars[at] {
            ' ' | '\t' | '\r' => {
                at += 1;
                column += 1;
            }
            '\n' => {
                at += 1;
                line += 1;
                column = 1;
            }
            '#' => {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                    column += 1;
                }
            }
            '/' if chars.get(at + 1) == Some(&'/') => {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                    column += 1;
                }
            }
            '"' => {
                at += 1;
                column += 1;
                let mut value = String::new();
                let mut closed = false;
                while at < chars.len() {
                    match chars[at] {
                        '"' => {
                            at += 1;
                            column += 1;
                            closed = true;
                            break;
                        }
                        '\n' | '\r' => {
                            return Err(DslError {
                                line: start_line,
                                column: start_column,
                                message: "unterminated string literal".into(),
                            });
                        }
                        '\\' => {
                            let escape = chars.get(at + 1).copied().ok_or_else(|| DslError {
                                line: start_line,
                                column: start_column,
                                message: "unterminated string escape".into(),
                            })?;
                            let decoded = match escape {
                                '"' => '"',
                                '\\' => '\\',
                                'n' => '\n',
                                'r' => '\r',
                                't' => '\t',
                                _ => {
                                    return Err(DslError {
                                        line,
                                        column,
                                        message: format!("unsupported string escape \\{escape}"),
                                    });
                                }
                            };
                            value.push(decoded);
                            at += 2;
                            column += 2;
                        }
                        character if character.is_control() => {
                            return Err(DslError {
                                line,
                                column,
                                message: "control character in string literal".into(),
                            });
                        }
                        character => {
                            value.push(character);
                            at += 1;
                            column += 1;
                        }
                    }
                }
                if !closed {
                    return Err(DslError {
                        line: start_line,
                        column: start_column,
                        message: "unterminated string literal".into(),
                    });
                }
                tokens.push(Token {
                    kind: TokenKind::String(value),
                    line: start_line,
                    column: start_column,
                });
            }
            '{' | '}' | '(' | ')' | ',' => {
                let kind = match chars[at] {
                    '{' => TokenKind::LeftBrace,
                    '}' => TokenKind::RightBrace,
                    '(' => TokenKind::LeftParen,
                    ')' => TokenKind::RightParen,
                    _ => TokenKind::Comma,
                };
                tokens.push(Token { kind, line, column });
                at += 1;
                column += 1;
            }
            '&' if chars.get(at + 1) == Some(&'&') => {
                tokens.push(Token {
                    kind: TokenKind::And,
                    line,
                    column,
                });
                at += 2;
                column += 2;
            }
            '|' if chars.get(at + 1) == Some(&'|') => {
                tokens.push(Token {
                    kind: TokenKind::Or,
                    line,
                    column,
                });
                at += 2;
                column += 2;
            }
            '=' if chars.get(at + 1) == Some(&'=') => {
                tokens.push(Token {
                    kind: TokenKind::Equal,
                    line,
                    column,
                });
                at += 2;
                column += 2;
            }
            '!' => {
                let (kind, width) = if chars.get(at + 1) == Some(&'=') {
                    (TokenKind::NotEqual, 2)
                } else {
                    (TokenKind::Not, 1)
                };
                tokens.push(Token { kind, line, column });
                at += width;
                column += width;
            }
            '<' | '>' => {
                let equal = chars.get(at + 1) == Some(&'=');
                let kind = match (chars[at], equal) {
                    ('<', false) => TokenKind::Less,
                    ('<', true) => TokenKind::LessEqual,
                    ('>', false) => TokenKind::Greater,
                    ('>', true) => TokenKind::GreaterEqual,
                    _ => unreachable!(),
                };
                let width = if equal { 2 } else { 1 };
                tokens.push(Token { kind, line, column });
                at += width;
                column += width;
            }
            character if character.is_ascii_digit() || character == '-' => {
                let start = at;
                at += 1;
                column += 1;
                while at < chars.len()
                    && (chars[at].is_ascii_alphanumeric()
                        || matches!(chars[at], '.' | '+' | '-' | '_'))
                {
                    at += 1;
                    column += 1;
                }
                tokens.push(Token {
                    kind: TokenKind::Number(chars[start..at].iter().collect()),
                    line: start_line,
                    column: start_column,
                });
            }
            character if character.is_ascii_alphabetic() || character == '_' => {
                let start = at;
                while at < chars.len()
                    && (chars[at].is_ascii_alphanumeric() || matches!(chars[at], '_' | '.' | '-'))
                {
                    at += 1;
                    column += 1;
                }
                tokens.push(Token {
                    kind: TokenKind::Word(chars[start..at].iter().collect()),
                    line: start_line,
                    column: start_column,
                });
            }
            character => {
                return Err(DslError {
                    line,
                    column,
                    message: format!("unexpected character {character:?}"),
                });
            }
        }
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        line,
        column,
    });
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, at: 0 }
    }

    fn program(mut self) -> Result<MilestoneProgram, DslError> {
        self.expect_word("milestones")?;
        let version_token = self.take();
        let version = match version_token.kind {
            TokenKind::Number(value) if value == "1.0" => LanguageVersion { major: 1, minor: 0 },
            TokenKind::Number(value) if value == "1.1" => LanguageVersion { major: 1, minor: 1 },
            TokenKind::Number(value) if value == "1.2" => LanguageVersion { major: 1, minor: 2 },
            TokenKind::Number(value) if value == "1.3" => LanguageVersion { major: 1, minor: 3 },
            TokenKind::Number(value) if value == "1.4" => LanguageVersion { major: 1, minor: 4 },
            TokenKind::Number(value) if value == "1.5" => LanguageVersion { major: 1, minor: 5 },
            TokenKind::Number(value) if value == "1.6" => LanguageVersion { major: 1, minor: 6 },
            _ => {
                return Err(self.at_error(
                    &version_token,
                    "unsupported or missing language version; expected 1.0 through 1.6",
                ));
            }
        };
        let mut definitions = Vec::new();
        let mut names = BTreeSet::new();
        while !matches!(self.peek().kind, TokenKind::Eof) {
            if definitions.len() == MAX_DEFINITIONS {
                return self.error(format!("more than {MAX_DEFINITIONS} milestones"));
            }
            self.expect_word("milestone")?;
            let name_token = self.take();
            let name = match name_token.kind.clone() {
                TokenKind::Word(value) | TokenKind::String(value) => value,
                _ => return Err(self.at_error(&name_token, "expected milestone name")),
            };
            validate_text(&name, MAX_NAME_BYTES, false)
                .map_err(|message| self.at_error(&name_token, message))?;
            if !names.insert(name.clone()) {
                return Err(self.at_error(&name_token, format!("duplicate milestone {name:?}")));
            }
            self.expect(TokenKind::LeftBrace, "expected `{` after milestone name")?;
            let mut phase = None;
            let mut stable_ticks = None;
            let mut when = None;
            let mut then = Vec::new();
            let mut within_ticks = None;
            let mut projections = Vec::new();
            while !self.consume(&TokenKind::RightBrace) {
                if matches!(self.peek().kind, TokenKind::Eof) {
                    return self.error("unterminated milestone block".into());
                }
                let key_token = self.take();
                let key = match &key_token.kind {
                    TokenKind::Word(value) => value.as_str(),
                    _ => return Err(self.at_error(&key_token, "expected milestone property")),
                };
                match key {
                    "phase" => {
                        if phase.is_some() {
                            return Err(self.at_error(&key_token, "duplicate phase property"));
                        }
                        let value = self.word()?;
                        phase = Some(match value.as_str() {
                            "pre_input" => EvaluationPhase::PreInput,
                            "post_sim" => EvaluationPhase::PostSim,
                            _ => return self.error(format!("unknown evaluation phase {value:?}")),
                        });
                    }
                    "stable" => {
                        if stable_ticks.is_some() {
                            return Err(self.at_error(&key_token, "duplicate stable property"));
                        }
                        let token = self.take();
                        let value = number_token(&token)?;
                        let parsed = value.parse::<u16>().map_err(|_| {
                            self.at_error(&token, "stable count must be an integer from 1 to 65535")
                        })?;
                        if parsed == 0 {
                            return Err(self.at_error(&token, "stable count must be at least 1"));
                        }
                        stable_ticks = Some(parsed);
                    }
                    "when" => {
                        if when.is_some() {
                            return Err(self.at_error(&key_token, "duplicate when property"));
                        }
                        when = Some(self.expression(1)?);
                    }
                    "then" => {
                        if then.len() == 15 {
                            return Err(self
                                .at_error(&key_token, "a sequence may contain at most 16 steps"));
                        }
                        then.push(self.expression(1)?);
                    }
                    "within" => {
                        if within_ticks.is_some() {
                            return Err(self.at_error(&key_token, "duplicate within property"));
                        }
                        let token = self.take();
                        let value = number_token(&token)?;
                        let parsed = value.parse::<u16>().map_err(|_| {
                            self.at_error(&token, "within count must be an integer from 1 to 65535")
                        })?;
                        if parsed == 0 {
                            return Err(self.at_error(&token, "within count must be at least 1"));
                        }
                        within_ticks = Some(parsed);
                    }
                    "projection" => {
                        if projections.len() == MAX_PROJECTIONS {
                            return Err(self.at_error(
                                &key_token,
                                format!(
                                    "a milestone may contain at most {MAX_PROJECTIONS} projections"
                                ),
                            ));
                        }
                        projections.push(self.value_projection()?);
                    }
                    _ => {
                        return Err(self
                            .at_error(&key_token, format!("unknown milestone property {key:?}")));
                    }
                }
            }
            definitions.push(MilestoneDefinition {
                name,
                phase: phase.ok_or_else(|| self.at_error(&name_token, "missing phase property"))?,
                stable_ticks: stable_ticks.unwrap_or(1),
                when: when.ok_or_else(|| self.at_error(&name_token, "missing when property"))?,
                then,
                within_ticks,
                projections,
            });
        }
        if definitions.is_empty() {
            return self.error("program must define at least one milestone".into());
        }
        let program = MilestoneProgram {
            version,
            definitions,
        };
        validate_program(&program).map_err(|message| DslError {
            line: 1,
            column: 1,
            message,
        })?;
        Ok(program)
    }

    fn value_projection(&mut self) -> Result<ValueProjection, DslError> {
        let name_token = self.take();
        let name = match name_token.kind.clone() {
            TokenKind::Word(value) | TokenKind::String(value) => value,
            _ => return Err(self.at_error(&name_token, "expected projection name")),
        };
        validate_text(&name, MAX_NAME_BYTES, false)
            .map_err(|message| self.at_error(&name_token, message))?;
        self.expect(TokenKind::LeftBrace, "expected `{` after projection name")?;
        let mut items = Vec::new();
        while !self.consume(&TokenKind::RightBrace) {
            if matches!(self.peek().kind, TokenKind::Eof) {
                return self.error("unterminated projection block".into());
            }
            if items.len() == MAX_PROJECTION_ITEMS {
                return self.error(format!(
                    "a projection may contain at most {MAX_PROJECTION_ITEMS} items"
                ));
            }
            let kind_token = self.take();
            let kind = match &kind_token.kind {
                TokenKind::Word(value) => value.as_str(),
                _ => return Err(self.at_error(&kind_token, "expected projection item")),
            };
            let item = match kind {
                "rng" => {
                    let stream = match self.word()?.as_str() {
                        "primary" => RngStream::Primary,
                        "secondary" => RngStream::Secondary,
                        value => return self.error(format!("unknown RNG stream {value:?}")),
                    };
                    ValueProjectionItem::Rng { stream }
                }
                "actor_population" => {
                    let stage_token = self.take();
                    let stage = match stage_token.kind.clone() {
                        TokenKind::Word(value) | TokenKind::String(value) => value,
                        _ => return Err(self.at_error(&stage_token, "expected stage name")),
                    };
                    let room_token = self.take();
                    let room = number_token(&room_token)?.parse::<i8>().map_err(|_| {
                        self.at_error(&room_token, "actor population room must be -1 through 63")
                    })?;
                    ValueProjectionItem::ActorPopulation { stage, room }
                }
                "flag" => {
                    let domain = match self.word()?.as_str() {
                        "event" => FlagDomain::Event,
                        "temporary" => FlagDomain::Temporary,
                        "dungeon" => FlagDomain::Dungeon,
                        "switch" => FlagDomain::Switch,
                        value => return self.error(format!("unknown flag domain {value:?}")),
                    };
                    let (room, index_token) = if domain == FlagDomain::Switch {
                        let room_token = self.take();
                        let room = number_token(&room_token)?.parse::<i8>().map_err(|_| {
                            self.at_error(&room_token, "switch room must be 0 through 63")
                        })?;
                        (room, self.take())
                    } else {
                        (-1, self.take())
                    };
                    let index = number_token(&index_token)?.parse::<u16>().map_err(|_| {
                        self.at_error(
                            &index_token,
                            "flag index must be an unsigned 16-bit integer",
                        )
                    })?;
                    ValueProjectionItem::Flag {
                        selector: FlagSelector {
                            domain,
                            room,
                            index,
                        },
                    }
                }
                _ => {
                    return Err(
                        self.at_error(&kind_token, format!("unknown projection item {kind:?}"))
                    );
                }
            };
            items.push(item);
        }
        Ok(ValueProjection { name, items })
    }

    fn expression(&mut self, depth: usize) -> Result<Expression, DslError> {
        self.or_expression(depth)
    }

    fn or_expression(&mut self, depth: usize) -> Result<Expression, DslError> {
        let mut expression = self.and_expression(depth)?;
        while self.consume(&TokenKind::Or) {
            check_depth(depth, self.peek())?;
            expression = Expression::Or(
                Box::new(expression),
                Box::new(self.and_expression(depth + 1)?),
            );
        }
        Ok(expression)
    }

    fn and_expression(&mut self, depth: usize) -> Result<Expression, DslError> {
        let mut expression = self.unary_expression(depth)?;
        while self.consume(&TokenKind::And) {
            check_depth(depth, self.peek())?;
            expression = Expression::And(
                Box::new(expression),
                Box::new(self.unary_expression(depth + 1)?),
            );
        }
        Ok(expression)
    }

    fn unary_expression(&mut self, depth: usize) -> Result<Expression, DslError> {
        check_depth(depth, self.peek())?;
        if self.consume(&TokenKind::Not) {
            return Ok(Expression::Not(Box::new(self.unary_expression(depth + 1)?)));
        }
        if self.consume(&TokenKind::LeftParen) {
            let expression = self.expression(depth + 1)?;
            self.expect(TokenKind::RightParen, "expected `)`")?;
            return Ok(expression);
        }
        self.predicate()
    }

    fn predicate(&mut self) -> Result<Expression, DslError> {
        let field_token = self.take();
        let path = match &field_token.kind {
            TokenKind::Word(value) => value,
            _ => return Err(self.at_error(&field_token, "expected field path")),
        };
        let field = Field::parse(path);
        let fact = if field.is_none() {
            Some(self.query_fact(path, &field_token)?)
        } else {
            None
        };
        let field_type = field
            .map(Field::field_type)
            .unwrap_or_else(|| fact.as_ref().unwrap().field_type());
        if matches!(&self.peek().kind, TokenKind::Word(value) if value == "between") {
            self.at += 1;
            return self.range_expression(field, fact, field_type, &field_token);
        }
        let operator = match self.peek().kind {
            TokenKind::Equal => Some(Comparison::Equal),
            TokenKind::NotEqual => Some(Comparison::NotEqual),
            TokenKind::Less => Some(Comparison::Less),
            TokenKind::LessEqual => Some(Comparison::LessEqual),
            TokenKind::Greater => Some(Comparison::Greater),
            TokenKind::GreaterEqual => Some(Comparison::GreaterEqual),
            TokenKind::Word(ref value) if value == "has_all" => Some(Comparison::HasAll),
            TokenKind::Word(ref value) if value == "has_any" => Some(Comparison::HasAny),
            _ => None,
        };
        let Some(operator) = operator else {
            if field_type == FieldType::Bool {
                return Ok(match (field, fact) {
                    (Some(field), None) => Expression::Compare {
                        field,
                        operator: Comparison::Equal,
                        value: Value::Bool(true),
                    },
                    (None, Some(fact)) => Expression::Query {
                        fact,
                        operator: Comparison::Equal,
                        value: Value::Bool(true),
                    },
                    _ => unreachable!(),
                });
            }
            return Err(self.at_error(&field_token, "non-boolean field requires a comparison"));
        };
        self.at += 1;
        let value_token = self.take();
        match (field, fact) {
            (Some(field), None) => {
                let value = parse_typed_value(field, &value_token)
                    .map_err(|message| self.at_error(&value_token, message))?;
                validate_comparison(field, operator, &value)
                    .map_err(|message| self.at_error(&field_token, message))?;
                Ok(Expression::Compare {
                    field,
                    operator,
                    value,
                })
            }
            (None, Some(fact)) => {
                let value =
                    parse_value_for_type(fact.field_type(), fact.display_name(), &value_token)
                        .map_err(|message| self.at_error(&value_token, message))?;
                validate_query_comparison(&fact, operator, &value)
                    .map_err(|message| self.at_error(&field_token, message))?;
                Ok(Expression::Query {
                    fact,
                    operator,
                    value,
                })
            }
            _ => unreachable!(),
        }
    }

    fn range_expression(
        &mut self,
        field: Option<Field>,
        fact: Option<QueryFact>,
        field_type: FieldType,
        field_token: &Token,
    ) -> Result<Expression, DslError> {
        if !matches!(
            field_type,
            FieldType::U32 | FieldType::U64 | FieldType::I32 | FieldType::F32
        ) {
            return Err(self.at_error(field_token, "between requires a numeric field or fact"));
        }
        let display_name = field
            .map(Field::path)
            .unwrap_or_else(|| fact.as_ref().unwrap().display_name());
        let minimum_token = self.take();
        let minimum = parse_value_for_type(field_type, display_name, &minimum_token)
            .map_err(|message| self.at_error(&minimum_token, message))?;
        self.expect_word("and")?;
        let maximum_token = self.take();
        let maximum = parse_value_for_type(field_type, display_name, &maximum_token)
            .map_err(|message| self.at_error(&maximum_token, message))?;

        let ordered = match (&minimum, &maximum) {
            (Value::U32(a), Value::U32(b)) => a <= b,
            (Value::U64(a), Value::U64(b)) => a <= b,
            (Value::I32(a), Value::I32(b)) => a <= b,
            (Value::F32(a), Value::F32(b)) => a <= b,
            _ => false,
        };
        if !ordered {
            return Err(self.at_error(
                &maximum_token,
                "between requires a minimum less than or equal to its maximum",
            ));
        }

        let comparison = |operator, value| match (&field, &fact) {
            (Some(field), None) => Expression::Compare {
                field: *field,
                operator,
                value,
            },
            (None, Some(fact)) => Expression::Query {
                fact: fact.clone(),
                operator,
                value,
            },
            _ => unreachable!(),
        };
        Ok(Expression::And(
            Box::new(comparison(Comparison::GreaterEqual, minimum)),
            Box::new(comparison(Comparison::LessEqual, maximum)),
        ))
    }

    fn query_fact(&mut self, path: &str, token: &Token) -> Result<QueryFact, DslError> {
        if matches!(path, "player.in_aabb" | "player.plane_signed_distance") {
            self.expect(TokenKind::LeftParen, "expected `(` after spatial fact")?;
            let values = self.six_f32_arguments()?;
            self.expect(TokenKind::RightParen, "expected `)` after spatial fact")?;
            let fact = if path == "player.in_aabb" {
                QueryFact::PlayerInAabb {
                    minimum: values[..3].try_into().unwrap(),
                    maximum: values[3..].try_into().unwrap(),
                }
            } else {
                QueryFact::PlayerPlaneSignedDistance {
                    point: values[..3].try_into().unwrap(),
                    normal: values[3..].try_into().unwrap(),
                }
            };
            validate_query_fact(&fact).map_err(|message| self.at_error(token, message))?;
            return Ok(fact);
        }
        if let Some(field) = ActorFact::parse(path) {
            self.expect(TokenKind::LeftParen, "expected `(` after placed-actor fact")?;
            let stage = self.string_literal("expected quoted stage name")?;
            self.expect(TokenKind::Comma, "expected `,` after stage name")?;
            let home_room = self.integer::<i8>("home room must be a signed 8-bit integer")?;
            self.expect(TokenKind::Comma, "expected `,` after home room")?;
            let set_id = self.integer::<u16>("set ID must be an unsigned 16-bit integer")?;
            self.expect(TokenKind::Comma, "expected `,` after set ID")?;
            let actor_name = self.integer::<i16>("actor name must be a signed 16-bit integer")?;
            self.expect(
                TokenKind::RightParen,
                "expected `)` after placed-actor selector",
            )?;
            let fact = QueryFact::PlacedActor {
                selector: PlacedActorSelector {
                    stage,
                    home_room,
                    set_id,
                    actor_name,
                },
                field,
            };
            validate_query_fact(&fact).map_err(|message| self.at_error(token, message))?;
            return Ok(fact);
        }
        if let Some(domain) = FlagDomain::parse(path) {
            self.expect(TokenKind::LeftParen, "expected `(` after flag domain")?;
            let room = if domain == FlagDomain::Switch {
                let room = self.integer::<i8>("switch room must be a signed 8-bit integer")?;
                self.expect(TokenKind::Comma, "expected `,` after switch room")?;
                room
            } else {
                -1
            };
            let index = self.integer::<u16>("flag index must be an unsigned 16-bit integer")?;
            self.expect(TokenKind::RightParen, "expected `)` after flag selector")?;
            let fact = QueryFact::Flag {
                selector: FlagSelector {
                    domain,
                    room,
                    index,
                },
            };
            validate_query_fact(&fact).map_err(|message| self.at_error(token, message))?;
            return Ok(fact);
        }
        Err(self.at_error(token, format!("unknown milestone field {path:?}")))
    }

    fn string_literal(&mut self, message: &str) -> Result<String, DslError> {
        let token = self.take();
        match token.kind {
            TokenKind::String(value) => Ok(value),
            _ => Err(self.at_error(&token, message)),
        }
    }

    fn integer<T: std::str::FromStr>(&mut self, message: &str) -> Result<T, DslError> {
        let token = self.take();
        let value = number_token(&token)?;
        value.parse().map_err(|_| self.at_error(&token, message))
    }

    fn six_f32_arguments(&mut self) -> Result<[f32; 6], DslError> {
        let mut values = [0.0_f32; 6];
        for (index, value) in values.iter_mut().enumerate() {
            if index != 0 {
                self.expect(TokenKind::Comma, "expected `,` between spatial arguments")?;
            }
            let token = self.take();
            let source = number_token(&token)?;
            let parsed = source.parse::<f32>().map_err(|_| {
                self.at_error(&token, "spatial arguments must be finite 32-bit floats")
            })?;
            if !parsed.is_finite() {
                return Err(self.at_error(&token, "spatial arguments must be finite 32-bit floats"));
            }
            *value = canonical_float(parsed);
        }
        Ok(values)
    }

    fn word(&mut self) -> Result<String, DslError> {
        let token = self.take();
        match token.kind {
            TokenKind::Word(value) => Ok(value),
            _ => Err(self.at_error(&token, "expected identifier")),
        }
    }

    fn expect_word(&mut self, expected: &str) -> Result<(), DslError> {
        let token = self.take();
        if token.kind == TokenKind::Word(expected.into()) {
            Ok(())
        } else {
            Err(self.at_error(&token, format!("expected `{expected}`")))
        }
    }

    fn expect(&mut self, expected: TokenKind, message: &str) -> Result<(), DslError> {
        let token = self.take();
        if token.kind == expected {
            Ok(())
        } else {
            Err(self.at_error(&token, message))
        }
    }

    fn consume(&mut self, expected: &TokenKind) -> bool {
        if &self.peek().kind == expected {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.at]
    }

    fn take(&mut self) -> Token {
        let token = self.tokens[self.at].clone();
        if !matches!(token.kind, TokenKind::Eof) {
            self.at += 1;
        }
        token
    }

    fn error<T>(&self, message: String) -> Result<T, DslError> {
        Err(self.at_error(self.peek(), message))
    }

    fn at_error(&self, token: &Token, message: impl Into<String>) -> DslError {
        DslError {
            line: token.line,
            column: token.column,
            message: message.into(),
        }
    }
}

fn check_depth(depth: usize, token: &Token) -> Result<(), DslError> {
    if depth > MAX_EXPRESSION_DEPTH {
        Err(DslError {
            line: token.line,
            column: token.column,
            message: format!("expression exceeds maximum depth {MAX_EXPRESSION_DEPTH}"),
        })
    } else {
        Ok(())
    }
}

fn number_token(token: &Token) -> Result<&str, DslError> {
    match &token.kind {
        TokenKind::Number(value) => Ok(value),
        _ => Err(DslError {
            line: token.line,
            column: token.column,
            message: "expected number".into(),
        }),
    }
}

fn parse_typed_value(field: Field, token: &Token) -> Result<Value, String> {
    parse_value_for_type(field.field_type(), field.path(), token)
}

fn parse_value_for_type(
    field_type: FieldType,
    display_name: &str,
    token: &Token,
) -> Result<Value, String> {
    let number = || match &token.kind {
        TokenKind::Number(value) => Ok(value.as_str()),
        _ => Err(format!("{display_name} requires a numeric value")),
    };
    let string = || match &token.kind {
        TokenKind::String(value) => {
            validate_text(value, MAX_SYMBOL_BYTES, false)?;
            Ok(value.clone())
        }
        _ => Err(format!("{display_name} requires a quoted symbolic value")),
    };
    match field_type {
        FieldType::Bool => match &token.kind {
            TokenKind::Word(value) if value == "true" => Ok(Value::Bool(true)),
            TokenKind::Word(value) if value == "false" => Ok(Value::Bool(false)),
            _ => Err(format!("{display_name} requires true or false")),
        },
        FieldType::U32 => number()?
            .parse()
            .map(Value::U32)
            .map_err(|_| "expected an unsigned 32-bit integer".into()),
        FieldType::U64 => number()?
            .parse::<u64>()
            .map(Value::U64)
            .map_err(|_| format!("{display_name} requires an unsigned 64-bit integer")),
        FieldType::I32 => number()?
            .parse::<i32>()
            .map(Value::I32)
            .map_err(|_| format!("{display_name} requires a signed 32-bit integer")),
        FieldType::F32 => {
            let value = number()?
                .parse::<f32>()
                .map_err(|_| format!("{display_name} requires a finite 32-bit float"))?;
            if !value.is_finite() {
                return Err(format!("{display_name} requires a finite 32-bit float"));
            }
            Ok(Value::F32(canonical_float(value)))
        }
        FieldType::Symbol => string().map(Value::Symbol),
        FieldType::Enum => match &token.kind {
            TokenKind::String(_) => string().map(Value::Symbol),
            TokenKind::Number(_) => number()?
                .parse::<u32>()
                .map(Value::U32)
                .map_err(|_| format!("{display_name} requires a u32 or quoted symbol")),
            _ => Err(format!("{display_name} requires a u32 or quoted symbol")),
        },
        FieldType::Procedure => match &token.kind {
            TokenKind::String(_) => {
                let symbol = string()?;
                Ok(Value::ProcedureSymbol(canonical_procedure_symbol(&symbol)))
            }
            TokenKind::Number(_) => number()?
                .parse::<u32>()
                .map(Value::ProcedureNumber)
                .map_err(|_| format!("{display_name} requires a u32 or quoted symbol")),
            _ => Err(format!("{display_name} requires a u32 or quoted symbol")),
        },
    }
}

fn canonical_float(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

fn validate_text(value: &str, maximum: usize, allow_empty: bool) -> Result<(), String> {
    if (!allow_empty && value.is_empty()) || value.len() > maximum {
        return Err(format!(
            "text must contain {} to {maximum} UTF-8 bytes",
            if allow_empty { 0 } else { 1 }
        ));
    }
    if value.chars().any(char::is_control) {
        return Err("text must not contain control characters".into());
    }
    Ok(())
}

fn validate_comparison(field: Field, operator: Comparison, value: &Value) -> Result<(), String> {
    let type_matches = matches!(
        (field.field_type(), value),
        (FieldType::Bool, Value::Bool(_))
            | (FieldType::U32, Value::U32(_))
            | (FieldType::U64, Value::U64(_))
            | (FieldType::I32, Value::I32(_))
            | (FieldType::F32, Value::F32(_))
            | (FieldType::Symbol, Value::Symbol(_))
            | (FieldType::Enum, Value::Symbol(_))
            | (FieldType::Enum, Value::U32(_))
            | (FieldType::Procedure, Value::ProcedureNumber(_))
            | (FieldType::Procedure, Value::ProcedureSymbol(_))
    );
    if !type_matches {
        return Err(format!("value type does not match field {}", field.path()));
    }
    if matches!(value, Value::F32(value) if !value.is_finite() || value.to_bits() != canonical_float(*value).to_bits())
    {
        return Err("floating-point constants must be finite and canonical".into());
    }
    if let Value::Symbol(symbol) | Value::ProcedureSymbol(symbol) = value {
        validate_text(symbol, MAX_SYMBOL_BYTES, false)?;
    }
    match (field, value) {
        (Field::BoundaryKind, Value::Symbol(symbol))
            if !matches!(symbol.as_str(), "boot" | "tick") =>
        {
            return Err("boundary.kind symbol must be \"boot\" or \"tick\"".into());
        }
        (Field::BoundaryKind, Value::U32(value)) if *value > 1 => {
            return Err("boundary.kind numeric value must be 0 or 1".into());
        }
        (Field::StageName | Field::NextStageName, Value::Symbol(symbol))
            if !valid_stage_name(symbol) =>
        {
            return Err(
                "stage names must be 1..8 ASCII uppercase, digit, or underscore bytes".into(),
            );
        }
        (Field::PlayerProcedure, Value::ProcedureSymbol(symbol))
            if !valid_procedure_symbol(symbol) =>
        {
            return Err("procedure symbols must be exact PROC_* enum tokens".into());
        }
        _ => {}
    }
    if !matches!(
        field.field_type(),
        FieldType::U32 | FieldType::U64 | FieldType::I32 | FieldType::F32
    ) && !matches!(operator, Comparison::Equal | Comparison::NotEqual)
    {
        return Err(format!("field {} supports only == and !=", field.path()));
    }
    if matches!(operator, Comparison::HasAll | Comparison::HasAny)
        && !matches!(field.field_type(), FieldType::U32 | FieldType::U64)
    {
        return Err(format!(
            "field {} does not support bit-mask comparisons",
            field.path()
        ));
    }
    if matches!(operator, Comparison::HasAll | Comparison::HasAny)
        && matches!(value, Value::U32(0) | Value::U64(0))
    {
        return Err("bit-mask comparisons require a nonzero mask".into());
    }
    Ok(())
}

fn validate_query_fact(fact: &QueryFact) -> Result<(), String> {
    match fact {
        QueryFact::PlacedActor { selector, .. } => {
            selector.validate().map_err(str::to_owned)?;
        }
        QueryFact::Flag { selector } => {
            let maximum = match selector.domain {
                FlagDomain::Event => 822,
                FlagDomain::Temporary => 185,
                FlagDomain::Dungeon => 64,
                FlagDomain::Switch => 240,
            };
            if usize::from(selector.index) >= maximum {
                return Err(format!(
                    "{} index must be below {maximum}",
                    selector.domain.path()
                ));
            }
            if selector.domain == FlagDomain::Switch {
                if !(0..=63).contains(&selector.room) {
                    return Err("switch flag room must be 0..63".into());
                }
            } else if selector.room != -1 {
                return Err(format!(
                    "{} flags do not accept a room",
                    selector.domain.path()
                ));
            }
        }
        QueryFact::PlayerInAabb { minimum, maximum } => {
            for axis in 0..3 {
                if !minimum[axis].is_finite()
                    || !maximum[axis].is_finite()
                    || minimum[axis].to_bits() != canonical_float(minimum[axis]).to_bits()
                    || maximum[axis].to_bits() != canonical_float(maximum[axis]).to_bits()
                    || minimum[axis] > maximum[axis]
                {
                    return Err(
                        "player.in_aabb requires canonical finite minimum <= maximum on every axis"
                            .into(),
                    );
                }
            }
        }
        QueryFact::PlayerPlaneSignedDistance { point, normal } => {
            if point.iter().chain(normal).any(|value| {
                !value.is_finite() || value.to_bits() != canonical_float(*value).to_bits()
            }) {
                return Err(
                    "player.plane_signed_distance requires canonical finite arguments".into(),
                );
            }
            let length_squared = normal
                .iter()
                .map(|value| f64::from(*value) * f64::from(*value))
                .sum::<f64>();
            if length_squared == 0.0 || !length_squared.is_finite() {
                return Err("player plane normal must be finite and nonzero".into());
            }
        }
    }
    Ok(())
}

fn validate_query_comparison(
    fact: &QueryFact,
    operator: Comparison,
    value: &Value,
) -> Result<(), String> {
    validate_query_fact(fact)?;
    let field_type = fact.field_type();
    let type_matches = matches!(
        (field_type, value),
        (FieldType::Bool, Value::Bool(_))
            | (FieldType::U32, Value::U32(_))
            | (FieldType::I32, Value::I32(_))
            | (FieldType::F32, Value::F32(_))
    );
    if !type_matches {
        return Err(format!(
            "value type does not match fact {}",
            fact.display_name()
        ));
    }
    if matches!(value, Value::F32(value) if !value.is_finite() || value.to_bits() != canonical_float(*value).to_bits())
    {
        return Err("floating-point constants must be finite and canonical".into());
    }
    if field_type == FieldType::Bool
        && !matches!(operator, Comparison::Equal | Comparison::NotEqual)
    {
        return Err(format!(
            "fact {} supports only == and !=",
            fact.display_name()
        ));
    }
    if matches!(operator, Comparison::HasAll | Comparison::HasAny) {
        if field_type != FieldType::U32 {
            return Err(format!(
                "fact {} does not support bit-mask comparisons",
                fact.display_name()
            ));
        }
        if matches!(value, Value::U32(0)) {
            return Err("bit-mask comparisons require a nonzero mask".into());
        }
    }
    Ok(())
}

fn valid_stage_name(value: &str) -> bool {
    (1..=8).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn valid_procedure_symbol(value: &str) -> bool {
    value.len() > 5
        && value.starts_with("PROC_")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn canonical_procedure_symbol(value: &str) -> String {
    match value {
        "crawl_start" => "PROC_CRAWL_START".into(),
        "crawl_move" => "PROC_CRAWL_MOVE".into(),
        "crawl_auto_move" => "PROC_CRAWL_AUTO_MOVE".into(),
        "crawl_end" => "PROC_CRAWL_END".into(),
        _ => value.into(),
    }
}

fn validate_program(program: &MilestoneProgram) -> Result<(), String> {
    if program.version.major != LANGUAGE_VERSION.0 || program.version.minor > LANGUAGE_VERSION.1 {
        return Err("unsupported milestone language version".into());
    }
    if program.definitions.is_empty() || program.definitions.len() > MAX_DEFINITIONS {
        return Err(format!(
            "program must contain 1 to {MAX_DEFINITIONS} milestones"
        ));
    }
    let mut names = BTreeSet::new();
    for definition in &program.definitions {
        validate_text(&definition.name, MAX_NAME_BYTES, false)?;
        if !names.insert(&definition.name) {
            return Err(format!("duplicate milestone {:?}", definition.name));
        }
        if definition.stable_ticks == 0 {
            return Err(format!(
                "milestone {:?} has a zero stable count",
                definition.name
            ));
        }
        if definition.then.is_empty() != definition.within_ticks.is_none() {
            return Err(format!(
                "milestone {:?} must use `within` exactly when it has ordered `then` steps",
                definition.name
            ));
        }
        if !definition.then.is_empty() {
            if program.version.minor < 3 {
                return Err(format!(
                    "milestone {:?} ordered sequences require language 1.3",
                    definition.name
                ));
            }
            if definition.stable_ticks != 1 {
                return Err(format!(
                    "milestone {:?} cannot combine stable with an ordered sequence",
                    definition.name
                ));
            }
            if definition.then.len() > 15 || definition.within_ticks == Some(0) {
                return Err(format!(
                    "milestone {:?} has an invalid bounded sequence",
                    definition.name
                ));
            }
        }
        if !definition.projections.is_empty() && program.version.minor < 4 {
            return Err(format!(
                "milestone {:?} value projections require language 1.4",
                definition.name
            ));
        }
        if definition.projections.len() > MAX_PROJECTIONS {
            return Err(format!(
                "milestone {:?} exceeds {MAX_PROJECTIONS} value projections",
                definition.name
            ));
        }
        let mut projection_names = BTreeSet::new();
        for projection in &definition.projections {
            validate_text(&projection.name, MAX_NAME_BYTES, false)?;
            if !projection_names.insert(&projection.name) {
                return Err(format!(
                    "milestone {:?} has duplicate projection {:?}",
                    definition.name, projection.name
                ));
            }
            if projection.items.is_empty() || projection.items.len() > MAX_PROJECTION_ITEMS {
                return Err(format!(
                    "projection {:?} must contain 1 to {MAX_PROJECTION_ITEMS} items",
                    projection.name
                ));
            }
            let mut items = BTreeSet::new();
            for item in &projection.items {
                let identity = match item {
                    ValueProjectionItem::Rng { stream } => format!("rng:{}", *stream as u8),
                    ValueProjectionItem::ActorPopulation { stage, room } => {
                        if !valid_stage_name(stage) || !(-1..=63).contains(room) {
                            return Err(format!(
                                "projection {:?} has an invalid actor population scope",
                                projection.name
                            ));
                        }
                        format!("actors:{stage}:{room}")
                    }
                    ValueProjectionItem::Flag { selector } => {
                        validate_query_fact(&QueryFact::Flag {
                            selector: selector.clone(),
                        })?;
                        format!(
                            "flag:{}:{}:{}",
                            selector.domain as u8, selector.room, selector.index
                        )
                    }
                };
                if !items.insert(identity) {
                    return Err(format!(
                        "projection {:?} contains a duplicate item",
                        projection.name
                    ));
                }
            }
        }
        let mut operations = 0;
        if !definition.then.is_empty() {
            operations += 2 + definition.then.len();
        }
        validate_expression(&definition.when, program.version.minor, 1, &mut operations)?;
        for step in &definition.then {
            validate_expression(step, program.version.minor, 1, &mut operations)?;
        }
        operations += definition
            .projections
            .iter()
            .map(|projection| 1 + projection.items.len())
            .sum::<usize>();
        if operations > MAX_OPS {
            return Err(format!(
                "milestone {:?} exceeds {MAX_OPS} operations",
                definition.name
            ));
        }
    }
    Ok(())
}

fn validate_expression(
    expression: &Expression,
    language_minor: u16,
    depth: usize,
    operations: &mut usize,
) -> Result<(), String> {
    if depth > MAX_EXPRESSION_DEPTH {
        return Err(format!(
            "expression exceeds maximum depth {MAX_EXPRESSION_DEPTH}"
        ));
    }
    match expression {
        Expression::Compare {
            field,
            operator,
            value,
        } => {
            if language_minor == 0
                && ((*field as u8) > Field::NextStageEnabled as u8
                    || matches!(operator, Comparison::HasAll | Comparison::HasAny))
            {
                return Err(format!(
                    "field/operator {} requires milestone language 1.1",
                    field.path()
                ));
            }
            if language_minor < 5 && (*field as u8) >= Field::PlayerDoStatus as u8 {
                return Err(format!(
                    "field {} requires milestone language 1.5",
                    field.path()
                ));
            }
            if language_minor < 6 && (*field as u8) >= Field::TalkPartnerHomePositionX as u8 {
                return Err(format!(
                    "field {} requires milestone language 1.6",
                    field.path()
                ));
            }
            validate_comparison(*field, *operator, value)?;
            *operations += 3;
        }
        Expression::Query {
            fact,
            operator,
            value,
        } => {
            let required_minor = if matches!(
                fact,
                QueryFact::PlayerInAabb { .. } | QueryFact::PlayerPlaneSignedDistance { .. }
            ) {
                3
            } else {
                2
            };
            if language_minor < required_minor {
                return Err(format!(
                    "fact {} requires milestone language 1.{required_minor}",
                    fact.display_name(),
                ));
            }
            validate_query_comparison(fact, *operator, value)?;
            *operations += 3;
        }
        Expression::Not(inner) => {
            validate_expression(inner, language_minor, depth + 1, operations)?;
            *operations += 1;
        }
        Expression::And(left, right) | Expression::Or(left, right) => {
            validate_expression(left, language_minor, depth + 1, operations)?;
            validate_expression(right, language_minor, depth + 1, operations)?;
            *operations += 1;
        }
    }
    Ok(())
}

/// Canonically format an AST for source control or visual-editor export.
pub fn format(program: &MilestoneProgram) -> Result<String, BinaryError> {
    validate_program(program).map_err(BinaryError)?;
    let mut output = format!(
        "milestones {}.{}\n\n",
        program.version.major, program.version.minor
    );
    for (index, definition) in program.definitions.iter().enumerate() {
        output.push_str("milestone ");
        output.push_str(&quoted(&definition.name));
        output.push_str(" {\n  phase ");
        output.push_str(match definition.phase {
            EvaluationPhase::PreInput => "pre_input",
            EvaluationPhase::PostSim => "post_sim",
        });
        output.push('\n');
        if definition.stable_ticks != 1 {
            output.push_str(&format!("  stable {}\n", definition.stable_ticks));
        }
        if let Some(within_ticks) = definition.within_ticks {
            output.push_str(&format!("  within {within_ticks}\n"));
        }
        output.push_str("  when ");
        format_expression(&definition.when, 0, &mut output);
        for step in &definition.then {
            output.push_str("\n  then ");
            format_expression(step, 0, &mut output);
        }
        for projection in &definition.projections {
            output.push_str("\n  projection ");
            output.push_str(&quoted(&projection.name));
            output.push_str(" {");
            for item in &projection.items {
                output.push_str("\n    ");
                match item {
                    ValueProjectionItem::Rng { stream } => {
                        output.push_str("rng ");
                        output.push_str(match stream {
                            RngStream::Primary => "primary",
                            RngStream::Secondary => "secondary",
                        });
                    }
                    ValueProjectionItem::ActorPopulation { stage, room } => {
                        output.push_str("actor_population ");
                        output.push_str(&quoted(stage));
                        output.push(' ');
                        output.push_str(&room.to_string());
                    }
                    ValueProjectionItem::Flag { selector } => {
                        output.push_str("flag ");
                        output.push_str(match selector.domain {
                            FlagDomain::Event => "event ",
                            FlagDomain::Temporary => "temporary ",
                            FlagDomain::Dungeon => "dungeon ",
                            FlagDomain::Switch => "switch ",
                        });
                        if selector.domain == FlagDomain::Switch {
                            output.push_str(&selector.room.to_string());
                            output.push(' ');
                        }
                        output.push_str(&selector.index.to_string());
                    }
                }
            }
            output.push_str("\n  }");
        }
        output.push_str("\n}");
        if index + 1 != program.definitions.len() {
            output.push_str("\n\n");
        } else {
            output.push('\n');
        }
    }
    Ok(output)
}

fn format_expression(expression: &Expression, parent_precedence: u8, output: &mut String) {
    let precedence = match expression {
        Expression::Or(..) => 1,
        Expression::And(..) => 2,
        Expression::Not(..) => 3,
        Expression::Compare { .. } | Expression::Query { .. } => 4,
    };
    let parentheses = precedence < parent_precedence;
    if parentheses {
        output.push('(');
    }
    match expression {
        Expression::Compare {
            field,
            operator,
            value,
        } => {
            output.push_str(field.path());
            output.push(' ');
            output.push_str(match operator {
                Comparison::Equal => "==",
                Comparison::NotEqual => "!=",
                Comparison::Less => "<",
                Comparison::LessEqual => "<=",
                Comparison::Greater => ">",
                Comparison::GreaterEqual => ">=",
                Comparison::HasAll => "has_all",
                Comparison::HasAny => "has_any",
            });
            output.push(' ');
            format_value(value, output);
        }
        Expression::Query {
            fact,
            operator,
            value,
        } => {
            format_query_fact(fact, output);
            output.push(' ');
            output.push_str(match operator {
                Comparison::Equal => "==",
                Comparison::NotEqual => "!=",
                Comparison::Less => "<",
                Comparison::LessEqual => "<=",
                Comparison::Greater => ">",
                Comparison::GreaterEqual => ">=",
                Comparison::HasAll => "has_all",
                Comparison::HasAny => "has_any",
            });
            output.push(' ');
            format_value(value, output);
        }
        Expression::Not(inner) => {
            output.push('!');
            format_expression(inner, precedence, output);
        }
        Expression::And(left, right) => {
            format_expression(left, precedence, output);
            output.push_str(" && ");
            format_expression(right, precedence + 1, output);
        }
        Expression::Or(left, right) => {
            format_expression(left, precedence, output);
            output.push_str(" || ");
            format_expression(right, precedence + 1, output);
        }
    }
    if parentheses {
        output.push(')');
    }
}

fn format_query_fact(fact: &QueryFact, output: &mut String) {
    match fact {
        QueryFact::PlacedActor { selector, field } => {
            output.push_str(field.path());
            output.push('(');
            output.push_str(&quoted(&selector.stage));
            output.push_str(", ");
            output.push_str(&selector.home_room.to_string());
            output.push_str(", ");
            output.push_str(&selector.set_id.to_string());
            output.push_str(", ");
            output.push_str(&selector.actor_name.to_string());
            output.push(')');
        }
        QueryFact::Flag { selector } => {
            output.push_str(selector.domain.path());
            output.push('(');
            if selector.domain == FlagDomain::Switch {
                output.push_str(&selector.room.to_string());
                output.push_str(", ");
            }
            output.push_str(&selector.index.to_string());
            output.push(')');
        }
        QueryFact::PlayerInAabb { minimum, maximum } => {
            output.push_str("player.in_aabb(");
            format_f32_arguments(minimum.iter().chain(maximum), output);
            output.push(')');
        }
        QueryFact::PlayerPlaneSignedDistance { point, normal } => {
            output.push_str("player.plane_signed_distance(");
            format_f32_arguments(point.iter().chain(normal), output);
            output.push(')');
        }
    }
}

fn format_f32_arguments<'a>(values: impl Iterator<Item = &'a f32>, output: &mut String) {
    for (index, value) in values.enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        format_value(&Value::F32(*value), output);
    }
}

fn format_value(value: &Value, output: &mut String) {
    match value {
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::U32(value) | Value::ProcedureNumber(value) => output.push_str(&value.to_string()),
        Value::U64(value) => output.push_str(&value.to_string()),
        Value::I32(value) => output.push_str(&value.to_string()),
        Value::F32(value) => {
            let mut rendered = value.to_string();
            if !rendered.contains(['.', 'e', 'E']) {
                rendered.push_str(".0");
            }
            output.push_str(&rendered);
        }
        Value::Symbol(value) | Value::ProcedureSymbol(value) => {
            output.push_str(&quoted(value));
        }
    }
}

fn quoted(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

/// Compile a validated AST to canonical DMSP v1 bytes.
pub fn compile(program: &MilestoneProgram) -> Result<CompiledMilestones, BinaryError> {
    validate_program(program).map_err(BinaryError)?;
    let mut records = Vec::new();
    let mut identities = Vec::with_capacity(program.definitions.len());
    for definition in &program.definitions {
        let mut bytecode = Vec::new();
        let mut operation_count = 0_u16;
        if definition.then.is_empty() {
            encode_expression(&definition.when, &mut bytecode, &mut operation_count)?;
        } else {
            bytecode.push(0x40);
            push_u16(&mut bytecode, definition.within_ticks.unwrap());
            bytecode.push((definition.then.len() + 1) as u8);
            increment_ops(&mut operation_count, 1)?;
            for step in std::iter::once(&definition.when).chain(&definition.then) {
                encode_expression(step, &mut bytecode, &mut operation_count)?;
                bytecode.push(0x41);
                increment_ops(&mut operation_count, 1)?;
            }
        }
        for projection in &definition.projections {
            encode_value_projection(projection, &mut bytecode, &mut operation_count)?;
        }
        let name = definition.name.as_bytes();
        let identity_bytes = definition_identity_bytes(
            name,
            definition.phase,
            definition.stable_ticks,
            operation_count,
            &bytecode,
        )?;
        let definition_sha256: [u8; 32] = Sha256::new()
            .chain_update(DEFINITION_DOMAIN)
            .chain_update(&identity_bytes)
            .finalize()
            .into();
        let record_len = RECORD_FIXED_BYTES
            .checked_add(name.len())
            .and_then(|length| length.checked_add(bytecode.len()))
            .ok_or_else(|| BinaryError("milestone record length overflow".into()))?;
        push_u32(&mut records, usize_u32(record_len, "milestone record")?);
        records.extend_from_slice(&identity_bytes[..identity_bytes.len() - bytecode.len()]);
        records.extend_from_slice(&definition_sha256);
        records.extend_from_slice(&bytecode);
        identities.push(CompiledDefinitionIdentity {
            name: definition.name.clone(),
            sha256: definition_sha256,
        });
    }

    let mut bytes = Vec::with_capacity(HEADER_BYTES + records.len());
    bytes.extend_from_slice(&MAGIC);
    push_u16(&mut bytes, WIRE_VERSION.0);
    push_u16(&mut bytes, program.version.minor);
    push_u16(&mut bytes, program.version.major);
    push_u16(&mut bytes, program.version.minor);
    push_u16(
        &mut bytes,
        u16::try_from(program.definitions.len())
            .map_err(|_| BinaryError("too many milestone definitions".into()))?,
    );
    push_u16(&mut bytes, 0);
    push_u32(&mut bytes, usize_u32(records.len(), "program payload")?);
    bytes.extend_from_slice(&[0; 32]);
    bytes.extend_from_slice(&records);
    if bytes.len() > MAX_BINARY_BYTES {
        return Err(BinaryError(format!(
            "compiled milestone program exceeds {MAX_BINARY_BYTES} bytes"
        )));
    }
    let program_sha256 = program_digest(&bytes);
    bytes[20..52].copy_from_slice(&program_sha256);
    Ok(CompiledMilestones {
        bytes,
        program_sha256,
        definitions: identities,
    })
}

fn encode_value_projection(
    projection: &ValueProjection,
    output: &mut Vec<u8>,
    operations: &mut u16,
) -> Result<(), BinaryError> {
    output.push(0x50);
    output.push(
        u8::try_from(projection.name.len())
            .map_err(|_| BinaryError("projection name is too long".into()))?,
    );
    output.extend_from_slice(projection.name.as_bytes());
    output.push(
        u8::try_from(projection.items.len())
            .map_err(|_| BinaryError("projection has too many items".into()))?,
    );
    increment_ops(operations, 1)?;
    for item in &projection.items {
        match item {
            ValueProjectionItem::Rng { stream } => {
                output.extend_from_slice(&[0x51, *stream as u8]);
            }
            ValueProjectionItem::ActorPopulation { stage, room } => {
                output.push(0x52);
                let mut fixed_stage = [0_u8; 8];
                fixed_stage[..stage.len()].copy_from_slice(stage.as_bytes());
                output.extend_from_slice(&fixed_stage);
                output.push(*room as u8);
            }
            ValueProjectionItem::Flag { selector } => {
                output.extend_from_slice(&[0x53, selector.domain as u8, selector.room as u8]);
                push_u16(output, selector.index);
            }
        }
        increment_ops(operations, 1)?;
    }
    Ok(())
}

/// Stable identity for one named projection, independent of milestone topology.
pub fn value_projection_identity(projection: &ValueProjection) -> Result<[u8; 32], BinaryError> {
    validate_text(&projection.name, MAX_NAME_BYTES, false).map_err(BinaryError)?;
    if projection.items.is_empty() || projection.items.len() > MAX_PROJECTION_ITEMS {
        return Err(BinaryError("invalid projection item count".into()));
    }
    for item in &projection.items {
        match item {
            ValueProjectionItem::Rng { .. } => {}
            ValueProjectionItem::ActorPopulation { stage, room }
                if valid_stage_name(stage) && (-1..=63).contains(room) => {}
            ValueProjectionItem::Flag { selector } => {
                validate_query_fact(&QueryFact::Flag {
                    selector: selector.clone(),
                })
                .map_err(BinaryError)?;
            }
            _ => {
                return Err(BinaryError(
                    "invalid actor population projection scope".into(),
                ));
            }
        }
    }
    let mut bytes = Vec::new();
    let mut operations = 0;
    encode_value_projection(projection, &mut bytes, &mut operations)?;
    Ok(Sha256::new()
        .chain_update(PROJECTION_DOMAIN)
        .chain_update(bytes)
        .finalize()
        .into())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RecordedTraceMilestoneHit {
    pub record_index: usize,
    pub boundary_index: u64,
    pub simulation_tick: u64,
    pub tape_frame: Option<u64>,
}

/// Evaluate authored predicates offline over immutable decoded gameplay records.
/// Facts not present in the trace schema (currently actor catalogs and flags)
/// are unavailable and therefore cannot make a comparison true.
pub fn evaluate_recorded_trace(
    program: &MilestoneProgram,
    trace: &crate::trace::DecodedTrace,
) -> Result<BTreeMap<String, Option<RecordedTraceMilestoneHit>>, BinaryError> {
    validate_program(program).map_err(BinaryError)?;
    #[derive(Default)]
    struct State {
        stable: u16,
        sequence_next: usize,
        sequence_elapsed: u16,
        hit: Option<RecordedTraceMilestoneHit>,
    }
    let mut states = (0..program.definitions.len())
        .map(|_| State::default())
        .collect::<Vec<_>>();
    for (record_index, record) in trace.records.iter().enumerate() {
        for (definition, state) in program.definitions.iter().zip(&mut states) {
            if state.hit.is_some()
                || !matches!(
                    (definition.phase, record.observation_phase),
                    (
                        EvaluationPhase::PreInput,
                        crate::trace::TracePhase::PreInput
                    ) | (
                        EvaluationPhase::PostSim,
                        crate::trace::TracePhase::PostSimulation
                    )
                )
            {
                continue;
            }
            let capture = || RecordedTraceMilestoneHit {
                record_index,
                boundary_index: record.boundary_index,
                simulation_tick: record.simulation_tick,
                tape_frame: record.tape_frame,
            };
            if !definition.then.is_empty() {
                let steps = std::iter::once(&definition.when)
                    .chain(&definition.then)
                    .collect::<Vec<_>>();
                if state.sequence_next == 0 {
                    if evaluate_trace_expression(steps[0], record) == Some(true) {
                        state.sequence_next = 1;
                        state.sequence_elapsed = 0;
                    }
                    continue;
                }
                let next_elapsed = state.sequence_elapsed.saturating_add(1);
                if next_elapsed > definition.within_ticks.unwrap() {
                    state.sequence_next =
                        usize::from(evaluate_trace_expression(steps[0], record) == Some(true));
                    state.sequence_elapsed = 0;
                    continue;
                }
                state.sequence_elapsed = next_elapsed;
                if evaluate_trace_expression(steps[state.sequence_next], record) == Some(true) {
                    state.sequence_next += 1;
                } else if evaluate_trace_expression(steps[0], record) == Some(true) {
                    state.sequence_next = 1;
                    state.sequence_elapsed = 0;
                }
                if state.sequence_next == steps.len() {
                    state.hit = Some(capture());
                }
                continue;
            }
            if evaluate_trace_expression(&definition.when, record) == Some(true) {
                state.stable = state.stable.saturating_add(1).min(definition.stable_ticks);
                if state.stable == definition.stable_ticks {
                    state.hit = Some(capture());
                }
            } else {
                state.stable = 0;
            }
        }
    }
    Ok(program
        .definitions
        .iter()
        .zip(states)
        .map(|(definition, state)| (definition.name.clone(), state.hit))
        .collect())
}

fn trace_channel_present(
    record: &crate::trace::TraceRecord,
    channel: crate::trace::TraceChannel,
) -> bool {
    record.channel_status.get(&channel) == Some(&crate::trace::TraceChannelStatus::Present)
}

fn evaluate_trace_expression(
    expression: &Expression,
    record: &crate::trace::TraceRecord,
) -> Option<bool> {
    let facts = typed_facts_from_trace_record(record);
    evaluate_trace_expression_with_facts(expression, record, &facts)
}

fn evaluate_trace_expression_with_facts(
    expression: &Expression,
    record: &crate::trace::TraceRecord,
    facts: &TypedFactResponse,
) -> Option<bool> {
    match expression {
        Expression::Compare {
            field,
            operator,
            value,
        } => trace_field(record, facts, *field)
            .map(|actual| compare_trace_values(&actual, *operator, value)),
        Expression::Query {
            fact,
            operator,
            value,
        } => {
            trace_query(record, fact).map(|actual| compare_trace_values(&actual, *operator, value))
        }
        Expression::Not(inner) => {
            evaluate_trace_expression_with_facts(inner, record, facts).map(|value| !value)
        }
        Expression::And(left, right) => match (
            evaluate_trace_expression_with_facts(left, record, facts),
            evaluate_trace_expression_with_facts(right, record, facts),
        ) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        },
        Expression::Or(left, right) => match (
            evaluate_trace_expression_with_facts(left, record, facts),
            evaluate_trace_expression_with_facts(right, record, facts),
        ) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        },
    }
}

fn trace_field(
    record: &crate::trace::TraceRecord,
    facts: &TypedFactResponse,
    field: Field,
) -> Option<Value> {
    use crate::trace::TraceChannel as Channel;
    let stage = trace_channel_present(record, Channel::Stage);
    let player = trace_channel_present(record, Channel::PlayerMotion);
    let event = trace_channel_present(record, Channel::Event);
    let action = record.player_action.as_ref();
    let rng = record.rng.as_ref();
    let collision = record.player_background_collision.as_ref();
    Some(match field {
        Field::BoundaryKind => Value::U32(u32::from(record.boundary_index != 0)),
        Field::BoundaryIndex => Value::U64(record.boundary_index),
        Field::TapeFrame => Value::U64(record.tape_frame?),
        Field::BoundaryReached => Value::Bool(true),
        Field::StageName => Value::Symbol(typed_stage_code(facts, TypedFactId::StageName)?.into()),
        Field::StageRoom => Value::I32(typed_i32(facts, TypedFactId::StageRoom)?),
        Field::StageLayer if stage => Value::I32(record.layer.into()),
        Field::StageSpawn => Value::I32(typed_i32(facts, TypedFactId::StageSpawn)?),
        Field::NextStageName if stage => Value::Symbol(record.next_stage_name.clone()),
        Field::NextStageRoom if stage => Value::I32(record.next_room.into()),
        Field::NextStageLayer if stage => Value::I32(record.next_layer.into()),
        Field::NextStageSpawn if stage => Value::I32(record.next_point.into()),
        Field::NextStageEnabled if stage => Value::Bool(record.next_stage_enabled),
        Field::PlayerExists => Value::Bool(typed_bool(facts, TypedFactId::PlayerExists)?),
        Field::PlayerIsLink => Value::Bool(typed_bool(facts, TypedFactId::PlayerIsLink)?),
        Field::PlayerProcessId if player => Value::U32(record.player_session_process_id?),
        Field::PlayerActorName if player => Value::I32(record.player_actor_name.into()),
        Field::PlayerPositionX => Value::F32(typed_vec3(facts, TypedFactId::PlayerPosition)?[0]),
        Field::PlayerPositionY => Value::F32(typed_vec3(facts, TypedFactId::PlayerPosition)?[1]),
        Field::PlayerPositionZ => Value::F32(typed_vec3(facts, TypedFactId::PlayerPosition)?[2]),
        Field::PlayerVelocityX if player => Value::F32(record.velocity[0]),
        Field::PlayerVelocityY if player => Value::F32(record.velocity[1]),
        Field::PlayerVelocityZ if player => Value::F32(record.velocity[2]),
        Field::PlayerSpeed if player => Value::F32(record.forward_speed),
        Field::PlayerProcedure if player => Value::ProcedureNumber(record.player_proc_id?.into()),
        Field::PlayerCurrentAngleX if player => Value::I32(record.current_angle[0].into()),
        Field::PlayerCurrentAngleY if player => Value::I32(record.current_angle[1].into()),
        Field::PlayerCurrentAngleZ if player => Value::I32(record.current_angle[2].into()),
        Field::PlayerShapeAngleX if player => Value::I32(record.shape_angle[0].into()),
        Field::PlayerShapeAngleY if player => Value::I32(record.shape_angle[1].into()),
        Field::PlayerShapeAngleZ if player => Value::I32(record.shape_angle[2].into()),
        Field::PlayerModeFlags => Value::U32(action?.mode_flags),
        Field::PlayerDamageWaitTimer => Value::I32(action?.damage_wait_timer.into()),
        Field::PlayerIceDamageWaitTimer => Value::I32(action?.ice_damage_wait_timer.into()),
        Field::PlayerSwordChangeWaitTimer => Value::U32(action?.sword_change_wait_timer.into()),
        Field::EventRunning => Value::Bool(typed_bool(facts, TypedFactId::EventRunning)?),
        Field::EventId => Value::I32(typed_i32(facts, TypedFactId::EventId)?),
        Field::EventMode if event => Value::U32(record.event_mode.into()),
        Field::EventStatus if event => Value::U32(record.event_status.into()),
        Field::EventMapToolId if event => Value::U32(record.event_map_tool_id.into()),
        Field::EventNameHashPresent if event => Value::Bool(record.event_name_hash_present),
        Field::EventNameHash if event && record.event_name_hash_present => {
            Value::U32(record.event_name_hash)
        }
        Field::RngPrimaryState0 => Value::I32(rng?.primary.state[0]),
        Field::RngPrimaryState1 => Value::I32(rng?.primary.state[1]),
        Field::RngPrimaryState2 => Value::I32(rng?.primary.state[2]),
        Field::RngPrimaryCalls => Value::U64(rng?.primary.call_count),
        Field::RngSecondaryState0 => Value::I32(rng?.secondary.state[0]),
        Field::RngSecondaryState1 => Value::I32(rng?.secondary.state[1]),
        Field::RngSecondaryState2 => Value::I32(rng?.secondary.state[2]),
        Field::RngSecondaryCalls => Value::U64(rng?.secondary.call_count),
        Field::CollisionGroundContact => Value::Bool(collision?.flags & (1 << 1) != 0),
        Field::CollisionWallContact => Value::Bool(collision?.flags & (1 << 6) != 0),
        Field::CollisionRoofContact => Value::Bool(collision?.flags & (1 << 8) != 0),
        Field::CollisionWaterContact => Value::Bool(collision?.flags & (1 << 11) != 0),
        Field::CollisionWaterIn => Value::Bool(collision?.flags & (1 << 12) != 0),
        Field::CollisionGroundHeight => Value::F32(collision?.ground_height),
        Field::CollisionRoofHeight => Value::F32(collision?.roof_height),
        Field::CollisionGroundClearance if player => {
            Value::F32(record.position[1] - collision?.ground_height)
        }
        Field::PlayerDoStatus => Value::U32(typed_u32(facts, TypedFactId::PlayerDoStatus)?),
        Field::TalkPartnerExists => {
            Value::Bool(typed_actor_exists(facts, TypedFactId::TalkPartner)?)
        }
        Field::TalkPartnerActorName => Value::I32(
            typed_actor(facts, TypedFactId::TalkPartner)?
                .actor_name
                .into(),
        ),
        Field::TalkPartnerSetId => {
            Value::U32(typed_actor(facts, TypedFactId::TalkPartner)?.set_id.into())
        }
        Field::TalkPartnerHomeRoom => Value::I32(
            typed_actor(facts, TypedFactId::TalkPartner)?
                .home_room
                .into(),
        ),
        Field::TalkPartnerCurrentRoom => Value::I32(
            typed_actor(facts, TypedFactId::TalkPartner)?
                .current_room
                .into(),
        ),
        Field::GrabbedActorExists => {
            Value::Bool(typed_actor_exists(facts, TypedFactId::GrabbedActor)?)
        }
        Field::GrabbedActorActorName => Value::I32(
            typed_actor(facts, TypedFactId::GrabbedActor)?
                .actor_name
                .into(),
        ),
        Field::GrabbedActorSetId => {
            Value::U32(typed_actor(facts, TypedFactId::GrabbedActor)?.set_id.into())
        }
        Field::GrabbedActorHomeRoom => Value::I32(
            typed_actor(facts, TypedFactId::GrabbedActor)?
                .home_room
                .into(),
        ),
        Field::GrabbedActorCurrentRoom => Value::I32(
            typed_actor(facts, TypedFactId::GrabbedActor)?
                .current_room
                .into(),
        ),
        Field::TalkPartnerHomePositionX => {
            Value::F32(typed_actor(facts, TypedFactId::TalkPartner)?.home_position?[0])
        }
        Field::TalkPartnerHomePositionY => {
            Value::F32(typed_actor(facts, TypedFactId::TalkPartner)?.home_position?[1])
        }
        Field::TalkPartnerHomePositionZ => {
            Value::F32(typed_actor(facts, TypedFactId::TalkPartner)?.home_position?[2])
        }
        Field::GrabbedActorHomePositionX => {
            Value::F32(typed_actor(facts, TypedFactId::GrabbedActor)?.home_position?[0])
        }
        Field::GrabbedActorHomePositionY => {
            Value::F32(typed_actor(facts, TypedFactId::GrabbedActor)?.home_position?[1])
        }
        Field::GrabbedActorHomePositionZ => {
            Value::F32(typed_actor(facts, TypedFactId::GrabbedActor)?.home_position?[2])
        }
        _ => return None,
    })
}

fn typed_value(facts: &TypedFactResponse, id: TypedFactId) -> Option<&TypedFactValue> {
    let entry = facts
        .entries
        .binary_search_by_key(&id, |entry| entry.id)
        .ok()
        .map(|index| &facts.entries[index])?;
    (entry.status == TypedFactStatus::Present)
        .then_some(entry.value.as_ref())
        .flatten()
}

fn typed_bool(facts: &TypedFactResponse, id: TypedFactId) -> Option<bool> {
    match typed_value(facts, id)? {
        TypedFactValue::Boolean(value) => Some(*value),
        _ => None,
    }
}

fn typed_i32(facts: &TypedFactResponse, id: TypedFactId) -> Option<i32> {
    match typed_value(facts, id)? {
        TypedFactValue::I32(value) => Some(*value),
        _ => None,
    }
}

fn typed_u32(facts: &TypedFactResponse, id: TypedFactId) -> Option<u32> {
    match typed_value(facts, id)? {
        TypedFactValue::U32(value) => Some(*value),
        _ => None,
    }
}

fn typed_vec3(facts: &TypedFactResponse, id: TypedFactId) -> Option<[f32; 3]> {
    match typed_value(facts, id)? {
        TypedFactValue::Vec3F32(value) => Some(*value),
        _ => None,
    }
}

fn typed_stage_code(facts: &TypedFactResponse, id: TypedFactId) -> Option<&str> {
    match typed_value(facts, id)? {
        TypedFactValue::StageCode(value) => Some(value),
        _ => None,
    }
}

fn typed_actor(facts: &TypedFactResponse, id: TypedFactId) -> Option<&TypedFactActorIdentity> {
    match typed_value(facts, id)? {
        TypedFactValue::ActorIdentity(value) => Some(value),
        _ => None,
    }
}

fn typed_actor_exists(facts: &TypedFactResponse, id: TypedFactId) -> Option<bool> {
    let entry = facts
        .entries
        .binary_search_by_key(&id, |entry| entry.id)
        .ok()
        .map(|index| &facts.entries[index])?;
    match entry.status {
        TypedFactStatus::Present => Some(true),
        TypedFactStatus::Absent => Some(false),
        _ => None,
    }
}

fn trace_query(record: &crate::trace::TraceRecord, fact: &QueryFact) -> Option<Value> {
    match fact {
        QueryFact::PlayerInAabb { minimum, maximum } if record.player_present() => {
            Some(Value::Bool((0..3).all(|axis| {
                record.position[axis] >= minimum[axis] && record.position[axis] <= maximum[axis]
            })))
        }
        QueryFact::PlayerPlaneSignedDistance { point, normal } if record.player_present() => {
            let length =
                (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
            Some(Value::F32(
                ((record.position[0] - point[0]) * normal[0]
                    + (record.position[1] - point[1]) * normal[1]
                    + (record.position[2] - point[2]) * normal[2])
                    / length,
            ))
        }
        _ => None,
    }
}

fn compare_trace_values(actual: &Value, operator: Comparison, expected: &Value) -> bool {
    macro_rules! ordered {
        ($left:expr, $right:expr) => {
            match operator {
                Comparison::Equal => $left == $right,
                Comparison::NotEqual => $left != $right,
                Comparison::Less => $left < $right,
                Comparison::LessEqual => $left <= $right,
                Comparison::Greater => $left > $right,
                Comparison::GreaterEqual => $left >= $right,
                Comparison::HasAll | Comparison::HasAny => false,
            }
        };
    }
    match (actual, expected) {
        (Value::Bool(left), Value::Bool(right)) => ordered!(*left, *right),
        (Value::U32(left), Value::U32(right)) => match operator {
            Comparison::HasAll => left & right == *right,
            Comparison::HasAny => left & right != 0,
            _ => ordered!(*left, *right),
        },
        (Value::U64(left), Value::U64(right)) => match operator {
            Comparison::HasAll => left & right == *right,
            Comparison::HasAny => left & right != 0,
            _ => ordered!(*left, *right),
        },
        (Value::I32(left), Value::I32(right)) => ordered!(*left, *right),
        (Value::F32(left), Value::F32(right)) => match operator {
            Comparison::Equal => left.to_bits() == right.to_bits(),
            Comparison::NotEqual => left.to_bits() != right.to_bits(),
            _ => ordered!(*left, *right),
        },
        (Value::Symbol(left), Value::Symbol(right)) => ordered!(left, right),
        (Value::U32(left), Value::Symbol(right)) if *left <= 1 => {
            let expected = match right.as_str() {
                "boot" => 0,
                "tick" => 1,
                _ => return false,
            };
            ordered!(*left, expected)
        }
        (Value::ProcedureNumber(left), Value::ProcedureNumber(right)) => ordered!(*left, *right),
        _ => false,
    }
}

/// Parse, validate, and compile source in one operation.
mod binary;
pub use binary::*;
#[cfg(test)]
#[path = "milestone_dsl/tests.rs"]
mod tests;
