//! Bounded reactive-controller programs and their canonical wire format.
//!
//! The textual language is intentionally small. It describes a fixed timeline
//! of stick-producing layers and button overlays; game-state-dependent layers
//! are evaluated by the native runtime once per simulation tick.

use serde::Serialize;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const MAGIC: &[u8; 8] = b"DUSKCTRL";
pub const VERSION_MAJOR: u16 = 1;
pub const VERSION_MINOR: u16 = 1;
const MIN_SUPPORTED_MINOR: u16 = 0;
pub const HEADER_SIZE: usize = 32;
pub const RECORD_SIZE: usize = 64;
pub const MAX_DURATION_FRAMES: u32 = 1_000_000;
pub const MAX_LAYERS: usize = 32;

const KIND_CUBIC_BEZIER: u8 = 1;
const KIND_SEEK_POINT: u8 = 2;
const KIND_SEEK_ACTOR: u8 = 3;
const KIND_BUTTONS: u8 = 4;
const BLEND_REPLACE: u8 = 0;
const BLEND_ADD: u8 = 1;
const BLEND_OR: u8 = 2;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ControllerProgram {
    pub duration_frames: u32,
    pub layers: Vec<Layer>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Layer {
    pub start_frame: u32,
    pub duration_frames: u32,
    #[serde(flatten)]
    pub operation: Operation,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Operation {
    CubicBezier {
        blend: StickBlend,
        points: [[i16; 2]; 4],
    },
    SeekPoint {
        blend: StickBlend,
        target: [f32; 3],
        offset: [f32; 3],
        stop_radius: f32,
        magnitude: u8,
    },
    SeekActor {
        blend: StickBlend,
        actor_name: i16,
        selector: ActorSelector,
        offset: [f32; 3],
        stop_radius: f32,
        magnitude: u8,
    },
    Buttons {
        mask: u16,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ActorSelector {
    Nearest,
    Process {
        process_id: u32,
    },
    Placed {
        set_id: u16,
        room: i8,
        stage_name: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StickBlend {
    Replace,
    Add,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControllerError {
    line: Option<usize>,
    message: String,
}

impl ControllerError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            line: None,
            message: message.into(),
        }
    }

    fn at(line: usize, message: impl Into<String>) -> Self {
        Self {
            line: Some(line),
            message: message.into(),
        }
    }
}

impl fmt::Display for ControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(line) = self.line {
            write!(formatter, "line {line}: {}", self.message)
        } else {
            formatter.write_str(&self.message)
        }
    }
}

impl Error for ControllerError {}

impl ControllerProgram {
    pub fn parse(source: &str) -> Result<Self, ControllerError> {
        parse(source)
    }

    pub fn validate(&self) -> Result<(), ControllerError> {
        if self.duration_frames == 0 || self.duration_frames > MAX_DURATION_FRAMES {
            return Err(ControllerError::new(format!(
                "program duration must be in 1..={MAX_DURATION_FRAMES} frames"
            )));
        }
        if self.layers.len() > MAX_LAYERS {
            return Err(ControllerError::new(format!(
                "program has {} layers; maximum is {MAX_LAYERS}",
                self.layers.len()
            )));
        }

        for (index, layer) in self.layers.iter().enumerate() {
            if layer.duration_frames == 0 {
                return Err(ControllerError::new(format!(
                    "layer {index} has zero duration"
                )));
            }
            let end = layer
                .start_frame
                .checked_add(layer.duration_frames)
                .ok_or_else(|| ControllerError::new(format!("layer {index} range overflows")))?;
            if end > self.duration_frames {
                return Err(ControllerError::new(format!(
                    "layer {index} range {}..{end} exceeds program duration {}",
                    layer.start_frame, self.duration_frames
                )));
            }
            match &layer.operation {
                Operation::CubicBezier { .. } => {}
                Operation::SeekPoint {
                    target,
                    offset,
                    stop_radius,
                    magnitude,
                    ..
                } => {
                    validate_floats(index, target, offset, *stop_radius)?;
                    validate_magnitude(index, *magnitude)?;
                }
                Operation::SeekActor {
                    selector,
                    offset,
                    stop_radius,
                    magnitude,
                    ..
                } => {
                    validate_floats(index, &[], offset, *stop_radius)?;
                    validate_magnitude(index, *magnitude)?;
                    match selector {
                        ActorSelector::Nearest => {}
                        ActorSelector::Process { process_id }
                            if *process_id != 0 && *process_id != u32::MAX => {}
                        ActorSelector::Process { .. } => {
                            return Err(ControllerError::new(format!(
                                "layer {index} process ID must not be 0 or 4294967295"
                            )));
                        }
                        ActorSelector::Placed {
                            set_id, stage_name, ..
                        } if *set_id != u16::MAX
                            && !stage_name.is_empty()
                            && stage_name.len() <= 8
                            && stage_name.is_ascii() => {}
                        ActorSelector::Placed { .. } => {
                            return Err(ControllerError::new(format!(
                                "layer {index} placed selector requires set ID below 65535 and a nonempty ASCII stage name of at most 8 bytes"
                            )));
                        }
                    }
                }
                Operation::Buttons { mask } => {
                    if *mask == 0 {
                        return Err(ControllerError::new(format!(
                            "layer {index} has an empty button mask"
                        )));
                    }
                }
            }
        }

        let replace_layers: Vec<(usize, &Layer)> = self
            .layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer.is_replace_stick())
            .collect();
        for (position, (left_index, left)) in replace_layers.iter().enumerate() {
            for (right_index, right) in &replace_layers[position + 1..] {
                if ranges_overlap(left, right) {
                    return Err(ControllerError::new(format!(
                        "replace stick layers {left_index} and {right_index} overlap"
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, ControllerError> {
        self.encode_for_version(VERSION_MINOR)
    }

    pub fn decode(input: &[u8]) -> Result<Self, ControllerError> {
        if input.len() < HEADER_SIZE {
            return Err(ControllerError::new(
                "controller file is shorter than its header",
            ));
        }
        if &input[..8] != MAGIC {
            return Err(ControllerError::new("invalid controller magic"));
        }
        let major = get_u16(input, 8);
        let minor = get_u16(input, 10);
        if major != VERSION_MAJOR || !(MIN_SUPPORTED_MINOR..=VERSION_MINOR).contains(&minor) {
            return Err(ControllerError::new(format!(
                "unsupported controller version {major}.{minor}"
            )));
        }
        if get_u16(input, 12) as usize != HEADER_SIZE {
            return Err(ControllerError::new("noncanonical header size"));
        }
        if get_u16(input, 14) as usize != RECORD_SIZE {
            return Err(ControllerError::new("noncanonical record size"));
        }
        let duration_frames = get_u32(input, 16);
        let layer_count = get_u16(input, 20) as usize;
        if get_u16(input, 22) != 0 || get_u32(input, 28) != 0 {
            return Err(ControllerError::new("nonzero reserved header field"));
        }
        if layer_count > MAX_LAYERS {
            return Err(ControllerError::new(format!(
                "controller has {layer_count} layers; maximum is {MAX_LAYERS}"
            )));
        }
        let expected_payload = layer_count * RECORD_SIZE;
        if get_u32(input, 24) as usize != expected_payload {
            return Err(ControllerError::new(
                "payload length does not match layer count",
            ));
        }
        if input.len() != HEADER_SIZE + expected_payload {
            return Err(ControllerError::new(format!(
                "controller length is {}, expected {}",
                input.len(),
                HEADER_SIZE + expected_payload
            )));
        }

        let mut layers = Vec::with_capacity(layer_count);
        for index in 0..layer_count {
            let start = HEADER_SIZE + index * RECORD_SIZE;
            layers.push(decode_layer(
                index,
                &input[start..start + RECORD_SIZE],
                minor,
            )?);
        }
        let program = Self {
            duration_frames,
            layers,
        };
        program.validate()?;
        // This guards every canonical requirement in one place, including
        // zero-filled payload bytes and the fixed enum encodings.
        if program.encode_for_version(minor)?.as_slice() != input {
            return Err(ControllerError::new("noncanonical controller encoding"));
        }
        Ok(program)
    }

    fn encode_for_version(&self, minor: u16) -> Result<Vec<u8>, ControllerError> {
        self.validate()?;
        if !(MIN_SUPPORTED_MINOR..=VERSION_MINOR).contains(&minor) {
            return Err(ControllerError::new(format!(
                "unsupported controller version {VERSION_MAJOR}.{minor}"
            )));
        }
        if minor == 0
            && self.layers.iter().any(|layer| {
                matches!(
                    layer.operation,
                    Operation::SeekActor {
                        selector: ActorSelector::Process { .. } | ActorSelector::Placed { .. },
                        ..
                    }
                )
            })
        {
            return Err(ControllerError::new(
                "exact actor selectors require controller version 1.1",
            ));
        }
        let payload_len = self.layers.len() * RECORD_SIZE;
        let mut output = vec![0_u8; HEADER_SIZE + payload_len];
        output[..8].copy_from_slice(MAGIC);
        put_u16(&mut output, 8, VERSION_MAJOR);
        put_u16(&mut output, 10, minor);
        put_u16(&mut output, 12, HEADER_SIZE as u16);
        put_u16(&mut output, 14, RECORD_SIZE as u16);
        put_u32(&mut output, 16, self.duration_frames);
        put_u16(&mut output, 20, self.layers.len() as u16);
        put_u32(&mut output, 24, payload_len as u32);
        for (index, layer) in self.layers.iter().enumerate() {
            let start = HEADER_SIZE + index * RECORD_SIZE;
            encode_layer(layer, &mut output[start..start + RECORD_SIZE], minor)?;
        }
        Ok(output)
    }
}

impl Layer {
    fn is_replace_stick(&self) -> bool {
        matches!(
            self.operation,
            Operation::CubicBezier {
                blend: StickBlend::Replace,
                ..
            } | Operation::SeekPoint {
                blend: StickBlend::Replace,
                ..
            } | Operation::SeekActor {
                blend: StickBlend::Replace,
                ..
            }
        )
    }
}

fn ranges_overlap(left: &Layer, right: &Layer) -> bool {
    left.start_frame < right.start_frame + right.duration_frames
        && right.start_frame < left.start_frame + left.duration_frames
}

fn validate_floats(
    index: usize,
    target: &[f32],
    offset: &[f32],
    stop_radius: f32,
) -> Result<(), ControllerError> {
    if target.iter().chain(offset).any(|value| !value.is_finite()) || !stop_radius.is_finite() {
        return Err(ControllerError::new(format!(
            "layer {index} contains a non-finite float"
        )));
    }
    if stop_radius < 0.0 {
        return Err(ControllerError::new(format!(
            "layer {index} stop radius must be nonnegative"
        )));
    }
    Ok(())
}

fn validate_magnitude(index: usize, magnitude: u8) -> Result<(), ControllerError> {
    if !(1..=127).contains(&magnitude) {
        return Err(ControllerError::new(format!(
            "layer {index} magnitude must be in 1..=127"
        )));
    }
    Ok(())
}

pub fn parse(source: &str) -> Result<ControllerProgram, ControllerError> {
    let mut saw_header = false;
    let mut duration_frames = None;
    let mut layers = Vec::new();

    for (line_index, original_line) in source.lines().enumerate() {
        let line_number = line_index + 1;
        let line = strip_comment(original_line).trim();
        if line.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if !saw_header {
            if tokens != ["duskcontrol", "1"] {
                return Err(ControllerError::at(
                    line_number,
                    "first declaration must be `duskcontrol 1`",
                ));
            }
            saw_header = true;
            continue;
        }
        match tokens[0] {
            "duskcontrol" => {
                return Err(ControllerError::at(
                    line_number,
                    "duplicate duskcontrol declaration",
                ));
            }
            "frames" => {
                if tokens.len() != 2 {
                    return Err(ControllerError::at(line_number, "expected `frames N`"));
                }
                if duration_frames.is_some() {
                    return Err(ControllerError::at(
                        line_number,
                        "duplicate frames declaration",
                    ));
                }
                duration_frames = Some(parse_number(tokens[1], line_number, "frame count")?);
            }
            "bezier" | "seek" | "buttons" => {
                if duration_frames.is_none() {
                    return Err(ControllerError::at(
                        line_number,
                        "declare `frames N` before controller layers",
                    ));
                }
                if layers.len() == MAX_LAYERS {
                    return Err(ControllerError::at(
                        line_number,
                        format!("controller may contain at most {MAX_LAYERS} layers"),
                    ));
                }
                layers.push(parse_layer(&tokens, line_number)?);
            }
            unknown => {
                return Err(ControllerError::at(
                    line_number,
                    format!("unknown declaration {unknown:?}"),
                ));
            }
        }
    }

    if !saw_header {
        return Err(ControllerError::new("missing `duskcontrol 1` declaration"));
    }
    let program = ControllerProgram {
        duration_frames: duration_frames
            .ok_or_else(|| ControllerError::new("missing `frames N` declaration"))?,
        layers,
    };
    program.validate()?;
    Ok(program)
}

fn strip_comment(line: &str) -> &str {
    let hash = line.find('#');
    let slash = line.find("//");
    match (hash, slash) {
        (Some(left), Some(right)) => &line[..left.min(right)],
        (Some(index), None) | (None, Some(index)) => &line[..index],
        (None, None) => line,
    }
}

fn parse_layer(tokens: &[&str], line: usize) -> Result<Layer, ControllerError> {
    match tokens[0] {
        "bezier" => parse_bezier(tokens, line),
        "seek" if tokens.get(1) == Some(&"point") => parse_seek_point(tokens, line),
        "seek" if tokens.get(1) == Some(&"actor") => parse_seek_actor(tokens, line),
        "seek" => Err(ControllerError::at(
            line,
            "seek kind must be `point` or `actor`",
        )),
        "buttons" => parse_buttons(tokens, line),
        _ => unreachable!(),
    }
}

fn parse_bezier(tokens: &[&str], line: usize) -> Result<Layer, ControllerError> {
    if tokens.len() < 6 {
        return Err(ControllerError::at(line, "incomplete bezier layer"));
    }
    let blend = parse_blend(tokens[1], line)?;
    let (start_frame, duration_frames) = parse_range(tokens, 2, line)?;
    let mut points = [None; 4];
    let mut cursor = 6;
    while cursor < tokens.len() {
        let point = match tokens[cursor] {
            "p0" => 0,
            "p1" => 1,
            "p2" => 2,
            "p3" => 3,
            unknown => {
                return Err(ControllerError::at(
                    line,
                    format!("unknown bezier field {unknown:?}"),
                ));
            }
        };
        if points[point].is_some() {
            return Err(ControllerError::at(
                line,
                format!("duplicate p{point} field"),
            ));
        }
        let x = required_token(tokens, cursor + 1, line, "bezier x coordinate")?;
        let y = required_token(tokens, cursor + 2, line, "bezier y coordinate")?;
        points[point] = Some([
            parse_number(x, line, "bezier x coordinate")?,
            parse_number(y, line, "bezier y coordinate")?,
        ]);
        cursor += 3;
    }
    let points = points
        .into_iter()
        .enumerate()
        .map(|(index, point)| {
            point.ok_or_else(|| ControllerError::at(line, format!("missing p{index} field")))
        })
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .expect("four points");
    Ok(Layer {
        start_frame,
        duration_frames,
        operation: Operation::CubicBezier { blend, points },
    })
}

fn parse_seek_point(tokens: &[&str], line: usize) -> Result<Layer, ControllerError> {
    let (start_frame, duration_frames, mut cursor) = parse_seek_prefix(tokens, line)?;
    let blend = parse_blend(tokens[2], line)?;
    let mut target = None;
    let mut offset = None;
    let mut magnitude = None;
    let mut stop_radius = None;
    while cursor < tokens.len() {
        match tokens[cursor] {
            "target" => {
                reject_duplicate(target.is_some(), line, "target")?;
                target = Some(parse_vec3(tokens, cursor + 1, line, "target")?);
                cursor += 4;
            }
            "offset" => {
                reject_duplicate(offset.is_some(), line, "offset")?;
                offset = Some(parse_vec3(tokens, cursor + 1, line, "offset")?);
                cursor += 4;
            }
            "magnitude" => {
                reject_duplicate(magnitude.is_some(), line, "magnitude")?;
                magnitude = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "magnitude")?,
                    line,
                    "magnitude",
                )?);
                cursor += 2;
            }
            "stop" => {
                reject_duplicate(stop_radius.is_some(), line, "stop")?;
                stop_radius = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "stop radius")?,
                    line,
                    "stop radius",
                )?);
                cursor += 2;
            }
            unknown => {
                return Err(ControllerError::at(
                    line,
                    format!("unknown seek point field {unknown:?}"),
                ));
            }
        }
    }
    Ok(Layer {
        start_frame,
        duration_frames,
        operation: Operation::SeekPoint {
            blend,
            target: required_field(target, line, "target")?,
            offset: required_field(offset, line, "offset")?,
            magnitude: required_field(magnitude, line, "magnitude")?,
            stop_radius: required_field(stop_radius, line, "stop")?,
        },
    })
}

fn parse_seek_actor(tokens: &[&str], line: usize) -> Result<Layer, ControllerError> {
    let (start_frame, duration_frames, mut cursor) = parse_seek_prefix(tokens, line)?;
    let blend = parse_blend(tokens[2], line)?;
    let mut actor_name = None;
    let mut process_id = None;
    let mut set_id = None;
    let mut room = None;
    let mut stage_name = None;
    let mut offset = None;
    let mut magnitude = None;
    let mut stop_radius = None;
    while cursor < tokens.len() {
        match tokens[cursor] {
            "actor" => {
                reject_duplicate(actor_name.is_some(), line, "actor")?;
                actor_name = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "actor name")?,
                    line,
                    "actor name",
                )?);
                cursor += 2;
            }
            "process" => {
                reject_duplicate(process_id.is_some(), line, "process")?;
                if set_id.is_some() || room.is_some() {
                    return Err(ControllerError::at(
                        line,
                        "process and placed actor selectors are mutually exclusive",
                    ));
                }
                process_id = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "process ID")?,
                    line,
                    "process ID",
                )?);
                cursor += 2;
            }
            "set" => {
                reject_duplicate(set_id.is_some(), line, "set")?;
                if process_id.is_some() {
                    return Err(ControllerError::at(
                        line,
                        "process and placed actor selectors are mutually exclusive",
                    ));
                }
                set_id = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "set ID")?,
                    line,
                    "set ID",
                )?);
                cursor += 2;
            }
            "room" => {
                reject_duplicate(room.is_some(), line, "room")?;
                if process_id.is_some() {
                    return Err(ControllerError::at(
                        line,
                        "process and placed actor selectors are mutually exclusive",
                    ));
                }
                room = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "room")?,
                    line,
                    "room",
                )?);
                cursor += 2;
            }
            "stage" => {
                reject_duplicate(stage_name.is_some(), line, "stage")?;
                if process_id.is_some() {
                    return Err(ControllerError::at(
                        line,
                        "process and placed actor selectors are mutually exclusive",
                    ));
                }
                let value = required_token(tokens, cursor + 1, line, "stage name")?;
                if value.is_empty() || value.len() > 8 || !value.is_ascii() {
                    return Err(ControllerError::at(
                        line,
                        "stage name must be nonempty ASCII of at most 8 bytes",
                    ));
                }
                stage_name = Some(value.to_owned());
                cursor += 2;
            }
            "offset" => {
                reject_duplicate(offset.is_some(), line, "offset")?;
                offset = Some(parse_vec3(tokens, cursor + 1, line, "offset")?);
                cursor += 4;
            }
            "magnitude" => {
                reject_duplicate(magnitude.is_some(), line, "magnitude")?;
                magnitude = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "magnitude")?,
                    line,
                    "magnitude",
                )?);
                cursor += 2;
            }
            "stop" => {
                reject_duplicate(stop_radius.is_some(), line, "stop")?;
                stop_radius = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "stop radius")?,
                    line,
                    "stop radius",
                )?);
                cursor += 2;
            }
            unknown => {
                return Err(ControllerError::at(
                    line,
                    format!("unknown seek actor field {unknown:?}"),
                ));
            }
        }
    }
    let selector = match (process_id, set_id, room, stage_name) {
        (None, None, None, None) => ActorSelector::Nearest,
        (Some(process_id), None, None, None) => ActorSelector::Process { process_id },
        (None, Some(set_id), Some(room), Some(stage_name)) => ActorSelector::Placed {
            set_id,
            room,
            stage_name,
        },
        (None, Some(_), None, _) => {
            return Err(ControllerError::at(
                line,
                "placed actor selector requires a room field",
            ));
        }
        (None, None, Some(_), _) => {
            return Err(ControllerError::at(
                line,
                "placed actor selector requires a set field",
            ));
        }
        (None, Some(_), Some(_), None) => {
            return Err(ControllerError::at(
                line,
                "placed actor selector requires a stage field",
            ));
        }
        (None, None, None, Some(_)) => {
            return Err(ControllerError::at(
                line,
                "stage field requires a placed actor selector",
            ));
        }
        _ => {
            return Err(ControllerError::at(
                line,
                "process and placed actor selectors are mutually exclusive",
            ));
        }
    };
    Ok(Layer {
        start_frame,
        duration_frames,
        operation: Operation::SeekActor {
            blend,
            actor_name: required_field(actor_name, line, "actor")?,
            selector,
            offset: required_field(offset, line, "offset")?,
            magnitude: required_field(magnitude, line, "magnitude")?,
            stop_radius: required_field(stop_radius, line, "stop")?,
        },
    })
}

fn parse_seek_prefix(tokens: &[&str], line: usize) -> Result<(u32, u32, usize), ControllerError> {
    if tokens.len() < 7 {
        return Err(ControllerError::at(line, "incomplete seek layer"));
    }
    let (start, duration) = parse_range(tokens, 3, line)?;
    Ok((start, duration, 7))
}

fn parse_buttons(tokens: &[&str], line: usize) -> Result<Layer, ControllerError> {
    let (start_frame, duration_frames) = parse_range(tokens, 1, line)?;
    if tokens.len() == 5 {
        return Err(ControllerError::at(
            line,
            "buttons layer requires at least one button name",
        ));
    }
    let mut mask = 0_u16;
    let mut seen = BTreeSet::new();
    for name in &tokens[5..] {
        let value = button_mask(name)
            .ok_or_else(|| ControllerError::at(line, format!("unknown button name {name:?}")))?;
        if !seen.insert(value) {
            return Err(ControllerError::at(
                line,
                format!("duplicate button name {name:?}"),
            ));
        }
        mask |= value;
    }
    Ok(Layer {
        start_frame,
        duration_frames,
        operation: Operation::Buttons { mask },
    })
}

fn parse_range(tokens: &[&str], start: usize, line: usize) -> Result<(u32, u32), ControllerError> {
    if tokens.get(start) != Some(&"from") || tokens.get(start + 2) != Some(&"for") {
        return Err(ControllerError::at(line, "expected `from N for N`"));
    }
    let start_frame = parse_number(
        required_token(tokens, start + 1, line, "start frame")?,
        line,
        "start frame",
    )?;
    let duration_frames = parse_number(
        required_token(tokens, start + 3, line, "duration")?,
        line,
        "duration",
    )?;
    Ok((start_frame, duration_frames))
}

fn parse_blend(token: &str, line: usize) -> Result<StickBlend, ControllerError> {
    match token {
        "replace" => Ok(StickBlend::Replace),
        "add" => Ok(StickBlend::Add),
        unknown => Err(ControllerError::at(
            line,
            format!("unknown stick blend {unknown:?}; expected replace or add"),
        )),
    }
}

fn parse_vec3(
    tokens: &[&str],
    start: usize,
    line: usize,
    name: &str,
) -> Result<[f32; 3], ControllerError> {
    Ok([
        parse_number(required_token(tokens, start, line, name)?, line, name)?,
        parse_number(required_token(tokens, start + 1, line, name)?, line, name)?,
        parse_number(required_token(tokens, start + 2, line, name)?, line, name)?,
    ])
}

fn required_token<'a>(
    tokens: &'a [&str],
    index: usize,
    line: usize,
    name: &str,
) -> Result<&'a str, ControllerError> {
    tokens
        .get(index)
        .copied()
        .ok_or_else(|| ControllerError::at(line, format!("missing {name}")))
}

fn parse_number<T>(token: &str, line: usize, name: &str) -> Result<T, ControllerError>
where
    T: std::str::FromStr,
{
    token
        .parse()
        .map_err(|_| ControllerError::at(line, format!("invalid {name} {token:?}")))
}

fn reject_duplicate(present: bool, line: usize, name: &str) -> Result<(), ControllerError> {
    if present {
        Err(ControllerError::at(line, format!("duplicate {name} field")))
    } else {
        Ok(())
    }
}

fn required_field<T>(value: Option<T>, line: usize, name: &str) -> Result<T, ControllerError> {
    value.ok_or_else(|| ControllerError::at(line, format!("missing {name} field")))
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

fn encode_layer(layer: &Layer, output: &mut [u8], minor: u16) -> Result<(), ControllerError> {
    put_u32(output, 4, layer.start_frame);
    put_u32(output, 8, layer.duration_frames);
    match &layer.operation {
        Operation::CubicBezier { blend, points } => {
            output[0] = KIND_CUBIC_BEZIER;
            output[1] = encode_blend(*blend);
            for (index, value) in points.iter().flatten().enumerate() {
                put_i16(output, 12 + index * 2, *value);
            }
        }
        Operation::SeekPoint {
            blend,
            target,
            offset,
            stop_radius,
            magnitude,
        } => {
            output[0] = KIND_SEEK_POINT;
            output[1] = encode_blend(*blend);
            for (index, value) in target.iter().chain(offset).enumerate() {
                put_f32(output, 12 + index * 4, *value);
            }
            put_f32(output, 36, *stop_radius);
            output[40] = *magnitude;
        }
        Operation::SeekActor {
            blend,
            actor_name,
            selector,
            offset,
            stop_radius,
            magnitude,
        } => {
            output[0] = KIND_SEEK_ACTOR;
            output[1] = encode_blend(*blend);
            put_i16(output, 12, *actor_name);
            match selector {
                ActorSelector::Nearest => {}
                ActorSelector::Process { process_id } if minor >= 1 => {
                    output[14] = 1;
                    put_u32(output, 33, *process_id);
                }
                ActorSelector::Placed {
                    set_id,
                    room,
                    stage_name,
                } if minor >= 1 => {
                    output[14] = 2;
                    output[15] = *room as u8;
                    put_u16(output, 37, *set_id);
                    output[39..39 + stage_name.len()].copy_from_slice(stage_name.as_bytes());
                }
                _ => {
                    return Err(ControllerError::new(
                        "exact actor selectors require controller version 1.1",
                    ));
                }
            }
            for (index, value) in offset.iter().enumerate() {
                put_f32(output, 16 + index * 4, *value);
            }
            put_f32(output, 28, *stop_radius);
            output[32] = *magnitude;
        }
        Operation::Buttons { mask } => {
            output[0] = KIND_BUTTONS;
            output[1] = BLEND_OR;
            put_u16(output, 12, *mask);
        }
    }
    Ok(())
}

fn decode_layer(index: usize, input: &[u8], minor: u16) -> Result<Layer, ControllerError> {
    if input[2] != 0 {
        return Err(ControllerError::new(format!(
            "layer {index} uses unsupported controller port {}",
            input[2]
        )));
    }
    if input[3] != 0 {
        return Err(ControllerError::new(format!(
            "layer {index} has nonzero flags"
        )));
    }
    let start_frame = get_u32(input, 4);
    let duration_frames = get_u32(input, 8);
    let operation = match input[0] {
        KIND_CUBIC_BEZIER => {
            require_zero(index, input, 28)?;
            let mut points = [[0_i16; 2]; 4];
            for (point_index, point) in points.iter_mut().enumerate() {
                point[0] = get_i16(input, 12 + point_index * 4);
                point[1] = get_i16(input, 14 + point_index * 4);
            }
            Operation::CubicBezier {
                blend: decode_stick_blend(index, input[1])?,
                points,
            }
        }
        KIND_SEEK_POINT => {
            require_zero(index, input, 41)?;
            Operation::SeekPoint {
                blend: decode_stick_blend(index, input[1])?,
                target: [get_f32(input, 12), get_f32(input, 16), get_f32(input, 20)],
                offset: [get_f32(input, 24), get_f32(input, 28), get_f32(input, 32)],
                stop_radius: get_f32(input, 36),
                magnitude: input[40],
            }
        }
        KIND_SEEK_ACTOR => {
            if minor == 0 && (input[14] != 0 || input[15] != 0) {
                return Err(ControllerError::new(format!(
                    "layer {index} has nonzero seek-actor reserved bytes"
                )));
            }
            let selector = match input[14] {
                0 => {
                    if input[15] != 0 {
                        return Err(ControllerError::new(format!(
                            "layer {index} nearest actor selector has nonzero room"
                        )));
                    }
                    require_zero(index, input, 33)?;
                    ActorSelector::Nearest
                }
                1 if minor >= 1 => {
                    if input[15] != 0 || input[37..].iter().any(|byte| *byte != 0) {
                        return Err(ControllerError::new(format!(
                            "layer {index} has noncanonical process actor selector"
                        )));
                    }
                    ActorSelector::Process {
                        process_id: get_u32(input, 33),
                    }
                }
                2 if minor >= 1 => {
                    if input[33..37].iter().any(|byte| *byte != 0)
                        || input[47..].iter().any(|byte| *byte != 0)
                    {
                        return Err(ControllerError::new(format!(
                            "layer {index} has noncanonical placed actor selector"
                        )));
                    }
                    let stage_bytes = &input[39..47];
                    let stage_length = stage_bytes
                        .iter()
                        .position(|byte| *byte == 0)
                        .unwrap_or(stage_bytes.len());
                    if stage_length == 0
                        || stage_bytes[stage_length..].iter().any(|byte| *byte != 0)
                        || !stage_bytes[..stage_length].is_ascii()
                    {
                        return Err(ControllerError::new(format!(
                            "layer {index} has invalid placed actor stage name"
                        )));
                    }
                    let stage_name = std::str::from_utf8(&stage_bytes[..stage_length])
                        .expect("ASCII stage name")
                        .to_owned();
                    ActorSelector::Placed {
                        set_id: get_u16(input, 37),
                        room: input[15] as i8,
                        stage_name,
                    }
                }
                mode => {
                    return Err(ControllerError::new(format!(
                        "layer {index} has invalid actor selector mode {mode} for version 1.{minor}"
                    )));
                }
            };
            Operation::SeekActor {
                blend: decode_stick_blend(index, input[1])?,
                actor_name: get_i16(input, 12),
                selector,
                offset: [get_f32(input, 16), get_f32(input, 20), get_f32(input, 24)],
                stop_radius: get_f32(input, 28),
                magnitude: input[32],
            }
        }
        KIND_BUTTONS => {
            if input[1] != BLEND_OR {
                return Err(ControllerError::new(format!(
                    "button layer {index} must use OR blend"
                )));
            }
            require_zero(index, input, 14)?;
            Operation::Buttons {
                mask: get_u16(input, 12),
            }
        }
        kind => {
            return Err(ControllerError::new(format!(
                "layer {index} has unknown kind {kind}"
            )));
        }
    };
    Ok(Layer {
        start_frame,
        duration_frames,
        operation,
    })
}

fn require_zero(index: usize, input: &[u8], start: usize) -> Result<(), ControllerError> {
    if input[start..].iter().any(|byte| *byte != 0) {
        Err(ControllerError::new(format!(
            "layer {index} has nonzero reserved payload bytes"
        )))
    } else {
        Ok(())
    }
}

fn encode_blend(blend: StickBlend) -> u8 {
    match blend {
        StickBlend::Replace => BLEND_REPLACE,
        StickBlend::Add => BLEND_ADD,
    }
}

fn decode_stick_blend(index: usize, value: u8) -> Result<StickBlend, ControllerError> {
    match value {
        BLEND_REPLACE => Ok(StickBlend::Replace),
        BLEND_ADD => Ok(StickBlend::Add),
        _ => Err(ControllerError::new(format!(
            "stick layer {index} has invalid blend {value}"
        ))),
    }
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

fn put_f32(output: &mut [u8], offset: usize, value: f32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn get_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(input[offset..offset + 2].try_into().expect("u16 slice"))
}

fn get_i16(input: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes(input[offset..offset + 2].try_into().expect("i16 slice"))
}

fn get_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(input[offset..offset + 4].try_into().expect("u32 slice"))
}

fn get_f32(input: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(input[offset..offset + 4].try_into().expect("f32 slice"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"
        # bounded controller example
        duskcontrol 1
        frames 120
        bezier replace from 0 for 120 p0 0 127 p1 0 127 p2 127 0 p3 127 0
        seek point add from 0 for 120 target 1 2 3 offset 0 0 0 magnitude 20 stop 5
        seek actor add from 10 for 40 actor 123 offset 1 0 2 magnitude 30 stop 10
        buttons from 5 for 1 B START // overlays are OR-composed
    "#;

    #[test]
    fn parses_encodes_and_decodes_all_layer_kinds() {
        let program = parse(SOURCE).unwrap();
        assert_eq!(program.duration_frames, 120);
        assert_eq!(program.layers.len(), 4);
        let bytes = program.encode().unwrap();
        assert_eq!(bytes.len(), HEADER_SIZE + 4 * RECORD_SIZE);
        assert_eq!(&bytes[..8], MAGIC);
        assert_eq!(get_u16(&bytes, 8), 1);
        assert_eq!(get_u16(&bytes, 10), 1);
        assert_eq!(get_u16(&bytes, 12), 32);
        assert_eq!(get_u16(&bytes, 14), 64);
        assert_eq!(get_u32(&bytes, 16), 120);
        assert_eq!(get_u16(&bytes, 20), 4);
        assert_eq!(get_u32(&bytes, 24), 256);
        assert_eq!(bytes[HEADER_SIZE], KIND_CUBIC_BEZIER);
        assert_eq!(get_i16(&bytes, HEADER_SIZE + 14), 127);
        assert_eq!(bytes[HEADER_SIZE + 2 * RECORD_SIZE], KIND_SEEK_ACTOR);
        assert_eq!(get_i16(&bytes, HEADER_SIZE + 2 * RECORD_SIZE + 12), 123);
        assert_eq!(bytes[HEADER_SIZE + 3 * RECORD_SIZE + 1], BLEND_OR);
        assert_eq!(get_u16(&bytes, HEADER_SIZE + 3 * RECORD_SIZE + 12), 0x1200);
        assert_eq!(ControllerProgram::decode(&bytes).unwrap(), program);
    }

    #[test]
    fn exact_actor_selectors_round_trip_in_version_1_1() {
        let source = r#"duskcontrol 1
frames 8
seek actor replace from 0 for 4 actor 123 process 42 offset 0 0 0 magnitude 127 stop 0
seek actor replace from 4 for 4 actor -77 set 14 room -3 stage F_SP103 offset 1 2 3 magnitude 80 stop 5
"#;
        let program = parse(source).unwrap();
        assert_eq!(
            program.layers[0].operation,
            Operation::SeekActor {
                blend: StickBlend::Replace,
                actor_name: 123,
                selector: ActorSelector::Process { process_id: 42 },
                offset: [0.0, 0.0, 0.0],
                stop_radius: 0.0,
                magnitude: 127,
            }
        );
        assert_eq!(
            program.layers[1].operation,
            Operation::SeekActor {
                blend: StickBlend::Replace,
                actor_name: -77,
                selector: ActorSelector::Placed {
                    set_id: 14,
                    room: -3,
                    stage_name: "F_SP103".to_owned(),
                },
                offset: [1.0, 2.0, 3.0],
                stop_radius: 5.0,
                magnitude: 80,
            }
        );

        let bytes = program.encode().unwrap();
        let first = HEADER_SIZE;
        let second = HEADER_SIZE + RECORD_SIZE;
        assert_eq!(bytes[first + 14], 1);
        assert_eq!(get_u32(&bytes, first + 33), 42);
        assert_eq!(bytes[second + 14], 2);
        assert_eq!(bytes[second + 15] as i8, -3);
        assert_eq!(get_u16(&bytes, second + 37), 14);
        assert_eq!(&bytes[second + 39..second + 46], b"F_SP103");
        assert_eq!(bytes[second + 46], 0);
        assert_eq!(ControllerProgram::decode(&bytes).unwrap(), program);
    }

    #[test]
    fn version_1_0_nearest_actor_programs_remain_decodable_and_strict() {
        let program = parse(SOURCE).unwrap();
        let legacy = program.encode_for_version(0).unwrap();
        assert_eq!(get_u16(&legacy, 10), 0);
        assert_eq!(ControllerProgram::decode(&legacy).unwrap(), program);

        let actor = HEADER_SIZE + 2 * RECORD_SIZE;
        let mut selector_in_legacy = legacy.clone();
        selector_in_legacy[actor + 14] = 1;
        assert!(
            ControllerProgram::decode(&selector_in_legacy)
                .unwrap_err()
                .to_string()
                .contains("reserved")
        );

        let exact = parse("duskcontrol 1\nframes 1\nseek actor add from 0 for 1 actor 1 process 2 offset 0 0 0 magnitude 1 stop 0\n").unwrap();
        assert!(
            exact
                .encode_for_version(0)
                .unwrap_err()
                .to_string()
                .contains("version 1.1")
        );
    }

    #[test]
    fn actor_selector_syntax_and_values_are_strict() {
        let prefix = "duskcontrol 1\nframes 1\nseek actor add from 0 for 1 actor 1 ";
        let suffix = " offset 0 0 0 magnitude 1 stop 0\n";
        for (selector, expected) in [
            ("process 0", "process ID"),
            ("process 4294967295", "process ID"),
            ("set 65535 room 0 stage F_SP103", "set ID"),
            ("set 1", "room field"),
            ("room 1", "set field"),
            ("set 1 room 0", "stage field"),
            ("stage F_SP103", "requires a placed"),
            ("set 1 room 128 stage F_SP103", "invalid room"),
            ("set 1 room -129 stage F_SP103", "invalid room"),
            ("set 1 room 0 stage TOO_LONG9", "at most 8"),
            ("set 1 room 0 stage F_SP10é", "ASCII"),
            ("process 5 set 1 room 0", "mutually exclusive"),
            ("process 5 stage F_SP103", "mutually exclusive"),
            ("process 5 process 6", "duplicate process"),
            ("set 1 room 0 stage F_SP103 room 1", "duplicate room"),
            ("set 1 room 0 stage F_SP103 stage D_MN01", "duplicate stage"),
        ] {
            let source = format!("{prefix}{selector}{suffix}");
            assert!(
                parse(&source).unwrap_err().to_string().contains(expected),
                "selector {selector:?} did not report {expected:?}"
            );
        }
    }

    #[test]
    fn actor_selector_binary_reservations_are_canonical() {
        let process = parse("duskcontrol 1\nframes 1\nseek actor add from 0 for 1 actor 1 process 2 offset 0 0 0 magnitude 1 stop 0\n").unwrap();
        let mut process_room = process.encode().unwrap();
        process_room[HEADER_SIZE + 15] = 1;
        assert!(ControllerProgram::decode(&process_room).is_err());
        let mut process_set = process.encode().unwrap();
        process_set[HEADER_SIZE + 37] = 1;
        assert!(ControllerProgram::decode(&process_set).is_err());

        let placed = parse("duskcontrol 1\nframes 1\nseek actor add from 0 for 1 actor 1 set 2 room 0 stage F_SP103 offset 0 0 0 magnitude 1 stop 0\n").unwrap();
        let mut placed_process = placed.encode().unwrap();
        placed_process[HEADER_SIZE + 33] = 1;
        assert!(ControllerProgram::decode(&placed_process).is_err());
        let mut placed_tail = placed.encode().unwrap();
        placed_tail[HEADER_SIZE + 47] = 1;
        assert!(ControllerProgram::decode(&placed_tail).is_err());
        let mut placed_gap = placed.encode().unwrap();
        placed_gap[HEADER_SIZE + 42] = 0;
        assert!(ControllerProgram::decode(&placed_gap).is_err());
        let mut placed_empty = placed.encode().unwrap();
        placed_empty[HEADER_SIZE + 39..HEADER_SIZE + 47].fill(0);
        assert!(ControllerProgram::decode(&placed_empty).is_err());
        let mut placed_non_ascii = placed.encode().unwrap();
        placed_non_ascii[HEADER_SIZE + 39] = 0x80;
        assert!(ControllerProgram::decode(&placed_non_ascii).is_err());
        let mut unknown_mode = placed.encode().unwrap();
        unknown_mode[HEADER_SIZE + 14] = 3;
        assert!(
            ControllerProgram::decode(&unknown_mode)
                .unwrap_err()
                .to_string()
                .contains("invalid actor selector mode")
        );
    }

    #[test]
    fn field_order_is_flexible_but_duplicates_and_unknowns_fail() {
        let reordered = "duskcontrol 1\nframes 4\nseek point add from 0 for 4 stop 0 magnitude 127 offset 0 0 0 target 1 2 3\n";
        assert!(parse(reordered).is_ok());
        let duplicate = "duskcontrol 1\nframes 4\nseek point add from 0 for 4 stop 0 stop 1 magnitude 1 offset 0 0 0 target 1 2 3\n";
        assert!(
            parse(duplicate)
                .unwrap_err()
                .to_string()
                .contains("duplicate stop")
        );
        let unknown = "duskcontrol 1\nframes 4\nseek point add from 0 for 4 target 1 2 3 offset 0 0 0 magnitude 1 stop 0 wat 4\n";
        assert!(parse(unknown).unwrap_err().to_string().contains("unknown"));
    }

    #[test]
    fn rejects_overlapping_replace_stick_layers_but_allows_additive_layers() {
        let overlapping = "duskcontrol 1\nframes 20\nbezier replace from 0 for 11 p0 0 0 p1 0 0 p2 0 0 p3 0 0\nseek point replace from 10 for 10 target 0 0 0 offset 0 0 0 magnitude 1 stop 0\n";
        assert!(
            parse(overlapping)
                .unwrap_err()
                .to_string()
                .contains("overlap")
        );
        let adjacent = overlapping.replace("from 10 for 10", "from 11 for 9");
        assert!(parse(&adjacent).is_ok());
    }

    #[test]
    fn validates_ranges_floats_magnitudes_and_buttons() {
        for (source, expected) in [
            (
                "duskcontrol 1\nframes 4\nbuttons from 4 for 1 A\n",
                "exceeds",
            ),
            (
                "duskcontrol 1\nframes 4\nbuttons from 0 for 0 A\n",
                "zero duration",
            ),
            (
                "duskcontrol 1\nframes 4\nseek point add from 0 for 4 target NaN 0 0 offset 0 0 0 magnitude 1 stop 0\n",
                "non-finite",
            ),
            (
                "duskcontrol 1\nframes 4\nseek actor add from 0 for 4 actor 1 offset 0 0 0 magnitude 0 stop 0\n",
                "magnitude",
            ),
            (
                "duskcontrol 1\nframes 4\nseek actor add from 0 for 4 actor 1 offset 0 0 0 magnitude 1 stop -1\n",
                "nonnegative",
            ),
            (
                "duskcontrol 1\nframes 4\nbuttons from 0 for 1 NOPE\n",
                "unknown button",
            ),
            (
                "duskcontrol 1\nframes 4\nbuttons from 0 for 1 START MENU\n",
                "duplicate button",
            ),
        ] {
            assert!(parse(source).unwrap_err().to_string().contains(expected));
        }
    }

    #[test]
    fn rejects_noncanonical_binary_fields_and_trailing_data() {
        let canonical = parse(SOURCE).unwrap().encode().unwrap();
        for (offset, expected) in [
            (22, "reserved header"),
            (28, "reserved header"),
            (HEADER_SIZE + 2, "port"),
            (HEADER_SIZE + 3, "flags"),
            (HEADER_SIZE + 28, "reserved payload"),
        ] {
            let mut corrupt = canonical.clone();
            corrupt[offset] = 1;
            assert!(
                ControllerProgram::decode(&corrupt)
                    .unwrap_err()
                    .to_string()
                    .contains(expected)
            );
        }
        let mut trailing = canonical;
        trailing.push(0);
        assert!(ControllerProgram::decode(&trailing).is_err());
    }

    #[test]
    fn rejects_wrong_blend_and_nonfinite_binary_float() {
        let program = parse(SOURCE).unwrap();
        let mut wrong_button_blend = program.encode().unwrap();
        wrong_button_blend[HEADER_SIZE + 3 * RECORD_SIZE + 1] = BLEND_ADD;
        assert!(
            ControllerProgram::decode(&wrong_button_blend)
                .unwrap_err()
                .to_string()
                .contains("OR blend")
        );

        let mut nan = program.encode().unwrap();
        nan[HEADER_SIZE + RECORD_SIZE + 12..HEADER_SIZE + RECORD_SIZE + 16]
            .copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(
            ControllerProgram::decode(&nan)
                .unwrap_err()
                .to_string()
                .contains("non-finite")
        );
    }

    #[test]
    fn every_truncation_and_unknown_kind_is_rejected() {
        let canonical = parse(SOURCE).unwrap().encode().unwrap();
        for length in 0..canonical.len() {
            assert!(
                ControllerProgram::decode(&canonical[..length]).is_err(),
                "accepted truncation at {length} bytes"
            );
        }
        let mut unknown_kind = canonical;
        unknown_kind[HEADER_SIZE] = 99;
        assert!(
            ControllerProgram::decode(&unknown_kind)
                .unwrap_err()
                .to_string()
                .contains("unknown kind")
        );
    }

    #[test]
    fn duration_and_layer_limits_are_inclusive_and_enforced() {
        let boundary = format!("duskcontrol 1\nframes {MAX_DURATION_FRAMES}\n");
        assert!(parse(&boundary).is_ok());
        let too_long = format!("duskcontrol 1\nframes {}\n", MAX_DURATION_FRAMES + 1);
        assert!(
            parse(&too_long)
                .unwrap_err()
                .to_string()
                .contains("duration")
        );

        let mut maximum_layers = String::from("duskcontrol 1\nframes 1\n");
        for _ in 0..MAX_LAYERS {
            maximum_layers.push_str("buttons from 0 for 1 A\n");
        }
        assert_eq!(parse(&maximum_layers).unwrap().layers.len(), MAX_LAYERS);
        maximum_layers.push_str("buttons from 0 for 1 B\n");
        assert!(
            parse(&maximum_layers)
                .unwrap_err()
                .to_string()
                .contains("at most")
        );
    }

    #[test]
    fn requires_unique_header_and_frames_declarations() {
        assert!(parse("").unwrap_err().to_string().contains("missing"));
        assert!(
            parse("frames 1\n")
                .unwrap_err()
                .to_string()
                .contains("first")
        );
        assert!(
            parse("duskcontrol 1\n")
                .unwrap_err()
                .to_string()
                .contains("frames")
        );
        assert!(
            parse("duskcontrol 1\nframes 1\nframes 1\n")
                .unwrap_err()
                .to_string()
                .contains("duplicate frames")
        );
    }
}
