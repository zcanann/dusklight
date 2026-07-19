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

mod codec;
mod parser;
mod validation;

pub use parser::parse;

#[cfg(test)]
use codec::{get_i16, get_u16, get_u32, put_f32, put_u32};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
