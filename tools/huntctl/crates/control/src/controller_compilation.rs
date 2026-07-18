//! Static DUSKCTRL flattening and declared observation provenance.

use crate::controller_program::{
    ActorSelector, ControllerProgram, CoordinateFrame, Operation, StickBlend, TurnDirection,
};
use crate::tape::{InputFrame, InputTape, RawPadState};
use serde::Serialize;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const CONTROLLER_PROVENANCE_SCHEMA_V1: &str = "dusklight-controller-observation-provenance/v1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerObservationField {
    PlayerPosition,
    PlayerYaw,
    PlayerVelocity,
    CameraYaw,
    StageName,
    ActorIdentity,
    ActorPosition,
    ActorSnapshotCompleteness,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReactiveLayerProvenance {
    pub layer_index: u16,
    pub start_frame: u32,
    pub duration_frames: u32,
    pub operation: String,
    pub fields: BTreeSet<ControllerObservationField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_selector: Option<ActorSelector>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerObservationProvenance {
    pub schema: String,
    pub reactive_layers: Vec<ReactiveLayerProvenance>,
}

impl ControllerObservationProvenance {
    pub fn for_program(program: &ControllerProgram) -> Self {
        let reactive_layers = program
            .layers
            .iter()
            .enumerate()
            .filter_map(|(index, layer)| {
                provenance_for_operation(&layer.operation).map(|(operation, fields, selector)| {
                    ReactiveLayerProvenance {
                        layer_index: index as u16,
                        start_frame: layer.start_frame,
                        duration_frames: layer.duration_frames,
                        operation: operation.into(),
                        fields,
                        actor_selector: selector,
                    }
                })
            })
            .collect();
        Self {
            schema: CONTROLLER_PROVENANCE_SCHEMA_V1.into(),
            reactive_layers,
        }
    }

    pub fn is_static(&self) -> bool {
        self.reactive_layers.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StaticControllerError {
    InvalidProgram(String),
    Reactive(ControllerObservationProvenance),
}

impl fmt::Display for StaticControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProgram(message) => write!(formatter, "invalid controller: {message}"),
            Self::Reactive(provenance) => write!(
                formatter,
                "controller has {} reactive layers; inspect observation provenance instead of flattening it",
                provenance.reactive_layers.len()
            ),
        }
    }
}

impl Error for StaticControllerError {}

/// Flattens an observation-free controller to an absolute port-zero tape.
pub fn compile_static_controller(
    program: &ControllerProgram,
) -> Result<InputTape, StaticControllerError> {
    program
        .validate()
        .map_err(|error| StaticControllerError::InvalidProgram(error.to_string()))?;
    let provenance = ControllerObservationProvenance::for_program(program);
    if !provenance.is_static() {
        return Err(StaticControllerError::Reactive(provenance));
    }

    let mut frames = Vec::with_capacity(program.duration_frames as usize);
    for frame in 0..program.duration_frames {
        let mut main_replacement = [0_i64; 2];
        let mut main_additions = [0_i64; 2];
        let mut camera_replacement = [0_i64; 2];
        let mut camera_additions = [0_i64; 2];
        let mut buttons = 0_u16;
        let mut clamp = None;
        for layer in &program.layers {
            if frame < layer.start_frame || frame - layer.start_frame >= layer.duration_frames {
                continue;
            }
            let local = frame - layer.start_frame;
            match &layer.operation {
                Operation::CubicBezier { blend, points } => {
                    compose(
                        *blend,
                        bezier_value(points, layer.duration_frames, local),
                        &mut main_replacement,
                        &mut main_additions,
                    );
                }
                Operation::Neutral => main_replacement = [0, 0],
                Operation::Turn {
                    blend,
                    direction,
                    magnitude,
                } => compose(
                    *blend,
                    [
                        match direction {
                            TurnDirection::Left => -i64::from(*magnitude),
                            TurnDirection::Right => i64::from(*magnitude),
                        },
                        0,
                    ],
                    &mut main_replacement,
                    &mut main_additions,
                ),
                Operation::Camera { blend, x, y } => compose(
                    *blend,
                    [i64::from(*x), i64::from(*y)],
                    &mut camera_replacement,
                    &mut camera_additions,
                ),
                Operation::SafetyClamp {
                    main_limit,
                    substick_limit,
                } => clamp = Some((*main_limit, *substick_limit)),
                Operation::Buttons { mask } => buttons |= mask,
                _ => unreachable!("reactive operations were rejected before flattening"),
            }
        }
        let mut main = [
            raw_clamp(main_replacement[0] + main_additions[0]),
            raw_clamp(main_replacement[1] + main_additions[1]),
        ];
        let mut camera = [
            raw_clamp(camera_replacement[0] + camera_additions[0]),
            raw_clamp(camera_replacement[1] + camera_additions[1]),
        ];
        if let Some((main_limit, substick_limit)) = clamp {
            main = main.map(|value| safety_clamp(value, main_limit));
            camera = camera.map(|value| safety_clamp(value, substick_limit));
        }
        let mut input = InputFrame {
            owned_ports: 1,
            ..InputFrame::default()
        };
        input.pads[0] = RawPadState {
            buttons,
            stick_x: main[0],
            stick_y: main[1],
            substick_x: camera[0],
            substick_y: camera[1],
            ..RawPadState::default()
        };
        frames.push(input);
    }
    Ok(InputTape {
        tick_rate_numerator: 30,
        tick_rate_denominator: 1,
        frames,
        ..InputTape::default()
    })
}

fn compose(
    blend: StickBlend,
    value: [i64; 2],
    replacement: &mut [i64; 2],
    additions: &mut [i64; 2],
) {
    match blend {
        StickBlend::Replace => *replacement = value,
        StickBlend::Add => {
            additions[0] += value[0];
            additions[1] += value[1];
        }
    }
}

fn bezier_value(points: &[[i16; 2]; 4], duration: u32, local: u32) -> [i64; 2] {
    if duration == 1 {
        return [i64::from(points[0][0]), i64::from(points[0][1])];
    }
    let root = i128::from(duration - 1);
    let u = i128::from(local);
    let v = root - u;
    let coefficients = [v * v * v, 3 * v * v * u, 3 * v * u * u, u * u * u];
    let denominator = root * root * root;
    [0, 1].map(|axis| {
        let numerator = points
            .iter()
            .zip(coefficients)
            .map(|(point, coefficient)| i128::from(point[axis]) * coefficient)
            .sum();
        round_ratio(numerator, denominator) as i64
    })
}

fn round_ratio(numerator: i128, denominator: i128) -> i128 {
    let magnitude = numerator.abs();
    let quotient = magnitude / denominator;
    let rounded = quotient + i128::from((magnitude % denominator) * 2 >= denominator);
    if numerator < 0 { -rounded } else { rounded }
}

fn raw_clamp(value: i64) -> i8 {
    value.clamp(-128, 127) as i8
}

fn safety_clamp(value: i8, limit: u8) -> i8 {
    i16::from(value).clamp(-i16::from(limit), i16::from(limit)) as i8
}

fn provenance_for_operation(
    operation: &Operation,
) -> Option<(
    &'static str,
    BTreeSet<ControllerObservationField>,
    Option<ActorSelector>,
)> {
    use ControllerObservationField as Field;
    let fields = |values: &[Field]| values.iter().copied().collect();
    let framed = |frame: CoordinateFrame, mut values: BTreeSet<Field>| {
        match frame {
            CoordinateFrame::World => {}
            CoordinateFrame::Player => {
                values.insert(Field::PlayerYaw);
            }
            CoordinateFrame::Camera => {
                values.insert(Field::CameraYaw);
            }
        }
        values
    };
    match operation {
        Operation::SeekPoint { .. } | Operation::SeekResolved { .. } => Some((
            if matches!(operation, Operation::SeekPoint { .. }) {
                "seek_point"
            } else {
                "seek_resolved"
            },
            fields(&[Field::PlayerPosition, Field::CameraYaw]),
            None,
        )),
        Operation::SeekActor { selector, .. } => {
            let mut used = fields(&[
                Field::PlayerPosition,
                Field::CameraYaw,
                Field::ActorIdentity,
                Field::ActorPosition,
                Field::ActorSnapshotCompleteness,
            ]);
            if matches!(selector, ActorSelector::Placed { .. }) {
                used.insert(Field::StageName);
            }
            Some(("seek_actor", used, Some(selector.clone())))
        }
        Operation::SeekCoordinate { frame, .. } => Some((
            "seek_coordinate",
            framed(*frame, fields(&[Field::PlayerPosition, Field::CameraYaw])),
            None,
        )),
        Operation::SeekPlane { frame, .. } => Some((
            "seek_plane",
            framed(*frame, fields(&[Field::PlayerPosition, Field::CameraYaw])),
            None,
        )),
        Operation::Brake { .. } => Some((
            "brake",
            fields(&[Field::PlayerVelocity, Field::CameraYaw]),
            None,
        )),
        Operation::Align { frame, .. } => Some((
            "align",
            framed(*frame, fields(&[Field::PlayerYaw, Field::CameraYaw])),
            None,
        )),
        Operation::MaintainHeading { frame, .. } => Some((
            "maintain_heading",
            framed(*frame, fields(&[Field::CameraYaw])),
            None,
        )),
        Operation::MaintainDistance { frame, .. } => Some((
            "maintain_distance",
            framed(*frame, fields(&[Field::PlayerPosition, Field::CameraYaw])),
            None,
        )),
        Operation::CubicBezier { .. }
        | Operation::Neutral
        | Operation::Turn { .. }
        | Operation::Camera { .. }
        | Operation::SafetyClamp { .. }
        | Operation::Buttons { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller_program::parse;

    #[test]
    fn static_program_flattens_with_native_composition_order() {
        let program = parse(
            r#"duskcontrol 1
frames 2
bezier replace from 0 for 2 p0 100 -100 p1 100 -100 p2 100 -100 p3 100 -100
turn add from 0 for 2 direction right magnitude 50
camera replace from 0 for 2 x 100 y -100
camera add from 0 for 2 x 40 y 40
buttons from 0 for 2 B
clamp from 0 for 2 main 90 substick 80
"#,
        )
        .unwrap();
        let tape = compile_static_controller(&program).unwrap();
        assert_eq!(tape.frames.len(), 2);
        let pad = tape.frames[0].pads[0];
        assert_eq!((pad.stick_x, pad.stick_y), (90, -90));
        assert_eq!((pad.substick_x, pad.substick_y), (80, -60));
        assert_eq!(pad.buttons, 0x0200);
    }

    #[test]
    fn reactive_program_returns_exact_declared_provenance() {
        let program = parse(
            r#"duskcontrol 1
frames 2
seek actor replace from 0 for 2 actor 42 set 7 room 1 stage F_SP103 offset 0 0 0 magnitude 90 stop 2
"#,
        )
        .unwrap();
        let Err(StaticControllerError::Reactive(provenance)) = compile_static_controller(&program)
        else {
            panic!("reactive controller unexpectedly flattened")
        };
        assert_eq!(provenance.schema, CONTROLLER_PROVENANCE_SCHEMA_V1);
        assert_eq!(provenance.reactive_layers.len(), 1);
        let layer = &provenance.reactive_layers[0];
        assert_eq!(layer.operation, "seek_actor");
        assert!(
            layer
                .fields
                .contains(&ControllerObservationField::StageName)
        );
        assert!(
            layer
                .fields
                .contains(&ControllerObservationField::ActorSnapshotCompleteness)
        );
        assert!(matches!(
            layer.actor_selector,
            Some(ActorSelector::Placed { set_id: 7, .. })
        ));
    }
}
