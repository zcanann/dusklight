//! Phase-correct, one-observation-at-a-time DUSKCTRL evaluation.
//!
//! This mirrors the native `InputControllerProgram::evaluateDetailed` contract
//! so orchestration can choose one exact PAD before replaying it through an
//! authenticated persistent checkpoint. It owns input decisions only.

use crate::controller_compilation::ControllerObservationField;
use crate::controller_program::{
    ActorSelector, ControllerProgram, CoordinateFrame, Operation, ResolvedTarget, StickBlend,
    TurnDirection,
};
use crate::tape::{InputFrame, RawPadState};
use serde::Serialize;
use std::error::Error;
use std::f64::consts::{PI, TAU};
use std::fmt;

pub const CONTROLLER_RUNTIME_QUERY_SCHEMA_V1: &str = "dusklight-controller-runtime-query/v1";
pub const MAX_CONTROLLER_RUNTIME_ACTORS: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct ControllerRuntimeActor {
    pub actor_name: i16,
    pub stable_id: u64,
    pub set_id: u16,
    pub home_room: i8,
    pub position: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ControllerRuntimeObservation {
    pub boundary_index: u64,
    pub simulation_tick: u64,
    pub tape_frame: u64,
    pub state_identity: [u8; 16],
    pub player_present: bool,
    pub player_position: [f32; 3],
    pub player_yaw_radians: Option<f32>,
    pub player_velocity_xz: Option<[f32; 2]>,
    pub camera_yaw_radians: Option<f32>,
    pub stage: String,
    pub actors_complete: bool,
    pub actors: Vec<ControllerRuntimeActor>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerRuntimeQueryRecord {
    pub schema: String,
    pub controller_frame: u32,
    pub boundary_index: u64,
    pub simulation_tick: u64,
    pub tape_frame: u64,
    pub state_identity: [u8; 16],
    pub active_layers: Vec<u16>,
    pub queried_fields: Vec<ControllerObservationField>,
    pub selected_actor_stable_ids: Vec<u64>,
    pub target_lost_layer: Option<u16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControllerRuntimeEnd {
    TargetLost { layer_index: u16 },
    MaximumDuration,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ControllerRuntimeStep {
    pub frame: Option<InputFrame>,
    pub query: ControllerRuntimeQueryRecord,
    pub end: Option<ControllerRuntimeEnd>,
}

#[derive(Clone, Debug)]
pub struct ControllerProgramStepper {
    program: ControllerProgram,
    next_frame: u32,
    previous_boundary: Option<(u64, u64, u64)>,
    stopped: bool,
}

impl ControllerProgramStepper {
    pub fn new(program: ControllerProgram) -> Result<Self, ControllerRuntimeError> {
        program
            .validate()
            .map_err(|error| ControllerRuntimeError::InvalidProgram(error.to_string()))?;
        Ok(Self {
            program,
            next_frame: 0,
            previous_boundary: None,
            stopped: false,
        })
    }

    pub fn program(&self) -> &ControllerProgram {
        &self.program
    }

    pub fn step(
        &mut self,
        observation: &ControllerRuntimeObservation,
    ) -> Result<ControllerRuntimeStep, ControllerRuntimeError> {
        if self.stopped || self.next_frame >= self.program.duration_frames {
            return Err(ControllerRuntimeError::Stopped);
        }
        observation.validate()?;
        if let Some((boundary_index, simulation_tick, tape_frame)) = self.previous_boundary
            && (observation.boundary_index != boundary_index + 1
                || observation.simulation_tick != simulation_tick + 1
                || observation.tape_frame != tape_frame + 1)
        {
            return Err(ControllerRuntimeError::DiscontinuousObservation);
        }
        self.previous_boundary = Some((
            observation.boundary_index,
            observation.simulation_tick,
            observation.tape_frame,
        ));
        let controller_frame = self.next_frame;
        let mut query = ControllerRuntimeQueryRecord {
            schema: CONTROLLER_RUNTIME_QUERY_SCHEMA_V1.into(),
            controller_frame,
            boundary_index: observation.boundary_index,
            simulation_tick: observation.simulation_tick,
            tape_frame: observation.tape_frame,
            state_identity: observation.state_identity,
            active_layers: Vec::new(),
            queried_fields: Vec::new(),
            selected_actor_stable_ids: Vec::new(),
            target_lost_layer: None,
        };
        let mut main_replacement = [0_i64; 2];
        let mut main_additions = [0_i64; 2];
        let mut camera_replacement = [0_i64; 2];
        let mut camera_additions = [0_i64; 2];
        let mut buttons = 0_u16;
        let mut clamp = None;

        for (index, layer) in self.program.layers.iter().enumerate() {
            if controller_frame < layer.start_frame
                || controller_frame - layer.start_frame >= layer.duration_frames
            {
                continue;
            }
            let layer_index =
                u16::try_from(index).map_err(|_| ControllerRuntimeError::InvalidLayer)?;
            query.active_layers.push(layer_index);
            let local = controller_frame - layer.start_frame;
            match &layer.operation {
                Operation::Buttons { mask } => buttons |= mask,
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
                operation => {
                    let evaluated =
                        evaluate_stick(operation, local, layer.duration_frames, observation)?;
                    append_unique(&mut query.queried_fields, &evaluated.queried_fields);
                    if let Some(stable_id) = evaluated.selected_actor_stable_id {
                        query.selected_actor_stable_ids.push(stable_id);
                    }
                    if evaluated.target_lost {
                        query.target_lost_layer = Some(layer_index);
                        self.stopped = true;
                        return Ok(ControllerRuntimeStep {
                            frame: None,
                            query,
                            end: Some(ControllerRuntimeEnd::TargetLost { layer_index }),
                        });
                    }
                    let blend = operation_blend(operation);
                    compose(
                        blend,
                        evaluated.stick,
                        &mut main_replacement,
                        &mut main_additions,
                    );
                }
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
        if let Some((main_limit, camera_limit)) = clamp {
            main = main.map(|value| safety_clamp(value, main_limit));
            camera = camera.map(|value| safety_clamp(value, camera_limit));
        }
        let mut frame = InputFrame {
            owned_ports: 1,
            ..InputFrame::default()
        };
        frame.pads[0] = RawPadState {
            buttons,
            stick_x: main[0],
            stick_y: main[1],
            substick_x: camera[0],
            substick_y: camera[1],
            ..RawPadState::default()
        };
        self.next_frame += 1;
        let end = (self.next_frame == self.program.duration_frames)
            .then_some(ControllerRuntimeEnd::MaximumDuration);
        self.stopped = end.is_some();
        Ok(ControllerRuntimeStep {
            frame: Some(frame),
            query,
            end,
        })
    }
}

impl ControllerRuntimeObservation {
    pub fn validate(&self) -> Result<(), ControllerRuntimeError> {
        if self.stage.is_empty()
            || self.stage.len() > 8
            || !self.stage.is_ascii()
            || self.actors.len() > MAX_CONTROLLER_RUNTIME_ACTORS
            || self.player_position.iter().any(|value| !value.is_finite())
            || self
                .player_yaw_radians
                .is_some_and(|value| !value.is_finite())
            || self
                .player_velocity_xz
                .is_some_and(|value| value.iter().any(|component| !component.is_finite()))
            || self
                .camera_yaw_radians
                .is_some_and(|value| !value.is_finite())
            || self
                .actors
                .iter()
                .any(|actor| actor.position.iter().any(|value| !value.is_finite()))
        {
            return Err(ControllerRuntimeError::InvalidObservation);
        }
        Ok(())
    }
}

struct EvaluatedStick {
    stick: [i64; 2],
    queried_fields: Vec<ControllerObservationField>,
    selected_actor_stable_id: Option<u64>,
    target_lost: bool,
}

fn evaluate_stick(
    operation: &Operation,
    local_frame: u32,
    duration: u32,
    observation: &ControllerRuntimeObservation,
) -> Result<EvaluatedStick, ControllerRuntimeError> {
    use ControllerObservationField as Field;
    let mut fields = Vec::new();
    let mut selected_actor_stable_id = None;
    let mut target_lost = false;
    let stick = match operation {
        Operation::CubicBezier { points, .. } => {
            [0, 1].map(|axis| bezier_axis(points, duration, local_frame, axis))
        }
        Operation::SeekPoint {
            target,
            offset,
            stop_radius,
            magnitude,
            ..
        } => {
            fields.extend([Field::PlayerPosition, Field::CameraYaw]);
            seek(
                observation,
                target[0] + offset[0],
                target[2] + offset[2],
                *stop_radius,
                *magnitude,
            )
        }
        Operation::SeekActor {
            actor_name,
            selector,
            offset,
            stop_radius,
            magnitude,
            ..
        } => {
            fields.extend([
                Field::PlayerPosition,
                Field::CameraYaw,
                Field::ActorIdentity,
                Field::ActorPosition,
                Field::ActorSnapshotCompleteness,
            ]);
            if matches!(selector, ActorSelector::Placed { .. }) {
                fields.push(Field::StageName);
            }
            let selected = select_actor(observation, *actor_name, selector);
            if let Some(actor) = selected {
                selected_actor_stable_id = Some(actor.stable_id);
                seek(
                    observation,
                    actor.position[0] + offset[0],
                    actor.position[2] + offset[2],
                    *stop_radius,
                    *magnitude,
                )
            } else {
                target_lost =
                    !matches!(selector, ActorSelector::Nearest) && observation.actors_complete;
                [0, 0]
            }
        }
        Operation::SeekCoordinate {
            frame,
            target,
            offset,
            stop_radius,
            magnitude,
            ..
        } => {
            fields.extend([Field::PlayerPosition, Field::CameraYaw]);
            frame_fields(*frame, &mut fields);
            let target = resolve_point(*frame, observation, *target);
            let offset = resolve_vector(*frame, observation, *offset);
            match (target, offset) {
                (Some(target), Some(offset)) => seek(
                    observation,
                    target[0] + offset[0],
                    target[2] + offset[2],
                    *stop_radius,
                    *magnitude,
                ),
                _ => [0, 0],
            }
        }
        Operation::SeekPlane {
            frame,
            point,
            normal,
            stop_radius,
            magnitude,
            ..
        } => {
            fields.extend([Field::PlayerPosition, Field::CameraYaw]);
            frame_fields(*frame, &mut fields);
            let point = resolve_point(*frame, observation, *point);
            let normal = resolve_vector(*frame, observation, *normal);
            match (point, normal) {
                (Some(point), Some(normal)) => {
                    let normal_squared =
                        f64::from(normal[0]).powi(2) + f64::from(normal[2]).powi(2);
                    if normal_squared > 0.0 && normal_squared.is_finite() {
                        let scale = ((f64::from(observation.player_position[0])
                            - f64::from(point[0]))
                            * f64::from(normal[0])
                            + (f64::from(observation.player_position[2]) - f64::from(point[2]))
                                * f64::from(normal[2]))
                            / normal_squared;
                        seek(
                            observation,
                            (f64::from(observation.player_position[0])
                                - scale * f64::from(normal[0])) as f32,
                            (f64::from(observation.player_position[2])
                                - scale * f64::from(normal[2])) as f32,
                            *stop_radius,
                            *magnitude,
                        )
                    } else {
                        [0, 0]
                    }
                }
                _ => [0, 0],
            }
        }
        Operation::SeekResolved {
            target,
            offset,
            stop_radius,
            magnitude,
            ..
        } => {
            fields.extend([Field::PlayerPosition, Field::CameraYaw]);
            let position = match target {
                ResolvedTarget::PathPoint { position, .. }
                | ResolvedTarget::Opening { position, .. } => *position,
            };
            seek(
                observation,
                position[0] + offset[0],
                position[2] + offset[2],
                *stop_radius,
                *magnitude,
            )
        }
        Operation::Neutral => [0, 0],
        Operation::Turn {
            direction,
            magnitude,
            ..
        } => [
            match direction {
                TurnDirection::Left => -i64::from(*magnitude),
                TurnDirection::Right => i64::from(*magnitude),
            },
            0,
        ],
        Operation::Brake {
            stop_speed,
            magnitude,
            ..
        } => {
            fields.extend([Field::PlayerVelocity, Field::CameraYaw]);
            match observation.player_velocity_xz {
                Some([x, z]) if f64::from(x).hypot(f64::from(z)) > f64::from(*stop_speed) => {
                    world_heading_stick(observation, f64::from(-x).atan2(f64::from(-z)), *magnitude)
                }
                _ => [0, 0],
            }
        }
        Operation::Align {
            frame,
            heading_radians,
            tolerance_radians,
            magnitude,
            ..
        } => {
            fields.extend([Field::PlayerYaw, Field::CameraYaw]);
            let heading = resolve_heading(*frame, observation, *heading_radians);
            match (heading, observation.player_yaw_radians) {
                (Some(heading), Some(player))
                    if wrap_angle(heading - f64::from(player)).abs()
                        > f64::from(*tolerance_radians) =>
                {
                    world_heading_stick(observation, heading, *magnitude)
                }
                _ => [0, 0],
            }
        }
        Operation::MaintainHeading {
            frame,
            heading_radians,
            magnitude,
            ..
        } => {
            fields.push(Field::CameraYaw);
            frame_fields(*frame, &mut fields);
            resolve_heading(*frame, observation, *heading_radians)
                .map(|heading| world_heading_stick(observation, heading, *magnitude))
                .unwrap_or([0, 0])
        }
        Operation::MaintainDistance {
            frame,
            target,
            distance,
            tolerance,
            magnitude,
            ..
        } => {
            fields.extend([Field::PlayerPosition, Field::CameraYaw]);
            frame_fields(*frame, &mut fields);
            match resolve_point(*frame, observation, *target) {
                Some(target) if observation.player_present => {
                    let dx = f64::from(target[0]) - f64::from(observation.player_position[0]);
                    let dz = f64::from(target[2]) - f64::from(observation.player_position[2]);
                    let actual = dx.hypot(dz);
                    if actual > f64::from(*distance + *tolerance) {
                        seek(observation, target[0], target[2], 0.0, *magnitude)
                    } else if actual < f64::from(*distance - *tolerance) && actual > 0.0 {
                        world_heading_stick(observation, (-dx).atan2(-dz), *magnitude)
                    } else {
                        [0, 0]
                    }
                }
                _ => [0, 0],
            }
        }
        Operation::Camera { .. } | Operation::SafetyClamp { .. } | Operation::Buttons { .. } => {
            return Err(ControllerRuntimeError::InvalidLayer);
        }
    };
    Ok(EvaluatedStick {
        stick,
        queried_fields: fields,
        selected_actor_stable_id,
        target_lost,
    })
}

fn operation_blend(operation: &Operation) -> StickBlend {
    match operation {
        Operation::CubicBezier { blend, .. }
        | Operation::SeekPoint { blend, .. }
        | Operation::SeekActor { blend, .. }
        | Operation::SeekCoordinate { blend, .. }
        | Operation::SeekPlane { blend, .. }
        | Operation::SeekResolved { blend, .. }
        | Operation::Turn { blend, .. }
        | Operation::Brake { blend, .. }
        | Operation::Align { blend, .. }
        | Operation::MaintainHeading { blend, .. }
        | Operation::MaintainDistance { blend, .. } => *blend,
        Operation::Neutral => StickBlend::Replace,
        Operation::Camera { .. } | Operation::SafetyClamp { .. } | Operation::Buttons { .. } => {
            unreachable!()
        }
    }
}

fn compose(blend: StickBlend, value: [i64; 2], replacement: &mut [i64; 2], add: &mut [i64; 2]) {
    match blend {
        StickBlend::Replace => *replacement = value,
        StickBlend::Add => {
            add[0] += value[0];
            add[1] += value[1];
        }
    }
}

fn select_actor<'a>(
    observation: &'a ControllerRuntimeObservation,
    actor_name: i16,
    selector: &ActorSelector,
) -> Option<&'a ControllerRuntimeActor> {
    let matching = observation
        .actors
        .iter()
        .filter(|actor| actor.actor_name == actor_name);
    match selector {
        ActorSelector::Process { process_id } => matching
            .filter(|actor| actor.stable_id == u64::from(*process_id))
            .min_by_key(|actor| actor.stable_id),
        ActorSelector::Placed {
            set_id,
            room,
            stage_name,
        } if observation.stage == *stage_name => matching
            .filter(|actor| actor.set_id == *set_id && actor.home_room == *room)
            .min_by_key(|actor| actor.stable_id),
        ActorSelector::Placed { .. } => None,
        ActorSelector::Nearest => matching.min_by(|left, right| {
            distance_squared(left, observation)
                .total_cmp(&distance_squared(right, observation))
                .then(left.stable_id.cmp(&right.stable_id))
        }),
    }
}

fn distance_squared(
    actor: &ControllerRuntimeActor,
    observation: &ControllerRuntimeObservation,
) -> f64 {
    let dx = f64::from(actor.position[0]) - f64::from(observation.player_position[0]);
    let dz = f64::from(actor.position[2]) - f64::from(observation.player_position[2]);
    dx * dx + dz * dz
}

fn seek(
    observation: &ControllerRuntimeObservation,
    target_x: f32,
    target_z: f32,
    stop_radius: f32,
    magnitude: u8,
) -> [i64; 2] {
    let Some(camera) = observation.camera_yaw_radians else {
        return [0, 0];
    };
    if !observation.player_present {
        return [0, 0];
    }
    let dx = f64::from(target_x) - f64::from(observation.player_position[0]);
    let dz = f64::from(target_z) - f64::from(observation.player_position[2]);
    if dx.hypot(dz) <= f64::from(stop_radius) {
        return [0, 0];
    }
    let relative = dx.atan2(dz) - f64::from(camera);
    [
        (-relative.sin() * f64::from(magnitude)).round() as i64,
        (relative.cos() * f64::from(magnitude)).round() as i64,
    ]
}

fn world_heading_stick(
    observation: &ControllerRuntimeObservation,
    heading: f64,
    magnitude: u8,
) -> [i64; 2] {
    observation
        .camera_yaw_radians
        .map(|camera| {
            let relative = heading - f64::from(camera);
            [
                (-relative.sin() * f64::from(magnitude)).round() as i64,
                (relative.cos() * f64::from(magnitude)).round() as i64,
            ]
        })
        .unwrap_or([0, 0])
}

fn frame_yaw(frame: CoordinateFrame, observation: &ControllerRuntimeObservation) -> Option<f64> {
    match frame {
        CoordinateFrame::World => Some(0.0),
        CoordinateFrame::Player => observation.player_yaw_radians.map(f64::from),
        CoordinateFrame::Camera => observation.camera_yaw_radians.map(f64::from),
    }
}

fn resolve_heading(
    frame: CoordinateFrame,
    observation: &ControllerRuntimeObservation,
    heading: f32,
) -> Option<f64> {
    frame_yaw(frame, observation).map(|yaw| yaw + f64::from(heading))
}

fn resolve_point(
    frame: CoordinateFrame,
    observation: &ControllerRuntimeObservation,
    point: [f32; 3],
) -> Option<[f32; 3]> {
    if !observation.player_present {
        return None;
    }
    if frame == CoordinateFrame::World {
        return Some(point);
    }
    let yaw = frame_yaw(frame, observation)?;
    let (sine, cosine) = yaw.sin_cos();
    Some([
        (f64::from(observation.player_position[0])
            + cosine * f64::from(point[0])
            + sine * f64::from(point[2])) as f32,
        observation.player_position[1] + point[1],
        (f64::from(observation.player_position[2]) - sine * f64::from(point[0])
            + cosine * f64::from(point[2])) as f32,
    ])
}

fn resolve_vector(
    frame: CoordinateFrame,
    observation: &ControllerRuntimeObservation,
    vector: [f32; 3],
) -> Option<[f32; 3]> {
    let yaw = frame_yaw(frame, observation)?;
    let (sine, cosine) = yaw.sin_cos();
    Some([
        (cosine * f64::from(vector[0]) + sine * f64::from(vector[2])) as f32,
        vector[1],
        (-sine * f64::from(vector[0]) + cosine * f64::from(vector[2])) as f32,
    ])
}

fn frame_fields(frame: CoordinateFrame, fields: &mut Vec<ControllerObservationField>) {
    use ControllerObservationField as Field;
    match frame {
        CoordinateFrame::World => {}
        CoordinateFrame::Player => fields.push(Field::PlayerYaw),
        CoordinateFrame::Camera => fields.push(Field::CameraYaw),
    }
}

fn append_unique(
    target: &mut Vec<ControllerObservationField>,
    source: &[ControllerObservationField],
) {
    for field in source {
        if !target.contains(field) {
            target.push(*field);
        }
    }
}

fn bezier_axis(points: &[[i16; 2]; 4], duration: u32, local: u32, axis: usize) -> i64 {
    if duration == 1 {
        return i64::from(points[0][axis]);
    }
    let root = i128::from(duration - 1);
    let u = i128::from(local);
    let v = root - u;
    let coefficients = [v * v * v, 3 * v * v * u, 3 * v * u * u, u * u * u];
    let denominator = root * root * root;
    let numerator = points
        .iter()
        .zip(coefficients)
        .map(|(point, coefficient)| i128::from(point[axis]) * coefficient)
        .sum::<i128>();
    let magnitude = numerator.abs();
    let rounded =
        magnitude / denominator + i128::from((magnitude % denominator) * 2 >= denominator);
    i64::try_from(if numerator < 0 { -rounded } else { rounded }).unwrap()
}

fn wrap_angle(mut angle: f64) -> f64 {
    angle = (angle + PI) % TAU;
    if angle < 0.0 {
        angle += TAU;
    }
    angle - PI
}

fn raw_clamp(value: i64) -> i8 {
    value.clamp(-128, 127) as i8
}

fn safety_clamp(value: i8, limit: u8) -> i8 {
    i16::from(value).clamp(-i16::from(limit), i16::from(limit)) as i8
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControllerRuntimeError {
    InvalidProgram(String),
    InvalidObservation,
    InvalidLayer,
    DiscontinuousObservation,
    Stopped,
}

impl fmt::Display for ControllerRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProgram(message) => {
                write!(formatter, "invalid DUSKCTRL program: {message}")
            }
            Self::InvalidObservation => formatter.write_str("invalid DUSKCTRL observation"),
            Self::InvalidLayer => formatter.write_str("invalid DUSKCTRL runtime layer"),
            Self::DiscontinuousObservation => {
                formatter.write_str("DUSKCTRL observations are not contiguous")
            }
            Self::Stopped => formatter.write_str("DUSKCTRL stepper already stopped"),
        }
    }
}

impl Error for ControllerRuntimeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller_compilation::compile_static_controller;

    fn observation() -> ControllerRuntimeObservation {
        ControllerRuntimeObservation {
            boundary_index: 1,
            simulation_tick: 10,
            tape_frame: 20,
            state_identity: [3; 16],
            player_present: true,
            player_position: [0.0; 3],
            player_yaw_radians: Some(0.0),
            player_velocity_xz: Some([0.0, 1.0]),
            camera_yaw_radians: Some(0.0),
            stage: "F_SP103".into(),
            actors_complete: true,
            actors: Vec::new(),
        }
    }

    #[test]
    fn static_stepper_matches_the_existing_exact_compiler() {
        let program = ControllerProgram::parse(
            "duskcontrol 1\nframes 2\nbezier replace from 0 for 2 p0 30 -40 p1 30 -40 p2 30 -40 p3 30 -40\nbuttons from 0 for 2 A\n",
        )
        .unwrap();
        let exact = compile_static_controller(&program).unwrap();
        let mut stepper = ControllerProgramStepper::new(program).unwrap();
        let mut observed = observation();
        for expected in exact.frames {
            let step = stepper.step(&observed).unwrap();
            assert_eq!(step.frame.unwrap(), expected);
            observed.boundary_index += 1;
            observed.simulation_tick += 1;
            observed.tape_frame += 1;
        }
    }

    #[test]
    fn reactive_seek_and_exact_target_loss_are_explicit() {
        let program = ControllerProgram::parse(
            "duskcontrol 1\nframes 2\nseek actor replace from 0 for 2 actor 42 set 7 room 1 stage F_SP103 offset 0 0 0 magnitude 90 stop 1\n",
        )
        .unwrap();
        let mut stepper = ControllerProgramStepper::new(program).unwrap();
        let mut observed = observation();
        observed.actors.push(ControllerRuntimeActor {
            actor_name: 42,
            stable_id: 9,
            set_id: 7,
            home_room: 1,
            position: [10.0, 0.0, 0.0],
        });
        let first = stepper.step(&observed).unwrap();
        let first_pad = first.frame.unwrap().pads[0];
        assert_eq!((first_pad.stick_x, first_pad.stick_y), (-90, 0));
        assert_eq!(first.query.selected_actor_stable_ids, vec![9]);

        observed.boundary_index += 1;
        observed.simulation_tick += 1;
        observed.tape_frame += 1;
        observed.actors.clear();
        let lost = stepper.step(&observed).unwrap();
        assert!(lost.frame.is_none());
        assert_eq!(
            lost.end,
            Some(ControllerRuntimeEnd::TargetLost { layer_index: 0 })
        );
        assert_eq!(lost.query.target_lost_layer, Some(0));
    }
}
