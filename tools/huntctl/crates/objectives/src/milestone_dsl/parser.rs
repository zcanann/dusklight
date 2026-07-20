use super::*;
use std::format;

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
            TokenKind::Number(value) if value == "1.7" => LanguageVersion { major: 1, minor: 7 },
            TokenKind::Number(value) if value == "1.8" => LanguageVersion { major: 1, minor: 8 },
            _ => {
                return Err(self.at_error(
                    &version_token,
                    "unsupported or missing language version; expected 1.0 through 1.8",
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
        if path == "event.temporary_byte" {
            self.expect(
                TokenKind::LeftParen,
                "expected `(` after temporary-event byte",
            )?;
            let index =
                self.integer::<u16>("temporary-event byte index must be an unsigned integer")?;
            self.expect(
                TokenKind::RightParen,
                "expected `)` after temporary-event byte index",
            )?;
            let fact = QueryFact::TemporaryEventByte { index };
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

pub(super) fn canonical_float(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

pub(super) fn validate_text(value: &str, maximum: usize, allow_empty: bool) -> Result<(), String> {
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

pub(super) fn validate_comparison(
    field: Field,
    operator: Comparison,
    value: &Value,
) -> Result<(), String> {
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

pub(super) fn validate_query_fact(fact: &QueryFact) -> Result<(), String> {
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
        QueryFact::TemporaryEventByte { index } => {
            if *index >= 256 {
                return Err("event.temporary_byte index must be below 256".into());
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

pub(super) fn validate_query_comparison(
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

pub(super) fn valid_stage_name(value: &str) -> bool {
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

pub(super) fn validate_program(program: &MilestoneProgram) -> Result<(), String> {
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
            if language_minor < 7 && (*field as u8) >= Field::TitleLogoSkipReady as u8 {
                return Err(format!(
                    "field {} requires milestone language 1.7",
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
            let required_minor = match fact {
                QueryFact::TemporaryEventByte { .. } => 8,
                QueryFact::PlayerInAabb { .. } | QueryFact::PlayerPlaneSignedDistance { .. } => 3,
                _ => 2,
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
