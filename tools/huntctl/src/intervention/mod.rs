//! Canonical experimental intervention timeline and bounded readable DSL.
//!
//! This artifact is deliberately unrelated to `DUSKTAPE` and `DUSKCTRL`.

pub mod evidence;
pub mod experiment;
pub mod parameter_search;
#[cfg(feature = "experimental-interventions")]
pub mod runtime;

use crate::actor_identity::PlacedActorSelector;
use serde::Serialize;
use std::error::Error;
use std::fmt;

pub const MAGIC: &[u8; 8] = b"DUSKINTR";
pub const VERSION_MAJOR: u16 = 1;
pub const VERSION_MINOR: u16 = 1;
pub const HEADER_SIZE: usize = 32;
pub const RECORD_SIZE: usize = 128;
pub const MAX_INTERVENTIONS: usize = 1_024;
pub const MAX_TIMELINE_TICKS: u32 = 1_000_000;
pub const MAX_DSL_BYTES: usize = 1_048_576;
pub const MAX_DSL_LINES: usize = 4_096;
const EXPERIMENTAL_INTERVENTION_FLAG: u32 = 1;
const MAX_ABSOLUTE_COMPONENT: f32 = 10_000_000.0;
const MAX_HEALTH: i16 = 1_000;
const MAX_TIMER_TICKS: u16 = 3_600;

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
    Process { process_id: u32 },
    Placed { identity: PlacedActorSelector },
}

impl InterventionSelector {
    /// Converts the exact selector forms used by reactive `seek actor`
    /// controllers into the intervention identity. The controller keeps the
    /// actor procedure beside its selector, so it is supplied separately here.
    pub fn from_controller_selector(
        actor_name: i16,
        selector: &crate::controller_program::ActorSelector,
    ) -> Result<Self, InterventionTapeError> {
        match selector {
            crate::controller_program::ActorSelector::Nearest => Err(error(
                "nearest actor selectors are not stable enough for interventions",
            )),
            crate::controller_program::ActorSelector::Process { process_id } => {
                if *process_id == 0 || *process_id == u32::MAX {
                    return Err(error("controller process selector is invalid"));
                }
                Ok(Self::Process {
                    process_id: *process_id,
                })
            }
            crate::controller_program::ActorSelector::Placed {
                set_id,
                room,
                stage_name,
            } => {
                let identity = PlacedActorSelector {
                    stage: stage_name.clone(),
                    home_room: *room,
                    actor_name,
                    set_id: *set_id,
                };
                identity.validate().map_err(|message| {
                    error(format!("controller placed selector is invalid: {message}"))
                })?;
                Ok(Self::Placed { identity })
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionPrecondition {
    ActorExists,
    ActorAbsent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionTimer {
    DamageWait,
    IceDamageWait,
    SwordChangeWait,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionFlagDomain {
    ActorStatus,
    RoomSwitch,
    EventBit,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InterventionOperation {
    SetPosition {
        value: [f32; 3],
    },
    AddPosition {
        value: [f32; 3],
    },
    SetVelocity {
        value: [f32; 3],
    },
    AddVelocity {
        value: [f32; 3],
    },
    SetFacingYaw {
        value: i16,
    },
    MoveAlongCubicCurve {
        control_points: [[f32; 3]; 4],
    },
    SetTargetPlayer {
        enabled: bool,
    },
    SetHealth {
        value: i16,
    },
    SetTimer {
        timer: InterventionTimer,
        ticks: u16,
    },
    SetFlag {
        domain: InterventionFlagDomain,
        index: u16,
        value: bool,
    },
    SpawnAtPosition {
        value: [f32; 3],
    },
    Despawn,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InterventionWriteField {
    Position,
    Velocity,
    Facing,
    TargetIntent,
    Health,
    Timer(InterventionTimer),
    Flag(InterventionFlagDomain, u16),
    Lifecycle,
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
    pub fn canonicalize(&mut self) {
        self.interventions.sort_by_key(|intervention| {
            (intervention.start_tick, intervention_sort_key(intervention))
        });
    }

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
        for (left_index, left) in self.interventions.iter().enumerate() {
            for (right_index, right) in self.interventions.iter().enumerate().skip(left_index + 1) {
                if left.selector == right.selector
                    && intervention_ranges_overlap(left, right)
                    && write_fields_conflict(
                        intervention_write_field(&left.operation),
                        intervention_write_field(&right.operation),
                    )
                {
                    return Err(error(format!(
                        "interventions {left_index} and {right_index} overlap writes to the same selector and field"
                    )));
                }
            }
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
        let minor_version = u16_at(input, 10);
        if u16_at(input, 8) != VERSION_MAJOR
            || minor_version > VERSION_MINOR
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
        let mut canonical = tape.encode()?;
        if minor_version == 0 {
            if tape.interventions.iter().any(|intervention| {
                intervention.precondition != InterventionPrecondition::ActorExists
                    || !matches!(
                        intervention.operation,
                        InterventionOperation::SetPosition { .. }
                            | InterventionOperation::AddVelocity { .. }
                    )
            }) {
                return Err(error("DUSKINTR v1.0 contains a v1.1 operation"));
            }
            put_u16(&mut canonical, 10, 0);
        }
        if canonical.as_slice() != input {
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
        tape.canonicalize();
        tape.validate()?;
        Ok(tape)
    }
}

fn intervention_ranges_overlap(left: &Intervention, right: &Intervention) -> bool {
    left.start_tick < right.start_tick + right.duration_ticks
        && right.start_tick < left.start_tick + left.duration_ticks
}

fn intervention_write_field(operation: &InterventionOperation) -> InterventionWriteField {
    match operation {
        InterventionOperation::SetPosition { .. }
        | InterventionOperation::AddPosition { .. }
        | InterventionOperation::MoveAlongCubicCurve { .. } => InterventionWriteField::Position,
        InterventionOperation::SetVelocity { .. } | InterventionOperation::AddVelocity { .. } => {
            InterventionWriteField::Velocity
        }
        InterventionOperation::SetFacingYaw { .. } => InterventionWriteField::Facing,
        InterventionOperation::SetTargetPlayer { .. } => InterventionWriteField::TargetIntent,
        InterventionOperation::SetHealth { .. } => InterventionWriteField::Health,
        InterventionOperation::SetTimer { timer, .. } => InterventionWriteField::Timer(*timer),
        InterventionOperation::SetFlag { domain, index, .. } => {
            InterventionWriteField::Flag(*domain, *index)
        }
        InterventionOperation::SpawnAtPosition { .. } | InterventionOperation::Despawn => {
            InterventionWriteField::Lifecycle
        }
    }
}

fn write_fields_conflict(left: InterventionWriteField, right: InterventionWriteField) -> bool {
    left == InterventionWriteField::Lifecycle
        || right == InterventionWriteField::Lifecycle
        || left == right
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
            InterventionSelector::Process { process_id }
                if *process_id == 0 || *process_id == u32::MAX =>
            {
                return Err(error("intervention process selector is invalid"));
            }
            InterventionSelector::Placed { identity } if identity.validate().is_err() => {
                return Err(error("intervention placed selector is invalid"));
            }
            _ => {}
        }
        let vectors = match &self.operation {
            InterventionOperation::SetPosition { value }
            | InterventionOperation::AddPosition { value }
            | InterventionOperation::SetVelocity { value }
            | InterventionOperation::AddVelocity { value }
            | InterventionOperation::SpawnAtPosition { value } => vec![value.as_slice()],
            InterventionOperation::MoveAlongCubicCurve { control_points } => control_points
                .iter()
                .map(|point| point.as_slice())
                .collect(),
            _ => Vec::new(),
        };
        if vectors
            .into_iter()
            .flatten()
            .any(|value| !value.is_finite() || value.abs() > MAX_ABSOLUTE_COMPONENT)
        {
            return Err(error("intervention vector is non-finite or out of bounds"));
        }
        match &self.operation {
            InterventionOperation::SpawnAtPosition { .. } => {
                if self.precondition != InterventionPrecondition::ActorAbsent
                    || !matches!(self.selector, InterventionSelector::Placed { .. })
                {
                    return Err(error(
                        "spawn requires an absent precondition and a placed selector",
                    ));
                }
            }
            _ if self.precondition != InterventionPrecondition::ActorExists => {
                return Err(error("non-spawn interventions require actor_exists"));
            }
            _ => {}
        }
        match &self.operation {
            InterventionOperation::SetHealth { value } if !(0..=MAX_HEALTH).contains(value) => {
                return Err(error("health intervention is outside the typed bound"));
            }
            InterventionOperation::SetTimer { ticks, .. } if *ticks > MAX_TIMER_TICKS => {
                return Err(error("timer intervention is outside the typed bound"));
            }
            InterventionOperation::SetFlag {
                domain: InterventionFlagDomain::ActorStatus,
                index,
                ..
            } if *index >= 32 => {
                return Err(error("actor-status flag index is outside the typed bound"));
            }
            InterventionOperation::SetFlag { index, .. } if *index >= 4_096 => {
                return Err(error("flag index is outside the typed bound"));
            }
            InterventionOperation::MoveAlongCubicCurve { .. } if self.duration_ticks < 2 => {
                return Err(error(
                    "cubic-curve intervention requires at least two ticks",
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

fn parse_intervention(
    tokens: &[&str],
    line_index: usize,
) -> Result<Intervention, InterventionTapeError> {
    if tokens.len() < 10
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
                identity: PlacedActorSelector {
                    stage: tokens[6].into(),
                    home_room: parse_i8(tokens[7], line_index, "home room")?,
                    actor_name: parse_i16(tokens[8], line_index, "actor name")?,
                    set_id: parse_u16(tokens[9], line_index, "set ID")?,
                },
            },
            10,
        ),
        _ => return Err(dsl_error(line_index, "invalid intervention selector")),
    };
    if tokens.get(cursor) != Some(&"require") {
        return Err(dsl_error(line_index, "explicit precondition is required"));
    }
    let precondition = match tokens.get(cursor + 1).copied() {
        Some("actor_exists") => InterventionPrecondition::ActorExists,
        Some("actor_absent") => InterventionPrecondition::ActorAbsent,
        _ => return Err(dsl_error(line_index, "unknown intervention precondition")),
    };
    cursor += 2;
    let operation = parse_operation(&tokens[cursor..], line_index)?;
    Ok(Intervention {
        start_tick,
        duration_ticks,
        phase: InterventionPhase::BeforeGameTick,
        selector,
        precondition,
        operation,
    })
}

fn parse_operation(
    tokens: &[&str],
    line: usize,
) -> Result<InterventionOperation, InterventionTapeError> {
    let name = tokens
        .first()
        .copied()
        .ok_or_else(|| dsl_error(line, "missing intervention operation"))?;
    let vector = |tokens: &[&str]| -> Result<[f32; 3], InterventionTapeError> {
        if tokens.len() != 4 {
            return Err(dsl_error(
                line,
                "vector operation requires three components",
            ));
        }
        Ok([
            parse_f32(tokens[1], line, "x component")?,
            parse_f32(tokens[2], line, "y component")?,
            parse_f32(tokens[3], line, "z component")?,
        ])
    };
    Ok(match name {
        "set_position" => InterventionOperation::SetPosition {
            value: vector(tokens)?,
        },
        "add_position" => InterventionOperation::AddPosition {
            value: vector(tokens)?,
        },
        "set_velocity" => InterventionOperation::SetVelocity {
            value: vector(tokens)?,
        },
        "add_velocity" => InterventionOperation::AddVelocity {
            value: vector(tokens)?,
        },
        "spawn_at" => InterventionOperation::SpawnAtPosition {
            value: vector(tokens)?,
        },
        "set_facing_yaw" if tokens.len() == 2 => InterventionOperation::SetFacingYaw {
            value: parse_i16(tokens[1], line, "facing yaw")?,
        },
        "move_cubic" if tokens.len() == 13 => {
            let mut control_points = [[0.0; 3]; 4];
            for (point, values) in control_points.iter_mut().enumerate() {
                for (axis, value) in values.iter_mut().enumerate() {
                    *value = parse_f32(tokens[1 + point * 3 + axis], line, "curve component")?;
                }
            }
            InterventionOperation::MoveAlongCubicCurve { control_points }
        }
        "set_target_player" if tokens.len() == 2 => InterventionOperation::SetTargetPlayer {
            enabled: parse_bool(tokens[1], line, "target intent")?,
        },
        "set_health" if tokens.len() == 2 => InterventionOperation::SetHealth {
            value: parse_i16(tokens[1], line, "health")?,
        },
        "set_timer" if tokens.len() == 3 => InterventionOperation::SetTimer {
            timer: parse_timer(tokens[1], line)?,
            ticks: parse_u16(tokens[2], line, "timer ticks")?,
        },
        "set_flag" if tokens.len() == 4 => InterventionOperation::SetFlag {
            domain: parse_flag_domain(tokens[1], line)?,
            index: parse_u16(tokens[2], line, "flag index")?,
            value: parse_bool(tokens[3], line, "flag value")?,
        },
        "despawn" if tokens.len() == 1 => InterventionOperation::Despawn,
        _ => return Err(dsl_error(line, "unknown operation or invalid operands")),
    })
}

fn encode_record(intervention: &Intervention, output: &mut [u8]) {
    put_u32(output, 0, intervention.start_tick);
    put_u32(output, 4, intervention.duration_ticks);
    output[8] = 1;
    output[10] = match intervention.precondition {
        InterventionPrecondition::ActorExists => 1,
        InterventionPrecondition::ActorAbsent => 2,
    };
    match &intervention.selector {
        InterventionSelector::Process { process_id } => {
            output[9] = 1;
            put_u32(output, 16, *process_id);
        }
        InterventionSelector::Placed { identity } => {
            output[9] = 2;
            output[16] = identity.stage.len() as u8;
            output[17..17 + identity.stage.len()].copy_from_slice(identity.stage.as_bytes());
            output[33] = identity.home_room as u8;
            put_i16(output, 34, identity.actor_name);
            put_u16(output, 36, identity.set_id);
        }
    }
    encode_operation(&intervention.operation, output);
}

fn encode_operation(operation: &InterventionOperation, output: &mut [u8]) {
    let write_vector = |output: &mut [u8], value: &[f32; 3]| {
        for (index, value) in value.iter().enumerate() {
            put_u32(output, 48 + index * 4, value.to_bits());
        }
    };
    match operation {
        InterventionOperation::SetPosition { value } => {
            output[11] = 1;
            write_vector(output, value);
        }
        InterventionOperation::AddVelocity { value } => {
            output[11] = 2;
            write_vector(output, value);
        }
        InterventionOperation::AddPosition { value } => {
            output[11] = 3;
            write_vector(output, value);
        }
        InterventionOperation::SetVelocity { value } => {
            output[11] = 4;
            write_vector(output, value);
        }
        InterventionOperation::SetFacingYaw { value } => {
            output[11] = 5;
            put_i16(output, 48, *value);
        }
        InterventionOperation::MoveAlongCubicCurve { control_points } => {
            output[11] = 6;
            for (index, value) in control_points.iter().flatten().enumerate() {
                put_u32(output, 48 + index * 4, value.to_bits());
            }
        }
        InterventionOperation::SetTargetPlayer { enabled } => {
            output[11] = 7;
            output[48] = u8::from(*enabled);
        }
        InterventionOperation::SetHealth { value } => {
            output[11] = 8;
            put_i16(output, 48, *value);
        }
        InterventionOperation::SetTimer { timer, ticks } => {
            output[11] = 9;
            output[48] = encode_timer(*timer);
            put_u16(output, 50, *ticks);
        }
        InterventionOperation::SetFlag {
            domain,
            index,
            value,
        } => {
            output[11] = 10;
            output[48] = encode_flag_domain(*domain);
            put_u16(output, 50, *index);
            output[52] = u8::from(*value);
        }
        InterventionOperation::SpawnAtPosition { value } => {
            output[11] = 11;
            write_vector(output, value);
        }
        InterventionOperation::Despawn => output[11] = 12,
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
            if length == 0 || length > crate::actor_identity::STAGE_NAME_CAPACITY {
                return Err(error("invalid placed-selector stage length"));
            }
            InterventionSelector::Placed {
                identity: PlacedActorSelector {
                    stage: std::str::from_utf8(&input[17..17 + length])
                        .map_err(|_| error("placed-selector stage is not UTF-8"))?
                        .into(),
                    home_room: input[33] as i8,
                    actor_name: i16_at(input, 34),
                    set_id: u16_at(input, 36),
                },
            }
        }
        _ => return Err(error("unknown intervention selector")),
    };
    let precondition = match input[10] {
        1 => InterventionPrecondition::ActorExists,
        2 => InterventionPrecondition::ActorAbsent,
        _ => return Err(error("unknown intervention precondition")),
    };
    let value = std::array::from_fn(|index| f32::from_bits(u32_at(input, 48 + index * 4)));
    let operation = match input[11] {
        1 => InterventionOperation::SetPosition { value },
        2 => InterventionOperation::AddVelocity { value },
        3 => InterventionOperation::AddPosition { value },
        4 => InterventionOperation::SetVelocity { value },
        5 => InterventionOperation::SetFacingYaw {
            value: i16_at(input, 48),
        },
        6 => InterventionOperation::MoveAlongCubicCurve {
            control_points: std::array::from_fn(|point| {
                std::array::from_fn(|axis| {
                    f32::from_bits(u32_at(input, 48 + (point * 3 + axis) * 4))
                })
            }),
        },
        7 if input[48] <= 1 => InterventionOperation::SetTargetPlayer {
            enabled: input[48] != 0,
        },
        8 => InterventionOperation::SetHealth {
            value: i16_at(input, 48),
        },
        9 => InterventionOperation::SetTimer {
            timer: decode_timer(input[48])?,
            ticks: u16_at(input, 50),
        },
        10 if input[52] <= 1 => InterventionOperation::SetFlag {
            domain: decode_flag_domain(input[48])?,
            index: u16_at(input, 50),
            value: input[52] != 0,
        },
        11 => InterventionOperation::SpawnAtPosition { value },
        12 => InterventionOperation::Despawn,
        _ => return Err(error("unknown intervention operation")),
    };
    Ok(Intervention {
        start_tick: u32_at(input, 0),
        duration_ticks: u32_at(input, 4),
        phase,
        selector,
        precondition,
        operation,
    })
}

fn intervention_sort_key(intervention: &Intervention) -> Vec<u8> {
    let mut record = vec![0; RECORD_SIZE];
    encode_record(intervention, &mut record);
    record[4..].to_vec()
}

fn parse_bool(value: &str, line: usize, label: &str) -> Result<bool, InterventionTapeError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(dsl_error(line, &format!("invalid {label}"))),
    }
}

fn parse_timer(value: &str, line: usize) -> Result<InterventionTimer, InterventionTapeError> {
    match value {
        "damage_wait" => Ok(InterventionTimer::DamageWait),
        "ice_damage_wait" => Ok(InterventionTimer::IceDamageWait),
        "sword_change_wait" => Ok(InterventionTimer::SwordChangeWait),
        _ => Err(dsl_error(line, "unknown typed timer")),
    }
}

fn encode_timer(timer: InterventionTimer) -> u8 {
    match timer {
        InterventionTimer::DamageWait => 1,
        InterventionTimer::IceDamageWait => 2,
        InterventionTimer::SwordChangeWait => 3,
    }
}

fn decode_timer(value: u8) -> Result<InterventionTimer, InterventionTapeError> {
    match value {
        1 => Ok(InterventionTimer::DamageWait),
        2 => Ok(InterventionTimer::IceDamageWait),
        3 => Ok(InterventionTimer::SwordChangeWait),
        _ => Err(error("unknown typed intervention timer")),
    }
}

fn parse_flag_domain(
    value: &str,
    line: usize,
) -> Result<InterventionFlagDomain, InterventionTapeError> {
    match value {
        "actor_status" => Ok(InterventionFlagDomain::ActorStatus),
        "room_switch" => Ok(InterventionFlagDomain::RoomSwitch),
        "event_bit" => Ok(InterventionFlagDomain::EventBit),
        _ => Err(dsl_error(line, "unknown typed flag domain")),
    }
}

fn encode_flag_domain(domain: InterventionFlagDomain) -> u8 {
    match domain {
        InterventionFlagDomain::ActorStatus => 1,
        InterventionFlagDomain::RoomSwitch => 2,
        InterventionFlagDomain::EventBit => 3,
    }
}

fn decode_flag_domain(value: u8) -> Result<InterventionFlagDomain, InterventionTapeError> {
    match value {
        1 => Ok(InterventionFlagDomain::ActorStatus),
        2 => Ok(InterventionFlagDomain::RoomSwitch),
        3 => Ok(InterventionFlagDomain::EventBit),
        _ => Err(error("unknown typed intervention flag domain")),
    }
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
    fn version_1_0_artifacts_remain_canonical_and_decodable() {
        let mut legacy = InterventionTape::compile_dsl(DSL)
            .unwrap()
            .encode()
            .unwrap();
        put_u16(&mut legacy, 10, 0);
        let decoded = InterventionTape::decode(&legacy).unwrap();
        assert_eq!(decoded.interventions.len(), 2);

        let mut falsely_legacy = InterventionTape::compile_dsl(
            "timeline 2\nat 0 for 1 before_game_tick process 7 require actor_exists set_health 3",
        )
        .unwrap()
        .encode()
        .unwrap();
        put_u16(&mut falsely_legacy, 10, 0);
        assert!(InterventionTape::decode(&falsely_legacy).is_err());
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

    #[test]
    fn complete_typed_operation_catalog_round_trips() {
        let source = r#"
timeline 30
at 1 for 1 before_game_tick process 7 require actor_exists add_position 1 2 3
at 2 for 1 before_game_tick process 7 require actor_exists set_velocity 4 5 6
at 3 for 1 before_game_tick process 7 require actor_exists add_velocity -1 0 1
at 4 for 1 before_game_tick process 7 require actor_exists set_facing_yaw -16384
at 5 for 2 before_game_tick process 7 require actor_exists move_cubic 0 0 0 1 2 3 4 5 6 7 8 9
at 7 for 1 before_game_tick process 7 require actor_exists set_target_player true
at 8 for 1 before_game_tick process 7 require actor_exists set_health 12
at 9 for 1 before_game_tick process 7 require actor_exists set_timer damage_wait 30
at 10 for 1 before_game_tick process 7 require actor_exists set_flag actor_status 3 true
at 11 for 1 before_game_tick placed F_SP104 1 9 12 require actor_absent spawn_at 1 2 3
at 12 for 1 before_game_tick placed F_SP104 1 9 12 require actor_exists despawn
"#;
        let tape = InterventionTape::compile_dsl(source).unwrap();
        assert_eq!(tape.interventions.len(), 11);
        assert_eq!(
            InterventionTape::decode(&tape.encode().unwrap()).unwrap(),
            tape
        );
    }

    #[test]
    fn typed_semantics_reject_wrong_preconditions_and_unbounded_fields() {
        assert!(InterventionTape::compile_dsl(
            "timeline 4\nat 0 for 1 before_game_tick placed F_SP104 1 9 12 require actor_exists spawn_at 0 0 0"
        )
        .is_err());
        assert!(InterventionTape::compile_dsl(
            "timeline 4\nat 0 for 1 before_game_tick process 7 require actor_absent set_health 10"
        )
        .is_err());
        assert!(InterventionTape::compile_dsl(
            "timeline 4\nat 0 for 1 before_game_tick process 7 require actor_exists set_timer damage_wait 3601"
        )
        .is_err());
        assert!(InterventionTape::compile_dsl(
            "timeline 4\nat 0 for 1 before_game_tick process 7 require actor_exists set_flag actor_status 32 true"
        )
        .is_err());
    }

    #[test]
    fn controller_selector_bridge_reuses_only_exact_forms() {
        use crate::controller_program::ActorSelector;

        assert!(
            InterventionSelector::from_controller_selector(12, &ActorSelector::Nearest).is_err()
        );
        assert!(
            InterventionSelector::from_controller_selector(
                12,
                &ActorSelector::Process {
                    process_id: u32::MAX,
                },
            )
            .is_err()
        );
        assert_eq!(
            InterventionSelector::from_controller_selector(
                12,
                &ActorSelector::Process { process_id: 7 }
            )
            .unwrap(),
            InterventionSelector::Process { process_id: 7 }
        );
        assert_eq!(
            InterventionSelector::from_controller_selector(
                12,
                &ActorSelector::Placed {
                    set_id: 9,
                    room: 1,
                    stage_name: "F_SP104".into(),
                }
            )
            .unwrap(),
            InterventionSelector::Placed {
                identity: PlacedActorSelector {
                    stage: "F_SP104".into(),
                    home_room: 1,
                    actor_name: 12,
                    set_id: 9,
                },
            }
        );
    }

    #[test]
    fn overlapping_writes_to_one_semantic_field_are_rejected() {
        let overlapping_position = r#"
timeline 6
at 0 for 3 before_game_tick process 7 require actor_exists set_position 0 0 0
at 1 for 2 before_game_tick process 7 require actor_exists move_cubic 0 0 0 1 1 1 2 2 2 3 3 3
"#;
        assert!(InterventionTape::compile_dsl(overlapping_position).is_err());

        let independent_fields = r#"
timeline 4
at 0 for 2 before_game_tick process 7 require actor_exists set_position 0 0 0
at 0 for 2 before_game_tick process 7 require actor_exists set_velocity 1 0 0
"#;
        assert!(InterventionTape::compile_dsl(independent_fields).is_ok());

        let independent_targets = r#"
timeline 4
at 0 for 2 before_game_tick process 7 require actor_exists set_health 1
at 0 for 2 before_game_tick process 8 require actor_exists set_health 1
"#;
        assert!(InterventionTape::compile_dsl(independent_targets).is_ok());
    }

    #[test]
    fn lifecycle_writes_conflict_with_every_field_on_the_same_actor() {
        let overlapping_lifecycle = r#"
timeline 4
at 0 for 2 before_game_tick placed F_SP104 1 9 12 require actor_exists set_health 1
at 1 for 1 before_game_tick placed F_SP104 1 9 12 require actor_exists despawn
"#;
        assert!(InterventionTape::compile_dsl(overlapping_lifecycle).is_err());
    }
}
