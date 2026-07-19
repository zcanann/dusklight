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
pub const WIRE_VERSION: (u16, u16) = (1, 7);
pub const LANGUAGE_VERSION: (u16, u16) = (1, 7);
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
    TitleLogoSkipReady = 76,
    TitleStartReady = 77,
    NameEntryActive = 78,
    NameEntryCharacterSelectReady = 79,
    NameEntryInputReady = 80,
    NameEntrySelectionProcedure = 81,
    FileSelectNoSaveReady = 82,
    FileSelectDataSelectReady = 83,
    FileSelectKeyWaitReady = 84,
    FileSelectYesNoReady = 85,
    TitlePresent = 86,
    TitleProcedure = 87,
    NameScenePresent = 88,
    NameSceneProcedure = 89,
    FileSelectPresent = 90,
    FileSelectProcedure = 91,
    FileSelectCardCheckProcedure = 92,
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
            Self::TitleLogoSkipReady => "menu.title.logo_skip_ready",
            Self::TitleStartReady => "menu.title.start_ready",
            Self::NameEntryActive => "menu.name_entry.active",
            Self::NameEntryCharacterSelectReady => "menu.name_entry.character_select_ready",
            Self::NameEntryInputReady => "menu.name_entry.input_ready",
            Self::NameEntrySelectionProcedure => "menu.name_entry.selection_procedure",
            Self::FileSelectNoSaveReady => "menu.file_select.no_save_ready",
            Self::FileSelectDataSelectReady => "menu.file_select.data_select_ready",
            Self::FileSelectKeyWaitReady => "menu.file_select.key_wait_ready",
            Self::FileSelectYesNoReady => "menu.file_select.yes_no_ready",
            Self::TitlePresent => "menu.title.present",
            Self::TitleProcedure => "menu.title.procedure",
            Self::NameScenePresent => "menu.name_scene.present",
            Self::NameSceneProcedure => "menu.name_scene.procedure",
            Self::FileSelectPresent => "menu.file_select.present",
            Self::FileSelectProcedure => "menu.file_select.procedure",
            Self::FileSelectCardCheckProcedure => "menu.file_select.card_check_procedure",
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
            Self::NameEntrySelectionProcedure
            | Self::TitleProcedure
            | Self::NameSceneProcedure
            | Self::FileSelectProcedure
            | Self::FileSelectCardCheckProcedure => FieldType::U32,
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
            Self::TitleLogoSkipReady
            | Self::TitleStartReady
            | Self::NameEntryActive
            | Self::NameEntryCharacterSelectReady
            | Self::NameEntryInputReady
            | Self::FileSelectNoSaveReady
            | Self::FileSelectDataSelectReady
            | Self::FileSelectKeyWaitReady
            | Self::FileSelectYesNoReady
            | Self::TitlePresent
            | Self::NameScenePresent
            | Self::FileSelectPresent => FieldType::Bool,
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
        (1..=92).find_map(|id| {
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
            76 => Self::TitleLogoSkipReady,
            77 => Self::TitleStartReady,
            78 => Self::NameEntryActive,
            79 => Self::NameEntryCharacterSelectReady,
            80 => Self::NameEntryInputReady,
            81 => Self::NameEntrySelectionProcedure,
            82 => Self::FileSelectNoSaveReady,
            83 => Self::FileSelectDataSelectReady,
            84 => Self::FileSelectKeyWaitReady,
            85 => Self::FileSelectYesNoReady,
            86 => Self::TitlePresent,
            87 => Self::TitleProcedure,
            88 => Self::NameScenePresent,
            89 => Self::NameSceneProcedure,
            90 => Self::FileSelectPresent,
            91 => Self::FileSelectProcedure,
            92 => Self::FileSelectCardCheckProcedure,
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

mod binary;
mod compile;
mod format;
mod parser;
mod recorded_trace;

use parser::{
    canonical_float, valid_stage_name, validate_comparison, validate_program,
    validate_query_comparison, validate_query_fact, validate_text,
};

pub use binary::*;
pub use compile::{compile, value_projection_identity};
pub use format::format;
pub use parser::parse;
pub use recorded_trace::{RecordedTraceMilestoneHit, evaluate_recorded_trace};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
