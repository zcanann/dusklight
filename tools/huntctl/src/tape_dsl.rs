use crate::tape_program::{
    FrameSpec, PROGRAM_SCHEMA, PadSpec, ProgramWaitCondition, Step, TapeProgram, TickRate,
};
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DslError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for DslError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl Error for DslError {}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Kind {
    Atom(String),
    LeftBrace,
    RightBrace,
    Slash,
    Separator,
    Eof,
}

#[derive(Clone, Debug)]
struct Token {
    kind: Kind,
    line: usize,
    column: usize,
}

pub fn parse(source: &str) -> Result<TapeProgram, DslError> {
    Parser::new(lex(source)?).parse()
}

fn lex(source: &str) -> Result<Vec<Token>, DslError> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let (mut index, mut line, mut column) = (0, 1, 1);
    while index < chars.len() {
        let start_line = line;
        let start_column = column;
        match chars[index] {
            ' ' | '\t' | '\r' => {
                index += 1;
                column += 1;
            }
            '\n' => {
                tokens.push(Token {
                    kind: Kind::Separator,
                    line,
                    column,
                });
                index += 1;
                line += 1;
                column = 1;
            }
            ';' => {
                tokens.push(Token {
                    kind: Kind::Separator,
                    line,
                    column,
                });
                index += 1;
                column += 1;
            }
            '#' => {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                    column += 1;
                }
            }
            '{' | '}' | '/' => {
                let kind = match chars[index] {
                    '{' => Kind::LeftBrace,
                    '}' => Kind::RightBrace,
                    _ => Kind::Slash,
                };
                tokens.push(Token { kind, line, column });
                index += 1;
                column += 1;
            }
            _ => {
                let start = index;
                while index < chars.len()
                    && !matches!(
                        chars[index],
                        ' ' | '\t' | '\r' | '\n' | ';' | '#' | '{' | '}' | '/'
                    )
                {
                    index += 1;
                    column += 1;
                }
                if start == index {
                    return Err(DslError {
                        line: start_line,
                        column: start_column,
                        message: format!("unexpected character {:?}", chars[index]),
                    });
                }
                tokens.push(Token {
                    kind: Kind::Atom(chars[start..index].iter().collect()),
                    line: start_line,
                    column: start_column,
                });
            }
        }
    }
    tokens.push(Token {
        kind: Kind::Eof,
        line,
        column,
    });
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    states: HashMap<String, FrameSpec>,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            at: 0,
            states: HashMap::new(),
        }
    }

    fn parse(mut self) -> Result<TapeProgram, DslError> {
        self.skip_separators();
        self.expect_atom("dusktape")?;
        let version = self.atom()?;
        if version != "1" {
            return self.error(format!("unsupported DSL version {version:?}; expected 1"));
        }
        self.require_separator("after `dusktape 1`")?;

        let mut tick_rate = TickRate::default();
        let mut default_owned_ports = 1;
        let mut steps = Vec::new();
        while !matches!(self.peek().kind, Kind::Eof) {
            self.skip_separators();
            if matches!(self.peek().kind, Kind::Eof) {
                break;
            }
            let command_token = self.peek().clone();
            let command = self.atom()?;
            match command.as_str() {
                "rate" => {
                    tick_rate.numerator = self.u32()?;
                    self.expect_kind(Kind::Slash, "expected `/` in tick rate")?;
                    tick_rate.denominator = self.u32()?;
                }
                "ports" => default_owned_ports = self.u8()?,
                "state" => {
                    let name_token = self.peek().clone();
                    let name = self.atom()?;
                    if self.states.contains_key(&name) {
                        return Err(DslError {
                            line: name_token.line,
                            column: name_token.column,
                            message: format!("state {name:?} is already defined"),
                        });
                    }
                    let frame = self.frame_body()?;
                    self.states.insert(name, frame);
                }
                "marker" => steps.push(Step::Marker { name: self.atom()? }),
                "frame" => steps.push(Step::Frame {
                    frame: self.frame_ref()?,
                }),
                "repeat" => steps.push(Step::Repeat {
                    count: self.u64()?,
                    frame: self.frame_ref()?,
                }),
                "hold" => steps.push(Step::Hold { count: self.u64()? }),
                "wait" => steps.push(Step::WaitUntil {
                    condition: self.condition()?,
                    timeout_ticks: self.u16()?,
                }),
                "pulse" => steps.push(Step::PulseUntil {
                    condition: self.condition()?,
                    timeout_ticks: self.u16()?,
                    frame: self.frame_ref()?,
                }),
                "cycle" => {
                    let count = self.u64()?;
                    self.expect_kind(Kind::LeftBrace, "expected `{` after cycle count")?;
                    let mut frames = Vec::new();
                    loop {
                        self.skip_separators();
                        if self.consume_kind(&Kind::RightBrace) {
                            break;
                        }
                        if matches!(self.peek().kind, Kind::Eof) {
                            return self.error("unterminated cycle block".into());
                        }
                        self.expect_atom("frame")?;
                        frames.push(self.frame_ref()?);
                        self.consume_separator();
                    }
                    steps.push(Step::Cycle { count, frames });
                }
                _ => {
                    return Err(DslError {
                        line: command_token.line,
                        column: command_token.column,
                        message: format!("unknown command {command:?}"),
                    });
                }
            }
            self.require_statement_end()?;
        }
        Ok(TapeProgram {
            schema: PROGRAM_SCHEMA.into(),
            tick_rate,
            default_owned_ports,
            steps,
        })
    }

    fn frame_ref(&mut self) -> Result<FrameSpec, DslError> {
        if matches!(self.peek().kind, Kind::LeftBrace) {
            return self.frame_body();
        }
        let token = self.peek().clone();
        let name = self.atom()?;
        self.states.get(&name).cloned().ok_or_else(|| DslError {
            line: token.line,
            column: token.column,
            message: format!("unknown state {name:?}"),
        })
    }

    fn frame_body(&mut self) -> Result<FrameSpec, DslError> {
        self.expect_kind(Kind::LeftBrace, "expected `{` to start frame state")?;
        let mut owned_ports = None;
        let mut pads: BTreeMap<String, PadSpec> = BTreeMap::new();
        let mut current_port: Option<String> = None;
        loop {
            self.skip_separators();
            if self.consume_kind(&Kind::RightBrace) {
                break;
            }
            if matches!(self.peek().kind, Kind::Eof) {
                return self.error("unterminated frame state".into());
            }
            let field_token = self.peek().clone();
            let field = self.atom()?;
            if field == "owned" {
                owned_ports = Some(self.u8()?);
                continue;
            }
            if let Some(port) = parse_port(&field) {
                current_port = Some(port.to_string());
                pads.entry(port.to_string()).or_default();
                continue;
            }
            let Some(port) = current_port.clone() else {
                return Err(DslError {
                    line: field_token.line,
                    column: field_token.column,
                    message: format!("pad field {field:?} requires p0, p1, p2, or p3 first"),
                });
            };
            let pad = pads.get_mut(&port).expect("current pad exists");
            match field.as_str() {
                "buttons" => {
                    let first = self.peek().clone();
                    let mut count = 0;
                    while let Kind::Atom(name) = &self.peek().kind {
                        if is_frame_field(name) {
                            break;
                        }
                        pad.buttons |= button_mask(name).ok_or_else(|| DslError {
                            line: self.peek().line,
                            column: self.peek().column,
                            message: format!("unknown button {name:?}"),
                        })?;
                        self.at += 1;
                        count += 1;
                    }
                    if count == 0 {
                        return Err(DslError {
                            line: first.line,
                            column: first.column,
                            message: "buttons requires at least one button name".into(),
                        });
                    }
                }
                "stick" => {
                    pad.stick_x = self.i8()?;
                    pad.stick_y = self.i8()?;
                }
                "substick" => {
                    pad.substick_x = self.i8()?;
                    pad.substick_y = self.i8()?;
                }
                "triggers" => {
                    pad.trigger_left = self.u8()?;
                    pad.trigger_right = self.u8()?;
                }
                "analogs" => {
                    pad.analog_a = self.u8()?;
                    pad.analog_b = self.u8()?;
                }
                "stick_x" => pad.stick_x = self.i8()?,
                "stick_y" => pad.stick_y = self.i8()?,
                "substick_x" => pad.substick_x = self.i8()?,
                "substick_y" => pad.substick_y = self.i8()?,
                "trigger_left" => pad.trigger_left = self.u8()?,
                "trigger_right" => pad.trigger_right = self.u8()?,
                "analog_a" => pad.analog_a = self.u8()?,
                "analog_b" => pad.analog_b = self.u8()?,
                "connected" => pad.connected = self.boolean()?,
                "error" => pad.error = self.i8()?,
                _ => {
                    return Err(DslError {
                        line: field_token.line,
                        column: field_token.column,
                        message: format!("unknown frame field {field:?}"),
                    });
                }
            }
        }
        Ok(FrameSpec { owned_ports, pads })
    }

    fn condition(&mut self) -> Result<ProgramWaitCondition, DslError> {
        let token = self.peek().clone();
        let name = self.atom()?;
        match name.as_str() {
            "name_entry_active" => Ok(ProgramWaitCondition::NameEntryActive),
            "name_entry_character_select" => Ok(ProgramWaitCondition::NameEntryCharacterSelect),
            "name_entry_input_ready" => Ok(ProgramWaitCondition::NameEntryInputReady),
            "file_select_no_save_ready" => Ok(ProgramWaitCondition::FileSelectNoSaveReady),
            "file_select_data_select_ready" => Ok(ProgramWaitCondition::FileSelectDataSelectReady),
            "file_select_accept_ready" => Ok(ProgramWaitCondition::FileSelectAcceptReady),
            _ => Err(DslError {
                line: token.line,
                column: token.column,
                message: format!("unknown wait condition {name:?}"),
            }),
        }
    }

    fn boolean(&mut self) -> Result<bool, DslError> {
        let token = self.peek().clone();
        match self.atom()?.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            value => Err(DslError {
                line: token.line,
                column: token.column,
                message: format!("expected true or false, found {value:?}"),
            }),
        }
    }

    fn u8(&mut self) -> Result<u8, DslError> {
        self.integer::<u8>("u8")
    }
    fn u16(&mut self) -> Result<u16, DslError> {
        self.integer::<u16>("u16")
    }
    fn u32(&mut self) -> Result<u32, DslError> {
        self.integer::<u32>("u32")
    }
    fn u64(&mut self) -> Result<u64, DslError> {
        self.integer::<u64>("u64")
    }
    fn i8(&mut self) -> Result<i8, DslError> {
        self.integer::<i8>("i8")
    }

    fn integer<T>(&mut self, expected: &str) -> Result<T, DslError>
    where
        T: TryFrom<i128>,
    {
        let token = self.peek().clone();
        let text = self.atom()?;
        let parsed = if let Some(hex) = text.strip_prefix("0x") {
            i128::from_str_radix(hex, 16)
        } else {
            text.parse::<i128>()
        };
        parsed
            .ok()
            .and_then(|value| T::try_from(value).ok())
            .ok_or_else(|| DslError {
                line: token.line,
                column: token.column,
                message: format!("expected {expected}, found {text:?}"),
            })
    }

    fn atom(&mut self) -> Result<String, DslError> {
        let token = self.peek().clone();
        if let Kind::Atom(value) = token.kind {
            self.at += 1;
            Ok(value)
        } else {
            self.error("expected a word or number".into())
        }
    }

    fn expect_atom(&mut self, expected: &str) -> Result<(), DslError> {
        let token = self.peek().clone();
        let actual = self.atom()?;
        if actual == expected {
            Ok(())
        } else {
            Err(DslError {
                line: token.line,
                column: token.column,
                message: format!("expected {expected:?}, found {actual:?}"),
            })
        }
    }

    fn require_statement_end(&mut self) -> Result<(), DslError> {
        if matches!(self.peek().kind, Kind::Separator | Kind::Eof) {
            self.skip_separators();
            Ok(())
        } else {
            self.error("expected a newline or `;` after statement".into())
        }
    }

    fn require_separator(&mut self, context: &str) -> Result<(), DslError> {
        if self.consume_separator() {
            self.skip_separators();
            Ok(())
        } else {
            self.error(format!("expected a newline or `;` {context}"))
        }
    }

    fn consume_separator(&mut self) -> bool {
        if matches!(self.peek().kind, Kind::Separator) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn skip_separators(&mut self) {
        while self.consume_separator() {}
    }

    fn consume_kind(&mut self, kind: &Kind) -> bool {
        if &self.peek().kind == kind {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect_kind(&mut self, kind: Kind, message: &str) -> Result<(), DslError> {
        if self.consume_kind(&kind) {
            Ok(())
        } else {
            self.error(message.into())
        }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.at]
    }

    fn error<T>(&self, message: String) -> Result<T, DslError> {
        Err(DslError {
            line: self.peek().line,
            column: self.peek().column,
            message,
        })
    }
}

fn parse_port(value: &str) -> Option<u8> {
    let port = value.strip_prefix('p')?.parse::<u8>().ok()?;
    (port < 4).then_some(port)
}

fn is_frame_field(value: &str) -> bool {
    parse_port(value).is_some()
        || matches!(
            value,
            "owned"
                | "buttons"
                | "stick"
                | "substick"
                | "triggers"
                | "analogs"
                | "stick_x"
                | "stick_y"
                | "substick_x"
                | "substick_y"
                | "trigger_left"
                | "trigger_right"
                | "analog_a"
                | "analog_b"
                | "connected"
                | "error"
        )
}

fn button_mask(value: &str) -> Option<u16> {
    Some(match value {
        "LEFT" => 0x0001,
        "RIGHT" => 0x0002,
        "DOWN" => 0x0004,
        "UP" => 0x0008,
        "Z" => 0x0010,
        "R" => 0x0020,
        "L" => 0x0040,
        "A" => 0x0100,
        "B" => 0x0200,
        "X" => 0x0400,
        "Y" => 0x0800,
        "START" | "MENU" => 0x1000,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_states_cycles_and_inline_frames() {
        let program = parse(
            r#"
            # compact deterministic source
            dusktape 1
            rate 60/2
            ports 0x0f
            state neutral {}
            state move { p0 buttons A START stick -127 4 }
            marker boot
            repeat 3 neutral
            cycle 2 { frame move; frame neutral }
            frame { owned 1 p1 buttons B triggers 2 3 connected false error -1 }
        "#,
        )
        .unwrap();
        let compiled = program.compile().unwrap();
        assert_eq!(compiled.tape.tick_rate_numerator, 60);
        assert_eq!(compiled.tape.frames.len(), 8);
        assert_eq!(compiled.tape.frames[3].pads[0].buttons, 0x1100);
        assert_eq!(compiled.tape.frames[3].pads[0].stick_x, -127);
        assert_eq!(compiled.tape.frames[7].owned_ports, 1);
        assert_eq!(compiled.tape.frames[7].pads[1].buttons, 0x0200);
        assert!(!compiled.tape.frames[7].pads[1].connected);
        assert_eq!(compiled.markers[0].tick, 0);
    }

    #[test]
    fn diagnostics_include_source_position() {
        let error = parse("dusktape 1\nstate x { p0 buttons NOPE }\n").unwrap_err();
        assert_eq!((error.line, error.column), (2, 22));
        assert!(error.to_string().contains("unknown button"));
    }

    #[test]
    fn rejects_unknown_and_duplicate_states() {
        assert!(
            parse("dusktape 1\nframe missing\n")
                .unwrap_err()
                .to_string()
                .contains("unknown state")
        );
        assert!(
            parse("dusktape 1\nstate x {}\nstate x {}\n")
                .unwrap_err()
                .to_string()
                .contains("already defined")
        );
    }
}
