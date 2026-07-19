use super::*;

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
