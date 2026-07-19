use super::*;

impl ControllerProgram {
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

    pub(super) fn encode_for_version(&self, minor: u16) -> Result<Vec<u8>, ControllerError> {
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

pub(super) fn put_i16(output: &mut [u8], offset: usize, value: i16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(output: &mut [u8], offset: usize, value: u64) {
    output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn put_f32(output: &mut [u8], offset: usize, value: f32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn get_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(input[offset..offset + 2].try_into().expect("u16 slice"))
}

pub(super) fn get_i16(input: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes(input[offset..offset + 2].try_into().expect("i16 slice"))
}

pub(super) fn get_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(input[offset..offset + 4].try_into().expect("u32 slice"))
}

fn get_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(input[offset..offset + 8].try_into().expect("u64 slice"))
}

fn get_f32(input: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(input[offset..offset + 4].try_into().expect("f32 slice"))
}
