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
pub const VERSION_MINOR: u16 = 4;
const MIN_SUPPORTED_MINOR: u16 = 0;
pub const HEADER_SIZE: usize = 32;
pub const RECORD_SIZE: usize = 64;
pub const MAX_DURATION_FRAMES: u32 = 1_000_000;
pub const MAX_LAYERS: usize = 32;

const KIND_CUBIC_BEZIER: u8 = 1;
const KIND_SEEK_POINT: u8 = 2;
const KIND_SEEK_ACTOR: u8 = 3;
const KIND_BUTTONS: u8 = 4;
const KIND_SEEK_COORDINATE: u8 = 5;
const KIND_SEEK_PLANE: u8 = 6;
const KIND_SEEK_RESOLVED: u8 = 7;
const KIND_NEUTRAL: u8 = 8;
const KIND_TURN: u8 = 9;
const KIND_BRAKE: u8 = 10;
const KIND_HEADING: u8 = 11;
const KIND_MAINTAIN_DISTANCE: u8 = 12;
const KIND_CAMERA: u8 = 13;
const KIND_SAFETY_CLAMP: u8 = 14;
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
    SeekCoordinate {
        blend: StickBlend,
        frame: CoordinateFrame,
        target: [f32; 3],
        offset: [f32; 3],
        stop_radius: f32,
        magnitude: u8,
    },
    SeekPlane {
        blend: StickBlend,
        frame: CoordinateFrame,
        point: [f32; 3],
        normal: [f32; 3],
        stop_radius: f32,
        magnitude: u8,
    },
    SeekResolved {
        blend: StickBlend,
        target: ResolvedTarget,
        offset: [f32; 3],
        stop_radius: f32,
        magnitude: u8,
    },
    Neutral,
    Turn {
        blend: StickBlend,
        direction: TurnDirection,
        magnitude: u8,
    },
    Brake {
        blend: StickBlend,
        stop_speed: f32,
        magnitude: u8,
    },
    Align {
        blend: StickBlend,
        frame: CoordinateFrame,
        heading_radians: f32,
        tolerance_radians: f32,
        magnitude: u8,
    },
    MaintainHeading {
        blend: StickBlend,
        frame: CoordinateFrame,
        heading_radians: f32,
        magnitude: u8,
    },
    MaintainDistance {
        blend: StickBlend,
        frame: CoordinateFrame,
        target: [f32; 3],
        distance: f32,
        tolerance: f32,
        magnitude: u8,
    },
    Camera {
        blend: StickBlend,
        x: i16,
        y: i16,
    },
    SafetyClamp {
        main_limit: u8,
        substick_limit: u8,
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
pub enum CoordinateFrame {
    World,
    Player,
    Camera,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnDirection {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedTarget {
    PathPoint {
        path_id: u64,
        point_index: u32,
        position: [f32; 3],
    },
    Opening {
        opening_id: u64,
        position: [f32; 3],
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
                        ActorSelector::Placed { stage_name, .. }
                            if !stage_name.is_empty()
                                && stage_name.len() <= 8
                                && stage_name.is_ascii() => {}
                        ActorSelector::Placed { .. } => {
                            return Err(ControllerError::new(format!(
                                "layer {index} placed selector requires a nonempty ASCII stage name of at most 8 bytes"
                            )));
                        }
                    }
                }
                Operation::SeekCoordinate {
                    target,
                    offset,
                    stop_radius,
                    magnitude,
                    ..
                } => {
                    validate_floats(index, target, offset, *stop_radius)?;
                    validate_magnitude(index, *magnitude)?;
                }
                Operation::SeekPlane {
                    point,
                    normal,
                    stop_radius,
                    magnitude,
                    ..
                } => {
                    validate_floats(index, point, normal, *stop_radius)?;
                    validate_magnitude(index, *magnitude)?;
                    let horizontal_length_squared =
                        f64::from(normal[0]).powi(2) + f64::from(normal[2]).powi(2);
                    if horizontal_length_squared <= f64::EPSILON {
                        return Err(ControllerError::new(format!(
                            "layer {index} plane normal has no horizontal component"
                        )));
                    }
                }
                Operation::SeekResolved {
                    target,
                    offset,
                    stop_radius,
                    magnitude,
                    ..
                } => {
                    let position = match target {
                        ResolvedTarget::PathPoint {
                            path_id, position, ..
                        } if *path_id != 0 => position,
                        ResolvedTarget::Opening {
                            opening_id,
                            position,
                        } if *opening_id != 0 => position,
                        _ => {
                            return Err(ControllerError::new(format!(
                                "layer {index} resolved target ID must be nonzero"
                            )));
                        }
                    };
                    validate_floats(index, position, offset, *stop_radius)?;
                    validate_magnitude(index, *magnitude)?;
                }
                Operation::Neutral => {}
                Operation::Turn { magnitude, .. } => {
                    validate_magnitude(index, *magnitude)?;
                }
                Operation::Brake {
                    stop_speed,
                    magnitude,
                    ..
                } => {
                    validate_nonnegative(index, "stop speed", *stop_speed)?;
                    validate_magnitude(index, *magnitude)?;
                }
                Operation::Align {
                    heading_radians,
                    tolerance_radians,
                    magnitude,
                    ..
                } => {
                    validate_heading(index, *heading_radians)?;
                    validate_tolerance(index, "heading tolerance", *tolerance_radians)?;
                    validate_magnitude(index, *magnitude)?;
                }
                Operation::MaintainHeading {
                    heading_radians,
                    magnitude,
                    ..
                } => {
                    validate_heading(index, *heading_radians)?;
                    validate_magnitude(index, *magnitude)?;
                }
                Operation::MaintainDistance {
                    target,
                    distance,
                    tolerance,
                    magnitude,
                    ..
                } => {
                    if target.iter().any(|value| !value.is_finite()) {
                        return Err(ControllerError::new(format!(
                            "layer {index} contains a non-finite float"
                        )));
                    }
                    validate_nonnegative(index, "distance", *distance)?;
                    validate_nonnegative(index, "distance tolerance", *tolerance)?;
                    if tolerance > distance {
                        return Err(ControllerError::new(format!(
                            "layer {index} distance tolerance must not exceed distance"
                        )));
                    }
                    validate_magnitude(index, *magnitude)?;
                }
                Operation::Camera { x, y, .. } => {
                    if !(-128..=127).contains(x) || !(-128..=127).contains(y) {
                        return Err(ControllerError::new(format!(
                            "layer {index} camera values must be in -128..=127"
                        )));
                    }
                }
                Operation::SafetyClamp {
                    main_limit,
                    substick_limit,
                } => {
                    if *main_limit > 127 || *substick_limit > 127 {
                        return Err(ControllerError::new(format!(
                            "layer {index} safety-clamp limits must be in 0..=127"
                        )));
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
        reject_overlapping_layers(
            &self.layers,
            |layer| {
                matches!(
                    layer.operation,
                    Operation::Camera {
                        blend: StickBlend::Replace,
                        ..
                    }
                )
            },
            "camera replacement",
        )?;
        reject_overlapping_layers(
            &self.layers,
            |layer| matches!(layer.operation, Operation::SafetyClamp { .. }),
            "safety clamp",
        )?;
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
        if minor < 2
            && self.layers.iter().any(|layer| {
                matches!(
                    layer.operation,
                    Operation::SeekCoordinate { .. }
                        | Operation::SeekPlane { .. }
                        | Operation::SeekResolved { .. }
                )
            })
        {
            return Err(ControllerError::new(
                "coordinate, plane, path-point, and opening targets require controller version 1.2",
            ));
        }
        if minor < 3
            && self.layers.iter().any(|layer| {
                matches!(
                    layer.operation,
                    Operation::Neutral
                        | Operation::Turn { .. }
                        | Operation::Brake { .. }
                        | Operation::Align { .. }
                        | Operation::MaintainHeading { .. }
                        | Operation::MaintainDistance { .. }
                )
            })
        {
            return Err(ControllerError::new(
                "turn, brake, neutral, align, heading, and distance controls require controller version 1.3",
            ));
        }
        if minor < 4
            && self.layers.iter().any(|layer| {
                matches!(
                    layer.operation,
                    Operation::Camera { .. } | Operation::SafetyClamp { .. }
                )
            })
        {
            return Err(ControllerError::new(
                "camera and safety-clamp layers require controller version 1.4",
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
            } | Operation::SeekCoordinate {
                blend: StickBlend::Replace,
                ..
            } | Operation::SeekPlane {
                blend: StickBlend::Replace,
                ..
            } | Operation::SeekResolved {
                blend: StickBlend::Replace,
                ..
            } | Operation::Neutral
                | Operation::Turn {
                    blend: StickBlend::Replace,
                    ..
                }
                | Operation::Brake {
                    blend: StickBlend::Replace,
                    ..
                }
                | Operation::Align {
                    blend: StickBlend::Replace,
                    ..
                }
                | Operation::MaintainHeading {
                    blend: StickBlend::Replace,
                    ..
                }
                | Operation::MaintainDistance {
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

fn reject_overlapping_layers(
    layers: &[Layer],
    selected: impl Fn(&Layer) -> bool,
    label: &str,
) -> Result<(), ControllerError> {
    let selected: Vec<_> = layers
        .iter()
        .enumerate()
        .filter(|(_, layer)| selected(layer))
        .collect();
    for (position, (left_index, left)) in selected.iter().enumerate() {
        for (right_index, right) in &selected[position + 1..] {
            if ranges_overlap(left, right) {
                return Err(ControllerError::new(format!(
                    "{label} layers {left_index} and {right_index} overlap"
                )));
            }
        }
    }
    Ok(())
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

fn validate_nonnegative(index: usize, name: &str, value: f32) -> Result<(), ControllerError> {
    if !value.is_finite() || value < 0.0 {
        return Err(ControllerError::new(format!(
            "layer {index} {name} must be finite and nonnegative"
        )));
    }
    Ok(())
}

fn validate_heading(index: usize, value: f32) -> Result<(), ControllerError> {
    if !value.is_finite() || !(-std::f32::consts::PI..=std::f32::consts::PI).contains(&value) {
        return Err(ControllerError::new(format!(
            "layer {index} heading must be finite and in [-pi, pi] radians"
        )));
    }
    Ok(())
}

fn validate_tolerance(index: usize, name: &str, value: f32) -> Result<(), ControllerError> {
    if !value.is_finite() || !(0.0..=std::f32::consts::PI).contains(&value) {
        return Err(ControllerError::new(format!(
            "layer {index} {name} must be finite and in [0, pi] radians"
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
            "bezier" | "seek" | "buttons" | "neutral" | "turn" | "brake" | "align" | "maintain"
            | "camera" | "clamp" => {
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
        "seek" if tokens.get(1) == Some(&"coordinate") => parse_seek_coordinate(tokens, line),
        "seek" if tokens.get(1) == Some(&"plane") => parse_seek_plane(tokens, line),
        "seek" if tokens.get(1) == Some(&"path-point") => parse_seek_resolved(tokens, line, true),
        "seek" if tokens.get(1) == Some(&"opening") => parse_seek_resolved(tokens, line, false),
        "seek" => Err(ControllerError::at(
            line,
            "seek kind must be point, actor, coordinate, plane, path-point, or opening",
        )),
        "neutral" => parse_neutral(tokens, line),
        "turn" => parse_turn(tokens, line),
        "brake" => parse_brake(tokens, line),
        "align" => parse_align(tokens, line),
        "maintain" if tokens.get(1) == Some(&"heading") => parse_maintain_heading(tokens, line),
        "maintain" if tokens.get(1) == Some(&"distance") => parse_maintain_distance(tokens, line),
        "maintain" => Err(ControllerError::at(
            line,
            "maintain kind must be heading or distance",
        )),
        "buttons" => parse_buttons(tokens, line),
        "camera" => parse_camera(tokens, line),
        "clamp" => parse_safety_clamp(tokens, line),
        _ => unreachable!(),
    }
}

fn parse_neutral(tokens: &[&str], line: usize) -> Result<Layer, ControllerError> {
    if tokens.get(1) != Some(&"replace") || tokens.len() != 6 {
        return Err(ControllerError::at(
            line,
            "expected `neutral replace from N for N`",
        ));
    }
    let (start_frame, duration_frames) = parse_range(tokens, 2, line)?;
    Ok(Layer {
        start_frame,
        duration_frames,
        operation: Operation::Neutral,
    })
}

fn parse_turn(tokens: &[&str], line: usize) -> Result<Layer, ControllerError> {
    if tokens.len() < 7 {
        return Err(ControllerError::at(line, "incomplete turn layer"));
    }
    let blend = parse_blend(tokens[1], line)?;
    let (start_frame, duration_frames) = parse_range(tokens, 2, line)?;
    let mut direction = None;
    let mut magnitude = None;
    let mut cursor = 6;
    while cursor < tokens.len() {
        match tokens[cursor] {
            "direction" => {
                reject_duplicate(direction.is_some(), line, "direction")?;
                direction = Some(
                    match required_token(tokens, cursor + 1, line, "direction")? {
                        "left" => TurnDirection::Left,
                        "right" => TurnDirection::Right,
                        value => {
                            return Err(ControllerError::at(
                                line,
                                format!("unknown turn direction {value:?}; expected left or right"),
                            ));
                        }
                    },
                );
                cursor += 2;
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
            unknown => {
                return Err(ControllerError::at(
                    line,
                    format!("unknown turn field {unknown:?}"),
                ));
            }
        }
    }
    Ok(Layer {
        start_frame,
        duration_frames,
        operation: Operation::Turn {
            blend,
            direction: required_field(direction, line, "direction")?,
            magnitude: required_field(magnitude, line, "magnitude")?,
        },
    })
}

fn parse_brake(tokens: &[&str], line: usize) -> Result<Layer, ControllerError> {
    if tokens.len() < 7 {
        return Err(ControllerError::at(line, "incomplete brake layer"));
    }
    let blend = parse_blend(tokens[1], line)?;
    let (start_frame, duration_frames) = parse_range(tokens, 2, line)?;
    let mut stop_speed = None;
    let mut magnitude = None;
    let mut cursor = 6;
    while cursor < tokens.len() {
        match tokens[cursor] {
            "stop-speed" => {
                reject_duplicate(stop_speed.is_some(), line, "stop-speed")?;
                stop_speed = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "stop speed")?,
                    line,
                    "stop speed",
                )?);
                cursor += 2;
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
            unknown => {
                return Err(ControllerError::at(
                    line,
                    format!("unknown brake field {unknown:?}"),
                ));
            }
        }
    }
    Ok(Layer {
        start_frame,
        duration_frames,
        operation: Operation::Brake {
            blend,
            stop_speed: required_field(stop_speed, line, "stop-speed")?,
            magnitude: required_field(magnitude, line, "magnitude")?,
        },
    })
}

fn parse_heading_fields(
    tokens: &[&str],
    line: usize,
    cursor_start: usize,
    require_tolerance: bool,
) -> Result<(CoordinateFrame, f32, Option<f32>, u8), ControllerError> {
    let mut frame = None;
    let mut heading = None;
    let mut tolerance = None;
    let mut magnitude = None;
    let mut cursor = cursor_start;
    while cursor < tokens.len() {
        match tokens[cursor] {
            "frame" => {
                reject_duplicate(frame.is_some(), line, "frame")?;
                frame = Some(parse_coordinate_frame(
                    required_token(tokens, cursor + 1, line, "frame")?,
                    line,
                )?);
                cursor += 2;
            }
            "heading" => {
                reject_duplicate(heading.is_some(), line, "heading")?;
                heading = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "heading")?,
                    line,
                    "heading",
                )?);
                cursor += 2;
            }
            "tolerance" if require_tolerance => {
                reject_duplicate(tolerance.is_some(), line, "tolerance")?;
                tolerance = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "tolerance")?,
                    line,
                    "tolerance",
                )?);
                cursor += 2;
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
            unknown => {
                return Err(ControllerError::at(
                    line,
                    format!("unknown heading field {unknown:?}"),
                ));
            }
        }
    }
    Ok((
        required_field(frame, line, "frame")?,
        required_field(heading, line, "heading")?,
        if require_tolerance {
            Some(required_field(tolerance, line, "tolerance")?)
        } else {
            None
        },
        required_field(magnitude, line, "magnitude")?,
    ))
}

fn parse_align(tokens: &[&str], line: usize) -> Result<Layer, ControllerError> {
    if tokens.len() < 7 {
        return Err(ControllerError::at(line, "incomplete align layer"));
    }
    let blend = parse_blend(tokens[1], line)?;
    let (start_frame, duration_frames) = parse_range(tokens, 2, line)?;
    let (frame, heading_radians, tolerance, magnitude) =
        parse_heading_fields(tokens, line, 6, true)?;
    Ok(Layer {
        start_frame,
        duration_frames,
        operation: Operation::Align {
            blend,
            frame,
            heading_radians,
            tolerance_radians: tolerance.expect("required tolerance"),
            magnitude,
        },
    })
}

fn parse_maintain_heading(tokens: &[&str], line: usize) -> Result<Layer, ControllerError> {
    if tokens.len() < 8 {
        return Err(ControllerError::at(
            line,
            "incomplete maintain heading layer",
        ));
    }
    let blend = parse_blend(tokens[2], line)?;
    let (start_frame, duration_frames) = parse_range(tokens, 3, line)?;
    let (frame, heading_radians, _, magnitude) = parse_heading_fields(tokens, line, 7, false)?;
    Ok(Layer {
        start_frame,
        duration_frames,
        operation: Operation::MaintainHeading {
            blend,
            frame,
            heading_radians,
            magnitude,
        },
    })
}

fn parse_maintain_distance(tokens: &[&str], line: usize) -> Result<Layer, ControllerError> {
    if tokens.len() < 8 {
        return Err(ControllerError::at(
            line,
            "incomplete maintain distance layer",
        ));
    }
    let blend = parse_blend(tokens[2], line)?;
    let (start_frame, duration_frames) = parse_range(tokens, 3, line)?;
    let mut frame = None;
    let mut target = None;
    let mut distance = None;
    let mut tolerance = None;
    let mut magnitude = None;
    let mut cursor = 7;
    while cursor < tokens.len() {
        match tokens[cursor] {
            "frame" => {
                reject_duplicate(frame.is_some(), line, "frame")?;
                frame = Some(parse_coordinate_frame(
                    required_token(tokens, cursor + 1, line, "frame")?,
                    line,
                )?);
                cursor += 2;
            }
            "target" => {
                reject_duplicate(target.is_some(), line, "target")?;
                target = Some(parse_vec3(tokens, cursor + 1, line, "target")?);
                cursor += 4;
            }
            "distance" => {
                reject_duplicate(distance.is_some(), line, "distance")?;
                distance = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "distance")?,
                    line,
                    "distance",
                )?);
                cursor += 2;
            }
            "tolerance" => {
                reject_duplicate(tolerance.is_some(), line, "tolerance")?;
                tolerance = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "tolerance")?,
                    line,
                    "tolerance",
                )?);
                cursor += 2;
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
            unknown => {
                return Err(ControllerError::at(
                    line,
                    format!("unknown maintain distance field {unknown:?}"),
                ));
            }
        }
    }
    Ok(Layer {
        start_frame,
        duration_frames,
        operation: Operation::MaintainDistance {
            blend,
            frame: required_field(frame, line, "frame")?,
            target: required_field(target, line, "target")?,
            distance: required_field(distance, line, "distance")?,
            tolerance: required_field(tolerance, line, "tolerance")?,
            magnitude: required_field(magnitude, line, "magnitude")?,
        },
    })
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

fn parse_seek_coordinate(tokens: &[&str], line: usize) -> Result<Layer, ControllerError> {
    let (start_frame, duration_frames, mut cursor) = parse_seek_prefix(tokens, line)?;
    let blend = parse_blend(tokens[2], line)?;
    let mut frame = None;
    let mut target = None;
    let mut offset = None;
    let mut magnitude = None;
    let mut stop_radius = None;
    while cursor < tokens.len() {
        match tokens[cursor] {
            "frame" => {
                reject_duplicate(frame.is_some(), line, "frame")?;
                frame = Some(parse_coordinate_frame(
                    required_token(tokens, cursor + 1, line, "coordinate frame")?,
                    line,
                )?);
                cursor += 2;
            }
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
                    format!("unknown seek coordinate field {unknown:?}"),
                ));
            }
        }
    }
    Ok(Layer {
        start_frame,
        duration_frames,
        operation: Operation::SeekCoordinate {
            blend,
            frame: required_field(frame, line, "frame")?,
            target: required_field(target, line, "target")?,
            offset: required_field(offset, line, "offset")?,
            magnitude: required_field(magnitude, line, "magnitude")?,
            stop_radius: required_field(stop_radius, line, "stop")?,
        },
    })
}

fn parse_seek_plane(tokens: &[&str], line: usize) -> Result<Layer, ControllerError> {
    let (start_frame, duration_frames, mut cursor) = parse_seek_prefix(tokens, line)?;
    let blend = parse_blend(tokens[2], line)?;
    let mut frame = None;
    let mut point = None;
    let mut normal = None;
    let mut magnitude = None;
    let mut stop_radius = None;
    while cursor < tokens.len() {
        match tokens[cursor] {
            "frame" => {
                reject_duplicate(frame.is_some(), line, "frame")?;
                frame = Some(parse_coordinate_frame(
                    required_token(tokens, cursor + 1, line, "coordinate frame")?,
                    line,
                )?);
                cursor += 2;
            }
            "point" => {
                reject_duplicate(point.is_some(), line, "point")?;
                point = Some(parse_vec3(tokens, cursor + 1, line, "plane point")?);
                cursor += 4;
            }
            "normal" => {
                reject_duplicate(normal.is_some(), line, "normal")?;
                normal = Some(parse_vec3(tokens, cursor + 1, line, "plane normal")?);
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
                    format!("unknown seek plane field {unknown:?}"),
                ));
            }
        }
    }
    Ok(Layer {
        start_frame,
        duration_frames,
        operation: Operation::SeekPlane {
            blend,
            frame: required_field(frame, line, "frame")?,
            point: required_field(point, line, "point")?,
            normal: required_field(normal, line, "normal")?,
            magnitude: required_field(magnitude, line, "magnitude")?,
            stop_radius: required_field(stop_radius, line, "stop")?,
        },
    })
}

fn parse_seek_resolved(
    tokens: &[&str],
    line: usize,
    path_point: bool,
) -> Result<Layer, ControllerError> {
    let (start_frame, duration_frames, mut cursor) = parse_seek_prefix(tokens, line)?;
    let blend = parse_blend(tokens[2], line)?;
    let mut stable_id = None;
    let mut point_index = None;
    let mut target = None;
    let mut offset = None;
    let mut magnitude = None;
    let mut stop_radius = None;
    while cursor < tokens.len() {
        match tokens[cursor] {
            "path" if path_point => {
                reject_duplicate(stable_id.is_some(), line, "path")?;
                stable_id = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "path ID")?,
                    line,
                    "path ID",
                )?);
                cursor += 2;
            }
            "point" if path_point => {
                reject_duplicate(point_index.is_some(), line, "point")?;
                point_index = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "path point index")?,
                    line,
                    "path point index",
                )?);
                cursor += 2;
            }
            "opening" if !path_point => {
                reject_duplicate(stable_id.is_some(), line, "opening")?;
                stable_id = Some(parse_number(
                    required_token(tokens, cursor + 1, line, "opening ID")?,
                    line,
                    "opening ID",
                )?);
                cursor += 2;
            }
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
                    format!("unknown resolved target field {unknown:?}"),
                ));
            }
        }
    }
    let stable_id = required_field(stable_id, line, if path_point { "path" } else { "opening" })?;
    let position = required_field(target, line, "target")?;
    let target = if path_point {
        ResolvedTarget::PathPoint {
            path_id: stable_id,
            point_index: required_field(point_index, line, "point")?,
            position,
        }
    } else {
        ResolvedTarget::Opening {
            opening_id: stable_id,
            position,
        }
    };
    Ok(Layer {
        start_frame,
        duration_frames,
        operation: Operation::SeekResolved {
            blend,
            target,
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

fn parse_camera(tokens: &[&str], line: usize) -> Result<Layer, ControllerError> {
    if tokens.len() != 10 || tokens.get(6) != Some(&"x") || tokens.get(8) != Some(&"y") {
        return Err(ControllerError::at(
            line,
            "expected `camera BLEND from N for N x X y Y`",
        ));
    }
    let blend = parse_blend(tokens[1], line)?;
    let (start_frame, duration_frames) = parse_range(tokens, 2, line)?;
    Ok(Layer {
        start_frame,
        duration_frames,
        operation: Operation::Camera {
            blend,
            x: parse_number(tokens[7], line, "camera X")?,
            y: parse_number(tokens[9], line, "camera Y")?,
        },
    })
}

fn parse_safety_clamp(tokens: &[&str], line: usize) -> Result<Layer, ControllerError> {
    if tokens.len() != 9 || tokens.get(5) != Some(&"main") || tokens.get(7) != Some(&"substick") {
        return Err(ControllerError::at(
            line,
            "expected `clamp from N for N main N substick N`",
        ));
    }
    let (start_frame, duration_frames) = parse_range(tokens, 1, line)?;
    Ok(Layer {
        start_frame,
        duration_frames,
        operation: Operation::SafetyClamp {
            main_limit: parse_number(tokens[6], line, "main clamp")?,
            substick_limit: parse_number(tokens[8], line, "substick clamp")?,
        },
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

fn parse_coordinate_frame(token: &str, line: usize) -> Result<CoordinateFrame, ControllerError> {
    match token {
        "world" => Ok(CoordinateFrame::World),
        "player" => Ok(CoordinateFrame::Player),
        "camera" => Ok(CoordinateFrame::Camera),
        unknown => Err(ControllerError::at(
            line,
            format!("unknown coordinate frame {unknown:?}; expected world, player, or camera"),
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
        Operation::SeekCoordinate {
            blend,
            frame,
            target,
            offset,
            stop_radius,
            magnitude,
        } => {
            output[0] = KIND_SEEK_COORDINATE;
            output[1] = encode_blend(*blend);
            output[12] = encode_coordinate_frame(*frame);
            for (index, value) in target.iter().enumerate() {
                put_f32(output, 16 + index * 4, *value);
            }
            for (index, value) in offset.iter().enumerate() {
                put_f32(output, 28 + index * 4, *value);
            }
            put_f32(output, 40, *stop_radius);
            output[44] = *magnitude;
        }
        Operation::SeekPlane {
            blend,
            frame,
            point,
            normal,
            stop_radius,
            magnitude,
        } => {
            output[0] = KIND_SEEK_PLANE;
            output[1] = encode_blend(*blend);
            output[12] = encode_coordinate_frame(*frame);
            for (index, value) in point.iter().enumerate() {
                put_f32(output, 16 + index * 4, *value);
            }
            for (index, value) in normal.iter().enumerate() {
                put_f32(output, 28 + index * 4, *value);
            }
            put_f32(output, 40, *stop_radius);
            output[44] = *magnitude;
        }
        Operation::SeekResolved {
            blend,
            target,
            offset,
            stop_radius,
            magnitude,
        } => {
            output[0] = KIND_SEEK_RESOLVED;
            output[1] = encode_blend(*blend);
            let (kind, stable_id, sub_index, position) = match target {
                ResolvedTarget::PathPoint {
                    path_id,
                    point_index,
                    position,
                } => (0, *path_id, *point_index, position),
                ResolvedTarget::Opening {
                    opening_id,
                    position,
                } => (1, *opening_id, 0, position),
            };
            output[12] = kind;
            put_u64(output, 16, stable_id);
            put_u32(output, 24, sub_index);
            for (index, value) in position.iter().enumerate() {
                put_f32(output, 28 + index * 4, *value);
            }
            for (index, value) in offset.iter().enumerate() {
                put_f32(output, 40 + index * 4, *value);
            }
            put_f32(output, 52, *stop_radius);
            output[56] = *magnitude;
        }
        Operation::Neutral => {
            output[0] = KIND_NEUTRAL;
            output[1] = BLEND_REPLACE;
        }
        Operation::Turn {
            blend,
            direction,
            magnitude,
        } => {
            output[0] = KIND_TURN;
            output[1] = encode_blend(*blend);
            output[12] = match direction {
                TurnDirection::Left => 0,
                TurnDirection::Right => 1,
            };
            output[13] = *magnitude;
        }
        Operation::Brake {
            blend,
            stop_speed,
            magnitude,
        } => {
            output[0] = KIND_BRAKE;
            output[1] = encode_blend(*blend);
            output[12] = *magnitude;
            put_f32(output, 16, *stop_speed);
        }
        Operation::Align {
            blend,
            frame,
            heading_radians,
            tolerance_radians,
            magnitude,
        } => {
            output[0] = KIND_HEADING;
            output[1] = encode_blend(*blend);
            output[12] = 0;
            output[13] = encode_coordinate_frame(*frame);
            output[14] = *magnitude;
            put_f32(output, 16, *heading_radians);
            put_f32(output, 20, *tolerance_radians);
        }
        Operation::MaintainHeading {
            blend,
            frame,
            heading_radians,
            magnitude,
        } => {
            output[0] = KIND_HEADING;
            output[1] = encode_blend(*blend);
            output[12] = 1;
            output[13] = encode_coordinate_frame(*frame);
            output[14] = *magnitude;
            put_f32(output, 16, *heading_radians);
        }
        Operation::MaintainDistance {
            blend,
            frame,
            target,
            distance,
            tolerance,
            magnitude,
        } => {
            output[0] = KIND_MAINTAIN_DISTANCE;
            output[1] = encode_blend(*blend);
            output[12] = encode_coordinate_frame(*frame);
            output[13] = *magnitude;
            for (index, value) in target.iter().enumerate() {
                put_f32(output, 16 + index * 4, *value);
            }
            put_f32(output, 28, *distance);
            put_f32(output, 32, *tolerance);
        }
        Operation::Camera { blend, x, y } => {
            output[0] = KIND_CAMERA;
            output[1] = encode_blend(*blend);
            put_i16(output, 12, *x);
            put_i16(output, 14, *y);
        }
        Operation::SafetyClamp {
            main_limit,
            substick_limit,
        } => {
            output[0] = KIND_SAFETY_CLAMP;
            output[1] = BLEND_REPLACE;
            output[12] = *main_limit;
            output[13] = *substick_limit;
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
        KIND_SEEK_COORDINATE if minor >= 2 => {
            if input[13..16].iter().any(|byte| *byte != 0) {
                return Err(ControllerError::new(format!(
                    "layer {index} has noncanonical coordinate reserved bytes"
                )));
            }
            require_zero(index, input, 45)?;
            Operation::SeekCoordinate {
                blend: decode_stick_blend(index, input[1])?,
                frame: decode_coordinate_frame(index, input[12])?,
                target: [get_f32(input, 16), get_f32(input, 20), get_f32(input, 24)],
                offset: [get_f32(input, 28), get_f32(input, 32), get_f32(input, 36)],
                stop_radius: get_f32(input, 40),
                magnitude: input[44],
            }
        }
        KIND_SEEK_PLANE if minor >= 2 => {
            if input[13..16].iter().any(|byte| *byte != 0) {
                return Err(ControllerError::new(format!(
                    "layer {index} has noncanonical plane reserved bytes"
                )));
            }
            require_zero(index, input, 45)?;
            Operation::SeekPlane {
                blend: decode_stick_blend(index, input[1])?,
                frame: decode_coordinate_frame(index, input[12])?,
                point: [get_f32(input, 16), get_f32(input, 20), get_f32(input, 24)],
                normal: [get_f32(input, 28), get_f32(input, 32), get_f32(input, 36)],
                stop_radius: get_f32(input, 40),
                magnitude: input[44],
            }
        }
        KIND_SEEK_RESOLVED if minor >= 2 => {
            if input[13..16].iter().any(|byte| *byte != 0) {
                return Err(ControllerError::new(format!(
                    "layer {index} has noncanonical resolved-target reserved bytes"
                )));
            }
            require_zero(index, input, 57)?;
            let stable_id = get_u64(input, 16);
            let sub_index = get_u32(input, 24);
            let position = [get_f32(input, 28), get_f32(input, 32), get_f32(input, 36)];
            let target = match input[12] {
                0 => ResolvedTarget::PathPoint {
                    path_id: stable_id,
                    point_index: sub_index,
                    position,
                },
                1 if sub_index == 0 => ResolvedTarget::Opening {
                    opening_id: stable_id,
                    position,
                },
                kind => {
                    return Err(ControllerError::new(format!(
                        "layer {index} has invalid resolved target kind {kind}"
                    )));
                }
            };
            Operation::SeekResolved {
                blend: decode_stick_blend(index, input[1])?,
                target,
                offset: [get_f32(input, 40), get_f32(input, 44), get_f32(input, 48)],
                stop_radius: get_f32(input, 52),
                magnitude: input[56],
            }
        }
        KIND_NEUTRAL if minor >= 3 => {
            if input[1] != BLEND_REPLACE {
                return Err(ControllerError::new(format!(
                    "neutral layer {index} must use replace blend"
                )));
            }
            require_zero(index, input, 12)?;
            Operation::Neutral
        }
        KIND_TURN if minor >= 3 => {
            require_zero(index, input, 14)?;
            let direction = match input[12] {
                0 => TurnDirection::Left,
                1 => TurnDirection::Right,
                value => {
                    return Err(ControllerError::new(format!(
                        "layer {index} has invalid turn direction {value}"
                    )));
                }
            };
            Operation::Turn {
                blend: decode_stick_blend(index, input[1])?,
                direction,
                magnitude: input[13],
            }
        }
        KIND_BRAKE if minor >= 3 => {
            if input[13..16].iter().any(|byte| *byte != 0) {
                return Err(ControllerError::new(format!(
                    "layer {index} has noncanonical brake reserved bytes"
                )));
            }
            require_zero(index, input, 20)?;
            Operation::Brake {
                blend: decode_stick_blend(index, input[1])?,
                stop_speed: get_f32(input, 16),
                magnitude: input[12],
            }
        }
        KIND_HEADING if minor >= 3 => {
            if input[15] != 0 {
                return Err(ControllerError::new(format!(
                    "layer {index} has noncanonical heading reserved byte"
                )));
            }
            require_zero(index, input, 24)?;
            let blend = decode_stick_blend(index, input[1])?;
            let frame = decode_coordinate_frame(index, input[13])?;
            let heading_radians = get_f32(input, 16);
            let magnitude = input[14];
            match input[12] {
                0 => Operation::Align {
                    blend,
                    frame,
                    heading_radians,
                    tolerance_radians: get_f32(input, 20),
                    magnitude,
                },
                1 if get_u32(input, 20) == 0 => Operation::MaintainHeading {
                    blend,
                    frame,
                    heading_radians,
                    magnitude,
                },
                mode => {
                    return Err(ControllerError::new(format!(
                        "layer {index} has invalid heading mode {mode}"
                    )));
                }
            }
        }
        KIND_MAINTAIN_DISTANCE if minor >= 3 => {
            if input[14] != 0 || input[15] != 0 {
                return Err(ControllerError::new(format!(
                    "layer {index} has noncanonical distance reserved bytes"
                )));
            }
            require_zero(index, input, 36)?;
            Operation::MaintainDistance {
                blend: decode_stick_blend(index, input[1])?,
                frame: decode_coordinate_frame(index, input[12])?,
                magnitude: input[13],
                target: [get_f32(input, 16), get_f32(input, 20), get_f32(input, 24)],
                distance: get_f32(input, 28),
                tolerance: get_f32(input, 32),
            }
        }
        KIND_CAMERA if minor >= 4 => {
            require_zero(index, input, 16)?;
            Operation::Camera {
                blend: decode_stick_blend(index, input[1])?,
                x: get_i16(input, 12),
                y: get_i16(input, 14),
            }
        }
        KIND_SAFETY_CLAMP if minor >= 4 => {
            if input[1] != BLEND_REPLACE {
                return Err(ControllerError::new(format!(
                    "safety clamp layer {index} must use replace blend"
                )));
            }
            require_zero(index, input, 14)?;
            Operation::SafetyClamp {
                main_limit: input[12],
                substick_limit: input[13],
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

fn encode_coordinate_frame(frame: CoordinateFrame) -> u8 {
    match frame {
        CoordinateFrame::World => 0,
        CoordinateFrame::Player => 1,
        CoordinateFrame::Camera => 2,
    }
}

fn decode_coordinate_frame(index: usize, value: u8) -> Result<CoordinateFrame, ControllerError> {
    match value {
        0 => Ok(CoordinateFrame::World),
        1 => Ok(CoordinateFrame::Player),
        2 => Ok(CoordinateFrame::Camera),
        _ => Err(ControllerError::new(format!(
            "layer {index} has invalid coordinate frame {value}"
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

fn put_u64(output: &mut [u8], offset: usize, value: u64) {
    output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
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

fn get_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(input[offset..offset + 8].try_into().expect("u64 slice"))
}

fn get_f32(input: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(input[offset..offset + 4].try_into().expect("f32 slice"))
}

#[cfg(test)]
#[path = "controller_program/tests.rs"]
mod tests;
