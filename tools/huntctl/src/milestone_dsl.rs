//! A bounded, deterministic milestone language for native per-tick evaluation.
//!
//! The source AST is intentionally suitable for visual editors. Compilation
//! produces canonical postfix bytecode: it has no jumps, loops, or mutable
//! state other than the evaluator-owned `stable` counter for each definition.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const MAGIC: [u8; 4] = *b"DMSP";
pub const WIRE_VERSION: (u16, u16) = (1, 4);
pub const LANGUAGE_VERSION: (u16, u16) = (1, 4);
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlacedActorSelector {
    pub stage: String,
    pub home_room: i8,
    pub set_id: u16,
    pub actor_name: i16,
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
            Self::PlayerExists
            | Self::EventRunning
            | Self::BoundaryReached
            | Self::PlayerIsLink
            | Self::NextStageEnabled => FieldType::Bool,
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
            Self::PlayerProcedure => FieldType::Procedure,
            Self::EventId => FieldType::I32,
        }
    }

    fn parse(path: &str) -> Option<Self> {
        (1..=58).find_map(|id| {
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
            _ => {
                return Err(self.at_error(
                    &version_token,
                    "unsupported or missing language version; expected 1.0 through 1.4",
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
            if !valid_stage_name(&selector.stage) {
                return Err(
                    "placed actor stage names must be 1..8 ASCII uppercase, digit, or underscore bytes"
                        .into(),
                );
            }
            if !(-1..=63).contains(&selector.home_room) {
                return Err("placed actor home room must be -1..63".into());
            }
            if selector.set_id == u16::MAX {
                return Err("placed actor set ID 65535 is reserved as unavailable".into());
            }
            if selector.actor_name < 0 {
                return Err("placed actor name must be nonnegative".into());
            }
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
                    if evaluate_trace_expression(steps[0], record) {
                        state.sequence_next = 1;
                        state.sequence_elapsed = 0;
                    }
                    continue;
                }
                let next_elapsed = state.sequence_elapsed.saturating_add(1);
                if next_elapsed > definition.within_ticks.unwrap() {
                    state.sequence_next = usize::from(evaluate_trace_expression(steps[0], record));
                    state.sequence_elapsed = 0;
                    continue;
                }
                state.sequence_elapsed = next_elapsed;
                if evaluate_trace_expression(steps[state.sequence_next], record) {
                    state.sequence_next += 1;
                } else if evaluate_trace_expression(steps[0], record) {
                    state.sequence_next = 1;
                    state.sequence_elapsed = 0;
                }
                if state.sequence_next == steps.len() {
                    state.hit = Some(capture());
                }
                continue;
            }
            if evaluate_trace_expression(&definition.when, record) {
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

fn evaluate_trace_expression(expression: &Expression, record: &crate::trace::TraceRecord) -> bool {
    match expression {
        Expression::Compare {
            field,
            operator,
            value,
        } => trace_field(record, *field)
            .is_some_and(|actual| compare_trace_values(&actual, *operator, value)),
        Expression::Query {
            fact,
            operator,
            value,
        } => trace_query(record, fact)
            .is_some_and(|actual| compare_trace_values(&actual, *operator, value)),
        Expression::Not(inner) => !evaluate_trace_expression(inner, record),
        Expression::And(left, right) => {
            evaluate_trace_expression(left, record) && evaluate_trace_expression(right, record)
        }
        Expression::Or(left, right) => {
            evaluate_trace_expression(left, record) || evaluate_trace_expression(right, record)
        }
    }
}

fn trace_field(record: &crate::trace::TraceRecord, field: Field) -> Option<Value> {
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
        Field::StageName if stage => Value::Symbol(record.stage_name.clone()),
        Field::StageRoom if stage => Value::I32(record.room.into()),
        Field::StageLayer if stage => Value::I32(record.layer.into()),
        Field::StageSpawn if stage => Value::I32(record.point.into()),
        Field::NextStageName if stage => Value::Symbol(record.next_stage_name.clone()),
        Field::NextStageRoom if stage => Value::I32(record.next_room.into()),
        Field::NextStageLayer if stage => Value::I32(record.next_layer.into()),
        Field::NextStageSpawn if stage => Value::I32(record.next_point.into()),
        Field::NextStageEnabled if stage => Value::Bool(record.next_stage_enabled),
        Field::PlayerExists => Value::Bool(player && record.player_present()),
        Field::PlayerIsLink if player => Value::Bool(record.player_is_link()),
        Field::PlayerProcessId if player => Value::U32(record.player_session_process_id?),
        Field::PlayerActorName if player => Value::I32(record.player_actor_name.into()),
        Field::PlayerPositionX if player => Value::F32(record.position[0]),
        Field::PlayerPositionY if player => Value::F32(record.position[1]),
        Field::PlayerPositionZ if player => Value::F32(record.position[2]),
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
        Field::EventRunning if event => Value::Bool(record.event_running()),
        Field::EventId if event => Value::I32(record.event_id.into()),
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
        _ => return None,
    })
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
pub fn compile_source(source: &str) -> Result<CompiledMilestones, DslError> {
    let program = parse(source)?;
    compile(&program).map_err(|error| DslError {
        line: 1,
        column: 1,
        message: error.to_string(),
    })
}

fn definition_identity_bytes(
    name: &[u8],
    phase: EvaluationPhase,
    stable_ticks: u16,
    operation_count: u16,
    bytecode: &[u8],
) -> Result<Vec<u8>, BinaryError> {
    let mut identity = Vec::with_capacity(12 + name.len() + bytecode.len());
    push_u16(
        &mut identity,
        u16::try_from(name.len()).map_err(|_| BinaryError("milestone name too long".into()))?,
    );
    identity.extend_from_slice(name);
    identity.push(phase as u8);
    identity.push(0);
    push_u16(&mut identity, stable_ticks);
    push_u16(&mut identity, operation_count);
    push_u32(&mut identity, usize_u32(bytecode.len(), "bytecode")?);
    identity.extend_from_slice(bytecode);
    Ok(identity)
}

fn encode_expression(
    expression: &Expression,
    output: &mut Vec<u8>,
    operations: &mut u16,
) -> Result<(), BinaryError> {
    match expression {
        Expression::Compare {
            field,
            operator,
            value,
        } => {
            output.extend_from_slice(&[0x01, *field as u8]);
            increment_ops(operations, 1)?;
            encode_value(value, output)?;
            increment_ops(operations, 1)?;
            output.push(*operator as u8);
            increment_ops(operations, 1)?;
        }
        Expression::Query {
            fact,
            operator,
            value,
        } => {
            encode_query_fact(fact, output)?;
            increment_ops(operations, 1)?;
            encode_value(value, output)?;
            increment_ops(operations, 1)?;
            output.push(*operator as u8);
            increment_ops(operations, 1)?;
        }
        Expression::Not(inner) => {
            encode_expression(inner, output, operations)?;
            output.push(0x30);
            increment_ops(operations, 1)?;
        }
        Expression::And(left, right) | Expression::Or(left, right) => {
            encode_expression(left, output, operations)?;
            encode_expression(right, output, operations)?;
            output.push(if matches!(expression, Expression::And(..)) {
                0x31
            } else {
                0x32
            });
            increment_ops(operations, 1)?;
        }
    }
    Ok(())
}

fn encode_query_fact(fact: &QueryFact, output: &mut Vec<u8>) -> Result<(), BinaryError> {
    validate_query_fact(fact).map_err(BinaryError)?;
    output.push(0x02);
    match fact {
        QueryFact::PlacedActor { selector, field } => {
            output.push(1);
            output.push(*field as u8);
            let mut stage = [0_u8; 8];
            stage[..selector.stage.len()].copy_from_slice(selector.stage.as_bytes());
            output.extend_from_slice(&stage);
            output.push(selector.home_room as u8);
            push_u16(output, selector.set_id);
            output.extend_from_slice(&selector.actor_name.to_le_bytes());
        }
        QueryFact::Flag { selector } => {
            output.push(2);
            output.push(selector.domain as u8);
            output.push(selector.room as u8);
            push_u16(output, selector.index);
        }
        QueryFact::PlayerInAabb { minimum, maximum } => {
            output.push(3);
            for value in minimum.iter().chain(maximum) {
                output.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
        QueryFact::PlayerPlaneSignedDistance { point, normal } => {
            output.push(4);
            for value in point.iter().chain(normal) {
                output.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
    }
    Ok(())
}

fn encode_value(value: &Value, output: &mut Vec<u8>) -> Result<(), BinaryError> {
    match value {
        Value::Bool(value) => output.extend_from_slice(&[0x10, u8::from(*value)]),
        Value::U32(value) => {
            output.push(0x11);
            push_u32(output, *value);
        }
        Value::U64(value) => {
            output.push(0x12);
            output.extend_from_slice(&value.to_le_bytes());
        }
        Value::I32(value) => {
            output.push(0x13);
            output.extend_from_slice(&value.to_le_bytes());
        }
        Value::F32(value) => {
            if !value.is_finite() || value.to_bits() != canonical_float(*value).to_bits() {
                return Err(BinaryError("noncanonical floating-point constant".into()));
            }
            output.push(0x14);
            output.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        Value::Symbol(value) => encode_string_value(0x15, value, output)?,
        Value::ProcedureNumber(value) => {
            output.push(0x16);
            push_u32(output, *value);
        }
        Value::ProcedureSymbol(value) => encode_string_value(0x17, value, output)?,
    }
    Ok(())
}

fn encode_string_value(opcode: u8, value: &str, output: &mut Vec<u8>) -> Result<(), BinaryError> {
    validate_text(value, MAX_SYMBOL_BYTES, false).map_err(BinaryError)?;
    output.push(opcode);
    output.push(u8::try_from(value.len()).map_err(|_| BinaryError("symbol is too long".into()))?);
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn increment_ops(operations: &mut u16, amount: u16) -> Result<(), BinaryError> {
    *operations = operations
        .checked_add(amount)
        .ok_or_else(|| BinaryError("operation count overflow".into()))?;
    if usize::from(*operations) > MAX_OPS {
        return Err(BinaryError(format!(
            "expression exceeds {MAX_OPS} operations"
        )));
    }
    Ok(())
}

fn program_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::new()
        .chain_update(PROGRAM_DOMAIN)
        .chain_update(&bytes[..20])
        .chain_update(&bytes[HEADER_BYTES..])
        .finalize()
        .into()
}

fn usize_u32(value: usize, context: &str) -> Result<u32, BinaryError> {
    u32::try_from(value).map_err(|_| BinaryError(format!("{context} is too large")))
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

/// Strictly decode canonical DMSP v1 bytes and verify all embedded identities.
pub fn decode(bytes: &[u8]) -> Result<DecodedMilestones, BinaryError> {
    if bytes.len() < HEADER_BYTES || bytes.len() > MAX_BINARY_BYTES {
        return Err(BinaryError("invalid milestone program size".into()));
    }
    let mut cursor = Cursor::new(bytes);
    if cursor.take(4)? != MAGIC {
        return Err(BinaryError("invalid milestone program magic".into()));
    }
    let wire = (cursor.u16()?, cursor.u16()?);
    if wire.0 != WIRE_VERSION.0 || wire.1 > WIRE_VERSION.1 {
        return Err(BinaryError(format!(
            "unsupported milestone wire version {}.{}",
            wire.0, wire.1
        )));
    }
    let version = LanguageVersion {
        major: cursor.u16()?,
        minor: cursor.u16()?,
    };
    if version.major != LANGUAGE_VERSION.0
        || version.minor > LANGUAGE_VERSION.1
        || version.minor != wire.1
    {
        return Err(BinaryError("unsupported milestone language version".into()));
    }
    let definition_count = usize::from(cursor.u16()?);
    if definition_count == 0 || definition_count > MAX_DEFINITIONS {
        return Err(BinaryError("invalid milestone definition count".into()));
    }
    if cursor.u16()? != 0 {
        return Err(BinaryError("nonzero milestone header reservation".into()));
    }
    let payload_len = cursor.u32()? as usize;
    let expected_program_digest = cursor.array32()?;
    if payload_len != cursor.remaining() {
        return Err(BinaryError("milestone payload length mismatch".into()));
    }
    let actual_program_digest = program_digest(bytes);
    if expected_program_digest != actual_program_digest {
        return Err(BinaryError("milestone program digest mismatch".into()));
    }

    let mut definitions = Vec::with_capacity(definition_count);
    let mut identities = Vec::with_capacity(definition_count);
    for _ in 0..definition_count {
        let record_len = cursor.u32()? as usize;
        if record_len < RECORD_FIXED_BYTES || record_len > cursor.remaining() {
            return Err(BinaryError("invalid milestone record length".into()));
        }
        let record_bytes = cursor.take(record_len)?;
        let (definition, identity) = decode_definition(record_bytes)?;
        definitions.push(definition);
        identities.push(identity);
    }
    if cursor.remaining() != 0 {
        return Err(BinaryError("trailing milestone program data".into()));
    }
    let program = MilestoneProgram {
        version,
        definitions,
    };
    validate_program(&program).map_err(BinaryError)?;
    let canonical = compile(&program)?;
    if canonical.bytes != bytes {
        return Err(BinaryError(
            "noncanonical milestone program encoding".into(),
        ));
    }
    Ok(DecodedMilestones {
        program,
        program_sha256: actual_program_digest,
        definitions: identities,
    })
}

fn decode_definition(
    bytes: &[u8],
) -> Result<(MilestoneDefinition, CompiledDefinitionIdentity), BinaryError> {
    let mut cursor = Cursor::new(bytes);
    let name_len = usize::from(cursor.u16()?);
    if name_len == 0 || name_len > MAX_NAME_BYTES {
        return Err(BinaryError("invalid milestone name length".into()));
    }
    let name = cursor.string(name_len)?;
    validate_text(&name, MAX_NAME_BYTES, false).map_err(BinaryError)?;
    let phase = match cursor.u8()? {
        0 => EvaluationPhase::PreInput,
        1 => EvaluationPhase::PostSim,
        _ => return Err(BinaryError("invalid milestone evaluation phase".into())),
    };
    if cursor.u8()? != 0 {
        return Err(BinaryError("nonzero milestone record reservation".into()));
    }
    let stable_ticks = cursor.u16()?;
    if stable_ticks == 0 {
        return Err(BinaryError("zero milestone stable count".into()));
    }
    let operation_count = cursor.u16()?;
    if operation_count == 0 || usize::from(operation_count) > MAX_OPS {
        return Err(BinaryError("invalid milestone operation count".into()));
    }
    let bytecode_len = cursor.u32()? as usize;
    let expected_digest = cursor.array32()?;
    if bytecode_len != cursor.remaining() {
        return Err(BinaryError("milestone bytecode length mismatch".into()));
    }
    let bytecode = cursor.take(bytecode_len)?;
    let identity_bytes = definition_identity_bytes(
        name.as_bytes(),
        phase,
        stable_ticks,
        operation_count,
        bytecode,
    )?;
    let actual_digest: [u8; 32] = Sha256::new()
        .chain_update(DEFINITION_DOMAIN)
        .chain_update(identity_bytes)
        .finalize()
        .into();
    if actual_digest != expected_digest {
        return Err(BinaryError("milestone definition digest mismatch".into()));
    }
    let (when, then, within_ticks, projections) = decode_expression(bytecode, operation_count)?;
    Ok((
        MilestoneDefinition {
            name: name.clone(),
            phase,
            stable_ticks,
            when,
            then,
            within_ticks,
            projections,
        },
        CompiledDefinitionIdentity {
            name,
            sha256: actual_digest,
        },
    ))
}

#[derive(Clone, Debug)]
enum StackItem {
    Field(Field),
    Query(QueryFact),
    Value(Value),
    Expression(Expression),
}

fn decode_expression(
    bytes: &[u8],
    operation_count: u16,
) -> Result<
    (
        Expression,
        Vec<Expression>,
        Option<u16>,
        Vec<ValueProjection>,
    ),
    BinaryError,
> {
    let mut cursor = Cursor::new(bytes);
    let mut stack = Vec::new();
    let mut sequence_within = None;
    let mut expected_steps = 0_usize;
    let mut sequence_steps = Vec::new();
    let mut projections = Vec::new();
    let mut current_projection: Option<ValueProjection> = None;
    let mut projection_items_remaining = 0_usize;
    let mut metadata_started = false;
    for operation_index in 0..operation_count {
        let opcode = cursor.u8()?;
        if metadata_started && !(0x50..=0x53).contains(&opcode) {
            return Err(BinaryError(
                "predicate opcodes may not follow value projections".into(),
            ));
        }
        match opcode {
            0x40 => {
                if operation_index != 0 || sequence_within.is_some() || !stack.is_empty() {
                    return Err(BinaryError("invalid sequence start opcode".into()));
                }
                let within = cursor.u16()?;
                expected_steps = usize::from(cursor.u8()?);
                if within == 0 || !(2..=16).contains(&expected_steps) {
                    return Err(BinaryError("invalid bounded sequence descriptor".into()));
                }
                sequence_within = Some(within);
            }
            0x41 => {
                if sequence_within.is_none() || sequence_steps.len() == expected_steps {
                    return Err(BinaryError("unexpected sequence step terminator".into()));
                }
                let step = pop_expression(&mut stack, "sequence step")?;
                if !stack.is_empty() {
                    return Err(BinaryError(
                        "sequence step does not yield exactly one boolean".into(),
                    ));
                }
                sequence_steps.push(step);
            }
            0x50 => {
                let expression_complete = if sequence_within.is_some() {
                    stack.is_empty() && sequence_steps.len() == expected_steps
                } else {
                    matches!(stack.as_slice(), [StackItem::Expression(_)])
                };
                if !expression_complete
                    || current_projection.is_some()
                    || projections.len() == MAX_PROJECTIONS
                {
                    return Err(BinaryError("invalid value projection start".into()));
                }
                metadata_started = true;
                let name_len = usize::from(cursor.u8()?);
                if name_len == 0 || name_len > MAX_NAME_BYTES {
                    return Err(BinaryError("invalid projection name length".into()));
                }
                let name = cursor.string(name_len)?;
                validate_text(&name, MAX_NAME_BYTES, false).map_err(BinaryError)?;
                projection_items_remaining = usize::from(cursor.u8()?);
                if projection_items_remaining == 0
                    || projection_items_remaining > MAX_PROJECTION_ITEMS
                {
                    return Err(BinaryError("invalid projection item count".into()));
                }
                current_projection = Some(ValueProjection {
                    name,
                    items: Vec::with_capacity(projection_items_remaining),
                });
            }
            0x51..=0x53 => {
                if !metadata_started || projection_items_remaining == 0 {
                    return Err(BinaryError(
                        "projection item has no active projection".into(),
                    ));
                }
                let item = match opcode {
                    0x51 => ValueProjectionItem::Rng {
                        stream: match cursor.u8()? {
                            0 => RngStream::Primary,
                            1 => RngStream::Secondary,
                            _ => return Err(BinaryError("invalid projected RNG stream".into())),
                        },
                    },
                    0x52 => {
                        let stage_bytes = cursor.take(8)?;
                        let stage_len = stage_bytes
                            .iter()
                            .position(|byte| *byte == 0)
                            .unwrap_or(stage_bytes.len());
                        if stage_bytes[stage_len..].iter().any(|byte| *byte != 0) {
                            return Err(BinaryError("noncanonical projected stage padding".into()));
                        }
                        ValueProjectionItem::ActorPopulation {
                            stage: String::from_utf8(stage_bytes[..stage_len].to_vec())
                                .map_err(|_| BinaryError("invalid projected stage".into()))?,
                            room: cursor.u8()? as i8,
                        }
                    }
                    _ => ValueProjectionItem::Flag {
                        selector: FlagSelector {
                            domain: FlagDomain::from_id(cursor.u8()?).ok_or_else(|| {
                                BinaryError("invalid projected flag domain".into())
                            })?,
                            room: cursor.u8()? as i8,
                            index: cursor.u16()?,
                        },
                    },
                };
                current_projection.as_mut().unwrap().items.push(item);
                projection_items_remaining -= 1;
                if projection_items_remaining == 0 {
                    projections.push(current_projection.take().unwrap());
                }
            }
            0x01 => {
                let id = cursor.u8()?;
                stack.push(StackItem::Field(Field::from_id(id).ok_or_else(|| {
                    BinaryError(format!("unknown milestone field ID {id}"))
                })?));
            }
            0x02 => {
                let kind = cursor.u8()?;
                let fact = match kind {
                    1 => {
                        let field_id = cursor.u8()?;
                        let field = ActorFact::from_id(field_id).ok_or_else(|| {
                            BinaryError(format!("unknown actor fact ID {field_id}"))
                        })?;
                        let stage_bytes = cursor.take(8)?;
                        let stage_len = stage_bytes
                            .iter()
                            .position(|byte| *byte == 0)
                            .unwrap_or(stage_bytes.len());
                        if stage_bytes[stage_len..].iter().any(|byte| *byte != 0) {
                            return Err(BinaryError(
                                "noncanonical placed-actor stage padding".into(),
                            ));
                        }
                        let stage = String::from_utf8(stage_bytes[..stage_len].to_vec())
                            .map_err(|_| BinaryError("invalid placed-actor stage".into()))?;
                        QueryFact::PlacedActor {
                            selector: PlacedActorSelector {
                                stage,
                                home_room: cursor.u8()? as i8,
                                set_id: cursor.u16()?,
                                actor_name: cursor.i16()?,
                            },
                            field,
                        }
                    }
                    2 => QueryFact::Flag {
                        selector: FlagSelector {
                            domain: FlagDomain::from_id(cursor.u8()?)
                                .ok_or_else(|| BinaryError("unknown flag fact domain".into()))?,
                            room: cursor.u8()? as i8,
                            index: cursor.u16()?,
                        },
                    },
                    3 | 4 => {
                        let mut values = [0.0_f32; 6];
                        for value in &mut values {
                            *value = f32::from_bits(cursor.u32()?);
                        }
                        if kind == 3 {
                            QueryFact::PlayerInAabb {
                                minimum: values[..3].try_into().unwrap(),
                                maximum: values[3..].try_into().unwrap(),
                            }
                        } else {
                            QueryFact::PlayerPlaneSignedDistance {
                                point: values[..3].try_into().unwrap(),
                                normal: values[3..].try_into().unwrap(),
                            }
                        }
                    }
                    _ => return Err(BinaryError(format!("unknown query fact kind {kind}"))),
                };
                validate_query_fact(&fact).map_err(BinaryError)?;
                stack.push(StackItem::Query(fact));
            }
            0x10 => match cursor.u8()? {
                0 => stack.push(StackItem::Value(Value::Bool(false))),
                1 => stack.push(StackItem::Value(Value::Bool(true))),
                _ => return Err(BinaryError("noncanonical boolean constant".into())),
            },
            0x11 => stack.push(StackItem::Value(Value::U32(cursor.u32()?))),
            0x12 => stack.push(StackItem::Value(Value::U64(cursor.u64()?))),
            0x13 => stack.push(StackItem::Value(Value::I32(cursor.i32()?))),
            0x14 => {
                let value = f32::from_bits(cursor.u32()?);
                if !value.is_finite() || value.to_bits() != canonical_float(value).to_bits() {
                    return Err(BinaryError("noncanonical floating-point constant".into()));
                }
                stack.push(StackItem::Value(Value::F32(value)));
            }
            0x15 => stack.push(StackItem::Value(Value::Symbol(cursor.symbol()?))),
            0x16 => stack.push(StackItem::Value(Value::ProcedureNumber(cursor.u32()?))),
            0x17 => stack.push(StackItem::Value(Value::ProcedureSymbol(cursor.symbol()?))),
            0x20..=0x27 => {
                let operator = match opcode {
                    0x20 => Comparison::Equal,
                    0x21 => Comparison::NotEqual,
                    0x22 => Comparison::Less,
                    0x23 => Comparison::LessEqual,
                    0x24 => Comparison::Greater,
                    0x25 => Comparison::GreaterEqual,
                    0x26 => Comparison::HasAll,
                    _ => Comparison::HasAny,
                };
                let value = match stack.pop() {
                    Some(StackItem::Value(value)) => value,
                    _ => return Err(BinaryError("comparison requires a literal value".into())),
                };
                let expression = match stack.pop() {
                    Some(StackItem::Field(field)) => {
                        validate_comparison(field, operator, &value).map_err(BinaryError)?;
                        Expression::Compare {
                            field,
                            operator,
                            value,
                        }
                    }
                    Some(StackItem::Query(fact)) => {
                        validate_query_comparison(&fact, operator, &value).map_err(BinaryError)?;
                        Expression::Query {
                            fact,
                            operator,
                            value,
                        }
                    }
                    _ => return Err(BinaryError("comparison requires a field or query".into())),
                };
                stack.push(StackItem::Expression(expression));
            }
            0x30 => {
                let inner = pop_expression(&mut stack, "not")?;
                stack.push(StackItem::Expression(Expression::Not(Box::new(inner))));
            }
            0x31 | 0x32 => {
                let right = pop_expression(&mut stack, "boolean operator")?;
                let left = pop_expression(&mut stack, "boolean operator")?;
                stack.push(StackItem::Expression(if opcode == 0x31 {
                    Expression::And(Box::new(left), Box::new(right))
                } else {
                    Expression::Or(Box::new(left), Box::new(right))
                }));
            }
            _ => {
                return Err(BinaryError(format!(
                    "unknown milestone opcode 0x{opcode:02x}"
                )));
            }
        }
        if stack.len() > MAX_OPS {
            return Err(BinaryError("milestone expression stack overflow".into()));
        }
    }
    if cursor.remaining() != 0 {
        return Err(BinaryError("trailing milestone bytecode".into()));
    }
    if current_projection.is_some() {
        return Err(BinaryError("incomplete value projection".into()));
    }
    if let Some(within) = sequence_within {
        if !stack.is_empty() || sequence_steps.len() != expected_steps {
            return Err(BinaryError(
                "bounded sequence does not contain its declared steps".into(),
            ));
        }
        let mut steps = sequence_steps.into_iter();
        let when = steps.next().unwrap();
        return Ok((when, steps.collect(), Some(within), projections));
    }
    if stack.len() != 1 {
        return Err(BinaryError(
            "milestone bytecode does not yield one boolean".into(),
        ));
    }
    Ok((
        pop_expression(&mut stack, "program result")?,
        Vec::new(),
        None,
        projections,
    ))
}

fn pop_expression(stack: &mut Vec<StackItem>, context: &str) -> Result<Expression, BinaryError> {
    match stack.pop() {
        Some(StackItem::Expression(expression)) => Ok(expression),
        _ => Err(BinaryError(format!(
            "{context} requires a boolean expression"
        ))),
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.at
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], BinaryError> {
        let end = self
            .at
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| BinaryError("truncated milestone program".into()))?;
        let value = &self.bytes[self.at..end];
        self.at = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, BinaryError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, BinaryError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn i16(&mut self) -> Result<i16, BinaryError> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, BinaryError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32, BinaryError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, BinaryError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn array32(&mut self) -> Result<[u8; 32], BinaryError> {
        Ok(self.take(32)?.try_into().unwrap())
    }

    fn string(&mut self, length: usize) -> Result<String, BinaryError> {
        String::from_utf8(self.take(length)?.to_vec())
            .map_err(|_| BinaryError("invalid UTF-8 in milestone program".into()))
    }

    fn symbol(&mut self) -> Result<String, BinaryError> {
        let length = usize::from(self.u8()?);
        if length == 0 || length > MAX_SYMBOL_BYTES {
            return Err(BinaryError("invalid milestone symbol length".into()));
        }
        self.string(length)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"
milestones 1.0

milestone boot_ready {
  phase pre_input
  stable 2
  when boundary.kind == "boot" && boundary.index == 0 && player.exists && player.is_link && event.id == -1 && !next_stage.enabled
}

milestone "leave_f_sp103" {
  phase post_sim
  when (stage.name == "F_SP103" && player.procedure == "PROC_WAIT" && player.speed >= 0.0) || (next_stage.enabled && next_stage.name == "F_SP104")
}
"#;

    #[test]
    fn source_ast_format_binary_and_json_round_trip() {
        let program = parse(SOURCE).unwrap();
        assert_eq!(program.definitions.len(), 2);
        assert_eq!(program.definitions[0].stable_ticks, 2);
        assert_eq!(program.definitions[1].stable_ticks, 1);

        let formatted = format(&program).unwrap();
        assert_eq!(parse(&formatted).unwrap(), program);
        let json = serde_json::to_vec(&program).unwrap();
        assert_eq!(
            serde_json::from_slice::<MilestoneProgram>(&json).unwrap(),
            program
        );

        let compiled = compile(&program).unwrap();
        assert_eq!(&compiled.bytes[..4], b"DMSP");
        assert_ne!(compiled.program_sha256, [0; 32]);
        assert_eq!(compiled.definitions.len(), 2);
        assert_ne!(
            compiled.definitions[0].sha256,
            compiled.definitions[1].sha256
        );
        let decoded = decode(&compiled.bytes).unwrap();
        assert_eq!(decoded.program, program);
        assert_eq!(decoded.program_sha256, compiled.program_sha256);
        assert_eq!(decoded.definitions, compiled.definitions);
        assert_eq!(compile(&decoded.program).unwrap().bytes, compiled.bytes);
    }

    #[test]
    fn precedence_parentheses_and_boolean_shorthand_are_exact() {
        let source = r#"milestones 1.0
milestone p {
 phase post_sim
 when player.exists || event.running && !(next_stage.enabled || boundary.reached == false)
}"#;
        let parsed = parse(source).unwrap();
        let Expression::Or(_, right) = &parsed.definitions[0].when else {
            panic!("or must have lowest precedence");
        };
        assert!(matches!(right.as_ref(), Expression::And(..)));
        let formatted = format(&parsed).unwrap();
        assert_eq!(parse(&formatted).unwrap(), parsed);
    }

    #[test]
    fn field_types_symbols_and_properties_are_strict() {
        for invalid in [
            SOURCE.replace("event.id == -1", "event.id == 1.5"),
            SOURCE.replace("player.speed >= 0.0", "player.speed >= NaN"),
            SOURCE.replace("player.exists", "player.exists > true"),
            SOURCE.replace("stage.name == \"F_SP103\"", "stage.name == 3"),
            SOURCE.replace("F_SP103", "f/sp103"),
            SOURCE.replace("PROC_WAIT", "WAIT"),
            SOURCE.replace("boundary.kind == \"boot\"", "boundary.kind == \"startup\""),
            SOURCE.replace("player.is_link", "player.is_zelda"),
            SOURCE.replace("phase pre_input", "phase whenever"),
            SOURCE.replace("stable 2", "stable 0"),
            SOURCE.replace("stable 2", "stable 2\n  mystery no"),
            SOURCE.replace("phase pre_input", "phase pre_input\n  phase post_sim"),
        ] {
            assert!(
                parse(&invalid).is_err(),
                "unexpectedly accepted:\n{invalid}"
            );
        }
        let duplicate = format!(
            "{SOURCE}\n{}",
            &SOURCE[SOURCE.find("milestone boot_ready").unwrap()..]
        );
        assert!(parse(&duplicate).is_err());

        let alias = parse(&SOURCE.replace("PROC_WAIT", "crawl_start")).unwrap();
        assert!(format(&alias).unwrap().contains("PROC_CRAWL_START"));
        assert!(parse(&SOURCE.replace("PROC_WAIT", "crawl")).is_err());
    }

    #[test]
    fn ast_validation_rejects_nonfinite_depth_operations_and_capacity() {
        let mut program = parse(SOURCE).unwrap();
        program.definitions[1].when = Expression::Compare {
            field: Field::PlayerSpeed,
            operator: Comparison::Equal,
            value: Value::F32(f32::NAN),
        };
        assert!(compile(&program).is_err());

        let mut deep = Expression::Compare {
            field: Field::PlayerExists,
            operator: Comparison::Equal,
            value: Value::Bool(true),
        };
        for _ in 0..MAX_EXPRESSION_DEPTH {
            deep = Expression::Not(Box::new(deep));
        }
        program.definitions[1].when = deep;
        assert!(compile(&program).is_err());

        let predicate = Expression::Compare {
            field: Field::EventId,
            operator: Comparison::Equal,
            value: Value::I32(-1),
        };
        let mut excessive = predicate.clone();
        for _ in 0..MAX_OPS {
            excessive = Expression::And(Box::new(excessive), Box::new(predicate.clone()));
        }
        program.definitions[1].when = excessive;
        assert!(compile(&program).is_err());

        let definition = parse(SOURCE).unwrap().definitions[0].clone();
        program.definitions = vec![definition; MAX_DEFINITIONS + 1];
        for (index, definition) in program.definitions.iter_mut().enumerate() {
            definition.name = format!("milestone-{index}");
        }
        assert!(compile(&program).is_err());
    }

    #[test]
    fn every_truncation_and_authenticated_unknown_opcode_is_rejected() {
        let compiled = compile(&parse(SOURCE).unwrap()).unwrap();
        for length in 0..compiled.bytes.len() {
            assert!(
                decode(&compiled.bytes[..length]).is_err(),
                "accepted {length}"
            );
        }

        let mut corrupted = compiled.bytes.clone();
        let record_start = HEADER_BYTES;
        let name_len = u16::from_le_bytes(
            corrupted[record_start + 4..record_start + 6]
                .try_into()
                .unwrap(),
        ) as usize;
        let metadata_start = record_start + 4;
        let digest_start = metadata_start + 2 + name_len + 1 + 1 + 2 + 2 + 4;
        let bytecode_start = digest_start + 32;
        corrupted[bytecode_start] = 0xff;
        let record_len = u32::from_le_bytes(
            corrupted[record_start..record_start + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let record_end = record_start + 4 + record_len;
        let mut identity = corrupted[metadata_start..digest_start].to_vec();
        identity.extend_from_slice(&corrupted[bytecode_start..record_end]);
        let definition_digest: [u8; 32] = Sha256::new()
            .chain_update(DEFINITION_DOMAIN)
            .chain_update(identity)
            .finalize()
            .into();
        corrupted[digest_start..bytecode_start].copy_from_slice(&definition_digest);
        let digest = program_digest(&corrupted);
        corrupted[20..52].copy_from_slice(&digest);
        assert!(
            decode(&corrupted)
                .unwrap_err()
                .0
                .contains("unknown milestone opcode")
        );
    }

    #[test]
    fn identity_covers_name_phase_stability_and_expression() {
        let base = parse(SOURCE).unwrap();
        let identity = compile(&base).unwrap();
        for mutate in [
            |program: &mut MilestoneProgram| program.definitions[0].name.push('x'),
            |program: &mut MilestoneProgram| {
                program.definitions[0].phase = EvaluationPhase::PostSim
            },
            |program: &mut MilestoneProgram| program.definitions[0].stable_ticks += 1,
            |program: &mut MilestoneProgram| {
                program.definitions[0].when = Expression::Compare {
                    field: Field::PlayerExists,
                    operator: Comparison::Equal,
                    value: Value::Bool(false),
                }
            },
        ] {
            let mut changed = base.clone();
            mutate(&mut changed);
            let changed = compile(&changed).unwrap();
            assert_ne!(changed.program_sha256, identity.program_sha256);
            assert_ne!(
                changed.definitions[0].sha256,
                identity.definitions[0].sha256
            );
        }
    }

    #[test]
    fn wire_field_ids_and_opcodes_are_stable() {
        assert_eq!(Field::BoundaryKind as u8, 1);
        assert_eq!(Field::EventId as u8, 15);
        assert_eq!(Field::PlayerIsLink as u8, 21);
        assert_eq!(Field::NextStageEnabled as u8, 22);
        let program =
            parse("milestones 1.0 milestone one { phase pre_input when event.id == -1 }").unwrap();
        let bytes = compile(&program).unwrap().bytes;
        let bytecode_start = HEADER_BYTES + 4 + RECORD_FIXED_BYTES + "one".len();
        assert_eq!(
            &bytes[bytecode_start..bytecode_start + 8],
            &[0x01, 15, 0x13, 0xff, 0xff, 0xff, 0xff, 0x20]
        );
    }

    #[test]
    fn language_1_1_types_flags_timers_hashes_rng_and_collision_facts() {
        let source = r#"milestones 1.1
milestone rich {
  phase post_sim
  stable 3
  when player.actor_name == 253 && player.velocity.y <= 0.0 &&
       player.mode_flags has_all 1024 && player.mode_flags has_any 1028 &&
       player.timer.damage_wait == 0 && player.timer.sword_change_wait <= 4 &&
       event.mode == 2 && event.status >= 1 && event.name_hash.present &&
       event.name_hash.fnv1a32 == 305419896 && rng.primary.state0 == 11 &&
       rng.secondary.calls >= 200 && collision.ground.contact &&
       collision.ground.clearance <= 0.5
}"#;
        let program = parse(source).unwrap();
        let compiled = compile(&program).unwrap();
        assert_eq!(&compiled.bytes[4..12], &[1, 0, 1, 0, 1, 0, 1, 0]);
        let decoded = decode(&compiled.bytes).unwrap();
        assert_eq!(decoded.program, program);
        let formatted = format(&program).unwrap();
        assert!(formatted.contains("player.mode_flags has_all 1024"));
        assert!(formatted.contains("player.mode_flags has_any 1028"));
        assert_eq!(parse(&formatted).unwrap(), program);

        assert!(parse(&source.replace("milestones 1.1", "milestones 1.0")).is_err());
        assert!(parse(&source.replace("has_all 1024", "has_all 0")).is_err());
        assert!(
            parse(&source.replace(
                "collision.ground.contact",
                "collision.ground.contact has_any true"
            ))
            .is_err()
        );
        assert_eq!(Field::PlayerModeFlags as u8, 34);
        assert_eq!(Field::EventNameHash as u8, 42);
        assert_eq!(Field::CollisionGroundClearance as u8, 58);
        assert_eq!(Comparison::HasAll as u8, 0x26);
        assert_eq!(Comparison::HasAny as u8, 0x27);
    }

    #[test]
    fn language_1_2_stable_actor_queries_geometry_and_indexed_flags_round_trip() {
        let source = r#"milestones 1.2

milestone local_actor_goal {
  phase post_sim
  stable 4
  when actor.placed.exists("F_SP103", -1, 7, 42) &&
       actor.placed.position.y("F_SP103", -1, 7, 42) >= -20.0 &&
       actor.placed.distance_to_player("F_SP103", -1, 7, 42) <= 125.5 &&
       actor.placed.current_room("F_SP103", -1, 7, 42) == 0 &&
       actor.placed.health("F_SP103", -1, 7, 42) > 0 &&
       actor.placed.status("F_SP103", -1, 7, 42) has_any 4 &&
       flag.event(821) && flag.temporary(184) == false &&
       flag.dungeon(63) && flag.switch(0, 239)
}
"#;
        let program = parse(source).unwrap();
        let formatted = format(&program).unwrap();
        assert_eq!(parse(&formatted).unwrap(), program);
        assert!(formatted.contains("actor.placed.distance_to_player(\"F_SP103\", -1, 7, 42)"));
        assert!(formatted.contains("flag.switch(0, 239) == true"));

        let compiled = compile(&program).unwrap();
        assert_eq!(&compiled.bytes[4..12], &[1, 0, 2, 0, 1, 0, 2, 0]);
        assert!(compiled.bytes.iter().any(|byte| *byte == 0x02));
        let decoded = decode(&compiled.bytes).unwrap();
        assert_eq!(decoded.program, program);
        assert_eq!(compile(&decoded.program).unwrap().bytes, compiled.bytes);

        for invalid in [
            source.replace("milestones 1.2", "milestones 1.1"),
            source.replace("F_SP103\", -1", "bad-stage\", -1"),
            source.replace("flag.event(821)", "flag.event(822)"),
            source.replace("flag.temporary(184)", "flag.temporary(185)"),
            source.replace("flag.dungeon(63)", "flag.dungeon(64)"),
            source.replace("flag.switch(0, 239)", "flag.switch(0, 240)"),
            source.replace(", 7, 42)", ", 65535, 42)"),
        ] {
            assert!(
                parse(&invalid).is_err(),
                "accepted invalid source: {invalid}"
            );
        }
    }

    #[test]
    fn language_1_3_ranges_regions_planes_transitions_and_sequences_round_trip() {
        let source = r#"milestones 1.3

milestone crossed_plane_after_contact {
  phase post_sim
  within 4
  when collision.ground.contact && player.position.x between -5.0 and 5.0
  then player.in_aabb(-10.0, -20.0, -30.0, 10.0, 20.0, 30.0)
  then event.id == 17
  then player.plane_signed_distance(0.0, 0.0, 0.0, 1.0, 0.0, 0.0) >= 0.0
}

milestone exact_next_tick_transition {
  phase post_sim
  within 1
  when player.procedure == 7
  then player.procedure == 8
}
"#;
        let program = parse(source).unwrap();
        assert_eq!(program.definitions[0].then.len(), 3);
        assert_eq!(program.definitions[0].within_ticks, Some(4));
        let formatted = format(&program).unwrap();
        assert!(!formatted.contains(" between "));
        assert!(formatted.contains("player.position.x >= -5.0 && player.position.x <= 5.0"));
        assert!(formatted.contains("player.in_aabb(-10.0, -20.0, -30.0, 10.0, 20.0, 30.0)"));
        assert_eq!(parse(&formatted).unwrap(), program);

        let compiled = compile(&program).unwrap();
        assert_eq!(&compiled.bytes[4..12], &[1, 0, 3, 0, 1, 0, 3, 0]);
        assert!(compiled.bytes.contains(&0x40));
        assert!(compiled.bytes.contains(&0x41));
        let decoded = decode(&compiled.bytes).unwrap();
        assert_eq!(decoded.program, program);
        assert_eq!(compile(&decoded.program).unwrap().bytes, compiled.bytes);

        for invalid in [
            source.replace("milestones 1.3", "milestones 1.2"),
            source.replace("  within 4\n", ""),
            source.replace("  then player.procedure == 8\n", ""),
            source.replace("  within 4", "  stable 2\n  within 4"),
            source.replace(
                "player.in_aabb(-10.0, -20.0, -30.0, 10.0, 20.0, 30.0)",
                "player.in_aabb(10.0, -20.0, -30.0, -10.0, 20.0, 30.0)",
            ),
            source.replace(
                "player.plane_signed_distance(0.0, 0.0, 0.0, 1.0, 0.0, 0.0)",
                "player.plane_signed_distance(0.0, 0.0, 0.0, 0.0, 0.0, 0.0)",
            ),
            source.replace("between -5.0 and 5.0", "between 5.0 and -5.0"),
        ] {
            assert!(
                parse(&invalid).is_err(),
                "accepted invalid source: {invalid}"
            );
        }
    }

    #[test]
    fn language_1_4_named_value_projections_round_trip() {
        let source = include_str!("../../../tests/fixtures/automation/value_projection.milestones");
        let program = parse(source).unwrap();
        let projections = &program.definitions[0].projections;
        assert_eq!(projections.len(), 1);
        assert_eq!(projections[0].name, "handoff-state");
        assert_eq!(projections[0].items.len(), 5);
        let projection_identity = value_projection_identity(&projections[0]).unwrap();
        let projection_identity_hex = projection_identity
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            projection_identity_hex,
            "acb5c9cd5570ebe610e321a3f5a33856a6af7cfaaf808b5b394f471170fcf5f3"
        );
        let formatted = format(&program).unwrap();
        assert_eq!(parse(&formatted).unwrap(), program);

        let compiled = compile(&program).unwrap();
        assert_eq!(&compiled.bytes[4..12], &[1, 0, 4, 0, 1, 0, 4, 0]);
        for opcode in 0x50..=0x53 {
            assert!(compiled.bytes.contains(&opcode));
        }
        let decoded = decode(&compiled.bytes).unwrap();
        assert_eq!(decoded.program, program);
        assert_eq!(compile(&decoded.program).unwrap().bytes, compiled.bytes);

        for invalid in [
            source.replace("milestones 1.4", "milestones 1.3"),
            source.replace("    rng secondary\n", "    rng primary\n"),
            source.replace("flag event 821", "flag event 822"),
            source.replace(
                "actor_population \"F_SP103\" 1",
                "actor_population \"bad\" 1",
            ),
        ] {
            assert!(
                parse(&invalid).is_err(),
                "accepted invalid source: {invalid}"
            );
        }
    }
}
