use super::*;

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
