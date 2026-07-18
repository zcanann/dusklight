//! Canonical experimental intervention timeline and bounded readable DSL.
//!
//! This artifact is deliberately unrelated to `DUSKTAPE` and `DUSKCTRL`.

use serde::Serialize;
use std::error::Error;
use std::fmt;

pub const MAGIC: &[u8; 8] = b"DUSKINTR";
pub const VERSION_MAJOR: u16 = 1;
pub const VERSION_MINOR: u16 = 0;
pub const HEADER_SIZE: usize = 32;
pub const RECORD_SIZE: usize = 128;
pub const MAX_INTERVENTIONS: usize = 1_024;
pub const MAX_TIMELINE_TICKS: u32 = 1_000_000;
pub const MAX_DSL_BYTES: usize = 1_048_576;
pub const MAX_DSL_LINES: usize = 4_096;
const EXPERIMENTAL_INTERVENTION_FLAG: u32 = 1;
const STAGE_NAME_CAPACITY: usize = 16;
const MAX_ABSOLUTE_COMPONENT: f32 = 10_000_000.0;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InterventionTape {
    pub duration_ticks: u32,
    pub interventions: Vec<Intervention>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Intervention {
    pub start_tick: u32,
    pub duration_ticks: u32,
    pub phase: InterventionPhase,
    pub selector: InterventionSelector,
    pub precondition: InterventionPrecondition,
    pub operation: InterventionOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionPhase {
    BeforeGameTick,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InterventionSelector {
    Process {
        process_id: u32,
    },
    Placed {
        stage_name: String,
        home_room: i8,
        actor_name: i16,
        set_id: u16,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionPrecondition {
    ActorExists,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InterventionOperation {
    SetPosition { value: [f32; 3] },
    AddVelocity { value: [f32; 3] },
}

#[derive(Debug)]
pub struct InterventionTapeError(String);

impl fmt::Display for InterventionTapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for InterventionTapeError {}

impl InterventionTape {
    pub fn validate(&self) -> Result<(), InterventionTapeError> {
        if self.duration_ticks == 0 || self.duration_ticks > MAX_TIMELINE_TICKS {
            return Err(error("intervention timeline duration is invalid"));
        }
        if self.interventions.is_empty() || self.interventions.len() > MAX_INTERVENTIONS {
            return Err(error("intervention count is empty or exceeds its bound"));
        }
        let mut previous_key = None;
        for intervention in &self.interventions {
            intervention.validate(self.duration_ticks)?;
            let key = (intervention.start_tick, intervention_sort_key(intervention));
            if previous_key
                .as_ref()
                .is_some_and(|previous| previous >= &key)
            {
                return Err(error(
                    "interventions must be in unique canonical tick/selector/operation order",
                ));
            }
            previous_key = Some(key);
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, InterventionTapeError> {
        self.validate()?;
        let size = HEADER_SIZE
            .checked_add(
                self.interventions
                    .len()
                    .checked_mul(RECORD_SIZE)
                    .ok_or_else(|| error("intervention artifact size overflow"))?,
            )
            .ok_or_else(|| error("intervention artifact size overflow"))?;
        let mut output = vec![0_u8; size];
        output[..8].copy_from_slice(MAGIC);
        put_u16(&mut output, 8, VERSION_MAJOR);
        put_u16(&mut output, 10, VERSION_MINOR);
        put_u16(&mut output, 12, HEADER_SIZE as u16);
        put_u16(&mut output, 14, RECORD_SIZE as u16);
        put_u32(&mut output, 16, self.duration_ticks);
        put_u32(&mut output, 20, self.interventions.len() as u32);
        put_u32(&mut output, 24, EXPERIMENTAL_INTERVENTION_FLAG);
        for (index, intervention) in self.interventions.iter().enumerate() {
            encode_record(
                intervention,
                &mut output[HEADER_SIZE + index * RECORD_SIZE..][..RECORD_SIZE],
            );
        }
        Ok(output)
    }

    pub fn decode(input: &[u8]) -> Result<Self, InterventionTapeError> {
        if input.len() < HEADER_SIZE || &input[..8] != MAGIC {
            return Err(error("not a DUSKINTR artifact"));
        }
        if u16_at(input, 8) != VERSION_MAJOR
            || u16_at(input, 10) != VERSION_MINOR
            || u16_at(input, 12) as usize != HEADER_SIZE
            || u16_at(input, 14) as usize != RECORD_SIZE
            || u32_at(input, 24) != EXPERIMENTAL_INTERVENTION_FLAG
            || input[28..32].iter().any(|byte| *byte != 0)
        {
            return Err(error("unsupported or noncanonical DUSKINTR header"));
        }
        let count = u32_at(input, 20) as usize;
        let expected = HEADER_SIZE
            .checked_add(
                count
                    .checked_mul(RECORD_SIZE)
                    .ok_or_else(|| error("intervention artifact size overflow"))?,
            )
            .ok_or_else(|| error("intervention artifact size overflow"))?;
        if count == 0 || count > MAX_INTERVENTIONS || input.len() != expected {
            return Err(error("DUSKINTR record count or length is invalid"));
        }
        let mut interventions = Vec::with_capacity(count);
        for index in 0..count {
            interventions.push(decode_record(
                &input[HEADER_SIZE + index * RECORD_SIZE..][..RECORD_SIZE],
            )?);
        }
        let tape = Self {
            duration_ticks: u32_at(input, 16),
            interventions,
        };
        tape.validate()?;
        if tape.encode()?.as_slice() != input {
            return Err(error("DUSKINTR artifact is not canonically encoded"));
        }
        Ok(tape)
    }

    pub fn compile_dsl(source: &str) -> Result<Self, InterventionTapeError> {
        if source.len() > MAX_DSL_BYTES || source.lines().count() > MAX_DSL_LINES {
            return Err(error("intervention DSL exceeds its input bounds"));
        }
        let mut duration_ticks = None;
        let mut interventions = Vec::new();
        for (line_index, raw_line) in source.lines().enumerate() {
            let line = raw_line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let tokens = line.split_whitespace().collect::<Vec<_>>();
            if tokens.first() == Some(&"timeline") {
                if tokens.len() != 2 || duration_ticks.is_some() {
                    return Err(dsl_error(
                        line_index,
                        "invalid or duplicate timeline declaration",
                    ));
                }
                duration_ticks = Some(parse_u32(tokens[1], line_index, "timeline duration")?);
                continue;
            }
            interventions.push(parse_intervention(&tokens, line_index)?);
        }
        let mut tape = Self {
            duration_ticks: duration_ticks
                .ok_or_else(|| error("intervention DSL has no timeline declaration"))?,
            interventions,
        };
        tape.interventions.sort_by_key(|intervention| {
            (intervention.start_tick, intervention_sort_key(intervention))
        });
        tape.validate()?;
        Ok(tape)
    }
}

impl Intervention {
    fn validate(&self, timeline_ticks: u32) -> Result<(), InterventionTapeError> {
        if self.duration_ticks == 0
            || self
                .start_tick
                .checked_add(self.duration_ticks)
                .is_none_or(|end| end > timeline_ticks)
        {
            return Err(error("intervention tick range escapes the timeline"));
        }
        match &self.selector {
            InterventionSelector::Process { process_id } if *process_id == 0 => {
                return Err(error("intervention process selector is zero"));
            }
            InterventionSelector::Placed {
                stage_name,
                actor_name,
                ..
            } if !valid_stage(stage_name) || *actor_name == -1 => {
                return Err(error("intervention placed selector is invalid"));
            }
            _ => {}
        }
        let vector = match &self.operation {
            InterventionOperation::SetPosition { value }
            | InterventionOperation::AddVelocity { value } => value,
        };
        if vector
            .iter()
            .any(|value| !value.is_finite() || value.abs() > MAX_ABSOLUTE_COMPONENT)
        {
            return Err(error("intervention vector is non-finite or out of bounds"));
        }
        Ok(())
    }
}

fn parse_intervention(
    tokens: &[&str],
    line_index: usize,
) -> Result<Intervention, InterventionTapeError> {
    if tokens.len() < 13
        || tokens[0] != "at"
        || tokens[2] != "for"
        || tokens[4] != "before_game_tick"
    {
        return Err(dsl_error(line_index, "invalid intervention prefix"));
    }
    let start_tick = parse_u32(tokens[1], line_index, "start tick")?;
    let duration_ticks = parse_u32(tokens[3], line_index, "duration")?;
    let (selector, mut cursor) = match tokens[5] {
        "process" if tokens.len() >= 8 => (
            InterventionSelector::Process {
                process_id: parse_u32(tokens[6], line_index, "process ID")?,
            },
            7,
        ),
        "placed" if tokens.len() >= 11 => (
            InterventionSelector::Placed {
                stage_name: tokens[6].into(),
                home_room: parse_i8(tokens[7], line_index, "home room")?,
                actor_name: parse_i16(tokens[8], line_index, "actor name")?,
                set_id: parse_u16(tokens[9], line_index, "set ID")?,
            },
            10,
        ),
        _ => return Err(dsl_error(line_index, "invalid intervention selector")),
    };
    if tokens.get(cursor) != Some(&"require") || tokens.get(cursor + 1) != Some(&"actor_exists") {
        return Err(dsl_error(
            line_index,
            "actor_exists precondition is required",
        ));
    }
    cursor += 2;
    let operation_name = tokens
        .get(cursor)
        .ok_or_else(|| dsl_error(line_index, "missing intervention operation"))?;
    if tokens.len() != cursor + 4 {
        return Err(dsl_error(
            line_index,
            "operation requires exactly three components",
        ));
    }
    let value = [
        parse_f32(tokens[cursor + 1], line_index, "x component")?,
        parse_f32(tokens[cursor + 2], line_index, "y component")?,
        parse_f32(tokens[cursor + 3], line_index, "z component")?,
    ];
    let operation = match *operation_name {
        "set_position" => InterventionOperation::SetPosition { value },
        "add_velocity" => InterventionOperation::AddVelocity { value },
        _ => return Err(dsl_error(line_index, "unknown intervention operation")),
    };
    Ok(Intervention {
        start_tick,
        duration_ticks,
        phase: InterventionPhase::BeforeGameTick,
        selector,
        precondition: InterventionPrecondition::ActorExists,
        operation,
    })
}

fn encode_record(intervention: &Intervention, output: &mut [u8]) {
    put_u32(output, 0, intervention.start_tick);
    put_u32(output, 4, intervention.duration_ticks);
    output[8] = 1;
    output[10] = 1;
    match &intervention.selector {
        InterventionSelector::Process { process_id } => {
            output[9] = 1;
            put_u32(output, 16, *process_id);
        }
        InterventionSelector::Placed {
            stage_name,
            home_room,
            actor_name,
            set_id,
        } => {
            output[9] = 2;
            output[16] = stage_name.len() as u8;
            output[17..17 + stage_name.len()].copy_from_slice(stage_name.as_bytes());
            output[33] = *home_room as u8;
            put_i16(output, 34, *actor_name);
            put_u16(output, 36, *set_id);
        }
    }
    let (kind, value) = match &intervention.operation {
        InterventionOperation::SetPosition { value } => (1, value),
        InterventionOperation::AddVelocity { value } => (2, value),
    };
    output[11] = kind;
    for (index, value) in value.iter().enumerate() {
        put_u32(output, 48 + index * 4, value.to_bits());
    }
}

fn decode_record(input: &[u8]) -> Result<Intervention, InterventionTapeError> {
    let phase = match input[8] {
        1 => InterventionPhase::BeforeGameTick,
        _ => return Err(error("unknown intervention phase")),
    };
    let selector = match input[9] {
        1 => InterventionSelector::Process {
            process_id: u32_at(input, 16),
        },
        2 => {
            let length = input[16] as usize;
            if length == 0 || length > STAGE_NAME_CAPACITY {
                return Err(error("invalid placed-selector stage length"));
            }
            InterventionSelector::Placed {
                stage_name: std::str::from_utf8(&input[17..17 + length])
                    .map_err(|_| error("placed-selector stage is not UTF-8"))?
                    .into(),
                home_room: input[33] as i8,
                actor_name: i16_at(input, 34),
                set_id: u16_at(input, 36),
            }
        }
        _ => return Err(error("unknown intervention selector")),
    };
    if input[10] != 1 {
        return Err(error("unknown intervention precondition"));
    }
    let value = std::array::from_fn(|index| f32::from_bits(u32_at(input, 48 + index * 4)));
    let operation = match input[11] {
        1 => InterventionOperation::SetPosition { value },
        2 => InterventionOperation::AddVelocity { value },
        _ => return Err(error("unknown intervention operation")),
    };
    Ok(Intervention {
        start_tick: u32_at(input, 0),
        duration_ticks: u32_at(input, 4),
        phase,
        selector,
        precondition: InterventionPrecondition::ActorExists,
        operation,
    })
}

fn intervention_sort_key(intervention: &Intervention) -> Vec<u8> {
    let mut record = vec![0; RECORD_SIZE];
    encode_record(intervention, &mut record);
    record[4..].to_vec()
}

fn valid_stage(stage: &str) -> bool {
    !stage.is_empty()
        && stage.len() <= STAGE_NAME_CAPACITY
        && stage.bytes().all(|byte| byte.is_ascii_graphic())
}

fn parse_u32(value: &str, line: usize, label: &str) -> Result<u32, InterventionTapeError> {
    value
        .parse()
        .map_err(|_| dsl_error(line, &format!("invalid {label}")))
}

fn parse_u16(value: &str, line: usize, label: &str) -> Result<u16, InterventionTapeError> {
    value
        .parse()
        .map_err(|_| dsl_error(line, &format!("invalid {label}")))
}

fn parse_i16(value: &str, line: usize, label: &str) -> Result<i16, InterventionTapeError> {
    value
        .parse()
        .map_err(|_| dsl_error(line, &format!("invalid {label}")))
}

fn parse_i8(value: &str, line: usize, label: &str) -> Result<i8, InterventionTapeError> {
    value
        .parse()
        .map_err(|_| dsl_error(line, &format!("invalid {label}")))
}

fn parse_f32(value: &str, line: usize, label: &str) -> Result<f32, InterventionTapeError> {
    value
        .parse()
        .map_err(|_| dsl_error(line, &format!("invalid {label}")))
}

fn dsl_error(line: usize, message: &str) -> InterventionTapeError {
    error(format!("intervention DSL line {}: {message}", line + 1))
}

fn error(message: impl Into<String>) -> InterventionTapeError {
    InterventionTapeError(message.into())
}

fn u16_at(input: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(input[offset..offset + 2].try_into().unwrap())
}

fn i16_at(input: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes(input[offset..offset + 2].try_into().unwrap())
}

fn u32_at(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(input[offset..offset + 4].try_into().unwrap())
}

fn put_u16(output: &mut [u8], offset: usize, value: u16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_i16(output: &mut [u8], offset: usize, value: i16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    const DSL: &str = r#"
timeline 120
at 10 for 1 before_game_tick process 42 require actor_exists set_position 1 2 3
at 11 for 2 before_game_tick placed F_SP104 1 7 12 require actor_exists add_velocity 0 4 -1
"#;

    #[test]
    fn readable_dsl_compiles_to_canonical_round_tripping_duskintr() {
        let tape = InterventionTape::compile_dsl(DSL).unwrap();
        let encoded = tape.encode().unwrap();
        assert_eq!(&encoded[..8], MAGIC);
        assert_eq!(encoded.len(), HEADER_SIZE + 2 * RECORD_SIZE);
        assert_eq!(InterventionTape::decode(&encoded).unwrap(), tape);
        assert_ne!(&encoded[..8], crate::tape::MAGIC.as_slice());
        assert_ne!(&encoded[..8], crate::controller_program::MAGIC.as_slice());
    }

    #[test]
    fn source_order_does_not_change_canonical_bytes() {
        let reversed = r#"
timeline 120
at 11 for 2 before_game_tick placed F_SP104 1 7 12 require actor_exists add_velocity 0 4 -1
at 10 for 1 before_game_tick process 42 require actor_exists set_position 1 2 3
"#;
        assert_eq!(
            InterventionTape::compile_dsl(DSL)
                .unwrap()
                .encode()
                .unwrap(),
            InterventionTape::compile_dsl(reversed)
                .unwrap()
                .encode()
                .unwrap()
        );
    }

    #[test]
    fn decoder_rejects_reserved_bytes_and_noncanonical_records() {
        let mut encoded = InterventionTape::compile_dsl(DSL)
            .unwrap()
            .encode()
            .unwrap();
        encoded[28] = 1;
        assert!(InterventionTape::decode(&encoded).is_err());

        let mut encoded = InterventionTape::compile_dsl(DSL)
            .unwrap()
            .encode()
            .unwrap();
        encoded[HEADER_SIZE + 100] = 1;
        assert!(InterventionTape::decode(&encoded).is_err());
    }

    #[test]
    fn dsl_requires_explicit_phase_selector_precondition_and_bounds() {
        assert!(
            InterventionTape::compile_dsl("timeline 4\nat 0 for 1 process 7 set_position 0 0 0")
                .is_err()
        );
        assert!(InterventionTape::compile_dsl(
            "timeline 4\nat 3 for 2 before_game_tick process 7 require actor_exists set_position 0 0 0"
        )
        .is_err());
        assert!(InterventionTape::compile_dsl(
            "timeline 4\nat 0 for 1 before_game_tick process 7 require actor_exists set_position NaN 0 0"
        )
        .is_err());
    }
}
