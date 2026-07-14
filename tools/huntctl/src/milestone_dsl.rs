//! A bounded, deterministic milestone language for native per-tick evaluation.
//!
//! The source AST is intentionally suitable for visual editors. Compilation
//! produces canonical postfix bytecode: it has no jumps, loops, or mutable
//! state other than the evaluator-owned `stable` counter for each definition.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const MAGIC: [u8; 4] = *b"DMSP";
pub const WIRE_VERSION: (u16, u16) = (1, 0);
pub const LANGUAGE_VERSION: (u16, u16) = (1, 0);
pub const MAX_DEFINITIONS: usize = 256;
pub const MAX_NAME_BYTES: usize = 96;
pub const MAX_SYMBOL_BYTES: usize = 64;
pub const MAX_OPS: usize = 256;
pub const MAX_EXPRESSION_DEPTH: usize = 32;
pub const MAX_BINARY_BYTES: usize = 1024 * 1024;

const DEFINITION_DOMAIN: &[u8] = b"dusklight.milestone.definition/v1\0";
const PROGRAM_DOMAIN: &[u8] = b"dusklight.milestone.program/v1\0";
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
    Not(Box<Expression>),
    And(Box<Expression>, Box<Expression>),
    Or(Box<Expression>, Box<Expression>),
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FieldType {
    Bool,
    U64,
    I32,
    F32,
    Symbol,
    Enum,
    Procedure,
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
        }
    }

    fn field_type(self) -> FieldType {
        match self {
            Self::BoundaryKind => FieldType::Enum,
            Self::BoundaryIndex | Self::TapeFrame => FieldType::U64,
            Self::StageName | Self::NextStageName => FieldType::Symbol,
            Self::StageRoom
            | Self::StageLayer
            | Self::StageSpawn
            | Self::NextStageRoom
            | Self::NextStageLayer
            | Self::NextStageSpawn => FieldType::I32,
            Self::PlayerExists
            | Self::EventRunning
            | Self::BoundaryReached
            | Self::PlayerIsLink
            | Self::NextStageEnabled => FieldType::Bool,
            Self::PlayerPositionX
            | Self::PlayerPositionY
            | Self::PlayerPositionZ
            | Self::PlayerSpeed => FieldType::F32,
            Self::PlayerProcedure => FieldType::Procedure,
            Self::EventId => FieldType::I32,
        }
    }

    fn parse(path: &str) -> Option<Self> {
        (1..=22).find_map(|id| {
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
            '{' | '}' | '(' | ')' => {
                let kind = match chars[at] {
                    '{' => TokenKind::LeftBrace,
                    '}' => TokenKind::RightBrace,
                    '(' => TokenKind::LeftParen,
                    _ => TokenKind::RightParen,
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
            _ => {
                return Err(self.at_error(
                    &version_token,
                    "unsupported or missing language version; expected 1.0",
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
        let field = Field::parse(path).ok_or_else(|| {
            self.at_error(&field_token, format!("unknown milestone field {path:?}"))
        })?;
        let operator = match self.peek().kind {
            TokenKind::Equal => Some(Comparison::Equal),
            TokenKind::NotEqual => Some(Comparison::NotEqual),
            TokenKind::Less => Some(Comparison::Less),
            TokenKind::LessEqual => Some(Comparison::LessEqual),
            TokenKind::Greater => Some(Comparison::Greater),
            TokenKind::GreaterEqual => Some(Comparison::GreaterEqual),
            _ => None,
        };
        let Some(operator) = operator else {
            if field.field_type() == FieldType::Bool {
                return Ok(Expression::Compare {
                    field,
                    operator: Comparison::Equal,
                    value: Value::Bool(true),
                });
            }
            return Err(self.at_error(&field_token, "non-boolean field requires a comparison"));
        };
        self.at += 1;
        let value_token = self.take();
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
    let number = || match &token.kind {
        TokenKind::Number(value) => Ok(value.as_str()),
        _ => Err(format!("{} requires a numeric value", field.path())),
    };
    let string = || match &token.kind {
        TokenKind::String(value) => {
            validate_text(value, MAX_SYMBOL_BYTES, false)?;
            Ok(value.clone())
        }
        _ => Err(format!("{} requires a quoted symbolic value", field.path())),
    };
    match field.field_type() {
        FieldType::Bool => match &token.kind {
            TokenKind::Word(value) if value == "true" => Ok(Value::Bool(true)),
            TokenKind::Word(value) if value == "false" => Ok(Value::Bool(false)),
            _ => Err(format!("{} requires true or false", field.path())),
        },
        FieldType::U64 => number()?
            .parse::<u64>()
            .map(Value::U64)
            .map_err(|_| format!("{} requires an unsigned 64-bit integer", field.path())),
        FieldType::I32 => number()?
            .parse::<i32>()
            .map(Value::I32)
            .map_err(|_| format!("{} requires a signed 32-bit integer", field.path())),
        FieldType::F32 => {
            let value = number()?
                .parse::<f32>()
                .map_err(|_| format!("{} requires a finite 32-bit float", field.path()))?;
            if !value.is_finite() {
                return Err(format!("{} requires a finite 32-bit float", field.path()));
            }
            Ok(Value::F32(canonical_float(value)))
        }
        FieldType::Symbol => string().map(Value::Symbol),
        FieldType::Enum => match &token.kind {
            TokenKind::String(_) => string().map(Value::Symbol),
            TokenKind::Number(_) => number()?
                .parse::<u32>()
                .map(Value::U32)
                .map_err(|_| format!("{} requires a u32 or quoted symbol", field.path())),
            _ => Err(format!("{} requires a u32 or quoted symbol", field.path())),
        },
        FieldType::Procedure => match &token.kind {
            TokenKind::String(_) => {
                let symbol = string()?;
                Ok(Value::ProcedureSymbol(canonical_procedure_symbol(&symbol)))
            }
            TokenKind::Number(_) => number()?
                .parse::<u32>()
                .map(Value::ProcedureNumber)
                .map_err(|_| format!("{} requires a u32 or quoted symbol", field.path())),
            _ => Err(format!("{} requires a u32 or quoted symbol", field.path())),
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
        FieldType::U64 | FieldType::I32 | FieldType::F32
    ) && !matches!(operator, Comparison::Equal | Comparison::NotEqual)
    {
        return Err(format!("field {} supports only == and !=", field.path()));
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
    if (program.version.major, program.version.minor) != LANGUAGE_VERSION {
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
        let mut operations = 0;
        validate_expression(&definition.when, 1, &mut operations)?;
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
            validate_comparison(*field, *operator, value)?;
            *operations += 3;
        }
        Expression::Not(inner) => {
            validate_expression(inner, depth + 1, operations)?;
            *operations += 1;
        }
        Expression::And(left, right) | Expression::Or(left, right) => {
            validate_expression(left, depth + 1, operations)?;
            validate_expression(right, depth + 1, operations)?;
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
        output.push_str("  when ");
        format_expression(&definition.when, 0, &mut output);
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
        Expression::Compare { .. } => 4,
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
        encode_expression(&definition.when, &mut bytecode, &mut operation_count)?;
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
    push_u16(&mut bytes, WIRE_VERSION.1);
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
    if wire != WIRE_VERSION {
        return Err(BinaryError(format!(
            "unsupported milestone wire version {}.{}",
            wire.0, wire.1
        )));
    }
    let version = LanguageVersion {
        major: cursor.u16()?,
        minor: cursor.u16()?,
    };
    if (version.major, version.minor) != LANGUAGE_VERSION {
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
    let when = decode_expression(bytecode, operation_count)?;
    Ok((
        MilestoneDefinition {
            name: name.clone(),
            phase,
            stable_ticks,
            when,
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
    Value(Value),
    Expression(Expression),
}

fn decode_expression(bytes: &[u8], operation_count: u16) -> Result<Expression, BinaryError> {
    let mut cursor = Cursor::new(bytes);
    let mut stack = Vec::new();
    for _ in 0..operation_count {
        let opcode = cursor.u8()?;
        match opcode {
            0x01 => {
                let id = cursor.u8()?;
                stack.push(StackItem::Field(Field::from_id(id).ok_or_else(|| {
                    BinaryError(format!("unknown milestone field ID {id}"))
                })?));
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
            0x20..=0x25 => {
                let operator = match opcode {
                    0x20 => Comparison::Equal,
                    0x21 => Comparison::NotEqual,
                    0x22 => Comparison::Less,
                    0x23 => Comparison::LessEqual,
                    0x24 => Comparison::Greater,
                    _ => Comparison::GreaterEqual,
                };
                let value = match stack.pop() {
                    Some(StackItem::Value(value)) => value,
                    _ => return Err(BinaryError("comparison requires a literal value".into())),
                };
                let field = match stack.pop() {
                    Some(StackItem::Field(field)) => field,
                    _ => return Err(BinaryError("comparison requires a field".into())),
                };
                validate_comparison(field, operator, &value).map_err(BinaryError)?;
                stack.push(StackItem::Expression(Expression::Compare {
                    field,
                    operator,
                    value,
                }));
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
    if stack.len() != 1 {
        return Err(BinaryError(
            "milestone bytecode does not yield one boolean".into(),
        ));
    }
    pop_expression(&mut stack, "program result")
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
}
